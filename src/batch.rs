//! Hold a batch of rows for query operators and their callers.
//!
//! A batch stores values by column: all INT64 values together, all DOUBLE values
//! together, and so on. Operators can process these arrays without allocating an
//! object for every cell. Each column has one validity bit per row; a cleared
//! bit means SQL NULL, regardless of the value left in the array.
//!
//! Operators fill the columns, then call `publish_rows` to expose the completed
//! rows. `clear` makes the batch empty while retaining its buffers for reuse.
//! Results borrow those buffers, so a caller must finish reading a batch before
//! asking the query for its next step.
//!
//! `Batch` holds the columns and row count. `OwnedBatch` also holds the memory
//! reservation that pays for those buffers, keeping the two lifetimes together.

pub(crate) mod input;

use crate::Error;
use crate::resources::{Reservation, allocate};
use crate::value::fixed_text::StringValue as FixedKey;
use crate::value::{DataType, DateValue, StringValue, Value};
use std::mem::size_of;

pub(crate) const MAX_COLUMNS: usize = crate::schema::MAX_COLUMNS * 2;
const MAX_ROW_VALUES: usize = MAX_COLUMNS;

#[derive(Clone, Copy)]
pub(crate) enum NumericValues<'a> {
    Int64(&'a [i64]),
    Double(&'a [f64]),
    // Reuse checked intermediates without converting their INT64 or DOUBLE bits.
    Bits { values: &'a [u64], kind: DataType },
}

pub(crate) const ROWS: usize = 256;

#[cfg(test)]
pub(crate) const MAX_BYTES: u64 = (MAX_ROW_VALUES * (size_of::<Column>() + ROWS * 8)) as u64;

pub(crate) const MAX_TEXT_BYTES: usize = 65_536;
// A full text buffer and its row offsets need more space than any fixed-width
// column. Use that larger layout when calculating the maximum batch size.
pub(crate) const MAX_BYTES_WITH_TEXT: u64 =
    (MAX_ROW_VALUES * (size_of::<Column>() + ROWS * size_of::<TextSpan>() + MAX_TEXT_BYTES)) as u64;

#[derive(Clone, Copy)]
struct TextSpan {
    start: u32,
    end: u32,
}

struct TextColumn {
    // Each row stores its own start and end offsets into `bytes`, allowing rows
    // to be written out of order. Allocate the offsets separately: their size is
    // a power of two, avoiding allocator rounding for a larger combined object.
    spans: Vec<TextSpan>,
    bytes: String,
}

impl TextColumn {
    fn new(capacity: usize, limit: u64) -> Result<Self, Error> {
        let mut spans = allocate(ROWS, ROWS, "text batch spans", limit)?;
        spans.resize(ROWS, TextSpan { start: 0, end: 0 });
        let mut bytes = String::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| Error::Resource {
                owner: "text batch allocation",
                required: capacity as u64,
                limit,
            })?;
        if bytes.capacity() != capacity {
            return Err(Error::Resource {
                owner: "text batch capacity",
                required: bytes.capacity() as u64,
                limit: capacity as u64,
            });
        }
        Ok(Self { spans, bytes })
    }

    fn set(&mut self, row: usize, value: &str) -> Result<(), Error> {
        let start = self.bytes.len();
        let end = start.checked_add(value.len()).ok_or(Error::Resource {
            owner: "batch UTF-8 bytes",
            required: u64::MAX,
            limit: self.bytes.capacity() as u64,
        })?;
        if end > self.bytes.capacity() {
            return Err(Error::Resource {
                owner: "batch UTF-8 bytes",
                required: end as u64,
                limit: self.bytes.capacity() as u64,
            });
        }
        // Check capacity before changing either the bytes or the row offsets.
        // Replacement appends new text and leaves the old bytes until `clear`.
        // This avoids moving existing text or growing the allocation mid-batch.
        self.bytes.push_str(value);
        self.spans[row] = TextSpan {
            start: start as u32,
            end: end as u32,
        };
        Ok(())
    }

    fn value(&self, row: usize) -> &str {
        let span = self.spans[row];
        &self.bytes[span.start as usize..span.end as usize]
    }
}

enum Data {
    Double(Vec<f64>),
    Int64(Vec<i64>),
    String(Vec<FixedKey>),
    Text(TextColumn),
    Date(Vec<DateValue>),
}

struct Column {
    data: Data,
    valid: [u64; ROWS / 64],
}

impl Column {
    fn require_nonnull(&self, rows: usize) -> Result<(), &'static str> {
        let words = rows / 64;
        let remainder = rows % 64;
        if self.valid[..words].iter().any(|bits| *bits != u64::MAX)
            || (remainder != 0
                && self.valid[words] & ((1_u64 << remainder) - 1) != (1_u64 << remainder) - 1)
        {
            return Err("required batch input contains NULL");
        }
        Ok(())
    }
}

pub(crate) enum ColumnMut<'a> {
    Double(&'a mut [f64]),
    String(&'a mut [FixedKey]),
    Date(&'a mut [DateValue]),
}

pub(crate) struct Batch {
    columns: Vec<Column>,
    rows: usize,
}

/// A batch together with the memory reserved for its buffers.
/// Moving it between operators also moves responsibility for that reservation.
pub(crate) struct OwnedBatch<'db> {
    pub(crate) batch: Batch,
    // Field order matters: Rust must free the batch before releasing its bytes
    // back to the memory budget.
    reservation: Reservation<'db>,
}

impl<'db> OwnedBatch<'db> {
    pub(crate) fn new(
        types: &[DataType],
        reservation: &mut Reservation<'db>,
    ) -> Result<Self, Error> {
        Self::new_with_text(
            types,
            &[None; MAX_ROW_VALUES][..types.len().min(MAX_ROW_VALUES)],
            reservation,
        )
    }

    pub(crate) fn new_with_text(
        types: &[DataType],
        text: &[Option<usize>],
        reservation: &mut Reservation<'db>,
    ) -> Result<Self, Error> {
        // Transfer the reserved bytes before allocating. If construction fails,
        // this local reservation releases them after the partial batch is freed.
        let reservation = reservation.split(Batch::required_bytes_with_text(types, text)?)?;
        let batch = Batch::new_with_text(types, text, reservation.bytes())?;
        Ok(Self { batch, reservation })
    }

    pub(crate) fn memory_bytes(&self) -> u64 {
        self.reservation.bytes()
    }
}

impl std::ops::Deref for OwnedBatch<'_> {
    type Target = Batch;

    fn deref(&self) -> &Batch {
        &self.batch
    }
}

impl std::ops::DerefMut for OwnedBatch<'_> {
    fn deref_mut(&mut self) -> &mut Batch {
        &mut self.batch
    }
}

impl Batch {
    #[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
    pub(crate) const fn empty() -> Self {
        Self {
            columns: Vec::new(),
            rows: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn required_bytes(types: &[DataType]) -> Result<u64, Error> {
        Self::required_bytes_with_text(
            types,
            &[None; MAX_ROW_VALUES][..types.len().min(MAX_ROW_VALUES)],
        )
    }

    pub(crate) fn required_bytes_with_text(
        types: &[DataType],
        text: &[Option<usize>],
    ) -> Result<u64, Error> {
        if types.len() > MAX_ROW_VALUES || types.len() != text.len() {
            return Err(Error::Corrupt("batch column limit"));
        }
        let mut bytes = types
            .len()
            .checked_mul(size_of::<Column>())
            .ok_or(Error::Corrupt("batch metadata extent"))?;
        for (kind, capacity) in types.iter().zip(text) {
            if let Some(capacity) = capacity {
                if *kind != DataType::String {
                    return Err(Error::Corrupt("text capacity on non-STRING column"));
                }
                if *capacity > MAX_TEXT_BYTES {
                    return Err(Error::Resource {
                        owner: "batch text layout",
                        required: *capacity as u64,
                        limit: MAX_TEXT_BYTES as u64,
                    });
                }
                bytes = bytes
                    .checked_add(ROWS * size_of::<TextSpan>())
                    .and_then(|bytes| bytes.checked_add(*capacity))
                    .ok_or(Error::Corrupt("text batch extent"))?;
            }
            let width = match kind {
                DataType::Double => size_of::<f64>(),
                DataType::Int64 => size_of::<i64>(),
                DataType::String if capacity.is_some() => 0, // Row spans live in the text owner.
                DataType::String => size_of::<FixedKey>(),
                DataType::Date => size_of::<DateValue>(),
            };
            bytes = ROWS
                .checked_mul(width)
                .and_then(|column| bytes.checked_add(column))
                .ok_or(Error::Corrupt("batch value extent"))?;
        }
        u64::try_from(bytes).map_err(|_| Error::Corrupt("batch bytes do not fit"))
    }

    #[cfg(test)]
    pub(crate) fn new(types: &[DataType], limit: u64) -> Result<Self, Error> {
        Self::new_with_text(
            types,
            &[None; MAX_ROW_VALUES][..types.len().min(MAX_ROW_VALUES)],
            limit,
        )
    }
    // The caller must reserve `required_bytes_with_text` before this allocation.
    // A STRING capacity selects general UTF-8 storage; None selects legacy keys.
    pub(crate) fn new_with_text(
        types: &[DataType],
        text: &[Option<usize>],
        limit: u64,
    ) -> Result<Self, Error> {
        Self::required_bytes_with_text(types, text)?;
        let mut columns = allocate::<Column>(types.len(), types.len(), "batch columns", limit)?;
        for (kind, capacity) in types.iter().zip(text) {
            let data = match kind {
                DataType::Double => {
                    let mut v = allocate(ROWS, ROWS, "DOUBLE batch", limit)?;
                    v.resize(ROWS, 0.0);
                    Data::Double(v)
                }
                DataType::Int64 => {
                    let mut v = allocate(ROWS, ROWS, "INT64 batch", limit)?;
                    v.resize(ROWS, 0);
                    Data::Int64(v)
                }
                DataType::String if capacity.is_some() => {
                    Data::Text(TextColumn::new(capacity.expect("text capacity"), limit)?)
                }
                DataType::String => {
                    let mut v = allocate(ROWS, ROWS, "STRING batch", limit)?;
                    v.resize(ROWS, FixedKey::from_byte(33).expect("printable key"));
                    Data::String(v)
                }
                DataType::Date => {
                    let mut v = allocate(ROWS, ROWS, "DATE batch", limit)?;
                    v.resize(ROWS, DateValue::from_days(0).expect("epoch date"));
                    Data::Date(v)
                }
            };
            columns.push(Column {
                data,
                valid: [0; ROWS / 64],
            });
        }
        Ok(Self { columns, rows: 0 })
    }

    pub(crate) fn len(&self) -> usize {
        self.rows
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rows == 0
    }

    pub(crate) fn column_count(&self) -> usize {
        self.columns.len()
    }

    /// Discard the rows but keep the allocations. Old numeric values and offsets
    /// may remain in memory; the empty row count and cleared validity bits hide them.
    pub(crate) fn clear(&mut self) {
        self.rows = 0;
        for column in &mut self.columns {
            column.valid.fill(0);
            if let Data::Text(text) = &mut column.data {
                text.bytes.clear();
            }
        }
    }

    /// Make the first `rows` rows readable. The producer must fill all columns
    /// its consumer needs before calling this; no values are validated here.
    pub(crate) fn publish_rows(&mut self, rows: usize) {
        assert!(rows <= ROWS);
        self.rows = rows;
    }

    pub(crate) fn value(&self, row: usize, column: usize) -> Option<Value<'_>> {
        if row >= self.rows {
            return None;
        }
        let column = self.columns.get(column)?;
        if column.valid[row / 64] & (1 << (row % 64)) == 0 {
            return Some(Value::Null);
        }
        Some(match &column.data {
            Data::Double(v) => Value::Double(v[row]),
            Data::Int64(v) => Value::Int64(v[row]),
            Data::String(v) => Value::String(StringValue::new(v[row].as_str())),
            Data::Text(v) => Value::String(StringValue::new(v.value(row))),
            Data::Date(v) => Value::Date(v[row]),
        })
    }

    pub(crate) fn has_text(&self) -> bool {
        self.columns
            .iter()
            .any(|column| matches!(column.data, Data::Text(_)))
    }

    /// Borrow the bits that distinguish present values from NULLs.
    /// Reject a mismatched type or NULL in a required column before lending them.
    pub(crate) fn validity(
        &self,
        index: usize,
        expected_type: DataType,
        nullable: bool,
    ) -> Result<&[u64; ROWS / 64], Error> {
        let column = self
            .columns
            .get(index)
            .ok_or(Error::Corrupt("validity column missing"))?;
        let kind = match &column.data {
            Data::Double(_) => DataType::Double,
            Data::Int64(_) => DataType::Int64,
            Data::String(_) | Data::Text(_) => DataType::String,
            Data::Date(_) => DataType::Date,
        };
        if kind != expected_type
            || (!nullable
                && (0..self.rows).any(|row| column.valid[row / 64] & (1 << (row % 64)) == 0))
        {
            return Err(Error::Corrupt(
                "validity column disagrees with semantic type",
            ));
        }
        Ok(&column.valid)
    }

    // Mark these rows present and lend their value array for the producer to fill.
    // The producer must fill every needed column before calling `publish_rows`.
    pub(crate) fn nonnull_column(
        &mut self,
        column: usize,
        rows: usize,
    ) -> Result<ColumnMut<'_>, &'static str> {
        if rows > ROWS {
            return Err("batch row limit");
        }
        let column = self.columns.get_mut(column).ok_or("batch column missing")?;
        column.valid.fill(u64::MAX);
        Ok(match &mut column.data {
            Data::Double(v) => ColumnMut::Double(&mut v[..rows]),
            Data::Int64(_) => return Err("INT64 is not an admitted stored source"),
            Data::String(v) => ColumnMut::String(&mut v[..rows]),
            Data::Text(_) => return Err("text batches require bounded cell writes"),
            Data::Date(v) => ColumnMut::Date(&mut v[..rows]),
        })
    }

    pub(crate) fn numeric(&self, index: usize) -> Result<(NumericValues<'_>, &[u64]), Error> {
        let column = self
            .columns
            .get(index)
            .ok_or(Error::Corrupt("numeric column missing"))?;
        let values = match &column.data {
            Data::Double(values) => NumericValues::Double(&values[..self.rows]),
            Data::Int64(values) => NumericValues::Int64(&values[..self.rows]),
            _ => return Err(Error::Corrupt("numeric batch type disagrees")),
        };
        Ok((values, &column.valid[..self.rows.div_ceil(64)]))
    }

    pub(crate) fn strings(&self, column: usize) -> Result<&[FixedKey], &'static str> {
        let column = self.columns.get(column).ok_or("key column missing")?;
        column.require_nonnull(self.rows)?;
        match &column.data {
            Data::String(v) => Ok(&v[..self.rows]),
            _ => Err("key batch type disagrees"),
        }
    }

    pub(crate) fn set(&mut self, row: usize, column: usize, value: Value<'_>) -> Result<(), Error> {
        if row >= ROWS {
            return Err(Error::Corrupt("batch row limit"));
        }
        let column = self
            .columns
            .get_mut(column)
            .ok_or(Error::Corrupt("batch column missing"))?;
        let mask = 1 << (row % 64);
        if matches!(value, Value::Null) {
            column.valid[row / 64] &= !mask;
            return Ok(());
        }
        match (&mut column.data, value) {
            (Data::Double(v), Value::Double(value)) => v[row] = value,
            (Data::Int64(v), Value::Int64(value)) => v[row] = value,
            (Data::String(v), Value::String(value)) => {
                let bytes = value.as_str().as_bytes();
                let key = if bytes.len() == 1 {
                    FixedKey::from_byte(bytes[0])
                } else {
                    None
                }
                .ok_or(Error::Corrupt("value outside fixed key domain"))?;
                v[row] = key;
            }
            (Data::Text(v), Value::String(value)) => v.set(row, value.as_str())?,
            (Data::Date(v), Value::Date(value)) => v[row] = value,
            _ => return Err(Error::Corrupt("batch output type disagrees")),
        }
        column.valid[row / 64] |= mask;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_admission_transfer_preserves_limit_and_rejects_short_reservation() {
        let types = [DataType::Int64];
        let bytes = Batch::required_bytes(&types).unwrap();
        let memory = crate::resources::MemoryAuthority::new(bytes);
        let mut short = memory.reserve(bytes - 1, "short batch test").unwrap();
        assert!(OwnedBatch::new(&types, &mut short).is_err());
        assert_eq!(short.bytes(), bytes - 1);
        assert_eq!(memory.reserved(), bytes - 1);
        drop(short);
        let mut parent = memory.reserve(bytes, "batch transfer test").unwrap();
        let batch = OwnedBatch::new(&types, &mut parent).unwrap();
        assert_eq!(parent.bytes(), 0);
        drop(parent);
        assert_eq!(memory.reserved(), bytes);
        assert!(matches!(
            memory.reserve(1, "competing owner"),
            Err(Error::Resource { .. })
        ));
        drop(batch);
        assert_eq!(memory.reserved(), 0);
    }

    #[test]
    fn utf8_capacity_refusal_and_reuse_preserve_owned_values() {
        let types = [DataType::String, DataType::String];
        let capacities = [Some(MAX_TEXT_BYTES), Some(0)];
        let bytes = Batch::required_bytes_with_text(&types, &capacities).unwrap();
        let memory = crate::resources::MemoryAuthority::new(bytes);
        let mut short = memory.reserve(bytes - 1, "short text batch test").unwrap();
        assert!(matches!(
            OwnedBatch::new_with_text(&types, &capacities, &mut short),
            Err(Error::Corrupt("reservation split exceeds admitted bytes"))
        ));
        assert_eq!(short.bytes(), bytes - 1);
        assert_eq!(memory.reserved(), bytes - 1);
        drop(short);
        assert_eq!(memory.reserved(), 0);
        let mut reservation = memory.reserve(bytes, "text batch test").unwrap();
        let mut batch = OwnedBatch::new_with_text(&types, &capacities, &mut reservation).unwrap();
        let full = "雪".repeat(MAX_TEXT_BYTES / 3) + "x";
        assert_eq!(full.len(), MAX_TEXT_BYTES);
        batch
            .set(0, 0, Value::String(StringValue::new(&full)))
            .unwrap();
        batch
            .set(0, 1, Value::String(StringValue::new("")))
            .unwrap();
        batch.set(1, 0, Value::Null).unwrap();
        batch.publish_rows(2);
        let allocation = match &batch.columns[0].data {
            Data::Text(v) => {
                assert_eq!(v.spans.len(), ROWS);
                assert_eq!(v.spans.capacity(), ROWS);
                (v.bytes.as_ptr(), v.spans.as_ptr())
            }
            _ => unreachable!(),
        };
        assert!(
            matches!(batch.set(0, 0, Value::String(StringValue::new("a"))), Err(Error::Resource { required, limit, .. }) if required == MAX_TEXT_BYTES as u64 + 1 && limit == MAX_TEXT_BYTES as u64)
        );
        assert_eq!(
            batch.value(0, 0),
            Some(Value::String(StringValue::new(&full)))
        );
        assert_eq!(batch.value(0, 1), Some(Value::String(StringValue::new(""))));
        assert_eq!(batch.value(1, 0), Some(Value::Null));
        batch.clear();
        assert!(batch.is_empty());
        batch
            .set(0, 0, Value::String(StringValue::new("é")))
            .unwrap();
        batch.set(0, 1, Value::Null).unwrap();
        batch.publish_rows(1);
        assert_eq!(
            batch.value(0, 0),
            Some(Value::String(StringValue::new("é")))
        );
        assert_eq!(batch.value(0, 1), Some(Value::Null));
        assert_eq!(
            match &batch.columns[0].data {
                Data::Text(v) => (v.bytes.as_ptr(), v.spans.as_ptr()),
                _ => unreachable!(),
            },
            allocation
        );
        // Write the last row before replacing the first. A row's start cannot
        // be inferred from the previous row's end when writes occur out of order.
        batch
            .set(ROWS - 1, 0, Value::String(StringValue::new("雪")))
            .unwrap();
        batch
            .set(0, 0, Value::String(StringValue::new("last write")))
            .unwrap();
        batch.publish_rows(ROWS);
        assert_eq!(
            batch.value(ROWS - 1, 0),
            Some(Value::String(StringValue::new("雪")))
        );
        assert_eq!(
            batch.value(0, 0),
            Some(Value::String(StringValue::new("last write")))
        );
        assert!(
            Batch::required_bytes_with_text(&[DataType::String], &[Some(MAX_TEXT_BYTES + 1)])
                .is_err()
        );
        assert!(Batch::required_bytes_with_text(&[DataType::Double], &[Some(1)]).is_err());
        assert!(Batch::required_bytes_with_text(&types, &[]).is_err());
        drop(batch);
        drop(reservation);
        assert_eq!(memory.reserved(), 0);
    }

    #[test]
    fn typed_columns_preserve_values_null_bits_and_reused_prefixes() {
        let mut batch = Batch::new(
            &[
                DataType::Double,
                DataType::Int64,
                DataType::String,
                DataType::Date,
            ],
            100_000,
        )
        .unwrap();
        let bits = [
            0,
            1_u64 << 63,
            1,
            0x7ff0000000000000,
            0x7ff0000000000001,
            0x7ff8000000000001,
        ];
        let keys: Vec<_> = (b' '..=b'~').filter(|byte| *byte != b'|').collect();
        for row in 0..ROWS {
            batch
                .set(
                    row,
                    0,
                    Value::Double(f64::from_bits(bits[row % bits.len()])),
                )
                .unwrap();
            batch
                .set(
                    row,
                    1,
                    Value::Int64(if row % 2 == 0 { i64::MIN } else { i64::MAX }),
                )
                .unwrap();
            batch
                .set(
                    row,
                    2,
                    Value::String(StringValue::from_byte(keys[row % keys.len()]).unwrap()),
                )
                .unwrap();
            batch
                .set(
                    row,
                    3,
                    Value::Date(DateValue::from_days(row as i32 - 128).unwrap()),
                )
                .unwrap();
        }
        batch.publish_rows(ROWS);
        for row in 0..ROWS {
            let Some(Value::Double(value)) = batch.value(row, 0) else {
                panic!("DOUBLE")
            };
            assert_eq!(value.to_bits(), bits[row % bits.len()]);
            assert_eq!(
                batch.value(row, 1),
                Some(Value::Int64(if row % 2 == 0 { i64::MIN } else { i64::MAX }))
            );
            assert_eq!(
                batch.value(row, 3),
                Some(Value::Date(DateValue::from_days(row as i32 - 128).unwrap()))
            );
            // Change each row to NULL and back, crossing every 64-bit bitmap
            // boundary. Typed non-NULL access must reject the temporary NULL.
            for column in [0, 2] {
                let before = if column == 0 {
                    Value::Double(f64::from_bits(bits[row % bits.len()]))
                } else {
                    Value::String(StringValue::from_byte(keys[row % keys.len()]).unwrap())
                };
                batch.set(row, column, Value::Null).unwrap();
                assert_eq!(batch.value(row, column), Some(Value::Null));
                if column == 0 {
                    assert!(batch.validity(0, DataType::Double, false).is_err());
                } else {
                    assert!(batch.strings(2).is_err());
                }
                batch.set(row, column, before).unwrap();
            }
        }
        batch.validity(0, DataType::Double, false).unwrap();
        let (NumericValues::Double(values), valid) = batch.numeric(0).unwrap() else {
            panic!("expected DOUBLE storage");
        };
        assert_eq!(values.len(), ROWS);
        assert_eq!(valid.len(), ROWS / 64);
        assert_eq!(batch.strings(2).unwrap().len(), ROWS);
        assert_eq!(batch.value(ROWS, 0), None);
        assert_eq!(batch.value(0, 4), None);
        assert!(batch.set(ROWS, 0, Value::Null).is_err());
        assert!(batch.nonnull_column(0, ROWS + 1).is_err());
        let Some(Value::Int64(before)) = batch.value(0, 1) else {
            panic!("INT64");
        };
        assert!(batch.set(0, 1, Value::Double(1.0)).is_err());
        assert_eq!(batch.value(0, 1), Some(Value::Int64(before)));
        batch.clear();
        assert_eq!(batch.value(0, 0), None);
        batch.validity(0, DataType::Double, false).unwrap();
        let (NumericValues::Double(values), valid) = batch.numeric(0).unwrap() else {
            panic!("expected DOUBLE storage");
        };
        assert!(values.is_empty());
        assert!(valid.is_empty());
        batch.set(0, 0, Value::Null).unwrap();
        batch.publish_rows(1);
        assert_eq!(batch.value(0, 0), Some(Value::Null));
        assert_eq!(batch.value(1, 0), None);
        assert!(batch.validity(0, DataType::Double, false).is_err());
    }
}
