//! Aggregate controller selection and bounded dense-key grouping.
//!
//! Numeric state owns evaluation and accumulation. Controllers own input,
//! replay, final validation, and result emission; general grouping also owns
//! hash storage and external-sort fallback.

mod arguments;
pub(super) mod grouping;
mod numeric;
use crate::batch::Batch;
use crate::execution::computed::RowValues;
use crate::execution::planning::Pipeline;
use crate::execution::{BATCH_ROWS, COMPUTE_ROWS, ConsumerInput, ConsumerStep};
use crate::fixed_text::{KEY_DOMAIN, StringValue as FixedKey};
use crate::frontend::{AggregatePlan, MAX_COLUMNS, SemanticColumn, SourceColumn};
use crate::{CancellationToken, Database, Error, StringValue, Value};
use numeric::{AggregateLayout, AggregateState};

#[cfg(test)]
use std::mem::size_of;

// Controllers occupy one admitted vector indexed by semantic aggregate identity.
// General state keeps its independently charged fallback owner indirect.
#[expect(
    clippy::large_enum_variant,
    reason = "dense state occupies the admitted controller vector"
)]
pub(super) enum Aggregation<'db> {
    Dense(Groups<'db>),
    General(Vec<grouping::General<'db>>),
}

impl<'db> Aggregation<'db> {
    pub(super) fn minimum_bytes(
        native: bool,
        aggregate: &'db AggregatePlan,
        demand: u16,
        input: impl Iterator<Item = SemanticColumn>,
        output: impl Iterator<Item = SemanticColumn>,
    ) -> Result<u64, Error> {
        if native && aggregate.group_count != 0 {
            grouping::General::minimum_bytes(aggregate, demand, input, output)
        } else {
            let capacity = KEY_DOMAIN
                .checked_pow(u32::from(aggregate.group_count))
                .ok_or(Error::Corrupt("group domain overflow"))?;
            AggregateLayout::new(aggregate, demand, input).required_bytes(capacity, 1)
        }
    }

    pub(super) fn open(
        database: &'db Database,
        native: bool,
        aggregate: &'db AggregatePlan,
        demand: u16,
        input: impl Iterator<Item = SemanticColumn> + Clone,
        output: impl Iterator<Item = SemanticColumn> + Clone,
    ) -> Result<Self, Error> {
        if native && aggregate.group_count != 0 {
            Ok(Self::General(grouping::General::open(
                database, aggregate, demand, input, output,
            )?))
        } else {
            let groups = Groups::new(database, aggregate, demand, input.clone())?;
            groups.validate_plan(aggregate, demand, input)?;
            Ok(Self::Dense(groups))
        }
    }

    pub(super) fn replay(&mut self, cancel: &CancellationToken) -> Result<(), Error> {
        match self {
            Self::Dense(groups) => groups.replay(cancel),
            Self::General(groups) => groups[0].replay(cancel),
        }
    }

    pub(super) fn memory_bytes(&self) -> u64 {
        match self {
            Self::Dense(groups) => groups.aggregate.reservation.bytes(),
            Self::General(owner) => owner[0].memory_bytes(),
        }
    }

    #[cfg(test)]
    pub(super) fn dense(&self) -> &Groups<'_> {
        let Self::Dense(groups) = self else {
            panic!("dense aggregate test boundary");
        };
        groups
    }
}

#[derive(Clone, Copy)]
enum AggregatePhase {
    Read,
    Consume,
    Check(usize),
    Emit(usize),
    Done,
    Failed,
}

pub(super) struct Groups<'db> {
    phase: AggregatePhase,
    replayed: bool,
    aggregate: AggregateState<'db>,
    group_count: u8,
    keys: [usize; 2],
}

impl<'db> Groups<'db> {
    #[cfg(test)]
    pub(super) fn expression_lanes(&self) -> usize {
        self.aggregate.lanes
    }

    #[cfg(test)]
    pub(super) fn extra_expression_bytes(&self) -> u64 {
        let lane_bytes = self.aggregate.scratch.len() / self.aggregate.lanes * size_of::<u64>();
        ((self.aggregate.lanes - 1) * lane_bytes) as u64
    }

    fn new(
        database: &'db Database,
        plan: &'db AggregatePlan,
        demand: u16,
        columns: impl Iterator<Item = SemanticColumn>,
    ) -> Result<Self, Error> {
        let mut inputs = [SourceColumn::QUANTITY.semantic(); MAX_COLUMNS];
        let mut count = 0;
        for column in columns {
            inputs[count] = column;
            count += 1;
        }
        let mut keys = [0; 2];
        for (index, column) in plan.group_columns().enumerate() {
            keys[index] = inputs[..count]
                .iter()
                .position(|input| *input == column)
                .ok_or(Error::Corrupt("aggregate input is not scanned"))?;
        }
        let capacity = KEY_DOMAIN
            .checked_pow(u32::from(plan.group_count))
            .ok_or(Error::Corrupt("group domain overflow"))?;
        Ok(Self {
            phase: AggregatePhase::Read,
            replayed: false,
            aggregate: AggregateState::new(
                &database.memory,
                plan,
                demand,
                capacity,
                inputs[..count].iter().copied(),
            )?,
            group_count: plan.group_count,
            keys,
        })
    }

    fn validate_plan(
        &self,
        semantic: &AggregatePlan,
        demand: u16,
        columns: impl Iterator<Item = SemanticColumn> + Clone,
    ) -> Result<(), Error> {
        if self.group_count != semantic.group_count {
            return Err(Error::Corrupt("aggregate physical shape disagrees"));
        }
        let capacity = KEY_DOMAIN
            .checked_pow(u32::from(self.group_count))
            .ok_or(Error::Corrupt("aggregate domain"))?;
        for (index, column) in semantic.group_columns().enumerate() {
            if columns.clone().nth(self.keys[index]) != Some(column) {
                return Err(Error::Corrupt("physical group key disagrees"));
            }
        }
        self.aggregate
            .validate_plan(semantic, demand, capacity, columns)
    }

    fn consume(&mut self, batch: &Batch) -> Result<(), Error> {
        assert!(batch.len() <= BATCH_ROWS);
        if batch.is_empty() {
            return Ok(());
        }
        let mut keys: [&[FixedKey]; 2] = [&[]; 2];
        for (slot, input) in self.keys[..usize::from(self.group_count)]
            .iter()
            .enumerate()
        {
            keys[slot] = batch.strings(*input).map_err(Error::Corrupt)?;
        }
        // Reuse one bounded position array for lookup and per-row fold counts.
        let mut positions = [(0_usize, 0_u32); BATCH_ROWS];
        for (index, (group, row_count)) in positions[..batch.len()].iter_mut().enumerate() {
            for input in &keys[..usize::from(self.group_count)] {
                *group = group
                    .checked_mul(KEY_DOMAIN)
                    .and_then(|group| group.checked_add(input[index].index()))
                    .ok_or(Error::Corrupt("aggregate key index overflow"))?;
            }
            *row_count = self.aggregate.cells.count_row(*group)?;
        }
        self.aggregate.consume(batch, &positions[..batch.len()])
    }

    fn replay(&mut self, cancel: &CancellationToken) -> Result<(), Error> {
        let phase = std::mem::replace(&mut self.phase, AggregatePhase::Failed);
        cancel.check()?;
        if self.replayed || !matches!(phase, AggregatePhase::Emit(_) | AggregatePhase::Done) {
            return Err(Error::Corrupt(
                "aggregate replay requires completed accumulation",
            ));
        }
        self.replayed = true;
        self.phase = AggregatePhase::Emit(0);
        Ok(())
    }

    pub(super) fn step(
        &mut self,
        input: ConsumerInput<'_>,
        output: &mut Batch,
        plan: &Pipeline,
        cancellation: &CancellationToken,
    ) -> Result<ConsumerStep, Error> {
        output.clear();
        cancellation.check()?;
        match self.phase {
            AggregatePhase::Read => {
                self.phase = AggregatePhase::Consume;
                return Ok(ConsumerStep::Input);
            }
            AggregatePhase::Consume => {
                if input.finished {
                    if !input.batch.is_empty() {
                        return Err(Error::Corrupt("finished aggregate input contains rows"));
                    }
                    self.phase = AggregatePhase::Check(0);
                } else {
                    if input.batch.is_empty() {
                        return Err(Error::Corrupt("aggregate input was not supplied"));
                    }
                    self.consume(input.batch)?;
                    self.phase = AggregatePhase::Read;
                }
                return Ok(ConsumerStep::Progress);
            }
            _ => (),
        }
        match self.phase {
            AggregatePhase::Check(start) => {
                let end = start
                    .checked_add(if !plan.has_computed_work() {
                        COMPUTE_ROWS
                    } else {
                        BATCH_ROWS
                    })
                    .ok_or(Error::Corrupt("group check cursor overflow"))?
                    .min(self.aggregate.cells.counts.len());
                self.validate_results(start, end, plan)?;
                self.phase = if end == self.aggregate.cells.counts.len() {
                    AggregatePhase::Emit(0)
                } else {
                    AggregatePhase::Check(end)
                };
                Ok(ConsumerStep::Progress)
            }
            AggregatePhase::Emit(start) => {
                let end = start
                    .checked_add(if !plan.has_computed_work() {
                        COMPUTE_ROWS
                    } else {
                        BATCH_ROWS
                    })
                    .ok_or(Error::Corrupt("group output cursor overflow"))?
                    .min(self.aggregate.cells.counts.len());
                let mut next = start;
                while next < end && output.len() < BATCH_ROWS {
                    if (self.aggregate.cells.counts[next] != 0 || self.group_count == 0)
                        && self.retains(next, plan)?
                    {
                        let row = output.len();
                        let count = plan.column_count;
                        let mut values =
                            RowValues::new(plan, |column| self.value(next, usize::from(column)));
                        for (position, column) in plan.columns[..count].iter().enumerate() {
                            output.set(row, position, values.value(*column)?)?;
                        }
                        output.publish_rows(row + 1);
                    }
                    next += 1;
                }
                self.phase = if next == self.aggregate.cells.counts.len() {
                    AggregatePhase::Done
                } else {
                    AggregatePhase::Emit(next)
                };
                Ok(if output.is_empty() {
                    ConsumerStep::Progress
                } else {
                    ConsumerStep::Rows
                })
            }
            AggregatePhase::Done => Ok(ConsumerStep::Finished),
            AggregatePhase::Failed => Err(Error::Corrupt("aggregate has failed")),
            AggregatePhase::Read | AggregatePhase::Consume => {
                unreachable!("input handled before finalization")
            }
        }
    }

    fn retains(&self, group: usize, plan: &Pipeline) -> Result<bool, Error> {
        RowValues::new(plan, |column| self.value(group, usize::from(column))).retains()
    }

    fn validate_results(&self, start: usize, end: usize, plan: &Pipeline) -> Result<(), Error> {
        for group in start..end {
            if (self.aggregate.cells.counts[group] != 0 || self.group_count == 0)
                && self.retains(group, plan)?
            {
                let mut values =
                    RowValues::new(plan, |column| self.value(group, usize::from(column)));
                for column in &plan.columns[..plan.column_count] {
                    values.value(*column)?;
                }
            }
        }
        Ok(())
    }

    fn value(&self, group: usize, column: usize) -> Result<Value<'_>, Error> {
        let keys = usize::from(self.group_count);
        if column < keys {
            let divisor = KEY_DOMAIN.pow(u32::try_from(keys - column - 1).expect("bounded keys"));
            let key = FixedKey::from_index(group / divisor % KEY_DOMAIN)
                .expect("group index is inside the validated key domain");
            return Ok(Value::String(StringValue::new(key.as_str())));
        }
        self.aggregate.value(group, column - keys)
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;

#[cfg(test)]
mod numerical_tests;
