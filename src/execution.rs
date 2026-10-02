//! Execute a prepared query and return its rows a batch at a time.
//!
//! `Database::execute` constructs a `QueryResult` from a prepared query. Each
//! call to `QueryResult::step` does a bounded amount of work and reports rows,
//! progress without rows, completion or failure. A batch borrows reusable buffers,
//! so the caller must finish reading it before stepping again. Only `Finished`
//! confirms success: a query can fail after returning some of its rows.
//!
//! The result owns the operators and buffers needed to run the query. Completion,
//! failure and drop release them; the prepared query remains separately owned.
//! A failed result keeps its error for the caller, and later steps report the
//! same failure without restarting execution.
//!
//! Follow `admission` for validation and resource reservation, `planning` for the
//! mapping from logical columns to batch positions, and `runtime` for scheduling
//! operators. Operators ask that scheduler for more input or a replay of earlier
//! input. They do not recursively run their upstream operators themselves.

mod admission;
mod aggregation;
mod blocking;
mod computed;
mod count;
mod limit;
mod planning;
mod predicate;
mod runtime;
mod scan;
mod union;

#[cfg(test)]
use crate::Database;
use crate::batch::Batch;
#[cfg(test)]
use crate::batch::OwnedBatch;
use crate::effects::Effects;
#[cfg(test)]
use crate::query::MAX_AGGREGATE_COLUMNS;
use crate::resources::{Reservation, allocate};
#[cfg(test)]
use crate::value::DataType;
#[cfg(test)]
use crate::value::StringValue;
use crate::value::Value;
use crate::{CancellationToken, Error};
#[cfg(test)]
use aggregation::Aggregation;
use planning::{PhysicalPlan, Pipeline};
use runtime::Runtime;

#[cfg(test)]
use scan::AdmittedScan;
use std::mem::size_of;

const COMPUTE_ROWS: usize = 4096;
const BATCH_ROWS: usize = crate::batch::ROWS;
const MAX_AGGREGATE_ROWS: u64 =
    crate::storage::catalog::MAX_UNITS as u64 * crate::storage::unit::MAX_ROWS as u64;

// Declared tables can contain general UTF-8 strings. Legacy input has only
// one-byte keys, but a literal can introduce a longer string. Keep enough room
// for those strings even after a later operator groups or copies them.
fn output_text_capacity(query: &crate::PreparedQuery<'_>) -> Option<usize> {
    if query.snapshot.is_some() {
        Some(crate::batch::MAX_TEXT_BYTES)
    } else if query.plan.computed.iter().any(|definition| {
        matches!(
            definition.expression,
            crate::query::Computation::Constant(crate::query::Constant::String(_))
        )
    }) {
        Some(BATCH_ROWS * crate::query::text_literal::MAX_LITERAL_BYTES)
    } else {
        None
    }
}

// Charge the result handle itself, whose size can vary by target. Its heap
// buffers have separate reservations and are not included in `size_of`.
const RESULT_BYTES: u64 = size_of::<QueryResult<'_, '_>>() as u64;

/// A view of complete rows in the query's reusable buffers.
/// Rust prevents stepping the query while this view or its borrowed text is in use.
/// Batches contain at most 256 rows; use [`Self::len`] rather than assuming a full
/// batch. Copy values that must outlive the batch, especially borrowed STRING text.
pub struct ResultBatch<'step> {
    batch: &'step Batch,
}

impl<'step> ResultBatch<'step> {
    /// Number of complete rows in this batch.
    pub fn len(&self) -> usize {
        self.batch.len()
    }

    /// Whether this batch contains no rows.
    pub fn is_empty(&self) -> bool {
        self.batch.is_empty()
    }

    /// Number of columns, in the prepared query's output order.
    pub fn column_count(&self) -> usize {
        self.batch.column_count()
    }

    /// Read a cell without allocating. Text borrows the batch; other values copy.
    /// SQL NULL is `Some(Value::Null)`; an out-of-range row or column is `None`.
    pub fn value(&self, row: usize, column: usize) -> Option<Value<'step>> {
        self.batch.value(row, column)
    }
}

/// The outcome of one execution step. Keep stepping until `Finished` or `Failed`;
/// receiving rows alone does not establish that the query succeeded.
pub enum QueryStep<'step> {
    /// Borrow complete rows until the caller releases the batch.
    Rows(ResultBatch<'step>),
    /// Work advanced without rows. Step again; this does not mean end of input.
    Progress,
    /// Execution completed successfully and released its retained buffers.
    Finished,
    /// Execution stopped and released its buffers. Earlier rows do not form a
    /// successfully completed result. The error remains owned by the query.
    Failed(&'step Error),
}

enum State<'db> {
    Running(Runtime<'db>),
    Finished,
    Failed(Error),
}

/// Execution state borrowing a prepared query and cancellation token.
/// Dropping it frees its buffers and scratch files, even before completion.
/// The prepared query can be executed again to start a separate run.
///
/// Successful construction does not establish successful execution: payload I/O,
/// arithmetic and cancellation can fail after earlier rows were returned. Accept
/// a complete answer only after [`QueryStep::Finished`]. Dropping an unfinished
/// result abandons that execution; it does not finish it.
///
/// Terminal steps release runtime storage but retain the result handle and its
/// borrowed plan. Dropping the prepared query later releases its snapshot pin.
/// A cancelled token stays cancelled; use a fresh token when reusing the plan.
pub struct QueryResult<'db, 'cancel> {
    plan: PhysicalPlan<'db>,
    state: State<'db>,
    cancellation: &'cancel CancellationToken,
    reservation: Reservation<'db>,
}

impl QueryResult<'_, '_> {
    #[cfg(test)]
    fn first_aggregate(&self) -> Option<&Aggregation<'_>> {
        match &self.state {
            State::Running(runtime) => runtime.aggregates.first(),
            _ => None,
        }
    }

    /// Consume the result and return its error if a step failed.
    /// This does not finish a running query: it drops that query and returns `None`.
    /// The returned error can outlive the result, prepared query and original SQL.
    /// Source spans retain byte offsets, not the source text; keep that text if
    /// a diagnostic needs to show the expression.
    pub fn into_error(self) -> Option<Error> {
        match self.state {
            State::Failed(error) => Some(error),
            _ => None,
        }
    }

    /// Bytes currently reserved for this execution, including its result handle.
    /// Excludes reservations held by the database and prepared query.
    pub fn accounted_memory_bytes(&self) -> u64 {
        self.reservation.bytes()
            + self.plan.memory_bytes()
            + match &self.state {
                State::Running(workspace) => workspace.memory_bytes(),
                _ => 0,
            }
    }

    /// Bytes for the result handle and per-step scratch alone.
    /// Excludes the physical plan and operator buffers; this is not the amount
    /// needed to execute an arbitrary query.
    pub const fn memory_requirement_bytes() -> u64 {
        RESULT_BYTES + computed::ROW_SCRATCH_BYTES
    }

    /// Advance execution and lend any resulting rows until the next step.
    /// After completion or failure, repeated calls return the same terminal outcome.
    pub fn step(&mut self) -> QueryStep<'_> {
        self.step_with_effects(&mut Effects::default())
    }

    fn step_with_effects(&mut self, effects: &mut Effects) -> QueryStep<'_> {
        if let State::Running(workspace) = &mut self.state {
            let advanced = workspace.step(&self.plan, self.cancellation, effects);
            match advanced {
                Ok(Advance::Finished) => {
                    self.state = State::Finished;
                    self.plan.clear();
                    self.reservation.shrink_to(RESULT_BYTES);
                }
                Err(error) => {
                    // Free runtime buffers and scratch before returning their
                    // reserved bytes to the budget. Keep the error in the result.
                    self.state = State::Failed(error);
                    self.plan.clear();
                    self.reservation.shrink_to(RESULT_BYTES);
                }
                Ok(Advance::Progress | Advance::Rows) => (),
            }
        }
        match &self.state {
            State::Running(workspace) => {
                let batch = workspace.output();
                if batch.is_empty() {
                    QueryStep::Progress
                } else {
                    QueryStep::Rows(ResultBatch { batch })
                }
            }
            State::Finished => QueryStep::Finished,
            State::Failed(error) => QueryStep::Failed(error),
        }
    }
}

enum Advance {
    Progress,
    Rows,
    Finished,
}

enum SetStep {
    Input(usize),
    Replay(usize),
    Progress,
    Rows,
    Finished,
}

enum ConsumerStep {
    Input,
    Replay,
    Progress,
    Rows,
    Finished,
}

struct ConsumerInput<'a> {
    batch: &'a Batch,
    finished: bool,
}

#[cfg(test)]
enum InputRequest {
    Ready { finished: bool },
    Next,
    Replay,
    Done,
    Failed,
}

#[cfg(test)]
struct InputDriver {
    request: InputRequest,
}

#[cfg(test)]
impl InputDriver {
    fn new() -> Self {
        Self {
            request: InputRequest::Ready { finished: false },
        }
    }
}

// Drive a scan and grouping consumer directly in tests. Retain the input batch
// until the consumer asks for the next batch or a replay, just as the runtime
// does. A failed step leaves the driver failed so it cannot retry partial work.
#[cfg(test)]
fn drive_consumer(
    consumer: &mut aggregation::grouping::General<'_>,
    workspace: &mut AdmittedScan<'_>,
    driver: &mut InputDriver,
    maps: (&Pipeline, &Pipeline),
    cancel: &CancellationToken,
    effects: &mut Effects,
) -> Result<Advance, Error> {
    workspace.output.clear();
    let finished = match std::mem::replace(&mut driver.request, InputRequest::Failed) {
        InputRequest::Done => {
            driver.request = InputRequest::Done;
            return Ok(Advance::Finished);
        }
        InputRequest::Failed => return Err(Error::Corrupt("consumer driver has failed")),
        InputRequest::Next => {
            match workspace.advance(maps.0, cancel, effects)? {
                Advance::Progress => driver.request = InputRequest::Next,
                Advance::Rows => driver.request = InputRequest::Ready { finished: false },
                Advance::Finished => driver.request = InputRequest::Ready { finished: true },
            }
            return Ok(Advance::Progress);
        }
        InputRequest::Replay => {
            workspace.restart(cancel)?;
            driver.request = InputRequest::Ready { finished: false };
            return Ok(Advance::Progress);
        }
        InputRequest::Ready { finished } => finished,
    };
    let input = ConsumerInput {
        batch: &workspace.input,
        finished,
    };
    let advanced = consumer.step(input, &mut workspace.output, maps.1, cancel, effects)?;
    Ok(match advanced {
        ConsumerStep::Input => {
            if finished {
                return Err(Error::Corrupt("consumer requested input after completion"));
            }
            workspace.input.clear();
            driver.request = InputRequest::Next;
            Advance::Progress
        }
        ConsumerStep::Replay => {
            workspace.input.clear();
            driver.request = InputRequest::Replay;
            Advance::Progress
        }
        ConsumerStep::Progress => {
            driver.request = InputRequest::Ready { finished };
            Advance::Progress
        }
        ConsumerStep::Rows => {
            driver.request = InputRequest::Ready { finished };
            Advance::Rows
        }
        ConsumerStep::Finished => {
            driver.request = InputRequest::Done;
            Advance::Finished
        }
    })
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod test_support;
