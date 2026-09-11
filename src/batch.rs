//! Owned typed columns for bounded execution and borrowed result exchange.
use crate::Error;
use crate::fixed_text::StringValue as FixedKey;
use crate::frontend::{DataType, MAX_COLUMNS};
use crate::resources::{Reservation, allocate};
use crate::value::{DateValue, StringValue, Value};
use std::mem::size_of;

pub(crate) const ROWS: usize = 256;
const _: () = assert!(ROWS <= crate::scalar::MAX_ROWS);
pub(crate) const MAX_BYTES: u64 = (MAX_COLUMNS * (size_of::<Column>() + ROWS * 8)) as u64;

pub(crate) const MAX_TEXT_BYTES: usize = 65_536;
// A variable-text owner dominates every supported fixed-width column payload.
pub(crate) const MAX_BYTES_WITH_TEXT: u64 =
    (MAX_COLUMNS * (size_of::<Column>() + size_of::<TextColumn>() + MAX_TEXT_BYTES)) as u64;

#[derive(Clone, Copy)]
struct TextSpan {
    start: u32,
    end: u32,
}

struct TextColumn {
    spans: [TextSpan; ROWS],
    bytes: String,
}

impl TextColumn {
    fn new(capacity: usize, limit: u64) -> Result<Vec<Self>, Error> {
        let mut owner = allocate(1, 1, "text batch metadata", limit)?;
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
        owner.push(Self {
            spans: [TextSpan { start: 0, end: 0 }; ROWS],
            bytes,
        });
        Ok(owner)
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
        // Capacity was admitted before mutation. Replacement also consumes bytes
        // until clear; no growth or compaction occurs while filling a batch.
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
    Text(Vec<TextColumn>),
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

/// A batch and its admission move and drop together, independently of a producer.
pub(crate) struct OwnedBatch<'db> {
    pub(crate) batch: Batch,
    // Rust drops fields in declaration order: payloads before their reservation.
    reservation: Reservation<'db>,
}

impl<'db> OwnedBatch<'db> {
    pub(crate) fn new(
        types: &[DataType],
        reservation: &mut Reservation<'db>,
    ) -> Result<Self, Error> {
        Self::new_with_text(
            types,
            &[None; MAX_COLUMNS][..types.len().min(MAX_COLUMNS)],
            reservation,
        )
    }

    pub(crate) fn new_with_text(
        types: &[DataType],
        text: &[Option<usize>],
        reservation: &mut Reservation<'db>,
    ) -> Result<Self, Error> {
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

    pub(crate) fn required_bytes(types: &[DataType]) -> Result<u64, Error> {
        Self::required_bytes_with_text(types, &[None; MAX_COLUMNS][..types.len().min(MAX_COLUMNS)])
    }

    pub(crate) fn required_bytes_with_text(
        types: &[DataType],
        text: &[Option<usize>],
    ) -> Result<u64, Error> {
        if types.len() > MAX_COLUMNS || types.len() != text.len() {
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
                    .checked_add(size_of::<TextColumn>())
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
            &[None; MAX_COLUMNS][..types.len().min(MAX_COLUMNS)],
            limit,
        )
    }
    // The caller reserves required_bytes_with_text before either physical owner.
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

    pub(crate) fn clear(&mut self) {
        self.rows = 0;
        for column in &mut self.columns {
            column.valid.fill(0);
            if let Data::Text(text) = &mut column.data {
                text[0].bytes.clear();
            }
        }
    }

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
            Data::Text(v) => Value::String(StringValue::new(v[0].value(row))),
            Data::Date(v) => Value::Date(v[row]),
        })
    }
    // The producer publishes rows only after every demanded column is filled.
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

    pub(crate) fn numeric(
        &self,
        index: usize,
        identity: crate::frontend::SemanticColumn,
    ) -> Result<crate::scalar::NumericInput<'_>, Error> {
        use crate::scalar::{NumericInput, NumericValues};
        let column = self
            .columns
            .get(index)
            .ok_or(Error::Corrupt("numeric column missing"))?;
        let values = match &column.data {
            Data::Double(values) => NumericValues::Double(&values[..self.rows]),
            Data::Int64(values) => NumericValues::Int64(&values[..self.rows]),
            _ => return Err(Error::Corrupt("numeric batch type disagrees")),
        };
        NumericInput::new(
            identity,
            values,
            Some(&column.valid[..self.rows.div_ceil(64)]),
        )
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
            (Data::Text(v), Value::String(value)) => v[0].set(row, value.as_str())?,
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
    use crate::frontend::SourceColumn;

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
            Data::Text(v) => v[0].bytes.as_ptr(),
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
                Data::Text(v) => v[0].bytes.as_ptr(),
                _ => unreachable!(),
            },
            allocation
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
            // Exercise every validity bit, including all word boundaries.
            for column in [0, 2] {
                let before = if column == 0 {
                    Value::Double(f64::from_bits(bits[row % bits.len()]))
                } else {
                    Value::String(StringValue::from_byte(keys[row % keys.len()]).unwrap())
                };
                batch.set(row, column, Value::Null).unwrap();
                assert_eq!(batch.value(row, column), Some(Value::Null));
                if column == 0 {
                    assert!(batch.numeric(0, SourceColumn::QUANTITY.semantic()).is_err());
                } else {
                    assert!(batch.strings(2).is_err());
                }
                batch.set(row, column, before).unwrap();
            }
        }
        assert_eq!(
            batch
                .numeric(0, SourceColumn::QUANTITY.semantic())
                .unwrap()
                .len(),
            ROWS
        );
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
        assert!(
            batch
                .numeric(0, SourceColumn::QUANTITY.semantic())
                .unwrap()
                .len()
                == 0
        );
        batch.set(0, 0, Value::Null).unwrap();
        batch.publish_rows(1);
        assert_eq!(batch.value(0, 0), Some(Value::Null));
        assert_eq!(batch.value(1, 0), None);
        assert!(batch.numeric(0, SourceColumn::QUANTITY.semantic()).is_err());
    }
}
