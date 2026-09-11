//! Admitted aggregate state shared by dense and general grouping.
//!
//! Expression sharing, typed cells, NULL counts, and overflow flags have one
//! owner. Construction borrows only the memory authority; evaluation has no
//! database, filesystem, or scheduler access. Independent validation checks the
//! admitted representation against the semantic plan before consumption.

use crate::Error;
use crate::batch::Batch;
use crate::execution::{BATCH_ROWS, MAX_AGGREGATE_ROWS};
use crate::frontend::{
    AggregateArgument, AggregateKind, AggregatePlan, DataType, MAX_AGGREGATE_COLUMNS, MAX_COLUMNS,
    SemanticColumn,
};
use crate::resources::{MemoryAuthority, Reservation, allocate};
use crate::scalar::ArithmeticFailure;
use crate::storage_format;
use crate::value::Value;
use std::mem::size_of;

/// The controller supplies the source representation's text bound. Legacy
/// pipelines carry one-byte fixed keys; declared pipelines carry bounded UTF-8.
#[derive(Clone, Copy)]
pub(super) enum TextDomain {
    FixedKey,
    Utf8,
}

impl TextDomain {
    pub(super) fn for_storage(native: bool) -> Self {
        if native { Self::Utf8 } else { Self::FixedKey }
    }

    fn bytes(self) -> usize {
        match self {
            Self::FixedKey => 1,
            Self::Utf8 => crate::batch::MAX_TEXT_BYTES,
        }
    }
}

pub(super) struct AggregateState<'db> {
    pub(super) plan: &'db AggregatePlan,
    pub(super) demand: u16,
    pub(super) inputs: [Option<&'db AggregateArgument>; MAX_AGGREGATE_COLUMNS],
    pub(super) scratch: Vec<u64>,
    pub(super) lanes: usize,
    pub(super) input_columns: [Option<SemanticColumn>; MAX_COLUMNS],
    pub(super) states: usize,
    // Demanded aggregate entries with identical arguments share one state.
    // COUNT(*) and undemanded entries have no argument state (usize::MAX).
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
    pub(super) inputs: [Option<&'db AggregateArgument>; MAX_AGGREGATE_COLUMNS],
    input_columns: [Option<SemanticColumn>; MAX_COLUMNS],
    pub(super) states: usize,
    entry_states: [usize; MAX_AGGREGATE_COLUMNS],
    value_slots: [u8; MAX_AGGREGATE_COLUMNS],
    count_slots: [u8; MAX_AGGREGATE_COLUMNS],
    sum_states: u32,
    extrema_slots: [[u8; MAX_AGGREGATE_COLUMNS]; 2],
    extrema: usize,
    text_offsets: [usize; MAX_AGGREGATE_COLUMNS + 1],
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
        Self::with_text_domain(plan, demand, columns, TextDomain::Utf8)
    }

    pub(super) fn with_text_domain(
        plan: &'db AggregatePlan,
        demand: u16,
        columns: impl Iterator<Item = SemanticColumn>,
        text_domain: TextDomain,
    ) -> Self {
        let mut inputs: [Option<&AggregateArgument>; MAX_AGGREGATE_COLUMNS] =
            [None; MAX_AGGREGATE_COLUMNS];
        let mut input_columns = [None; MAX_COLUMNS];
        for (index, column) in columns.enumerate() {
            input_columns[index] = Some(column);
        }
        let mut states = 0;
        let mut sum_states = 0_u32;
        let mut value_states = 0_u32;
        let mut extrema_states = [0_u32; 2];
        let mut entry_states = [usize::MAX; MAX_AGGREGATE_COLUMNS];
        for (index, entry) in plan.entries.iter().enumerate() {
            if demand & (1 << index) == 0 {
                continue;
            }
            if let Some(expression) = &entry.argument {
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
                if matches!(entry.kind, AggregateKind::Sum | AggregateKind::Avg) {
                    value_states |= 1_u32 << state;
                }
                if entry.kind == AggregateKind::Sum {
                    sum_states |= 1_u32 << state;
                }
                for (direction, kind) in [AggregateKind::Min, AggregateKind::Max].iter().enumerate()
                {
                    if entry.kind == *kind {
                        extrema_states[direction] |= 1 << state;
                    }
                }
            }
        }
        // Compact typed arrays avoid charging nonnullable DOUBLE workloads for
        // integer state or per-expression NULL counters.
        let mut value_slots = [u8::MAX; MAX_AGGREGATE_COLUMNS];
        let mut count_slots = [u8::MAX; MAX_AGGREGATE_COLUMNS];
        let mut extrema_slots = [[u8::MAX; MAX_AGGREGATE_COLUMNS]; 2];
        let mut extrema = 0;
        let mut text_offsets = [0; MAX_AGGREGATE_COLUMNS + 1];
        let (mut doubles, mut integers, mut nullable) = (0_usize, 0_usize, 0_usize);
        for (state, expression) in inputs[..states].iter().enumerate() {
            let expression = expression.expect("bounded state input");
            if value_states & (1 << state) != 0 {
                let count = if expression.data_type() == DataType::Int64 {
                    &mut integers
                } else {
                    &mut doubles
                };
                value_slots[state] = u8::try_from(*count).expect("bounded state slot");
                *count += 1;
            }
            for direction in 0..2 {
                if extrema_states[direction] & (1 << state) != 0 {
                    extrema_slots[direction][state] = extrema;
                    let slot = usize::from(extrema);
                    text_offsets[slot + 1] = text_offsets[slot]
                        + if expression.data_type() == DataType::String {
                            text_domain.bytes()
                        } else {
                            0
                        };
                    extrema += 1;
                }
            }
            if expression.nullable() {
                count_slots[state] = u8::try_from(nullable).expect("bounded count slot");
                nullable += 1;
            }
        }
        Self {
            plan,
            demand,
            inputs,
            input_columns,
            states,
            entry_states,
            value_slots,
            count_slots,
            sum_states,
            extrema_slots,
            extrema: usize::from(extrema),
            text_offsets,
            doubles,
            integers,
            nullable,
        }
    }

    pub(super) fn count_only_states(&self) -> u16 {
        self.value_slots[..self.states]
            .iter()
            .enumerate()
            .fold(0, |mask, (state, slot)| {
                mask | (u16::from(
                    *slot == u8::MAX
                        && self
                            .extrema_slots
                            .iter()
                            .all(|slots| slots[state] == u8::MAX),
                ) << state)
            })
    }

    fn depth(&self) -> usize {
        self.inputs[..self.states]
            .iter()
            .flatten()
            .map(|expression| expression.stack_depth())
            .max()
            .unwrap_or(0)
    }

    fn counts(&self, groups: usize) -> Result<[usize; 6], Error> {
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
            groups
                .checked_mul(self.extrema)
                .ok_or(Error::Corrupt("aggregate extrema cells"))?,
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
                size_of::<u64>(),
            ])
            .try_fold(0_usize, |total, (count, width)| {
                count
                    .checked_mul(width)
                    .and_then(|bytes| total.checked_add(bytes))
            })
            .and_then(|bytes| {
                groups
                    .checked_mul(self.text_offsets[self.extrema])
                    .and_then(|text| bytes.checked_add(text))
            })
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or(Error::Corrupt("aggregate fixed bytes overflow"))
    }
}

impl<'db> AggregateState<'db> {
    #[cfg(test)]
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
        let [
            cells,
            integer_cells,
            count_cells,
            _,
            flag_cells,
            extrema_cells,
        ] = layout.counts(groups)?;
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
        let text_bytes = groups
            .checked_mul(layout.text_offsets[layout.extrema])
            .ok_or(Error::Corrupt("aggregate text bytes"))?;
        let AggregateLayout {
            plan,
            demand,
            inputs,
            input_columns,
            states,
            entry_states,
            value_slots,
            count_slots,
            sum_states,
            extrema_slots,
            text_offsets,
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
        let mut extrema =
            allocate::<u64>(extrema_cells, extrema_cells, "aggregate extrema", bytes)?;
        extrema.resize(extrema_cells, 0);
        let mut text = allocate::<u8>(text_bytes, text_bytes, "aggregate text extrema", bytes)?;
        text.resize(text_bytes, 0);
        let mut scratch =
            allocate::<u64>(scratch_cells, scratch_cells, "scalar batch scratch", bytes)?;
        scratch.resize(scratch_cells, 0);
        Ok(Self {
            plan,
            demand,
            inputs,
            scratch,
            lanes,
            input_columns,
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
                extrema,
                extrema_slots,
                text,
                text_spans: Vec::new(),
                text_offsets,
            },
            reservation,
        })
    }

    pub(super) fn validate_plan(
        &self,
        semantic: &AggregatePlan,
        demand: u16,
        groups: usize,
        columns: impl Iterator<Item = SemanticColumn>,
    ) -> Result<(), Error> {
        self.validate_with_text_domain(semantic, demand, groups, columns, TextDomain::Utf8)
    }

    pub(super) fn validate_with_text_domain(
        &self,
        semantic: &AggregatePlan,
        demand: u16,
        groups: usize,
        mut columns: impl Iterator<Item = SemanticColumn>,
        text_domain: TextDomain,
    ) -> Result<(), Error> {
        if self.plan != semantic
            || self.states > MAX_AGGREGATE_COLUMNS
            || self.demand != demand
            || demand >> semantic.entries.len() != 0
        {
            return Err(Error::Corrupt("aggregate physical shape disagrees"));
        }
        for input in &self.input_columns {
            let expected = columns.next();
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
            .filter_map(|(_, entry)| entry.argument.as_ref())
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
            let needs_value = semantic.entries.iter().enumerate().any(|(index, entry)| {
                demand & (1 << index) != 0
                    && self.entry_states[index] == state
                    && matches!(entry.kind, AggregateKind::Sum | AggregateKind::Avg)
            });
            if needs_value {
                let count = match expression.data_type() {
                    DataType::Int64 => &mut integers,
                    DataType::Double => &mut doubles,
                    _ => return Err(Error::Corrupt("aggregate numeric type")),
                };
                if usize::from(self.cells.value_slots[state]) != *count {
                    return Err(Error::Corrupt("aggregate value slot"));
                }
                *count += 1;
            } else if self.cells.value_slots[state] != u8::MAX {
                return Err(Error::Corrupt("non-summing state owns a sum cell"));
            }
            if expression.nullable() {
                if usize::from(self.cells.count_slots[state]) != nullable {
                    return Err(Error::Corrupt("aggregate count slot"));
                }
                nullable += 1;
            } else if self.cells.count_slots[state] != u8::MAX {
                return Err(Error::Corrupt("nonnullable aggregate counter"));
            }
        }
        let mut extrema = 0;
        let mut text_offsets = [0; MAX_AGGREGATE_COLUMNS + 1];
        for state in 0..self.states {
            for (direction, kind) in [AggregateKind::Min, AggregateKind::Max].iter().enumerate() {
                let needed = semantic.entries.iter().enumerate().any(|(index, entry)| {
                    demand & (1 << index) != 0
                        && self.entry_states[index] == state
                        && entry.kind == *kind
                });
                let expected = if needed {
                    let slot = extrema;
                    extrema += 1;
                    text_offsets[usize::from(extrema)] = text_offsets[usize::from(slot)]
                        + if self.inputs[state].expect("checked state input").data_type()
                            == DataType::String
                        {
                            text_domain.bytes()
                        } else {
                            0
                        };
                    slot
                } else {
                    u8::MAX
                };
                if self.cells.extrema_slots[direction][state] != expected {
                    return Err(Error::Corrupt("aggregate extrema slot"));
                }
            }
        }
        if !self.cells.text_spans.is_empty()
            || self.cells.text_offsets != text_offsets
            || self.cells.text.len() != groups * text_offsets[usize::from(extrema)]
            || self.cells.extrema.len() != groups * usize::from(extrema)
            || self
                .cells
                .extrema_slots
                .iter()
                .any(|slots| slots[self.states..].iter().any(|slot| *slot != u8::MAX))
        {
            return Err(Error::Corrupt("aggregate extrema extent"));
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
            if demand & (1 << index) == 0 || entry.argument.is_none() {
                if self.entry_states[index] != usize::MAX {
                    return Err(Error::Corrupt("unused aggregate state"));
                }
                continue;
            }
            if let Some(expression) = &entry.argument {
                let state = self.entry_states[index];
                if state >= self.states || self.inputs[state] != Some(expression) {
                    return Err(Error::Corrupt("aggregate physical expression disagrees"));
                }
                used_states |= 1_u32 << state;
                if entry.kind == AggregateKind::Sum {
                    sum_states |= 1_u32 << state;
                }
                for column in expression.columns() {
                    if !self.input_columns.contains(&Some(column)) {
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
            if let Some(input) = input
                && matches!(input.data_type(), DataType::Int64 | DataType::Double)
            {
                numeric[index] = Some(batch.numeric(index, *input)?);
            }
        }
        for state in 0..self.states {
            let argument = self.inputs[state].expect("state input");
            let expression = match argument {
                AggregateArgument::Numeric(expression) => expression,
                AggregateArgument::Column(column) => {
                    let index = self
                        .input_columns
                        .iter()
                        .position(|input| *input == Some(*column))
                        .ok_or(Error::Corrupt("count input mapping missing"))?;
                    let valid = batch.validity(index, *column)?;
                    for (row, &(group, row_count)) in positions.iter().enumerate() {
                        if valid[row / 64] & (1 << (row % 64)) != 0 {
                            if column.data_type() == DataType::Date {
                                let Some(Value::Date(value)) = batch.value(row, index) else {
                                    return Err(Error::Corrupt("typed DATE argument missing"));
                                };
                                let bits = [i64::from(value.days_since_unix_epoch()) as u64];
                                let output = crate::scalar::NumericOutput::from_bits(
                                    &bits,
                                    [u64::MAX; BATCH_ROWS / 64],
                                )?;
                                self.cells
                                    .fold(state, true, &[(group, row_count)], &output)?;
                            } else {
                                let Some(Value::String(value)) = batch.value(row, index) else {
                                    return Err(Error::Corrupt("typed STRING argument missing"));
                                };
                                self.cells
                                    .fold_text(state, group, row_count, value.as_str())?;
                            }
                        }
                    }
                    continue;
                }
            };
            let is_integer = expression.data_type == DataType::Int64;
            for (chunk, positions) in positions[..batch.len()].chunks(self.lanes).enumerate() {
                let first = chunk * self.lanes;
                let evaluated = match expression.evaluate_batch(
                    &numeric,
                    first..first + positions.len(),
                    &mut self.scratch,
                ) {
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
        if kind == AggregateKind::Count && self.plan.entries[index].argument.is_none() {
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
        if kind == AggregateKind::Count {
            return Ok(Value::Int64(i64::from(count)));
        }
        if count == 0 {
            return Ok(Value::Null);
        }
        if matches!(kind, AggregateKind::Min | AggregateKind::Max) {
            let direction = usize::from(kind == AggregateKind::Max);
            let slot = usize::from(self.cells.extrema_slots[direction][state]);
            let bits = self.cells.extrema
                [group * (self.cells.extrema.len() / self.cells.counts.len()) + slot];
            return match self.inputs[state].expect("state input").data_type() {
                DataType::Int64 => Ok(Value::Int64(bits as i64)),
                DataType::Double => Ok(Value::Double(f64::from_bits(bits))),
                DataType::Date => i32::try_from(bits as i64)
                    .ok()
                    .and_then(crate::DateValue::from_days)
                    .map(Value::Date)
                    .ok_or(Error::Corrupt("aggregate DATE value")),
                DataType::String => self
                    .cells
                    .text_value(group, slot)
                    .map(|text| Value::String(crate::StringValue::new(text))),
            };
        }
        if self.inputs[state].expect("state input").data_type() == DataType::Int64 {
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

// Accumulator storage is separate from expression scratch so folding borrows both
// owners directly, without moving a workspace or allocating per row.
pub(super) struct AggregateCells {
    pub(super) values: Vec<f64>,
    pub(super) integers: Vec<i128>,
    // Numeric extrema store bits; text extrema store lengths in the same slots.
    pub(super) extrema: Vec<u64>,
    pub(super) text: Vec<u8>,
    // Empty for the admitted fixed layout. Hash grouping uses one span per
    // extremum slot and owns arena growth outside argument folding.
    pub(super) text_spans: Vec<TextSpan>,
    // Prefix offsets within one group's text storage, indexed by extremum slot.
    // Numeric slots have zero width. Text slots reserve their source bound so each
    // replacement reuses its own capacity without allocation or compaction.
    pub(super) text_offsets: [usize; MAX_AGGREGATE_COLUMNS + 1],
    // Direction 0 is MIN and 1 is MAX; u8::MAX denotes an undemanded slot.
    pub(super) extrema_slots: [[u8; MAX_AGGREGATE_COLUMNS]; 2],
    pub(super) nonnull_counts: Vec<u32>,
    // State-to-column offsets in group-major typed arrays. A state without
    // SUM/AVG has no sum cell (u8::MAX). A nonnullable state
    // uses u8::MAX for its count slot and borrows the group's row count instead.
    pub(super) value_slots: [u8; MAX_AGGREGATE_COLUMNS],
    pub(super) count_slots: [u8; MAX_AGGREGATE_COLUMNS],
    pub(super) counts: Vec<u32>,
    // One bit per DOUBLE state: scaled SUM when sum_states has that bit,
    // otherwise an AVG that switched from a private sum to a running mean.
    pub(super) flags: Vec<u32>,
    pub(super) sum_states: u32,
}

/// One optional hash slot's region. Its capacity never shrinks, so repeated
/// replacement cannot accumulate space in proportion to the input row count.
#[derive(Clone, Copy, Default)]
pub(super) struct TextSpan {
    pub(super) start: usize,
    pub(super) capacity: usize,
}

impl AggregateCells {
    pub(super) fn text_value(&self, group: usize, slot: usize) -> Result<&str, Error> {
        let slots = self.extrema.len() / self.counts.len();
        let cell = group * slots + slot;
        let length = usize::try_from(self.extrema[cell])
            .map_err(|_| Error::Corrupt("text extremum length"))?;
        let width = self.text_offsets[slot + 1] - self.text_offsets[slot];
        if !matches!(width, 1 | crate::batch::MAX_TEXT_BYTES) || length > width {
            return Err(Error::Corrupt("text extremum extent"));
        }
        let start = if self.text_spans.is_empty() {
            group * self.text_offsets[slots] + self.text_offsets[slot]
        } else {
            let span = self
                .text_spans
                .get(cell)
                .ok_or(Error::Corrupt("hash text slot"))?;
            if length > span.capacity
                || span.capacity > width
                || (span.capacity != 0 && !span.capacity.is_power_of_two())
            {
                return Err(Error::Corrupt("hash text capacity"));
            }
            let end = span
                .start
                .checked_add(span.capacity)
                .ok_or(Error::Corrupt("hash text extent overflow"))?;
            if end > self.text.len() {
                return Err(Error::Corrupt("hash text extent"));
            }
            span.start
        };
        let end = start
            .checked_add(length)
            .ok_or(Error::Corrupt("text extremum end"))?;
        let bytes = self
            .text
            .get(start..end)
            .ok_or(Error::Corrupt("text extremum bytes"))?;
        std::str::from_utf8(bytes).map_err(|_| Error::Corrupt("text extremum UTF-8"))
    }

    fn replace_text(&mut self, group: usize, slot: usize, value: &str) -> Result<(), Error> {
        let slots = self.extrema.len() / self.counts.len();
        let cell = group * slots + slot;
        let start = if self.text_spans.is_empty() {
            group * self.text_offsets[slots] + self.text_offsets[slot]
        } else {
            let span = self
                .text_spans
                .get_mut(cell)
                .ok_or(Error::Corrupt("hash text slot"))?;
            if value.len() > span.capacity {
                let capacity = value
                    .len()
                    .checked_next_power_of_two()
                    .ok_or(Error::Corrupt("hash text region capacity"))?;
                let start = self.text.len();
                let end = start
                    .checked_add(capacity)
                    .ok_or(Error::Corrupt("hash text arena extent"))?;
                // The controller admits growth before folding. No row operation
                // may allocate or silently cross that admitted capacity.
                if end > self.text.capacity() {
                    return Err(Error::Corrupt("hash text growth was not admitted"));
                }
                self.text.resize(end, 0);
                *span = TextSpan { start, capacity };
            }
            span.start
        };
        self.text[start..start + value.len()].copy_from_slice(value.as_bytes());
        self.extrema[cell] = value.len() as u64;
        Ok(())
    }

    pub(super) fn fold_text(
        &mut self,
        state: usize,
        group: usize,
        row_count: u32,
        value: &str,
    ) -> Result<(), Error> {
        if value.len() > crate::batch::MAX_TEXT_BYTES {
            return Err(Error::Corrupt("text extremum input length"));
        }
        let count = self.count_present(state, group, row_count)?;
        for direction in 0..2 {
            let slot = self.extrema_slots[direction][state];
            if slot == u8::MAX {
                continue;
            }
            let slot = usize::from(slot);
            let width = self.text_offsets[slot + 1] - self.text_offsets[slot];
            if value.len() > width {
                return Err(Error::Corrupt("text extremum exceeds source domain"));
            }
            let replace = count == 1 || {
                let prior = self.text_value(group, slot)?;
                if direction == 0 {
                    value < prior
                } else {
                    value > prior
                }
            };
            if replace {
                self.replace_text(group, slot, value)?;
            }
        }
        Ok(())
    }

    fn count_present(&mut self, state: usize, group: usize, row_count: u32) -> Result<u32, Error> {
        if self.count_slots[state] == u8::MAX {
            return Ok(row_count);
        }
        let width = self.nonnull_counts.len() / self.counts.len();
        let count = &mut self.nonnull_counts[group * width + usize::from(self.count_slots[state])];
        *count = count
            .checked_add(1)
            .ok_or(Error::Corrupt("aggregate nonnull count overflow"))?;
        Ok(*count)
    }

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
        // Extrema remain private stale bits until the next present value replaces
        // them. The reset counts make empty and all-NULL groups return NULL.
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
        let value_slot = usize::from(self.value_slots[state]);
        for (lane, (group, row_count)) in positions.iter().copied().enumerate() {
            let Some(bits) = evaluated.value(lane) else {
                continue;
            };
            let count = self.count_present(state, group, row_count)?;
            for direction in 0..2 {
                let slot = self.extrema_slots[direction][state];
                if slot != u8::MAX {
                    let width = self.extrema.len() / groups;
                    let current = &mut self.extrema[group * width + usize::from(slot)];
                    update_extremum(current, bits, is_integer, count == 1, direction == 0);
                }
            }
            if self.value_slots[state] == u8::MAX {
                continue;
            }
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

// MIN/MAX propagate the first NaN payload. Total ordering is used only after
// excluding NaNs, so ties between zero signs select -0 for MIN and +0 for MAX.
fn update_extremum(current: &mut u64, incoming: u64, integer: bool, first: bool, minimum: bool) {
    if first {
        *current = incoming;
        return;
    }
    let order = if integer {
        (incoming as i64).cmp(&(*current as i64))
    } else {
        let next = f64::from_bits(incoming);
        let prior = f64::from_bits(*current);
        if prior.is_nan() {
            return;
        }
        if next.is_nan() {
            *current = incoming;
            return;
        }
        next.total_cmp(&prior)
    };
    if (minimum && order.is_lt()) || (!minimum && order.is_gt()) {
        *current = incoming;
    }
}

#[cfg(test)]
mod extrema_tests {
    use super::{AggregateCells, TextSpan, update_extremum};
    use crate::frontend::MAX_AGGREGATE_COLUMNS;

    #[test]
    fn compact_text_reuses_regions_and_rejects_invalid_extents() {
        let mut offsets = [0; MAX_AGGREGATE_COLUMNS + 1];
        offsets[1] = crate::batch::MAX_TEXT_BYTES;
        let mut cells = AggregateCells {
            values: Vec::new(),
            integers: Vec::new(),
            extrema: vec![0],
            text: Vec::with_capacity(64),
            text_spans: vec![TextSpan::default()],
            text_offsets: offsets,
            extrema_slots: [[u8::MAX; MAX_AGGREGATE_COLUMNS]; 2],
            nonnull_counts: Vec::new(),
            value_slots: [u8::MAX; MAX_AGGREGATE_COLUMNS],
            count_slots: [u8::MAX; MAX_AGGREGATE_COLUMNS],
            counts: vec![1],
            flags: Vec::new(),
            sum_states: 0,
        };
        let allocation = cells.text.as_ptr();
        for value in ["", "a", "bc", "def", "x", "ghijk", "", "lmnop"] {
            cells.replace_text(0, 0, value).unwrap();
            assert_eq!(cells.text_value(0, 0).unwrap(), value);
            assert_eq!(cells.text.as_ptr(), allocation, "folding cannot allocate");
        }
        assert_eq!(cells.text.len(), 15, "regions of 1, 2, 4, and 8 bytes");
        assert_eq!(cells.text_spans[0].capacity, 8);
        for _ in 0..100 {
            cells.replace_text(0, 0, "short").unwrap();
            cells.replace_text(0, 0, "").unwrap();
        }
        assert_eq!(cells.text.len(), 15, "repeated replacement reuses capacity");
        cells.text_spans[0].start = usize::MAX;
        assert!(cells.text_value(0, 0).is_err());
    }

    #[test]
    fn floating_extrema_preserve_nan_payloads_and_order_zero_signs() {
        let first_nan = 0x7ff8_0000_0000_0001;
        let second_nan = 0x7ff8_0000_0000_0002;
        for (input, low, high) in [
            (
                vec![0.0_f64.to_bits(), (-0.0_f64).to_bits()],
                (-0.0_f64).to_bits(),
                0.0_f64.to_bits(),
            ),
            (
                vec![(-0.0_f64).to_bits(), 0.0_f64.to_bits()],
                (-0.0_f64).to_bits(),
                0.0_f64.to_bits(),
            ),
            (
                vec![f64::INFINITY.to_bits(), f64::NEG_INFINITY.to_bits()],
                f64::NEG_INFINITY.to_bits(),
                f64::INFINITY.to_bits(),
            ),
            (
                vec![
                    1.0_f64.to_bits(),
                    first_nan,
                    second_nan,
                    (-9.0_f64).to_bits(),
                ],
                first_nan,
                first_nan,
            ),
        ] {
            let mut minimum = 0;
            let mut maximum = 0;
            for (index, bits) in input.into_iter().enumerate() {
                update_extremum(&mut minimum, bits, false, index == 0, true);
                update_extremum(&mut maximum, bits, false, index == 0, false);
            }
            assert_eq!(minimum, low);
            assert_eq!(maximum, high);
        }
    }
}
