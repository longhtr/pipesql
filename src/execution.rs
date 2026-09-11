//! Public result lifetime and the shared producer/consumer step protocol.

mod admission;
mod aggregation;
mod blocking;
mod computed;
mod limit;
mod planning;
mod predicate;
mod runtime;
mod scan;

#[cfg(test)]
use crate::Database;
use crate::batch::Batch;
#[cfg(test)]
use crate::batch::OwnedBatch;
use crate::effects::Effects;
#[cfg(test)]
use crate::frontend::{DataType, MAX_AGGREGATE_COLUMNS};
use crate::resources::{Reservation, allocate};
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
    crate::catalog::MAX_UNITS as u64 * crate::native_unit::MAX_ROWS as u64;

// The inline result owner includes runtime metadata and physical mappings.
// Charge its target-specific storage; heap owners retain separate reservations.
const RESULT_BYTES: u64 = size_of::<QueryResult<'_, '_>>() as u64;

/// A borrowed batch of complete result rows.
/// The next mutable step is forbidden while this view remains borrowed.
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

    /// Borrow batch storage without allocation; text cannot survive its reuse.
    /// NULL is a value; an invalid row or column returns None.
    pub fn value(&self, row: usize, column: usize) -> Option<Value<'step>> {
        self.batch.value(row, column)
    }
}

/// One bounded execution step. Rows are a prefix until `Finished` is observed.
pub enum QueryStep<'step> {
    /// Borrow complete rows until the caller releases the batch.
    Rows(ResultBatch<'step>),
    /// Work advanced without publishing a batch.
    Progress,
    /// Execution completed successfully and released its retained buffers.
    Finished,
    /// Execution stopped and released its buffers; earlier rows are incomplete.
    Failed(&'step Error),
}

enum State<'db> {
    Running(Runtime<'db>),
    Finished,
    Failed(Error),
}

/// A running query that borrows its prepared plan and cancellation token.
/// Dropping it releases execution owners; it does not cancel other queries.
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

    /// Consume the result and move out its terminal error, if it failed.
    pub fn into_error(self) -> Option<Error> {
        match self.state {
            State::Failed(error) => Some(error),
            _ => None,
        }
    }

    /// Current execution charges, excluding the database and prepared query.
    pub fn accounted_memory_bytes(&self) -> u64 {
        self.reservation.bytes()
            + self.plan.memory_bytes()
            + match &self.state {
                State::Running(workspace) => workspace.memory_bytes(),
                _ => 0,
            }
    }

    /// Handle and per-step scratch allowance, excluding the plan and buffers.
    pub const fn memory_requirement_bytes() -> u64 {
        RESULT_BYTES + computed::ROW_SCRATCH_BYTES
    }

    /// Advance execution. Returned text and rows borrow this result until reuse.
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

// One source or consumer work quantum. Input stays borrowed and charged until
// the consumer requests the next batch or replay. No consumer owns a scan cursor.
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
