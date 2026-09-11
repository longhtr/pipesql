//! Charged argument capture and replay through the shared checked row codec.
use super::numeric::{AggregateCells, AggregateState};
use crate::batch::Batch;
use crate::execution::blocking::{
    ArgumentShape, RECORD_HEADER, RowLayout, SortRecord, append_bytes,
};
use crate::execution::{BATCH_ROWS, MAX_AGGREGATE_ROWS};
use crate::frontend::{DataType, MAX_AGGREGATE_COLUMNS, MAX_COLUMNS};
use crate::resources::{Reservation, allocate};
use crate::scalar::Expression;
use crate::{CancellationToken, Database, Error};
use std::mem::size_of;

// The same charged column vectors hold evaluated input and checked disk input.
// Grouping keys remain in the source batch or record; they are never converted to numbers.
pub(super) struct ArgumentBatch<'db> {
    pub(super) values: Vec<u64>,
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
        record.finish(
            self.shape.layout(keys),
            key_len,
            ordinal,
            &values[..self.shape.count],
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
            || (shape.nonnull | shape.integers) >> shape.count != 0
        {
            return Err(Error::Corrupt("argument batch shape"));
        }
        let cells = capacity
            .checked_mul(shape.count)
            .ok_or(Error::Corrupt("argument batch cells"))?;
        let bytes = cells
            .checked_mul(size_of::<u64>())
            .and_then(|n| n.checked_add(size_of::<Self>()))
            .ok_or(Error::Corrupt("argument batch memory"))? as u64;
        let reservation = database.reserve_memory(bytes, "group argument batch")?;
        let mut values = allocate(cells, cells, "group argument values", bytes)?;
        values.resize(cells, 0);
        Ok(Self {
            values,
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
        let mut numeric = [None; MAX_COLUMNS];
        for (index, source) in aggregate.input_columns.iter().enumerate() {
            if let Some(source) = source {
                numeric[index] = Some(input.numeric(index, *source)?);
            }
        }
        for state in 0..self.shape.count {
            let expression = aggregate.inputs[state].expect("validated demanded expression");
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
                    self.store(state, row, output.value(row - start));
                }
            }
        }
        // Failure leaves no published rows, even if earlier states were evaluated.
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

    pub(super) fn append(&mut self, record: &SortRecord) -> Result<(), Error> {
        if self.source_start.is_some() {
            return Err(Error::Corrupt(
                "disk arguments require an empty replay batch",
            ));
        }
        if self.rows == self.capacity
            || usize::from(record.bytes[26]) != self.shape.count
            || record.valid() & self.shape.nonnull != self.shape.nonnull
        {
            return Err(Error::Corrupt("reduced argument shape"));
        }
        // The disk reader has already checked CRC, key shape and validity tails.
        for state in 0..self.shape.count {
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
            let values = &self.values[state * self.capacity..state * self.capacity + self.rows];
            let output = crate::scalar::NumericOutput::from_bits(values, self.valid[state])?;
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
        Self::from_inputs(&aggregate.inputs[..aggregate.states])
    }

    pub(super) fn from_inputs(inputs: &[Option<&Expression>]) -> Self {
        let mut nonnull = 0;
        let mut integers = 0;
        for (index, input) in inputs.iter().enumerate() {
            let input = input.expect("validated aggregate input");
            if !input.nullable() {
                nonnull |= 1 << index;
            }
            if input.data_type == DataType::Int64 {
                integers |= 1 << index;
            }
        }
        Self {
            count: inputs.len(),
            nonnull,
            integers,
        }
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;
