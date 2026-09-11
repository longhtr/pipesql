//! Typed row frames: field layout, SQL key comparison, and checked encoding.
use super::io::{Io, ReadAt, ReadBuffer};
use super::{MAX_FRAME_BYTES, RECORD_HEADER, RECORD_MAGIC};
use crate::batch::Batch;
use crate::execution::{MAX_AGGREGATE_ROWS, planning};
use crate::frontend::{
    self, DataType, Direction, MAX_AGGREGATE_COLUMNS, MAX_COLUMNS, NullPlacement, SemanticColumn,
};
use crate::resources::allocate;
use crate::value::{DateValue, StringValue, Value};
use crate::{Error, storage_format};
use std::cmp::Ordering;

#[derive(Clone, Copy)]
pub(in crate::execution) struct KeyColumn {
    pub(in crate::execution) input: usize,
    pub(in crate::execution) kind: DataType,
    pub(in crate::execution) nullable: bool,
    pub(in crate::execution) direction: Direction,
    pub(in crate::execution) nulls: NullPlacement,
}

pub(in crate::execution) struct RowLayout {
    pub(in crate::execution) columns: [KeyColumn; MAX_COLUMNS],
    pub(in crate::execution) count: usize,
    pub(in crate::execution) key_count: usize,
    pub(in crate::execution) max_bytes: usize,
    pub(in crate::execution) key_max_bytes: usize,
    pub(in crate::execution) layout: u32,
}

impl RowLayout {
    pub(super) fn for_join(
        inputs: impl Iterator<Item = SemanticColumn>,
        key: usize,
    ) -> Result<Self, Error> {
        Self::for_order(
            inputs,
            &[planning::OrderColumn {
                column: u8::try_from(key).map_err(|_| Error::Corrupt("join key position"))?,
                direction: Direction::Ascending,
                nulls: NullPlacement::First,
            }],
        )
    }
    // Store each field once. Ordering fields form a unique prefix; remaining
    // fields retain payloads. Repeated key identities cannot refine a tie.
    pub(super) fn for_order(
        inputs: impl Iterator<Item = SemanticColumn>,
        order: &[planning::OrderColumn],
    ) -> Result<Self, Error> {
        let mut columns = [KeyColumn {
            input: 0,
            kind: DataType::Int64,
            nullable: false,
            direction: Direction::Ascending,
            nulls: NullPlacement::First,
        }; MAX_COLUMNS];
        let mut count = 0;
        let mut max_bytes = 0_usize;
        for column in inputs {
            if count == MAX_COLUMNS {
                return Err(Error::Corrupt("sorted row column bound"));
            }
            columns[count] = KeyColumn {
                input: count,
                kind: column.data_type(),
                nullable: column.nullable(),
                direction: Direction::Ascending,
                nulls: NullPlacement::First,
            };
            max_bytes = max_bytes
                .checked_add(Self::field_bytes(column.data_type()))
                .ok_or(Error::Corrupt("sorted row byte bound"))?;
            count += 1;
        }
        if order.is_empty() || order.len() > frontend::MAX_ORDER_ITEMS {
            return Err(Error::Corrupt("sorted key count"));
        }
        let mut key_count = 0;
        let mut key_max_bytes = 0_usize;
        for key in order {
            let input = usize::from(key.column);
            if input >= count {
                return Err(Error::Corrupt("sort key outside input"));
            }
            if columns[..key_count]
                .iter()
                .any(|column| column.input == input)
            {
                continue;
            }
            let position = columns[key_count..count]
                .iter()
                .position(|column| column.input == input)
                .ok_or(Error::Corrupt("sort field permutation"))?
                + key_count;
            columns.swap(key_count, position);
            columns[key_count].direction = key.direction;
            columns[key_count].nulls = key.nulls;
            key_max_bytes = key_max_bytes
                .checked_add(Self::field_bytes(columns[key_count].kind))
                .ok_or(Error::Corrupt("sorted key byte bound"))?;
            key_count += 1;
        }
        // The transient frame belongs to this permutation and comparison policy.
        let mut layout = [0_u8; 2 + MAX_COLUMNS * 5];
        layout[0] = count as u8;
        layout[1] = key_count as u8;
        for (index, column) in columns[..count].iter().enumerate() {
            let at = 2 + index * 5;
            layout[at] = column.input as u8;
            layout[at + 1] = match column.kind {
                DataType::Int64 => 1,
                DataType::Double => 2,
                DataType::Date => 3,
                DataType::String => 4,
            };
            layout[at + 2] = u8::from(column.nullable);
            layout[at + 3] = u8::from(column.direction == Direction::Descending);
            layout[at + 4] = u8::from(column.nulls == NullPlacement::Last);
        }
        Ok(Self {
            columns,
            count,
            key_count,
            max_bytes,
            key_max_bytes,
            layout: storage_format::crc32c(&layout),
        })
    }

    fn field_bytes(kind: DataType) -> usize {
        match kind {
            DataType::Int64 | DataType::Double => 9,
            DataType::Date => 5,
            DataType::String => 5 + crate::batch::MAX_TEXT_BYTES,
        }
    }

    pub(in crate::execution) fn encode(
        &self,
        batch: &Batch,
        row: usize,
        output: &mut Vec<u8>,
    ) -> Result<(), Error> {
        output.clear();
        self.append_row(batch, row, output)
    }

    pub(in crate::execution) fn append_row(
        &self,
        batch: &Batch,
        row: usize,
        output: &mut Vec<u8>,
    ) -> Result<(), Error> {
        let start = output.len();
        for column in &self.columns[..self.count] {
            let value = batch
                .value(row, column.input)
                .ok_or(Error::Corrupt("group input row absent"))?;
            append_value(output, value, column.kind, column.nullable)?;
        }
        assert!(output.len() - start <= self.max_bytes);
        Ok(())
    }

    pub(super) fn validate(&self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() > self.max_bytes {
            return Err(Error::Corrupt("group key exceeds byte bound"));
        }
        let mut remaining = bytes;
        for column in &self.columns[..self.count] {
            read_value(&mut remaining, column.kind, column.nullable)?;
        }
        if !remaining.is_empty() {
            return Err(Error::Corrupt("group key trailing bytes"));
        }
        Ok(())
    }

    pub(in crate::execution) fn value<'a>(
        &self,
        bytes: &'a [u8],
        column: usize,
    ) -> Result<Value<'a>, Error> {
        if column >= self.count {
            return Err(Error::Corrupt("group key column bound"));
        }
        let mut remaining = bytes;
        for (index, spec) in self.columns[..self.count].iter().enumerate() {
            let value = read_value(&mut remaining, spec.kind, spec.nullable)?;
            if index == column {
                return Ok(value);
            }
        }
        unreachable!("validated group column exists")
    }

    pub(in crate::execution) fn compare(
        &self,
        left: &[u8],
        right: &[u8],
    ) -> Result<Ordering, Error> {
        let (mut left, mut right) = (left, right);
        for column in &self.columns[..self.key_count] {
            let left_value = read_value(&mut left, column.kind, column.nullable)?;
            let right_value = read_value(&mut right, column.kind, column.nullable)?;
            let order = compare_values(left_value, right_value);
            // The scalar comparator is NULL-first. Direction reverses only
            // non-NULL values; NULL placement has its own independent policy.
            let reverse = if matches!(left_value, Value::Null) || matches!(right_value, Value::Null)
            {
                column.nulls == NullPlacement::Last
            } else {
                column.direction == Direction::Descending
            };
            let order = if reverse { order.reverse() } else { order };
            if order != Ordering::Equal {
                return Ok(order);
            }
        }
        Ok(Ordering::Equal)
    }

    pub(super) fn sort_prefix<'a>(&self, bytes: &'a [u8]) -> Result<&'a [u8], Error> {
        if self.key_count == self.count {
            return Ok(bytes);
        }
        let mut remaining = bytes;
        for column in &self.columns[..self.key_count] {
            read_value(&mut remaining, column.kind, column.nullable)?;
        }
        Ok(&bytes[..bytes.len() - remaining.len()])
    }

    pub(in crate::execution) fn hash(&self, bytes: &[u8]) -> Result<u64, Error> {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        let mut remaining = bytes;
        for column in &self.columns[..self.key_count] {
            let value = read_value(&mut remaining, column.kind, column.nullable)?;
            hash_bytes(&mut hash, &[u8::from(value != Value::Null)]);
            match value {
                Value::Null => {}
                Value::Int64(value) => hash_bytes(&mut hash, &value.to_le_bytes()),
                Value::Date(value) => {
                    hash_bytes(&mut hash, &value.days_since_unix_epoch().to_le_bytes())
                }
                Value::Double(value) => {
                    // Equality, ordering and hashing share these SQL equivalence
                    // classes. The stored key still retains its first raw bits.
                    let bits = if value.is_nan() {
                        f64::NAN.to_bits()
                    } else if value == 0.0 {
                        0
                    } else {
                        value.to_bits()
                    };
                    hash_bytes(&mut hash, &bits.to_le_bytes());
                }
                Value::String(value) => {
                    hash_bytes(&mut hash, &(value.as_str().len() as u32).to_le_bytes());
                    hash_bytes(&mut hash, value.as_str().as_bytes());
                }
            }
        }
        Ok(hash)
    }
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    // This is a deterministic lookup hash, not authentication. A bounded probe
    // count must end the hash attempt even for deliberately colliding keys.
    for byte in bytes {
        *hash = (*hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3);
    }
}

pub(super) fn compare_values(left: Value<'_>, right: Value<'_>) -> Ordering {
    match (left, right) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Less,
        (_, Value::Null) => Ordering::Greater,
        (Value::Int64(left), Value::Int64(right)) => left.cmp(&right),
        (Value::Date(left), Value::Date(right)) => left
            .days_since_unix_epoch()
            .cmp(&right.days_since_unix_epoch()),
        (Value::String(left), Value::String(right)) => left.as_str().cmp(right.as_str()),
        (Value::Double(left), Value::Double(right)) => match (left.is_nan(), right.is_nan()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => left
                .partial_cmp(&right)
                .expect("non-NaN values are ordered"),
        },
        _ => unreachable!("group values have one validated column type"),
    }
}

pub(in crate::execution) fn append_bytes(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), Error> {
    let end = output
        .len()
        .checked_add(bytes.len())
        .ok_or(Error::Corrupt("group buffer length"))?;
    if end > output.capacity() {
        return Err(Error::Resource {
            owner: "group record buffer",
            required: end as u64,
            limit: output.capacity() as u64,
        });
    }
    output.extend_from_slice(bytes);
    Ok(())
}

pub(in crate::execution) fn append_value(
    output: &mut Vec<u8>,
    value: Value<'_>,
    kind: DataType,
    nullable: bool,
) -> Result<(), Error> {
    if value == Value::Null {
        if !nullable {
            return Err(Error::Corrupt("NULL in required group value"));
        }
        return append_bytes(output, &[0]);
    }
    append_bytes(output, &[1])?;
    match (kind, value) {
        (DataType::Int64, Value::Int64(value)) => append_bytes(output, &value.to_le_bytes()),
        (DataType::Double, Value::Double(value)) => {
            append_bytes(output, &value.to_bits().to_le_bytes())
        }
        (DataType::Date, Value::Date(value)) => {
            append_bytes(output, &value.days_since_unix_epoch().to_le_bytes())
        }
        (DataType::String, Value::String(value)) => {
            let value = value.as_str().as_bytes();
            if value.len() > crate::batch::MAX_TEXT_BYTES {
                return Err(Error::Corrupt("group text bound"));
            }
            append_bytes(output, &(value.len() as u32).to_le_bytes())?;
            append_bytes(output, value)
        }
        _ => Err(Error::Corrupt("group value type")),
    }
}

fn take<'a>(bytes: &mut &'a [u8], count: usize) -> Result<&'a [u8], Error> {
    let (value, rest) = bytes
        .split_at_checked(count)
        .ok_or(Error::Corrupt("truncated group record"))?;
    *bytes = rest;
    Ok(value)
}

pub(in crate::execution) fn read_value<'a>(
    bytes: &mut &'a [u8],
    kind: DataType,
    nullable: bool,
) -> Result<Value<'a>, Error> {
    match take(bytes, 1)?[0] {
        0 if nullable => return Ok(Value::Null),
        1 => {}
        _ => return Err(Error::Corrupt("group value validity")),
    }
    Ok(match kind {
        DataType::Int64 => Value::Int64(i64::from_le_bytes(
            take(bytes, 8)?.try_into().expect("eight bytes"),
        )),
        DataType::Double => Value::Double(f64::from_bits(u64::from_le_bytes(
            take(bytes, 8)?.try_into().expect("eight bytes"),
        ))),
        DataType::Date => Value::Date(
            DateValue::from_days_since_unix_epoch(i32::from_le_bytes(
                take(bytes, 4)?.try_into().expect("four bytes"),
            ))
            .ok_or(Error::Corrupt("group date range"))?,
        ),
        DataType::String => {
            let len = u32::from_le_bytes(take(bytes, 4)?.try_into().expect("four bytes")) as usize;
            if len > crate::batch::MAX_TEXT_BYTES {
                return Err(Error::Corrupt("group text extent"));
            }
            let value = std::str::from_utf8(take(bytes, len)?)
                .map_err(|_| Error::Corrupt("group UTF-8"))?;
            Value::String(StringValue::new(value))
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::execution) struct ArgumentShape {
    pub(in crate::execution) count: usize,
    pub(in crate::execution) nonnull: u16,
    pub(in crate::execution) integers: u16,
}

impl ArgumentShape {
    pub(in crate::execution) fn layout(self, keys: &RowLayout) -> u32 {
        assert!(self.count <= MAX_AGGREGATE_COLUMNS);
        let mut bytes = [0; 9];
        bytes[..4].copy_from_slice(&keys.layout.to_le_bytes());
        bytes[4] = self.count as u8;
        bytes[5..7].copy_from_slice(&self.nonnull.to_le_bytes());
        bytes[7..9].copy_from_slice(&self.integers.to_le_bytes());
        storage_format::crc32c(&bytes)
    }
}

pub(in crate::execution) struct SortRecord {
    pub(in crate::execution) bytes: Vec<u8>,
}

impl SortRecord {
    pub(super) fn encode_row(
        &mut self,
        layout: &RowLayout,
        source: &Batch,
        row: usize,
        ordinal: u64,
    ) -> Result<(), Error> {
        let required = ordinal
            .checked_add(1)
            .ok_or(Error::Corrupt("sort input ordinal overflow"))?;
        if required > MAX_AGGREGATE_ROWS {
            return Err(Error::Resource {
                owner: "sorted input rows",
                required,
                limit: MAX_AGGREGATE_ROWS,
            });
        }
        self.bytes.clear();
        append_bytes(&mut self.bytes, &[0; RECORD_HEADER])?;
        layout.append_row(source, row, &mut self.bytes)?;
        let length = self.bytes.len() - RECORD_HEADER;
        let arguments = ArgumentShape {
            count: 0,
            nonnull: 0,
            integers: 0,
        };
        self.finish(arguments.layout(layout), length, ordinal, &[])
    }

    pub(in crate::execution) fn new(capacity: usize, charge: u64) -> Result<Self, Error> {
        if !(RECORD_HEADER..=MAX_FRAME_BYTES).contains(&capacity) {
            return Err(Error::Corrupt("argument record capacity"));
        }
        Ok(Self {
            bytes: allocate(capacity, capacity, "group argument record", charge)?,
        })
    }

    #[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
    pub(super) fn encode(
        &mut self,
        keys: &RowLayout,
        shape: ArgumentShape,
        key: &[u8],
        ordinal: u64,
        arguments: &[Option<u64>],
    ) -> Result<(), Error> {
        if arguments.len() > MAX_AGGREGATE_COLUMNS
            || arguments.len() != shape.count
            || (shape.nonnull | shape.integers) >> shape.count != 0
            || ordinal >= MAX_AGGREGATE_ROWS
        {
            return Err(Error::Corrupt("group argument shape"));
        }
        keys.validate(key)?;
        self.bytes.clear();
        append_bytes(&mut self.bytes, &[0; RECORD_HEADER])?;
        append_bytes(&mut self.bytes, key)?;
        self.finish(shape.layout(keys), key.len(), ordinal, arguments)
    }

    pub(in crate::execution) fn finish(
        &mut self,
        layout: u32,
        key_len: usize,
        ordinal: u64,
        arguments: &[Option<u64>],
    ) -> Result<(), Error> {
        assert_eq!(self.bytes.len(), RECORD_HEADER + key_len);
        assert!(arguments.len() <= MAX_AGGREGATE_COLUMNS && ordinal < MAX_AGGREGATE_ROWS);
        let mut valid = 0_u16;
        for (index, value) in arguments.iter().enumerate() {
            valid |= u16::from(value.is_some()) << index;
            append_bytes(&mut self.bytes, &value.unwrap_or(0).to_le_bytes())?;
        }
        self.bytes[..8].copy_from_slice(RECORD_MAGIC);
        self.bytes[8..16].copy_from_slice(&ordinal.to_le_bytes());
        self.bytes[16..20].copy_from_slice(&(key_len as u32).to_le_bytes());
        self.bytes[20..24].copy_from_slice(&layout.to_le_bytes());
        self.bytes[24..26].copy_from_slice(&valid.to_le_bytes());
        self.bytes[26] = arguments.len() as u8;
        // The checksum field is zero while computing the complete frame CRC.
        let crc = storage_format::crc32c(&self.bytes);
        self.bytes[28..32].copy_from_slice(&crc.to_le_bytes());
        Ok(())
    }

    pub(super) fn read(
        &mut self,
        reader: &mut ReadBuffer,
        at: ReadAt,
        keys: &RowLayout,
        arguments: ArgumentShape,
        io: &mut Io<'_, '_>,
    ) -> Result<(), Error> {
        if arguments.count > MAX_AGGREGATE_COLUMNS
            || (arguments.nonnull | arguments.integers) >> arguments.count != 0
        {
            return Err(Error::Corrupt("group argument type shape"));
        }
        let ReadAt {
            slot,
            offset,
            limit,
        } = at;

        let states = arguments.count;
        self.bytes.resize(RECORD_HEADER, 0);
        reader.read(
            ReadAt {
                slot,
                offset,
                limit,
            },
            &mut self.bytes,
            io,
        )?;
        if &self.bytes[..8] != RECORD_MAGIC
            || self.bytes[27] != 0
            || u32::from_le_bytes(self.bytes[20..24].try_into().expect("layout bytes"))
                != arguments.layout(keys)
            || usize::from(self.bytes[26]) != states
            || states > MAX_AGGREGATE_COLUMNS
            || self.ordinal() >= MAX_AGGREGATE_ROWS
        {
            return Err(Error::Corrupt("group argument header"));
        }
        let key_len = self.key_len();
        let end = RECORD_HEADER
            .checked_add(key_len)
            .and_then(|n| n.checked_add(states * 8))
            .ok_or(Error::Corrupt("group argument extent"))?;
        if key_len > keys.max_bytes || end > self.bytes.capacity() {
            return Err(Error::Corrupt("group argument exceeds admitted buffer"));
        }
        self.bytes.resize(end, 0);
        let payload = offset
            .checked_add(RECORD_HEADER as u64)
            .ok_or(Error::Corrupt("group read offset"))?;
        reader.read(
            ReadAt {
                slot,
                offset: payload,
                limit,
            },
            &mut self.bytes[RECORD_HEADER..],
            io,
        )?;
        let expected = u32::from_le_bytes(self.bytes[28..32].try_into().expect("CRC bytes"));
        self.bytes[28..32].fill(0);
        let actual = storage_format::crc32c(&self.bytes);
        self.bytes[28..32].copy_from_slice(&expected.to_le_bytes());
        if expected != actual {
            return Err(Error::Corrupt("group argument checksum"));
        }
        keys.validate(self.key())?;
        let valid = self.valid();
        if valid & arguments.nonnull != arguments.nonnull {
            return Err(Error::Corrupt("required group argument is NULL"));
        }
        if valid >> states != 0 {
            return Err(Error::Corrupt("group argument validity tail"));
        }
        for state in 0..states {
            if valid & (1 << state) == 0 && self.bits(state) != 0 {
                return Err(Error::Corrupt("NULL group argument payload"));
            }
        }
        Ok(())
    }

    fn key_len(&self) -> usize {
        u32::from_le_bytes(self.bytes[16..20].try_into().expect("key length")) as usize
    }

    pub(in crate::execution) fn key(&self) -> &[u8] {
        &self.bytes[RECORD_HEADER..RECORD_HEADER + self.key_len()]
    }

    pub(in crate::execution) fn ordinal(&self) -> u64 {
        u64::from_le_bytes(self.bytes[8..16].try_into().expect("ordinal bytes"))
    }

    pub(in crate::execution) fn valid(&self) -> u16 {
        u16::from_le_bytes(self.bytes[24..26].try_into().expect("validity bytes"))
    }

    pub(in crate::execution) fn bits(&self, state: usize) -> u64 {
        assert!(state < usize::from(self.bytes[26]));
        let at = RECORD_HEADER + self.key_len() + state * 8;
        u64::from_le_bytes(self.bytes[at..at + 8].try_into().expect("argument bits"))
    }

    // The run arena contains only previously encoded records. Its spans must
    // retain complete frames; this comparison does not validate untrusted input.
    pub(super) fn compare_encoded(
        left: &[u8],
        right: &[u8],
        keys: &RowLayout,
    ) -> Result<Ordering, Error> {
        let key_len = |bytes: &[u8]| {
            u32::from_le_bytes(bytes[16..20].try_into().expect("key length")) as usize
        };
        let ordinal = |bytes: &[u8]| u64::from_le_bytes(bytes[8..16].try_into().expect("ordinal"));
        Ok(keys
            .compare(
                &left[RECORD_HEADER..RECORD_HEADER + key_len(left)],
                &right[RECORD_HEADER..RECORD_HEADER + key_len(right)],
            )?
            .then_with(|| ordinal(left).cmp(&ordinal(right))))
    }

    pub(super) fn compare(&self, other: &Self, keys: &RowLayout) -> Result<Ordering, Error> {
        Ok(keys
            .compare(self.key(), other.key())?
            .then_with(|| self.ordinal().cmp(&other.ordinal())))
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;
    use crate::execution::blocking::test_support::schema;
    fn encoded(keys: &RowLayout, values: &[Value<'_>]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(keys.max_bytes);
        for (index, value) in values.iter().copied().enumerate() {
            append_value(
                &mut bytes,
                value,
                keys.columns[index].kind,
                keys.columns[index].nullable,
            )
            .unwrap();
        }
        keys.validate(&bytes).unwrap();
        bytes
    }

    #[test]
    fn key_equivalence_order_hash_and_original_values_agree() {
        let keys = schema(&[(DataType::Double, true)]);
        let values = [
            Value::Null,
            Value::Double(f64::from_bits(0xfff8_0000_0000_0007)),
            Value::Double(f64::NAN),
            Value::Double(f64::NEG_INFINITY),
            Value::Double(-1.0),
            Value::Double(-0.0),
            Value::Double(0.0),
            Value::Double(f64::from_bits(1)),
            Value::Double(f64::INFINITY),
        ];
        let classes = [0, 1, 1, 2, 3, 4, 4, 5, 6];
        for (i, left) in values.iter().copied().enumerate() {
            let left_key = encoded(&keys, &[left]);
            match (left, keys.value(&left_key, 0).unwrap()) {
                (Value::Null, Value::Null) => {}
                (Value::Double(before), Value::Double(after)) => {
                    assert_eq!(before.to_bits(), after.to_bits())
                }
                _ => panic!("decoded key type"),
            }
            for (j, right) in values.iter().copied().enumerate() {
                let right_key = encoded(&keys, &[right]);
                let expected = classes[i].cmp(&classes[j]);
                assert_eq!(keys.compare(&left_key, &right_key).unwrap(), expected);
                if expected == Ordering::Equal {
                    assert_eq!(
                        keys.hash(&left_key).unwrap(),
                        keys.hash(&right_key).unwrap()
                    );
                }
            }
        }
        let keys = schema(&[
            (DataType::Int64, true),
            (DataType::Date, false),
            (DataType::String, true),
        ]);
        let date = DateValue::from_days_since_unix_epoch(-1).unwrap();
        let left = encoded(
            &keys,
            &[
                Value::Int64(i64::MIN),
                Value::Date(date),
                Value::String(StringValue::new("é")),
            ],
        );
        let right = encoded(
            &keys,
            &[
                Value::Int64(i64::MIN),
                Value::Date(date),
                Value::String(StringValue::new("éa")),
            ],
        );
        assert_eq!(keys.compare(&left, &right).unwrap(), Ordering::Less);
        assert_eq!(keys.value(&left, 0).unwrap(), Value::Int64(i64::MIN));
        assert_eq!(keys.value(&left, 1).unwrap(), Value::Date(date));
        assert_eq!(
            keys.value(&left, 2).unwrap(),
            Value::String(StringValue::new("é"))
        );
        for end in 0..left.len() {
            assert!(keys.validate(&left[..end]).is_err());
        }
        let mut bad = left.clone();
        bad.push(0);
        assert!(keys.validate(&bad).is_err());
        let last = bad.len() - 2;
        bad.truncate(last + 1);
        bad[last] = 0xff;
        assert!(keys.validate(&bad).is_err());
        let mut short = Vec::with_capacity(1);
        assert!(matches!(
            append_value(&mut short, Value::Int64(1), DataType::Int64, false),
            Err(Error::Resource { .. })
        ));
    }
}
