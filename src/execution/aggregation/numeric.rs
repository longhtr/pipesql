//! Admitted numeric state shared by dense and general grouping.
//!
//! Expression sharing, typed cells, NULL counts, and overflow flags have one
//! owner. Construction borrows only the memory authority; evaluation has no
//! database, filesystem, or scheduler access. Independent validation checks the
//! admitted representation against the semantic plan before consumption.

use crate::Error;
use crate::batch::Batch;
use crate::execution::{BATCH_ROWS, MAX_AGGREGATE_ROWS};
use crate::frontend::{
    AggregateKind, AggregatePlan, DataType, MAX_AGGREGATE_COLUMNS, MAX_COLUMNS, SemanticColumn,
};
use crate::resources::{MemoryAuthority, Reservation, allocate};
use crate::scalar::{ArithmeticFailure, Expression, Op};
use crate::storage_format;
use crate::value::Value;
use std::mem::size_of;

pub(super) struct AggregateState<'db> {
    pub(super) plan: &'db AggregatePlan,
    pub(super) demand: u16,
    pub(super) inputs: [Option<&'db Expression>; MAX_AGGREGATE_COLUMNS],
    pub(super) scratch: Vec<u64>,
    pub(super) lanes: usize,
    pub(super) input_columns: [Option<SemanticColumn>; MAX_COLUMNS],
    pub(super) states: usize,
    // Demanded SUM/AVG entries with identical expressions share one state.
    // COUNT and undemanded entries use usize::MAX and never index numeric cells.
    pub(super) entry_states: [usize; MAX_AGGREGATE_COLUMNS],
    pub(super) cells: AggregateCells,
    // All physical allocations are destroyed before their reservation.
    pub(super) reservation: Reservation<'db>,
}

// Immutable admission facts shared by dense and general grouping. Resolving
// expression sharing here does not allocate or evaluate input.
pub(super) struct AggregateLayout<'db> {
    plan: &'db AggregatePlan,
    demand: u16,
    pub(super) inputs: [Option<&'db Expression>; MAX_AGGREGATE_COLUMNS],
    numeric_columns: [Option<SemanticColumn>; MAX_COLUMNS],
    pub(super) states: usize,
    entry_states: [usize; MAX_AGGREGATE_COLUMNS],
    value_slots: [u8; MAX_AGGREGATE_COLUMNS],
    count_slots: [u8; MAX_AGGREGATE_COLUMNS],
    sum_states: u32,
    doubles: usize,
    integers: usize,
    nullable: usize,
}

impl<'db> AggregateLayout<'db> {
    pub(super) fn new(
        plan: &'db AggregatePlan,
        demand: u16,
        columns: impl Iterator<Item = SemanticColumn>,
    ) -> Self {
        let mut inputs: [Option<&Expression>; MAX_AGGREGATE_COLUMNS] =
            [None; MAX_AGGREGATE_COLUMNS];
        let mut numeric_columns = [None; MAX_COLUMNS];
        for (index, column) in columns.enumerate() {
            if matches!(column.data_type(), DataType::Int64 | DataType::Double) {
                numeric_columns[index] = Some(column);
            }
        }
        let mut states = 0;
        let mut sum_states = 0_u32;
        let mut entry_states = [usize::MAX; MAX_AGGREGATE_COLUMNS];
        for (index, entry) in plan.entries.iter().enumerate() {
            if demand & (1 << index) == 0 {
                continue;
            }
            if let Some(expression) = &entry.expression {
                let state = if let Some(state) = inputs[..states]
                    .iter()
                    .position(|prior| *prior == Some(expression))
                {
                    state
                } else {
                    inputs[states] = Some(expression);
                    states += 1;
                    states - 1
                };
                entry_states[index] = state;
                if entry.kind == AggregateKind::Sum {
                    sum_states |= 1_u32 << state;
                }
            }
        }
        // Compact typed arrays avoid charging nonnullable DOUBLE workloads for
        // integer state or per-expression NULL counters.
        let mut value_slots = [u8::MAX; MAX_AGGREGATE_COLUMNS];
        let mut count_slots = [u8::MAX; MAX_AGGREGATE_COLUMNS];
        let (mut doubles, mut integers, mut nullable) = (0_usize, 0_usize, 0_usize);
        for (state, expression) in inputs[..states].iter().enumerate() {
            let expression = expression.expect("bounded state input");
            let count = if expression.data_type == DataType::Int64 {
                &mut integers
            } else {
                &mut doubles
            };
            value_slots[state] = u8::try_from(*count).expect("bounded state slot");
            *count += 1;
            if expression.nullable() {
                count_slots[state] = u8::try_from(nullable).expect("bounded count slot");
                nullable += 1;
            }
        }
        Self {
            plan,
            demand,
            inputs,
            numeric_columns,
            states,
            entry_states,
            value_slots,
            count_slots,
            sum_states,
            doubles,
            integers,
            nullable,
        }
    }

    fn depth(&self) -> usize {
        self.inputs[..self.states]
            .iter()
            .flatten()
            .map(|expression| expression.stack_depth())
            .max()
            .unwrap_or(0)
    }

    fn counts(&self, groups: usize) -> Result<[usize; 5], Error> {
        Ok([
            groups
                .checked_mul(self.doubles)
                .ok_or(Error::Corrupt("aggregate double cells"))?,
            groups
                .checked_mul(self.integers)
                .ok_or(Error::Corrupt("aggregate integer cells"))?,
            groups
                .checked_mul(self.nullable)
                .ok_or(Error::Corrupt("aggregate nonnull cells"))?,
            groups,
            if self.doubles == 0 { 0 } else { groups },
        ])
    }

    pub(super) fn required_bytes(&self, groups: usize, lanes: usize) -> Result<u64, Error> {
        let fixed = self.fixed_bytes(groups)?;
        self.depth()
            .checked_mul(lanes)
            .and_then(|cells| cells.checked_mul(size_of::<u64>()))
            .and_then(|bytes| u64::try_from(bytes).ok())
            .and_then(|scratch| fixed.checked_add(scratch))
            .ok_or(Error::Corrupt("aggregate state bytes overflow"))
    }

    fn fixed_bytes(&self, groups: usize) -> Result<u64, Error> {
        self.counts(groups)?
            .into_iter()
            .zip([
                size_of::<f64>(),
                size_of::<i128>(),
                size_of::<u32>(),
                size_of::<u32>(),
                size_of::<u32>(),
            ])
            .try_fold(0_usize, |total, (count, width)| {
                count
                    .checked_mul(width)
                    .and_then(|bytes| total.checked_add(bytes))
            })
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or(Error::Corrupt("aggregate fixed bytes overflow"))
    }
}

impl<'db> AggregateState<'db> {
    pub(super) fn new(
        memory: &'db MemoryAuthority,
        plan: &'db AggregatePlan,
        demand: u16,
        groups: usize,
        columns: impl Iterator<Item = SemanticColumn>,
    ) -> Result<Self, Error> {
        Self::from_layout(memory, AggregateLayout::new(plan, demand, columns), groups)
    }

    pub(super) fn from_layout(
        memory: &'db MemoryAuthority,
        layout: AggregateLayout<'db>,
        groups: usize,
    ) -> Result<Self, Error> {
        assert!(groups != 0, "aggregate capacity is positive");
        let [cells, integer_cells, count_cells, _, flag_cells] = layout.counts(groups)?;
        let depth = layout.depth();
        let fixed_bytes = layout.fixed_bytes(groups)?;
        let lane_bytes = depth
            .checked_mul(size_of::<u64>())
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or(Error::Corrupt("scalar lane bytes overflow"))?;
        // A small budget reduces expression width, never semantic work. This
        // snapshot selects a width; the atomic reservation below still owns
        // admission if another query changes the available budget concurrently.
        let available = memory
            .limit()
            .checked_sub(memory.reserved())
            .ok_or(Error::Corrupt("memory account exceeds configured limit"))?;
        let lanes = if depth == 0 {
            0
        } else {
            let affordable = available
                .checked_sub(fixed_bytes)
                .map_or(0, |bytes| bytes / lane_bytes);
            usize::try_from(affordable.clamp(1, BATCH_ROWS as u64)).expect("bounded scalar width")
        };
        let scratch_cells = depth
            .checked_mul(lanes)
            .ok_or(Error::Corrupt("scalar scratch extent overflow"))?;
        let bytes = layout.required_bytes(groups, lanes)?;
        let AggregateLayout {
            plan,
            demand,
            inputs,
            numeric_columns,
            states,
            entry_states,
            value_slots,
            count_slots,
            sum_states,
            ..
        } = layout;
        let reservation = memory.reserve(bytes, "aggregate state")?;
        let mut values = allocate::<f64>(cells, cells, "aggregate values", bytes)?;
        values.resize(cells, 0.0);
        let mut integers = allocate::<i128>(
            integer_cells,
            integer_cells,
            "aggregate integer values",
            bytes,
        )?;
        integers.resize(integer_cells, 0);
        let mut nonnull_counts =
            allocate::<u32>(count_cells, count_cells, "aggregate nonnull counts", bytes)?;
        nonnull_counts.resize(count_cells, 0);
        let mut counts = allocate::<u32>(groups, groups, "aggregate counts", bytes)?;
        counts.resize(groups, 0);
        let mut flags = allocate::<u32>(flag_cells, flag_cells, "aggregate flags", bytes)?;
        flags.resize(flag_cells, 0);
        let mut scratch =
            allocate::<u64>(scratch_cells, scratch_cells, "scalar batch scratch", bytes)?;
        scratch.resize(scratch_cells, 0);
        Ok(Self {
            plan,
            demand,
            inputs,
            scratch,
            lanes,
            input_columns: numeric_columns,
            states,
            entry_states,
            cells: AggregateCells {
                values,
                integers,
                nonnull_counts,
                value_slots,
                count_slots,
                counts,
                flags,
                sum_states,
            },
            reservation,
        })
    }

    pub(super) fn validate_plan(
        &self,
        semantic: &AggregatePlan,
        demand: u16,
        groups: usize,
        mut columns: impl Iterator<Item = SemanticColumn>,
    ) -> Result<(), Error> {
        if self.plan != semantic
            || self.states > MAX_AGGREGATE_COLUMNS
            || self.demand != demand
            || demand >> semantic.entries.len() != 0
        {
            return Err(Error::Corrupt("aggregate physical shape disagrees"));
        }
        for input in &self.input_columns {
            let expected = columns
                .next()
                .filter(|column| matches!(column.data_type(), DataType::Int64 | DataType::Double));
            if *input != expected {
                return Err(Error::Corrupt("aggregate input mapping disagrees"));
            }
        }
        if columns.next().is_some() {
            return Err(Error::Corrupt("aggregate input column capacity"));
        }
        let depth = semantic
            .entries
            .iter()
            .enumerate()
            .filter(|(index, _)| demand & (1 << index) != 0)
            .filter_map(|(_, entry)| entry.expression.as_ref())
            .map(|expression| expression.stack_depth())
            .max()
            .unwrap_or(0);
        if self.lanes > BATCH_ROWS
            || (self.lanes == 0) != (depth == 0)
            || self.scratch.len() != depth * self.lanes
        {
            return Err(Error::Corrupt("scalar workspace disagrees with plan"));
        }
        let (mut doubles, mut integers, mut nullable) = (0_usize, 0_usize, 0_usize);
        for state in 0..self.states {
            let expression =
                self.inputs[state].ok_or(Error::Corrupt("aggregate state input absent"))?;
            let count = match expression.data_type {
                DataType::Int64 => &mut integers,
                DataType::Double => &mut doubles,
                _ => return Err(Error::Corrupt("aggregate numeric type")),
            };
            if usize::from(self.cells.value_slots[state]) != *count {
                return Err(Error::Corrupt("aggregate value slot"));
            }
            *count += 1;
            if expression.nullable() {
                if usize::from(self.cells.count_slots[state]) != nullable {
                    return Err(Error::Corrupt("aggregate count slot"));
                }
                nullable += 1;
            } else if self.cells.count_slots[state] != u8::MAX {
                return Err(Error::Corrupt("nonnullable aggregate counter"));
            }
        }
        if self.cells.values.len() != groups * doubles
            || self.cells.integers.len() != groups * integers
            || self.cells.nonnull_counts.len() != groups * nullable
            || self.cells.counts.len() != groups
            || self.cells.flags.len() != if doubles == 0 { 0 } else { groups }
            || self.cells.value_slots[self.states..]
                .iter()
                .any(|slot| *slot != u8::MAX)
            || self.cells.count_slots[self.states..]
                .iter()
                .any(|slot| *slot != u8::MAX)
        {
            return Err(Error::Corrupt("aggregate array extent"));
        }
        let mut sum_states = 0;
        let mut used_states = 0;
        for (index, entry) in semantic.entries.iter().enumerate() {
            if demand & (1 << index) == 0 || entry.expression.is_none() {
                if self.entry_states[index] != usize::MAX {
                    return Err(Error::Corrupt("unused aggregate state"));
                }
                continue;
            }
            if let Some(expression) = &entry.expression {
                let state = self.entry_states[index];
                if state >= self.states || self.inputs[state] != Some(expression) {
                    return Err(Error::Corrupt("aggregate physical expression disagrees"));
                }
                used_states |= 1_u32 << state;
                if entry.kind == AggregateKind::Sum {
                    sum_states |= 1_u32 << state;
                }
                for op in &expression.ops[..usize::from(expression.len)] {
                    if let Op::Column(column) = op
                        && !self.input_columns.contains(&Some(*column))
                    {
                        return Err(Error::Corrupt("physical expression input is missing"));
                    }
                }
            }
        }
        if sum_states != self.cells.sum_states || used_states != (1_u32 << self.states) - 1 {
            return Err(Error::Corrupt("aggregate state sharing disagrees"));
        }
        Ok(())
    }

    pub(super) fn consume(
        &mut self,
        batch: &Batch,
        positions: &[(usize, u32)],
    ) -> Result<(), Error> {
        assert!(batch.len() <= BATCH_ROWS);
        assert_eq!(positions.len(), batch.len());
        if batch.is_empty() {
            return Ok(());
        }
        let mut numeric = [None; MAX_COLUMNS];
        for (index, input) in self.input_columns.iter().enumerate() {
            if let Some(input) = input {
                numeric[index] = Some(batch.numeric(index, *input)?);
            }
        }
        for state in 0..self.states {
            let is_integer = self.inputs[state].expect("state input").data_type == DataType::Int64;
            for (chunk, positions) in positions[..batch.len()].chunks(self.lanes).enumerate() {
                let first = chunk * self.lanes;
                let evaluated = match self.inputs[state]
                    .expect("validated state expression")
                    .evaluate_batch(&numeric, first..first + positions.len(), &mut self.scratch)
                {
                    Ok(output) => output,
                    Err(failure) => return Err(self.argument_error(state, failure)),
                };
                self.cells.fold(state, is_integer, positions, &evaluated)?;
            }
        }
        Ok(())
    }
    // Shared programs have several source occurrences. Only demanded entries
    // can own an argument failure; resolve that identity on the error path.
    pub(super) fn argument_error(&self, state: usize, failure: ArithmeticFailure) -> Error {
        let entry = self
            .plan
            .entries
            .iter()
            .enumerate()
            .find(|(index, _)| {
                self.demand & (1 << index) != 0 && self.entry_states[*index] == state
            })
            .expect("validated state has a demanded entry")
            .1;
        failure.into_error(entry.span)
    }

    pub(super) fn value(&self, group: usize, index: usize) -> Result<Value<'_>, Error> {
        if index >= self.plan.entries.len() || self.demand & (1 << index) == 0 {
            return Err(Error::Corrupt("undemanded aggregate value"));
        }
        let count = self.cells.counts[group];
        let kind = self.plan.entries[index].kind;
        if kind == AggregateKind::Count {
            return Ok(Value::Int64(i64::from(count)));
        }
        let state = self.entry_states[index];
        let count = if self.cells.count_slots[state] == u8::MAX {
            count
        } else {
            self.cells.nonnull_counts[group
                * (self.cells.nonnull_counts.len() / self.cells.counts.len())
                + usize::from(self.cells.count_slots[state])]
        };
        if count == 0 {
            return Ok(Value::Null);
        }
        if self.inputs[state].expect("state input").data_type == DataType::Int64 {
            let value = self.cells.integers[group
                * (self.cells.integers.len() / self.cells.counts.len())
                + usize::from(self.cells.value_slots[state])];
            return if kind == AggregateKind::Sum {
                i64::try_from(value)
                    .map(Value::Int64)
                    .map_err(|_| Error::ArithmeticOverflow {
                        operation: "SUM",
                        span: self.plan.entries[index].span,
                    })
            } else {
                Ok(Value::Double((value as f64) / f64::from(count)))
            };
        }
        let value = self.cells.values[group * (self.cells.values.len() / self.cells.counts.len())
            + usize::from(self.cells.value_slots[state])];
        let flag = self.cells.flags[group] & (1 << state) != 0;
        if kind == AggregateKind::Sum {
            return if flag {
                Err(Error::ArithmeticOverflow {
                    operation: "SUM",
                    span: self.plan.entries[index].span,
                })
            } else {
                Ok(Value::Double(value))
            };
        }
        let value = if self.cells.sum_states & (1 << state) != 0 {
            if flag {
                (value / f64::from(count)) * SUM_SCALE
            } else {
                value / f64::from(count)
            }
        } else if flag {
            value
        } else {
            value / f64::from(count)
        };
        Ok(Value::Double(value))
    }
}

// Numeric storage is separate from expression scratch so folding borrows both
// owners directly, without moving a workspace or allocating per row.
pub(super) struct AggregateCells {
    pub(super) values: Vec<f64>,
    pub(super) integers: Vec<i128>,
    pub(super) nonnull_counts: Vec<u32>,
    // State-to-column offsets in group-major typed arrays. A nonnullable state
    // uses u8::MAX for its count slot and borrows the group's row count instead.
    pub(super) value_slots: [u8; MAX_AGGREGATE_COLUMNS],
    pub(super) count_slots: [u8; MAX_AGGREGATE_COLUMNS],
    pub(super) counts: Vec<u32>,
    // One bit per DOUBLE state: scaled SUM when sum_states has that bit,
    // otherwise an AVG that switched from a private sum to a running mean.
    pub(super) flags: Vec<u32>,
    pub(super) sum_states: u32,
}

impl AggregateCells {
    pub(super) fn clear_group(&mut self, group: usize) {
        let groups = self.counts.len();
        assert!(group < groups, "clear an admitted group slot");
        let doubles = self.values.len() / groups;
        let integers = self.integers.len() / groups;
        let counts = self.nonnull_counts.len() / groups;
        self.values[group * doubles..(group + 1) * doubles].fill(0.0);
        self.integers[group * integers..(group + 1) * integers].fill(0);
        self.nonnull_counts[group * counts..(group + 1) * counts].fill(0);
        self.counts[group] = 0;
        if !self.flags.is_empty() {
            self.flags[group] = 0;
        }
    }

    pub(super) fn count_row(&mut self, group: usize) -> Result<u32, Error> {
        let count = self.counts[group]
            .checked_add(1)
            .filter(|count| u64::from(*count) <= MAX_AGGREGATE_ROWS)
            .ok_or(Error::Corrupt("aggregate row count overflow"))?;
        self.counts[group] = count;
        Ok(count)
    }

    pub(super) fn fold(
        &mut self,
        state: usize,
        is_integer: bool,
        positions: &[(usize, u32)],
        evaluated: &crate::scalar::NumericOutput<'_>,
    ) -> Result<(), Error> {
        let groups = self.counts.len();
        let double_width = self.values.len() / groups;
        let integer_width = self.integers.len() / groups;
        let count_width = self.nonnull_counts.len() / groups;
        let value_slot = usize::from(self.value_slots[state]);
        for (lane, (group, row_count)) in positions.iter().copied().enumerate() {
            let Some(bits) = evaluated.value(lane) else {
                continue;
            };
            let count = if self.count_slots[state] == u8::MAX {
                row_count
            } else {
                let count = &mut self.nonnull_counts
                    [group * count_width + usize::from(self.count_slots[state])];
                *count = count
                    .checked_add(1)
                    .ok_or(Error::Corrupt("aggregate nonnull count overflow"))?;
                *count
            };
            if is_integer {
                let cell = &mut self.integers[group * integer_width + value_slot];
                let value = i128::from(i64::from_ne_bytes(bits.to_ne_bytes()));
                *cell = cell
                    .checked_add(value)
                    .ok_or(Error::Corrupt("integer aggregate range proof"))?;
                continue;
            }
            let input = f64::from_bits(bits);
            let cell = &mut self.values[group * double_width + value_slot];
            if count == 1 {
                *cell = input;
                continue;
            }
            let mask = 1_u32 << state;
            let previous = self.flags[group] & mask != 0;
            let (value, flag) = if self.sum_states & mask != 0 {
                sum_add(*cell, previous, input)
            } else {
                average_add(*cell, previous, input, count)
            };
            *cell = value;
            self.flags[group] = (self.flags[group] & !mask) | (u32::from(flag) << state);
        }
        Ok(())
    }
}

const SUM_SCALE: f64 = 4_294_967_296.0; // 2^32, exact in binary64.
const SUM_SCALE_INVERSE: f64 = 1.0 / SUM_SCALE;
const SUM_UNSCALE_LIMIT: f64 = f64::MAX * SUM_SCALE_INVERSE;
// Even MAX_ROWS copies of f64::MAX fit after this power-of-two scaling.
// The factor-of-two margin also exceeds accumulated binary64 rounding error.

const _: () =
    assert!(storage_format::MAX_ROWS <= MAX_AGGREGATE_ROWS && MAX_AGGREGATE_ROWS < (1_u64 << 31));
const _: () = assert!((MAX_AGGREGATE_ROWS as u128) * (1_u128 << 63) <= i128::MAX as u128);
// Even this bound times INT64::MIN fits in signed 128-bit state. Narrow only
// demanded final SUM results; AVG need not fit an intermediate INT64 sum.

// Preserve binary64 addition on its ordinary range. Extend only the exponent
// range of a finite partial sum; a later cancellation or nonfinite input must
// still be observed. A scaled state is finite and has magnitude above
// SUM_UNSCALE_LIMIT. Smaller inputs lost during scaling are below its rounding
// precision. Return to ordinary units as soon as the partial sum fits again.
pub(super) fn sum_add(sum: f64, scaled: bool, input: f64) -> (f64, bool) {
    if !scaled {
        let value = sum + input;
        if value.is_finite() || !sum.is_finite() || !input.is_finite() {
            return (value, false);
        }
        return (sum * SUM_SCALE_INVERSE + input * SUM_SCALE_INVERSE, true);
    }
    assert!(sum.is_finite() && sum.abs() > SUM_UNSCALE_LIMIT);
    if !input.is_finite() {
        return (sum + input, false);
    }
    let value = sum + input * SUM_SCALE_INVERSE;
    assert!(
        value.is_finite(),
        "validated row bound contains the scaled sum"
    );
    if value.abs() <= SUM_UNSCALE_LIMIT {
        (value * SUM_SCALE, false)
    } else {
        (value, true)
    }
}

// Keep the existing sum/divide result while its private sum fits. If adding the
// next finite value overflows, divide both finite terms first and retain a mean.
// n >= 2 bounds that transition. Further updates are convex combinations; the
// opposite-sign fallback avoids overflowing the subtraction itself.
pub(super) fn average_add(state: f64, is_mean: bool, input: f64, count: u32) -> (f64, bool) {
    assert!(count >= 2);
    let divisor = f64::from(count);
    if !is_mean {
        let value = state + input;
        if value.is_finite() || !state.is_finite() || !input.is_finite() {
            return (value, false);
        }
        return (state / divisor + input / divisor, true);
    }
    if !state.is_finite() || !input.is_finite() {
        return (state + input, true);
    }
    let delta = input - state;
    let mean = if delta.is_finite() {
        state + delta / divisor
    } else {
        state * (1.0 - 1.0 / divisor) + input / divisor
    };
    assert!(mean.is_finite(), "a finite mean stays in the input range");
    (mean, true)
}
