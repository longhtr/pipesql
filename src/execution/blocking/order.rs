//! Bounded standalone ordering over the shared checked sorted-input owner.
use super::{RowLayout, SortPhase, SortedInput, append_bytes};
use crate::batch::Batch;
use crate::effects::Effects;
use crate::execution::computed::RowValues;
use crate::execution::planning::{self, Pipeline};
use crate::execution::{ConsumerInput, ConsumerStep};
use crate::frontend::{Direction, MAX_COLUMNS, NullPlacement, SemanticColumn};
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
    Load,
    Emit,
    Done,
    Failed,
}

pub(in crate::execution) struct Order<'db> {
    input: SortedInput<'db>,
    phase: Phase,
    replayed: bool,
    distinct: bool,
    reservation: Reservation<'db>,
}

impl<'db> Order<'db> {
    pub(in crate::execution) fn distinct(
        database: &'db Database,
        inputs: impl Iterator<Item = SemanticColumn> + Clone,
    ) -> Result<Vec<Self>, Error> {
        let count = inputs.clone().count();
        if count == 0 || count > MAX_COLUMNS {
            return Err(Error::Corrupt("DISTINCT input width"));
        }
        let keys = std::array::from_fn::<_, MAX_COLUMNS, _>(|column| planning::OrderColumn {
            column: column as u8,
            direction: Direction::Ascending,
            nulls: NullPlacement::First,
        });
        let mut owner = Self::new(database, inputs, &keys[..count])?;
        owner[0].distinct = true;
        Ok(owner)
    }

    pub(in crate::execution) fn new(
        database: &'db Database,
        inputs: impl Iterator<Item = SemanticColumn>,
        keys: &[planning::OrderColumn],
    ) -> Result<Vec<Self>, Error> {
        let layout = RowLayout::for_order(inputs, keys)?;
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
            distinct: false,
            reservation,
        });
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
        self.input.begin_read();
        let rows = self.input.sort.sorted_rows();
        rows.previous_key.clear();
        *rows.previous_ordinal = None;
    }

    pub(in crate::execution) fn replay(&mut self, cancel: &CancellationToken) -> Result<(), Error> {
        cancel.check()?;
        if self.replayed || !matches!(self.phase, Phase::Load | Phase::Emit | Phase::Done) {
            return Err(Error::Corrupt("order replay requires a completed sort"));
        }
        self.begin_read();
        self.replayed = true;
        self.phase = Phase::Load;
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
                    )?;
                    Phase::Push(row)
                }
            }
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
                    Phase::Load
                } else {
                    phase
                }
            }
            Phase::Load => {
                if self.input.load(cancel, effects)? {
                    phase
                } else if self.input.finished() {
                    Phase::Done
                } else {
                    Phase::Emit
                }
            }
            Phase::Emit => {
                let rows = self.input.sort.sorted_rows();
                let record = rows.cursor.record().expect("order emits a loaded row");
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
                let duplicate =
                    self.distinct && comparison.is_some_and(|(keys, _)| keys == Ordering::Equal);
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
                // Duplicate rows cannot demand expressions or filters after DISTINCT.
                let retained = !duplicate && evaluated.retains()?;
                if retained {
                    for (position, column) in plan.columns[..plan.column_count].iter().enumerate() {
                        output.set(0, position, evaluated.value(*column)?)?;
                    }
                    output.publish_rows(1);
                    step = ConsumerStep::Rows;
                }
                self.input.consume()?;
                Phase::Load
            }
            Phase::Done | Phase::Failed => unreachable!("terminal phases handled before effects"),
        };
        Ok(step)
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;
