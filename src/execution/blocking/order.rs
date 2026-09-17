//! Sort input for ORDER BY, duplicate removal and analytic counts or running sums.
//!
//! These operations share one collection and emission loop. ORDER BY compares
//! the requested columns. DISTINCT compares every column and emits one row per
//! equal group. COUNT(*) OVER () stores rows without comparison keys; input row
//! numbers preserve their sequence and supply the final count. PARTITION BY
//! sorts equal keys together, counts each group, then reads those rows again to
//! attach the count. A group larger than memory stays in the sorted file.
//! Running SUM uses the same visits for each group of equal order keys. It keeps
//! an exact cumulative total within the partition, so peers receive one total.
//!
//! `SortedInput` owns the row buffers, external sorter and temporary files. This
//! controller feeds it one record at a time, finishes the sort, then reads back
//! rows. Downstream expressions run during emission, after duplicate removal.
//! They can still fail after earlier rows have been returned.
//!
//! Replay reads the retained rows again once, without rerunning the source.
//! A failed step prevents further work; the enclosing query releases the buffers
//! and temporary files. A completed controller remains complete if cancelled.

use super::{CursorPosition, RowLayout, SortPhase, SortedInput, append_bytes};
use crate::batch::Batch;
use crate::effects::Effects;
use crate::execution::computed::{AnalyticValue, RowValues};
use crate::execution::planning::{self, Pipeline};
use crate::execution::{ConsumerInput, ConsumerStep};
use crate::query::{Direction, MAX_ROW_VALUES, NullPlacement, SemanticColumn};
use crate::resources::{Reservation, allocate};
use crate::value::Value;
use crate::{CancellationToken, Database, Error};
use std::cmp::Ordering;
use std::mem::size_of;

#[derive(Clone, Copy)]
enum Phase {
    Create,
    Read,
    Await,
    Capture(usize),
    Push(usize),
    Spill(usize),
    Sort,
    CountLoad,
    Count,
    Load,
    Emit,
    Done,
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Order,
    Distinct,
    WindowCount,
    PartitionCount,
    RunningSum {
        partition_keys: usize,
        argument: usize,
    },
}

// A group is a complete partition for COUNT or one peer group for running SUM.
// Save its first file position, accumulate its rows, then visit them again to
// emit the result. The sorter's prior-key buffer checks membership after rewind.
// No state grows with group size.
#[derive(Clone, Copy, Default)]
struct ReplayGroup {
    start: Option<CursorPosition>,
    rows: u64,
    remaining: u64,
}

// The admitted row bound makes every exact INT64 prefix sum fit in i128.
const _: () =
    assert!((crate::execution::MAX_AGGREGATE_ROWS as u128) * (1_u128 << 63) <= i128::MAX as u128);

pub(in crate::execution) struct Order<'db> {
    input: SortedInput<'db>,
    phase: Phase,
    replayed: bool,
    mode: Mode,
    group: ReplayGroup,
    running_sum: Option<i128>,
    reservation: Reservation<'db>,
}

impl<'db> Order<'db> {
    pub(in crate::execution) fn distinct(
        database: &'db Database,
        inputs: impl Iterator<Item = SemanticColumn> + Clone,
        runtime: &mut Reservation<'db>,
    ) -> Result<Vec<Self>, Error> {
        let count = inputs.clone().count();
        if count == 0 || count > MAX_ROW_VALUES {
            return Err(Error::Corrupt("DISTINCT input width"));
        }
        let keys = std::array::from_fn::<_, MAX_ROW_VALUES, _>(|column| planning::OrderColumn {
            column: column as u8,
            direction: Direction::Ascending,
            nulls: NullPlacement::First,
        });
        let mut owner = Self::new(database, inputs, &keys[..count], runtime)?;
        owner[0].mode = Mode::Distinct;
        Ok(owner)
    }

    pub(in crate::execution) fn new(
        database: &'db Database,
        inputs: impl Iterator<Item = SemanticColumn>,
        keys: &[planning::OrderColumn],
        runtime: &mut Reservation<'db>,
    ) -> Result<Vec<Self>, Error> {
        Self::with_layout(database, RowLayout::for_order(inputs, keys)?, runtime)
    }

    pub(in crate::execution) fn window(
        database: &'db Database,
        inputs: impl Iterator<Item = SemanticColumn>,
        partition: &[Option<u8>; crate::query::MAX_PARTITION_KEYS],
        function: &planning::WindowFunction,
        runtime: &mut Reservation<'db>,
    ) -> Result<Vec<Self>, Error> {
        let mut keys = [planning::OrderColumn {
            column: 0,
            direction: Direction::Ascending,
            nulls: NullPlacement::First,
        };
            crate::query::MAX_PARTITION_KEYS + crate::query::MAX_WINDOW_ORDER_KEYS];
        let mut count = 0;
        for column in partition.iter().flatten() {
            if !keys[..count].iter().any(|key| key.column == *column) {
                keys[count].column = *column;
                count += 1;
            }
        }
        let partition_keys = count;
        if let planning::WindowFunction::RunningSum { order, .. } = function {
            for key in order.iter().flatten() {
                if !keys[..count].iter().any(|prior| prior.column == key.column) {
                    keys[count] = *key;
                    count += 1;
                }
            }
        }
        let layout = if count == 0 {
            RowLayout::for_partition(inputs)?
        } else {
            RowLayout::for_order(inputs, &keys[..count])?
        };
        let mode = match function {
            planning::WindowFunction::Count if count == 0 => Mode::WindowCount,
            planning::WindowFunction::Count => Mode::PartitionCount,
            planning::WindowFunction::RunningSum { argument, .. } => {
                let argument = layout.columns[..layout.count]
                    .iter()
                    .position(|column| column.input == usize::from(*argument))
                    .ok_or(Error::Corrupt("running SUM input absent from retained row"))?;
                Mode::RunningSum {
                    partition_keys,
                    argument,
                }
            }
        };
        let mut owner = Self::with_layout(database, layout, runtime)?;
        owner[0].mode = mode;
        Ok(owner)
    }

    fn grouped(&self) -> bool {
        matches!(self.mode, Mode::PartitionCount | Mode::RunningSum { .. })
    }

    fn with_layout(
        database: &'db Database,
        layout: RowLayout,
        runtime: &mut Reservation<'db>,
    ) -> Result<Vec<Self>, Error> {
        let reservation = database.reserve_memory(
            (size_of::<Self>() - size_of::<SortedInput<'_>>()) as u64,
            "order controller",
        )?;
        let input = SortedInput::new(database, layout)?;
        let mut owner = allocate(
            1,
            1,
            "order owner",
            reservation.bytes() + input.memory_bytes(),
        )?;
        owner.push(Self {
            input,
            phase: Phase::Create,
            replayed: false,
            mode: Mode::Order,
            group: ReplayGroup::default(),
            running_sum: None,
            reservation,
        });
        // Dropping Order's fields does not free its containing Vec allocation.
        // Keep that allocation's charge in the runtime until the Vec is freed.
        let order = &mut owner[0];
        order.reservation.transfer_to(
            runtime,
            (size_of::<Self>() - size_of::<SortedInput<'_>>()) as u64,
        )?;
        order.input.transfer_inline_to(runtime)?;
        Ok(owner)
    }

    pub(in crate::execution) fn memory_bytes(&self) -> u64 {
        self.reservation.bytes() + self.input.memory_bytes()
    }

    #[cfg(test)]
    pub(in crate::execution) fn was_replayed(&self) -> bool {
        self.replayed
    }

    fn begin_read(&mut self) {
        self.group = ReplayGroup::default();
        self.running_sum = None;
        self.input.begin_read();
        let rows = self.input.sort.sorted_rows();
        rows.previous_key.clear();
        *rows.previous_ordinal = None;
    }

    fn read_phase(&self) -> Phase {
        if self.grouped() {
            Phase::CountLoad
        } else {
            Phase::Load
        }
    }

    fn emit_group(&mut self) -> Result<Phase, Error> {
        if self.group.rows == 0 {
            return Err(Error::Corrupt("empty counted group"));
        }
        let start = self
            .group
            .start
            .ok_or(Error::Corrupt("analytic group has no start"))?;
        let rows = self.input.sort.sorted_rows();
        rows.cursor.rewind(start)?;
        // The key must survive rewind so emission can verify its membership.
        // The first replayed ordinal precedes the count pass's last ordinal.
        *rows.previous_ordinal = None;
        self.group.remaining = self.group.rows;
        Ok(Phase::Load)
    }

    // Equal keys are valid; equal or reversed row numbers within a key are not.
    // Both counting and emission check this, including rows a filter will discard.
    fn checked_key_order(&mut self) -> Result<Option<Ordering>, Error> {
        let rows = self.input.sort.sorted_rows();
        let record = rows
            .cursor
            .record()
            .ok_or(Error::Corrupt("order requires a loaded row"))?;
        let comparison = rows
            .previous_ordinal
            .map(|previous| {
                self.input
                    .layout
                    .compare(rows.previous_key, record.key())
                    .map(|keys| (keys, previous.cmp(&record.ordinal())))
            })
            .transpose()?;
        if comparison.is_some_and(|(keys, ordinal)| keys.then(ordinal) != Ordering::Less) {
            return Err(Error::Corrupt("order output is not monotonic"));
        }
        Ok(comparison.map(|(keys, _)| keys))
    }

    // Reset both the file cursor and comparison history, even after only part
    // of the output was consumed. Emission evaluates downstream expressions again.
    pub(in crate::execution) fn replay(&mut self, cancel: &CancellationToken) -> Result<(), Error> {
        cancel.check()?;
        if self.replayed
            || !matches!(
                self.phase,
                Phase::CountLoad | Phase::Count | Phase::Load | Phase::Emit | Phase::Done
            )
        {
            return Err(Error::Corrupt("order replay requires a completed sort"));
        }
        self.begin_read();
        self.replayed = true;
        self.phase = self.read_phase();
        Ok(())
    }

    pub(in crate::execution) fn step(
        &mut self,
        input: ConsumerInput<'_>,
        output: &mut Batch,
        plan: &Pipeline,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<ConsumerStep, Error> {
        output.clear();
        // Any early error leaves Failed installed. Restore a live phase only
        // after this step has completed its work.
        let phase = std::mem::replace(&mut self.phase, Phase::Failed);
        if matches!(phase, Phase::Done) {
            self.phase = Phase::Done;
            return Ok(ConsumerStep::Finished);
        }
        if matches!(phase, Phase::Failed) {
            return Err(Error::Corrupt("order has failed"));
        }
        cancel.check()?;
        let mut step = ConsumerStep::Progress;
        self.phase = match phase {
            Phase::Create => {
                self.input.files.create(cancel, effects)?;
                Phase::Read
            }
            Phase::Read => {
                step = ConsumerStep::Input;
                Phase::Await
            }
            Phase::Await => {
                if input.finished {
                    if !input.batch.is_empty() {
                        return Err(Error::Corrupt("finished order input contains rows"));
                    }
                    self.input.sort.finish()?;
                    Phase::Sort
                } else {
                    if input.batch.is_empty() {
                        return Err(Error::Corrupt("order input not supplied"));
                    }
                    Phase::Capture(0)
                }
            }
            Phase::Capture(row) => {
                if row == input.batch.len() {
                    Phase::Read
                } else {
                    self.input.record.encode_row(
                        &self.input.layout,
                        input.batch,
                        row,
                        self.input.ordinal,
                        None,
                    )?;
                    Phase::Push(row)
                }
            }
            // A full run has not accepted this record. Keep it while spilling,
            // then retry Push without recapturing or advancing the input row.
            Phase::Push(row) => {
                if self.input.sort.push(&self.input.record)? {
                    self.input.ordinal = self
                        .input
                        .ordinal
                        .checked_add(1)
                        .ok_or(Error::Corrupt("order input ordinal overflow"))?;
                    Phase::Capture(row + 1)
                } else {
                    Phase::Spill(row)
                }
            }
            Phase::Spill(row) => {
                self.input.sort_step(cancel, effects)?;
                if self.input.sort.phase() == SortPhase::Collect {
                    Phase::Push(row)
                } else {
                    phase
                }
            }
            Phase::Sort => {
                if self.input.sort_step(cancel, effects)? {
                    self.begin_read();
                    self.read_phase()
                } else {
                    phase
                }
            }
            Phase::CountLoad => {
                if self.input.load(cancel, effects)? {
                    phase
                } else if self.input.finished() {
                    if self.group.rows == 0 {
                        Phase::Done
                    } else {
                        self.emit_group()?
                    }
                } else {
                    Phase::Count
                }
            }
            Phase::Count => {
                let comparison = self.checked_key_order()?;
                if self.group.rows != 0 && comparison == Some(Ordering::Less) {
                    // This loaded row belongs to the next group. Leave it
                    // unconsumed; emission will reach its position again.
                    self.emit_group()?
                } else {
                    let rows = self.input.sort.sorted_rows();
                    if self.group.rows == 0 {
                        if comparison == Some(Ordering::Equal) {
                            return Err(Error::Corrupt(
                                "analytic group ended before its key changed",
                            ));
                        }
                        self.group.start = rows.cursor.position();
                    }
                    let record = rows
                        .cursor
                        .record()
                        .ok_or(Error::Corrupt("analytic group row"))?;
                    if let Mode::RunningSum {
                        partition_keys,
                        argument,
                    } = self.mode
                    {
                        if self.group.rows == 0
                            && rows.previous_ordinal.is_some()
                            && self.input.layout.compare_prefix(
                                rows.previous_key,
                                record.key(),
                                partition_keys,
                            )? != Ordering::Equal
                        {
                            self.running_sum = None;
                        }
                        match self.input.layout.value(record.key(), argument)? {
                            Value::Null => (),
                            Value::Int64(value) => {
                                self.running_sum = Some(
                                    self.running_sum
                                        .unwrap_or(0)
                                        .checked_add(i128::from(value))
                                        .ok_or(Error::Corrupt(
                                            "bounded running SUM exceeds i128",
                                        ))?,
                                );
                            }
                            _ => return Err(Error::Corrupt("running SUM input type")),
                        }
                    }
                    rows.previous_key.clear();
                    append_bytes(
                        rows.previous_key,
                        self.input.layout.sort_prefix(record.key())?,
                    )?;
                    *rows.previous_ordinal = Some(record.ordinal());
                    self.group.rows = self
                        .group
                        .rows
                        .checked_add(1)
                        .filter(|count| *count <= self.input.ordinal)
                        .ok_or(Error::Corrupt("analytic group count exceeds input rows"))?;
                    rows.cursor.consume()?;
                    Phase::CountLoad
                }
            }
            Phase::Load => {
                if self.input.load(cancel, effects)? {
                    phase
                } else if self.input.finished() {
                    if self.grouped() && self.group.remaining != 0 {
                        return Err(Error::Corrupt("analytic replay ended before its count"));
                    }
                    Phase::Done
                } else {
                    Phase::Emit
                }
            }
            Phase::Emit => {
                let comparison = self.checked_key_order()?;
                let grouped = self.grouped();
                let rows = self.input.sort.sorted_rows();
                let record = rows.cursor.record().expect("order emits a loaded row");
                if grouped
                    && (self.group.remaining == 0
                        || self.input.layout.compare(rows.previous_key, record.key())?
                            != Ordering::Equal)
                {
                    return Err(Error::Corrupt("analytic replay differs from counted key"));
                }
                let duplicate = self.mode == Mode::Distinct && comparison == Some(Ordering::Equal);
                // Remember this key even if a later filter rejects the row.
                // Otherwise another copy could survive DISTINCT as a new row.
                rows.previous_key.clear();
                append_bytes(
                    rows.previous_key,
                    self.input.layout.sort_prefix(record.key())?,
                )?;
                *rows.previous_ordinal = Some(record.ordinal());
                let values = self.input.values()?;
                let value = |column: u8| -> Result<Value<'_>, Error> {
                    values[..self.input.layout.count]
                        .get(usize::from(column))
                        .copied()
                        .ok_or(Error::Corrupt("order output position"))
                };
                let mut evaluated = RowValues::new(plan, value);
                if self.mode == Mode::WindowCount {
                    evaluated = evaluated.with_analytic(AnalyticValue::Count(self.input.ordinal));
                } else if self.mode == Mode::PartitionCount {
                    evaluated = evaluated.with_analytic(AnalyticValue::Count(self.group.rows));
                } else if matches!(self.mode, Mode::RunningSum { .. }) {
                    evaluated = evaluated.with_analytic(AnalyticValue::Sum(self.running_sum));
                }
                // Removing a duplicate must also skip errors in expressions
                // and filters that occur after DISTINCT in the pipeline.
                let retained = !duplicate && evaluated.retains()?;
                if retained {
                    for (position, column) in plan.columns[..plan.column_count].iter().enumerate() {
                        output.set(0, position, evaluated.value(*column)?)?;
                    }
                    output.publish_rows(1);
                    step = ConsumerStep::Rows;
                }
                self.input.consume()?;
                if self.grouped() {
                    self.group.remaining -= 1;
                    if self.group.remaining == 0 {
                        self.group = ReplayGroup::default();
                        Phase::CountLoad
                    } else {
                        Phase::Load
                    }
                } else {
                    Phase::Load
                }
            }
            Phase::Done | Phase::Failed => unreachable!("terminal phases handled before effects"),
        };
        Ok(step)
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
#[path = "order_tests.rs"]
mod tests;
