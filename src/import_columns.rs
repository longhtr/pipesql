//! Lend native column slices while one decoded input batch is alive.
//!
//! Numeric vectors and validity bits are reused. Temporary STRING descriptors
//! borrow decoder text only during a write, so no second text arena is needed.
//! Their maximum capacity stays charged between writes; allocation can still
//! fail, in which case the importer aborts every earlier unit.

use crate::effects::Effects;
use crate::resources::{Reservation, allocate, buffer_charge};
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
        // Geometry is bounded by the decoder's validated 64 columns and at most 512 rows.
        // Charge native large-block reuse for each buffer, including STRING
        // descriptors that exist only while write borrows the decoded text.
        // Include the local views and partition counters as well as retained data.
        let requested = size_of::<Self>()
            + size_of::<[ColumnInput<'_>; 64]>()
            + size_of::<[ColumnDeclaration<'_>; 64]>()
            + size_of::<[usize; 64]>()
            + size_of::<Vec<&str>>()
            + [
                validity,
                counts[0] * size_of::<i64>(),
                counts[1] * size_of::<f64>(),
                counts[2] * size_of::<DateValue>(),
                counts[3] * size_of::<&str>(),
            ]
            .into_iter()
            .map(|bytes| buffer_charge(bytes).expect("bounded import buffer"))
            .sum::<usize>()
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
                if let Some(bytes) = batch.text_bytes(row, column) {
                    text[column] += bytes;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::input::Cell;
    use crate::resources::MemoryAuthority;

    #[test]
    fn fixed_width_batches_fit_without_splitting_and_observe_cancellation() {
        for capacity in [256, 512] {
            for width in [1, 3, 64] {
                let schema: Vec<_> = (0..width)
                    .map(|column| ColumnDeclaration {
                        name: "value",
                        data_type: [DataType::Int64, DataType::Double, DataType::Date][column % 3],
                        nullable: true,
                    })
                    .collect();
                let memory = MemoryAuthority::new(8_000_000);
                let columns = Columns::new(&memory, &schema, capacity).unwrap();
                let reserved = memory.reserved();
                for rows in [1, 17, usize::from(capacity)] {
                    let cells: Vec<_> = (0..rows * width)
                        .map(|cell| {
                            let row = cell / width;
                            let column = cell % width;
                            if (row + column) % 7 == 0 {
                                Cell::Null
                            } else {
                                match column % 3 {
                                    0 => Cell::Int64(i64::MIN + row as i64),
                                    1 => Cell::Double(-0.0),
                                    _ => Cell::Date(
                                        DateValue::from_days_since_unix_epoch(-1).unwrap(),
                                    ),
                                }
                            }
                        })
                        .collect();
                    let batch = InputBatch {
                        cells: &cells,
                        text: &[],
                        columns: width,
                    };
                    for start in [0, rows / 2] {
                        let cancel = CancellationToken::new();
                        assert_eq!(columns.next_end(&batch, start, &cancel).unwrap(), rows);
                        cancel.cancel();
                        assert!(matches!(
                            columns.next_end(&batch, start, &cancel),
                            Err(Error::Cancelled)
                        ));
                        assert_eq!(memory.reserved(), reserved);
                    }
                }
                drop(columns);
                assert_eq!(memory.reserved(), 0);
            }
        }
    }

    #[test]
    fn wide_conversion_admission_covers_large_descriptor_reuse_and_releases() {
        let schema = [ColumnDeclaration {
            name: "text",
            data_type: DataType::String,
            nullable: true,
        }; 64];
        let memory = MemoryAuthority::new(8_000_000);
        let columns = Columns::new(&memory, &schema, 512).unwrap();
        // 64 columns × 512 borrowed &str descriptors occupy 512 KiB. Darwin's
        // large-block cache can return a whole block up to the charged 1 MiB.
        let minimum = if cfg!(target_os = "macos") {
            1_048_576
        } else {
            524_288
        };
        assert!(memory.reserved() >= minimum);
        let required = memory.reserved();
        drop(columns);
        assert_eq!(memory.reserved(), 0);
        let short = MemoryAuthority::new(required - 1);
        assert!(matches!(
            Columns::new(&short, &schema, 512),
            Err(Error::Resource {
                owner: "import columns",
                ..
            })
        ));
        assert_eq!(short.reserved(), 0);
        let exact = MemoryAuthority::new(required);
        drop(Columns::new(&exact, &schema, 512).unwrap());
        assert_eq!(exact.reserved(), 0);
    }

    #[test]
    fn larger_borrowed_batch_still_splits_maximum_text_before_native_column_limit() {
        let schema = [ColumnDeclaration {
            name: "text",
            data_type: DataType::String,
            nullable: false,
        }];
        let text = vec![b'x'; 65_536];
        let cells = vec![
            Cell::Text {
                start: 0,
                end: text.len()
            };
            512
        ];
        let batch = InputBatch {
            cells: &cells,
            text: &text,
            columns: 1,
        };
        let memory = MemoryAuthority::new(1_000_000);
        let columns = Columns::new(&memory, &schema, 512).unwrap();
        let cancel = CancellationToken::new();
        let mut start = 0;
        while start < 512 {
            let end = columns.next_end(&batch, start, &cancel).unwrap();
            // Eight maximum strings alone fill 512 KiB; offsets and validity
            // mean at most seven can fit in one native column.
            assert_eq!(end, (start + 7).min(512));
            start = end;
        }
        cancel.cancel();
        assert!(matches!(
            columns.next_end(&batch, 0, &cancel),
            Err(Error::Cancelled)
        ));
    }
}
