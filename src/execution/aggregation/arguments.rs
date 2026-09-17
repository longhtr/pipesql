//! Evaluate aggregate arguments once, then reuse them for hashing or disk sorting.
//!
//! SUM(x) and AVG(x), for example, can share one captured x value per input row.
//! `ArgumentBatch` owns those values and their NULL bits. Numeric values retain
//! their bits; strings are copied into bounded text buffers. Group keys remain
//! in the source batch and are encoded separately when writing a sort record.
//!
//! If only COUNT needs an argument, capture whether it is present. A numeric
//! expression must still be evaluated: COUNT(x / y) cannot skip a division error
//! merely because it discards the value afterward. No captured value points into
//! a source buffer that the next input batch could overwrite.
//!
//! Evaluation exposes rows only after all arguments succeed. The grouping
//! controller can then fold them into hash cells or write them to disk. Sorted
//! replay fills the same buffers from checked records, preserving record order
//! and evaluated values. Reaching row or text capacity ends one replay batch so
//! it can be folded before more records arrive.

use super::accumulator::{AggregateCells, AggregateState};
use crate::batch::Batch;
use crate::execution::blocking::{
    ArgumentShape, RECORD_HEADER, RowLayout, SortRecord, append_bytes,
};
use crate::execution::{BATCH_ROWS, MAX_AGGREGATE_ROWS};
use crate::query::{AggregateArgument, MAX_AGGREGATE_COLUMNS, MAX_ROW_VALUES};
use crate::resources::{Reservation, allocate};
use crate::value::DataType;

use crate::{CancellationToken, Database, Error};
use std::mem::size_of;

// Use the same column storage for freshly evaluated arguments and disk replay.
// The reservation outlives both allocations; group keys have separate owners.
pub(super) struct ArgumentBatch<'db> {
    pub(super) values: Vec<u64>,
    // Each text argument gets one fixed region. Its value words encode a u32
    // offset and u32 length within that region, so moving this owner is safe.
    text: Vec<u8>,
    text_used: [usize; MAX_AGGREGATE_COLUMNS],
    pub(super) valid: [[u64; BATCH_ROWS / 64]; MAX_AGGREGATE_COLUMNS],
    pub(super) shape: ArgumentShape,
    pub(super) capacity: usize,
    pub(super) rows: usize,
    pub(super) source_start: Option<usize>,
    pub(super) reservation: Reservation<'db>,
}

impl<'db> ArgumentBatch<'db> {
    pub(super) fn encode_record(
        &self,
        record: &mut SortRecord,
        keys: &RowLayout,
        source: &Batch,
        row: usize,
        ordinal: u64,
    ) -> Result<(), Error> {
        if row >= self.rows || ordinal >= MAX_AGGREGATE_ROWS {
            return Err(Error::Corrupt("captured argument row"));
        }
        let start = self
            .source_start
            .ok_or(Error::Corrupt("arguments have no source batch"))?;
        record.bytes.clear();
        append_bytes(&mut record.bytes, &[0; RECORD_HEADER])?;
        keys.append_row(source, start + row, &mut record.bytes)?;
        let key_len = record.bytes.len() - RECORD_HEADER;
        let mut values = [None; MAX_AGGREGATE_COLUMNS];
        for (state, value) in values[..self.shape.count].iter_mut().enumerate() {
            *value = self.value(state, row);
        }
        let mut text = [""; MAX_AGGREGATE_COLUMNS];
        let mut text_count = 0;
        for (state, encoded) in values[..self.shape.count].iter_mut().enumerate() {
            if self.shape.text & (1 << state) != 0 {
                let value = self.text_value(state, row)?;
                *encoded = value.map(|value| value.len() as u64);
                text[text_count] = value.unwrap_or("");
                text_count += 1;
            }
        }
        record.finish_with_text(
            self.shape.layout(keys),
            key_len,
            ordinal,
            &values[..self.shape.count],
            &text[..text_count],
        )
    }

    pub(super) fn new(
        database: &'db Database,
        shape: ArgumentShape,
        capacity: usize,
    ) -> Result<Self, Error> {
        if capacity == 0
            || capacity > BATCH_ROWS
            || shape.count > MAX_AGGREGATE_COLUMNS
            || (shape.nonnull | shape.integers | shape.presence | shape.dates | shape.text)
                >> shape.count
                != 0
            || shape.dates & (!shape.integers | shape.presence) != 0
            || shape.text & (shape.integers | shape.presence | shape.dates) != 0
        {
            return Err(Error::Corrupt("argument batch shape"));
        }
        let cells = capacity
            .checked_mul(shape.count)
            .ok_or(Error::Corrupt("argument batch cells"))?;
        let bytes = cells
            .checked_mul(size_of::<u64>())
            .and_then(|n| n.checked_add(shape.text_bytes()))
            .and_then(|n| n.checked_add(size_of::<Self>()))
            .ok_or(Error::Corrupt("argument batch memory"))? as u64;
        let reservation = database.reserve_memory(bytes, "group argument batch")?;
        let mut values = allocate(cells, cells, "group argument values", bytes)?;
        values.resize(cells, 0);
        let mut text = allocate(
            shape.text_bytes(),
            shape.text_bytes(),
            "group argument text",
            bytes,
        )?;
        text.resize(shape.text_bytes(), 0);
        Ok(Self {
            values,
            text,
            text_used: [0; MAX_AGGREGATE_COLUMNS],
            valid: [[u64::MAX; BATCH_ROWS / 64]; MAX_AGGREGATE_COLUMNS],
            shape,
            capacity,
            rows: 0,
            source_start: None,
            reservation,
        })
    }

    pub(super) fn clear(&mut self) {
        self.rows = 0;
        self.source_start = None;
        self.valid.fill([u64::MAX; BATCH_ROWS / 64]);
        self.text_used.fill(0);
    }

    pub(super) fn check_program(&self, aggregate: &AggregateState<'_>) -> Result<(), Error> {
        let expected = ArgumentShape::from_aggregate(aggregate);
        if self.shape != expected {
            return Err(Error::Corrupt("argument batch disagrees with aggregate"));
        }
        Ok(())
    }

    pub(super) fn evaluate(
        &mut self,
        aggregate: &mut AggregateState<'_>,
        input: &Batch,
        range: std::ops::Range<usize>,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        self.clear();
        self.check_program(aggregate)?;
        cancel.check()?;
        let rows = range
            .end
            .checked_sub(range.start)
            .filter(|rows| *rows <= self.capacity)
            .ok_or(Error::Corrupt("argument batch input range"))?;
        if range.end > input.len() {
            return Err(Error::Corrupt("argument input exceeds batch"));
        }
        if rows == 0 {
            return Ok(());
        }
        let mut numeric = [None; MAX_ROW_VALUES];
        for (index, source) in aggregate.input_columns.iter().enumerate() {
            if let Some(source) = source
                && matches!(source.data_type(), DataType::Int64 | DataType::Double)
            {
                numeric[index] = Some(crate::query::scalar::NumericInput::from_batch(
                    input, index, *source,
                )?);
            }
        }
        for state in 0..self.shape.count {
            let argument = aggregate.inputs[state].expect("validated demanded argument");
            let expression = match argument {
                AggregateArgument::Numeric(expression) => expression,
                AggregateArgument::Column(column) => {
                    let index = aggregate
                        .input_columns
                        .iter()
                        .position(|input| *input == Some(*column))
                        .ok_or(Error::Corrupt("count input mapping missing"))?;
                    let valid = input.validity(index, column.data_type(), column.nullable())?;
                    for row in 0..rows {
                        let source = range.start + row;
                        let value = if valid[source / 64] & (1 << (source % 64)) == 0 {
                            None
                        } else if self.shape.presence & (1 << state) != 0 {
                            Some(0)
                        } else if self.shape.text & (1 << state) != 0 {
                            let Some(crate::Value::String(value)) = input.value(source, index)
                            else {
                                return Err(Error::Corrupt("typed STRING argument missing"));
                            };
                            self.store_text(state, row, value.as_str())?;
                            continue;
                        } else {
                            let Some(crate::Value::Date(value)) = input.value(source, index) else {
                                return Err(Error::Corrupt("typed DATE argument missing"));
                            };
                            Some(i64::from(value.days_since_unix_epoch()) as u64)
                        };
                        self.store(state, row, value);
                    }
                    continue;
                }
            };
            assert!(aggregate.lanes > 0, "numeric programs own expression lanes");
            for start in (0..rows).step_by(aggregate.lanes) {
                cancel.check()?;
                let end = (start + aggregate.lanes).min(rows);
                let output = match expression.evaluate_batch(
                    &numeric,
                    range.start + start..range.start + end,
                    &mut aggregate.scratch,
                ) {
                    Ok(output) => output,
                    Err(failure) => return Err(aggregate.argument_error(state, failure)),
                };
                for row in start..end {
                    let value = output.value(row - start).map(|bits| {
                        if self.shape.presence & (1 << state) != 0 {
                            0
                        } else {
                            bits
                        }
                    });
                    self.store(state, row, value);
                }
            }
        }
        // clear left rows at zero. Publish the count only after every argument
        // succeeds, preventing a caller from folding partially evaluated input.
        self.rows = rows;
        self.source_start = Some(range.start);
        Ok(())
    }

    pub(super) fn store(&mut self, state: usize, row: usize, value: Option<u64>) {
        self.values[state * self.capacity + row] = value.unwrap_or(0);
        if value.is_none() {
            self.valid[state][row / 64] &= !(1 << (row % 64));
        }
    }

    pub(super) fn value(&self, state: usize, row: usize) -> Option<u64> {
        assert!(state < self.shape.count && row < self.rows);
        (self.valid[state][row / 64] & (1 << (row % 64)) != 0)
            .then_some(self.values[state * self.capacity + row])
    }

    // Count preceding text arguments to find this region; numeric arguments
    // occupy no text space.
    fn text_base(&self, state: usize) -> usize {
        (self.shape.text & ((1 << state) - 1)).count_ones() as usize * crate::batch::MAX_TEXT_BYTES
    }

    fn store_text(&mut self, state: usize, row: usize, value: &str) -> Result<(), Error> {
        let start = self.text_used[state];
        let end = start
            .checked_add(value.len())
            .filter(|end| *end <= crate::batch::MAX_TEXT_BYTES)
            .ok_or(Error::Corrupt("captured text exceeds admitted capacity"))?;
        let base = self.text_base(state);
        self.text[base + start..base + end].copy_from_slice(value.as_bytes());
        self.text_used[state] = end;
        self.store(state, row, Some((value.len() as u64) << 32 | start as u64));
        Ok(())
    }

    pub(super) fn text_value(&self, state: usize, row: usize) -> Result<Option<&str>, Error> {
        let Some(span) = self.value(state, row) else {
            return Ok(None);
        };
        let start = (span as u32) as usize;
        let length = (span >> 32) as usize;
        let end = start
            .checked_add(length)
            .filter(|end| *end <= self.text_used[state])
            .ok_or(Error::Corrupt("captured text span"))?;
        let base = self.text_base(state);
        std::str::from_utf8(&self.text[base + start..base + end])
            .map(Some)
            .map_err(|_| Error::Corrupt("captured text UTF-8"))
    }

    pub(super) fn can_append(&self, record: &SortRecord) -> Result<bool, Error> {
        if self.rows == self.capacity {
            return Ok(false);
        }
        for state in 0..self.shape.count {
            if self.shape.text & (1 << state) != 0 {
                let value = record.text_value(state, self.shape)?;
                if value.len() > crate::batch::MAX_TEXT_BYTES - self.text_used[state] {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    pub(super) fn append(&mut self, record: &SortRecord) -> Result<(), Error> {
        if self.source_start.is_some() {
            return Err(Error::Corrupt(
                "disk arguments require an empty replay batch",
            ));
        }
        if !self.can_append(record)?
            || usize::from(record.bytes[26]) != self.shape.count
            || record.valid() & self.shape.nonnull != self.shape.nonnull
        {
            return Err(Error::Corrupt("reduced argument shape"));
        }
        // The caller supplies a record already checked by the disk reader,
        // including checksum, key layout and unused validity bits. Preserve its
        // argument values here; do not run the source expressions again.
        for state in 0..self.shape.count {
            if self.shape.text & record.valid() & (1 << state) != 0 {
                self.store_text(state, self.rows, record.text_value(state, self.shape)?)?;
                continue;
            }
            self.store(
                state,
                self.rows,
                (record.valid() & (1 << state) != 0).then(|| record.bits(state)),
            );
        }
        self.rows += 1;
        Ok(())
    }

    pub(super) fn fold_group(
        &self,
        aggregate: &mut AggregateState<'_>,
        group: usize,
    ) -> Result<(), Error> {
        self.check_program(aggregate)?;
        if group >= aggregate.cells.counts.len() {
            return Err(Error::Corrupt("reduced group slot"));
        }
        let mut positions = [(0, 0); BATCH_ROWS];
        for position in &mut positions[..self.rows] {
            *position = (group, aggregate.cells.count_row(group)?);
        }
        self.fold_positions(&mut aggregate.cells, &positions[..self.rows])
    }

    pub(super) fn fold_positions(
        &self,
        cells: &mut AggregateCells,
        positions: &[(usize, u32)],
    ) -> Result<(), Error> {
        if positions.len() != self.rows {
            return Err(Error::Corrupt("argument position extent"));
        }
        for state in 0..self.shape.count {
            if self.shape.text & (1 << state) != 0 {
                for (row, &(group, row_count)) in positions.iter().enumerate() {
                    if let Some(value) = self.text_value(state, row)? {
                        cells.fold_text(state, group, row_count, value)?;
                    }
                }
                continue;
            }
            let values = &self.values[state * self.capacity..state * self.capacity + self.rows];
            let output = crate::query::scalar::NumericOutput::from_bits(values, self.valid[state])?;
            cells.fold(
                state,
                self.shape.integers & (1 << state) != 0,
                positions,
                &output,
            )?;
        }
        Ok(())
    }
}

impl ArgumentShape {
    pub(super) fn from_aggregate(aggregate: &AggregateState<'_>) -> Self {
        // Without sum/average or extrema storage, an argument serves only COUNT.
        // Keep its NULL status and store zero in place of the discarded value.
        let mut presence = 0;
        for state in 0..aggregate.states {
            if aggregate.cells.value_slots[state] == u8::MAX
                && aggregate
                    .cells
                    .extrema_slots
                    .iter()
                    .all(|slots| slots[state] == u8::MAX)
            {
                presence |= 1 << state;
            }
        }
        Self::from_inputs(&aggregate.inputs[..aggregate.states], presence)
    }

    pub(super) fn from_inputs(inputs: &[Option<&AggregateArgument>], presence: u16) -> Self {
        let mut nonnull = 0;
        let mut integers = 0;
        let mut dates = 0;
        let mut text = 0;
        for (index, input) in inputs.iter().enumerate() {
            let input = input.expect("validated aggregate input");
            if !input.nullable() {
                nonnull |= 1 << index;
            }
            if input.data_type() == DataType::String && presence & (1 << index) == 0 {
                text |= 1 << index;
            }
            if input.data_type() == DataType::Date && presence & (1 << index) == 0 {
                dates |= 1 << index;
            }
            if input.data_type() == DataType::Int64 || dates & (1 << index) != 0 {
                integers |= 1 << index;
            }
        }
        Self {
            count: inputs.len(),
            nonnull,
            integers,
            presence,
            dates,
            text,
        }
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
#[path = "arguments_tests.rs"]
mod tests;
