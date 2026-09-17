//! Lend native column slices while one decoded input batch is alive.
//!
//! Numeric vectors and validity bits are reused. Temporary STRING descriptors
//! borrow decoder text only during a write, so no second text arena is needed.
//! Their maximum capacity stays charged between writes; allocation can still
//! fail, in which case the importer aborts every earlier unit.

use crate::effects::Effects;
use crate::resources::{Reservation, allocate};
use crate::storage::append::Append;
use crate::{
    CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, DataType, DateValue, Error,
    InputBatch, Value,
};
use std::mem::size_of;
use std::ops::Range;

const ROUNDING: usize = 16_384;

pub(crate) struct Columns<'db, 'schema> {
    integers: Vec<i64>,
    doubles: Vec<f64>,
    dates: Vec<DateValue>,
    validity: Vec<u8>,
    schema: &'schema [ColumnDeclaration<'schema>],
    starts: [usize; 64],
    bitmap_stride: usize,
    strings: usize,
    // Physical buffers precede their reservation in drop order.
    reservation: Reservation<'db>,
}

impl<'db, 'schema> Columns<'db, 'schema> {
    pub(crate) fn new(
        memory: &'db crate::resources::MemoryAuthority,
        schema: &'schema [ColumnDeclaration<'schema>],
        rows: u16,
    ) -> Result<Self, Error> {
        let mut counts = [0; 4];
        for column in schema {
            counts[match column.data_type {
                DataType::Int64 => 0,
                DataType::Double => 1,
                DataType::Date => 2,
                DataType::String => 3,
            }] += usize::from(rows);
        }
        let bitmap_stride = usize::from(rows).div_ceil(8);
        let validity = schema.len() * bitmap_stride;
        // Geometry is bounded by the decoder's validated 64 columns and 256 rows.
        // Include the local views and partition counters as well as retained data.
        let requested = size_of::<Self>()
            + size_of::<[ColumnInput<'_>; 64]>()
            + size_of::<[ColumnDeclaration<'_>; 64]>()
            + size_of::<[usize; 64]>()
            + size_of::<Vec<&str>>()
            + validity
            + counts[0] * size_of::<i64>()
            + counts[1] * size_of::<f64>()
            + counts[2] * size_of::<DateValue>()
            + counts[3] * size_of::<&str>()
            + (1 + counts.iter().filter(|&&count| count != 0).count()) * ROUNDING;
        let reservation = memory.reserve(requested as u64, "import columns")?;
        let integers = allocate(
            counts[0],
            counts[0],
            "import INT64 columns",
            requested as u64,
        )?;
        let doubles = allocate(
            counts[1],
            counts[1],
            "import DOUBLE columns",
            requested as u64,
        )?;
        let dates = allocate(
            counts[2],
            counts[2],
            "import DATE columns",
            requested as u64,
        )?;
        let mut bits = allocate(validity, validity, "import validity", requested as u64)?;
        bits.resize(validity, 0);
        Ok(Self {
            integers,
            doubles,
            dates,
            validity: bits,
            schema,
            starts: [0; 64],
            bitmap_stride,
            strings: counts[3],
            reservation,
        })
    }

    pub(crate) fn next_end(
        &self,
        batch: &InputBatch<'_>,
        start: usize,
        cancel: &CancellationToken,
    ) -> Result<usize, Error> {
        let mut text = [0_usize; 64];
        for row in start..batch.row_count() {
            cancel.check()?;
            let rows = row - start + 1;
            for (column, definition) in self.schema.iter().enumerate() {
                if definition.data_type != DataType::String {
                    continue;
                }
                if let Some(Value::String(value)) = batch.value(row, column) {
                    text[column] += value.as_str().len();
                }
                let bytes = rows.div_ceil(8) + (rows + 1) * 4 + text[column];
                if bytes > crate::storage::unit::MAX_COLUMN_BYTES {
                    if row == start {
                        return Err(Error::Corrupt("import row cannot fit a native column"));
                    }
                    return Ok(row);
                }
            }
        }
        Ok(batch.row_count())
    }

    pub(crate) fn write(
        &mut self,
        batch: &InputBatch<'_>,
        range: Range<usize>,
        append: &mut Append<'_>,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        self.integers.clear();
        self.doubles.clear();
        self.dates.clear();
        self.validity.fill(0);
        let mut strings = allocate(
            self.strings,
            self.strings,
            "import STRING descriptors",
            self.reservation.bytes(),
        )?;
        let rows = range.len();
        for (column, definition) in self.schema.iter().enumerate() {
            cancel.check()?;
            self.starts[column] = match definition.data_type {
                DataType::Int64 => self.integers.len(),
                DataType::Double => self.doubles.len(),
                DataType::Date => self.dates.len(),
                DataType::String => strings.len(),
            };
            for (output, row) in range.clone().enumerate() {
                let value = batch.value(row, column).expect("bounded import cell");
                if !matches!(value, Value::Null) {
                    self.validity[column * self.bitmap_stride + output / 8] |= 1 << (output % 8);
                }
                match (definition.data_type, value) {
                    (DataType::Int64, Value::Int64(value)) => self.integers.push(value),
                    (DataType::Double, Value::Double(value)) => self.doubles.push(value),
                    (DataType::Date, Value::Date(value)) => self.dates.push(value),
                    (DataType::String, Value::String(value)) => strings.push(value.as_str()),
                    (DataType::Int64, Value::Null) => self.integers.push(0),
                    (DataType::Double, Value::Null) => self.doubles.push(0.0),
                    (DataType::Date, Value::Null) => self
                        .dates
                        .push(DateValue::from_days_since_unix_epoch(0).expect("epoch")),
                    (DataType::String, Value::Null) => strings.push(""),
                    _ => return Err(Error::Corrupt("import value differs from its schema")),
                }
            }
        }
        let mut inputs = [ColumnInput {
            values: ColumnValues::Int64(&[]),
            validity: &[],
        }; 64];
        for (column, definition) in self.schema.iter().enumerate() {
            let values = self.starts[column]..self.starts[column] + rows;
            inputs[column] = ColumnInput {
                values: match definition.data_type {
                    DataType::Int64 => ColumnValues::Int64(&self.integers[values]),
                    DataType::Double => ColumnValues::Double(&self.doubles[values]),
                    DataType::Date => ColumnValues::Date(&self.dates[values]),
                    DataType::String => ColumnValues::String(&strings[values]),
                },
                validity: &self.validity
                    [column * self.bitmap_stride..column * self.bitmap_stride + rows.div_ceil(8)],
            };
        }
        append.write_columns(&inputs[..self.schema.len()], cancel, effects)
    }
}
