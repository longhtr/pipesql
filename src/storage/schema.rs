//! Encode and validate a declared table's column definitions.
//!
//! A schema records each column's name, type, persistent ID and whether NULL is allowed.
//! Position and identity are different: the first column might have ID 29, and
//! its values might appear second in a data file. Native-unit readers match IDs
//! to find the right payload. The binder assigns separate IDs for a query.
//!
//! `encode` writes into the caller's buffer. `decode` checks that the bytes match
//! the expected database, table and checksum, then validates every column. Its
//! returned `Schema` borrows those immutable bytes, so inspecting columns needs
//! no additional allocation. The catalog reader owns file access; this module
//! neither opens files nor publishes changes. Legacy fixed-schema tables use a
//! separate representation.

use crate::DatabaseId;
use crate::storage::format::{FormatError, crc32c};
use crate::value::DataType;

// A fixed maximum lets callers reserve the complete schema buffer before I/O.
// The header and 64 column records require at most 3,136 bytes.
pub(crate) const MAX_COLUMNS: usize = crate::schema::MAX_COLUMNS;
const MAX_NAME_BYTES: usize = crate::schema::MAX_NAME_BYTES;
const HEADER: usize = 64;
const COLUMN: usize = 48;
const MAGIC: &[u8; 8] = b"PSQLSCHM";
const FORMAT: u32 = 6;
pub(crate) const MAX_BYTES: usize = HEADER + MAX_COLUMNS * COLUMN;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TableId(u64);

impl TableId {
    pub(crate) fn new(value: u64) -> Result<Self, FormatError> {
        if value == 0 {
            return Err(FormatError::Identity);
        }
        Ok(Self(value))
    }

    pub(super) fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ColumnId(u32);

impl ColumnId {
    pub(crate) fn new(value: u32) -> Result<Self, FormatError> {
        if value == 0 {
            return Err(FormatError::Identity);
        }
        Ok(Self(value))
    }

    pub(crate) fn value(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ColumnSpec<'a> {
    id: ColumnId,
    name: &'a str,
    data_type: DataType,
    nullable: bool,
}

impl<'a> ColumnSpec<'a> {
    pub(crate) fn new(
        id: ColumnId,
        name: &'a str,
        data_type: DataType,
        nullable: bool,
    ) -> Result<Self, FormatError> {
        validate_name(name.as_bytes())?;
        Ok(Self {
            id,
            name,
            data_type,
            nullable,
        })
    }

    pub(crate) fn id(self) -> ColumnId {
        self.id
    }

    pub(crate) fn name(self) -> &'a str {
        self.name
    }

    pub(crate) fn data_type(self) -> DataType {
        self.data_type
    }

    pub(crate) fn nullable(self) -> bool {
        self.nullable
    }
}

pub(super) fn validate_name(name: &[u8]) -> Result<(), FormatError> {
    if !crate::schema::valid_name(name) {
        return Err(FormatError::Field);
    }
    Ok(())
}

pub(super) fn valid_extent(bytes: u32) -> bool {
    let header = HEADER as u32;
    let column = COLUMN as u32;
    bytes >= header + column && bytes <= MAX_BYTES as u32 && (bytes - header).is_multiple_of(column)
}

fn extent(count: usize) -> Result<usize, FormatError> {
    if count == 0 || count > MAX_COLUMNS {
        return Err(FormatError::Field);
    }
    count
        .checked_mul(COLUMN)
        .and_then(|bytes| bytes.checked_add(HEADER))
        .ok_or(FormatError::Overflow)
}

pub(super) fn tag(kind: DataType) -> u8 {
    match kind {
        DataType::Int64 => 1,
        DataType::Double => 2,
        DataType::String => 3,
        DataType::Date => 4,
    }
}

pub(super) fn data_type(tag: u8) -> Result<DataType, FormatError> {
    match tag {
        1 => Ok(DataType::Int64),
        2 => Ok(DataType::Double),
        3 => Ok(DataType::String),
        4 => Ok(DataType::Date),
        _ => Err(FormatError::Field),
    }
}

// ColumnSpec already guarantees valid individual fields. Check the remaining
// list-wide rules and destination size before writing, so rejection leaves out
// unchanged. The caller computes and stores the checksum in the catalog reference.
pub(super) fn encode(
    out: &mut [u8],
    database: DatabaseId,
    table: TableId,
    columns: &[ColumnSpec<'_>],
) -> Result<usize, FormatError> {
    let length = extent(columns.len())?;
    if out.len() < length {
        return Err(FormatError::Length);
    }
    for (index, column) in columns.iter().enumerate() {
        for earlier in &columns[..index] {
            if column.id == earlier.id || column.name.eq_ignore_ascii_case(earlier.name) {
                return Err(FormatError::Field);
            }
        }
    }
    let count = u32::try_from(columns.len()).map_err(|_| FormatError::Overflow)?;
    out[..length].fill(0);
    out[..8].copy_from_slice(MAGIC);
    out[8..12].copy_from_slice(&FORMAT.to_le_bytes());
    out[12..16].copy_from_slice(&count.to_le_bytes());
    out[16..32].copy_from_slice(database.as_bytes());
    out[32..40].copy_from_slice(&table.value().to_le_bytes());
    for (column, bytes) in columns
        .iter()
        .zip(out[HEADER..length].as_chunks_mut::<COLUMN>().0.iter_mut())
    {
        bytes[..4].copy_from_slice(&column.id.value().to_le_bytes());
        bytes[4] = tag(column.data_type);
        bytes[5] = u8::from(column.nullable);
        bytes[6] = u8::try_from(column.name.len()).expect("validated name length");
        bytes[8..8 + column.name.len()].copy_from_slice(column.name.as_bytes());
    }
    Ok(length)
}

pub(crate) struct Schema<'a> {
    bytes: &'a [u8],
    table: TableId,
    columns: usize,
}

impl<'a> Schema<'a> {
    pub(crate) fn encoded_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub(super) fn database(&self) -> DatabaseId {
        DatabaseId::new(
            self.bytes[16..32]
                .try_into()
                .expect("validated database field"),
        )
        .expect("validated database identity")
    }

    pub(crate) fn table(&self) -> TableId {
        self.table
    }

    pub(crate) fn len(&self) -> usize {
        self.columns
    }

    pub(crate) fn column(&self, ordinal: usize) -> Option<ColumnSpec<'a>> {
        if ordinal >= self.columns {
            return None;
        }
        // decode checked every record before exposing this immutable view.
        // Re-reading one record avoids allocating a second array of columns.
        let columns = self.bytes[HEADER..].as_chunks::<COLUMN>().0;
        Some(decode_column(&columns[ordinal]).expect("validated column"))
    }
}

fn decode_column(bytes: &[u8; COLUMN]) -> Result<ColumnSpec<'_>, FormatError> {
    let id = ColumnId::new(u32::from_le_bytes(
        bytes[..4].try_into().expect("fixed column ID"),
    ))?;
    let kind = data_type(bytes[4])?;
    if bytes[5] > 1 || bytes[7] != 0 || bytes[40..].iter().any(|byte| *byte != 0) {
        return Err(FormatError::Field);
    }
    let length = usize::from(bytes[6]);
    if length == 0 || length > MAX_NAME_BYTES {
        return Err(FormatError::Field);
    }
    if bytes[8 + length..40].iter().any(|byte| *byte != 0) {
        return Err(FormatError::Reserved);
    }
    let name = std::str::from_utf8(&bytes[8..8 + length]).map_err(|_| FormatError::Field)?;
    ColumnSpec::new(id, name, kind, bytes[5] != 0)
}

// The checksum comes from the referring catalog, not from these bytes. Matching
// it is necessary but insufficient: reject invalid fields and duplicate IDs or
// names even when the checksum is correct.
pub(crate) fn decode(
    bytes: &[u8],
    database: DatabaseId,
    table: TableId,
    expected_crc: u32,
) -> Result<Schema<'_>, FormatError> {
    if bytes.len() < 12 {
        return Err(FormatError::Length);
    }
    if &bytes[..8] != MAGIC {
        return Err(FormatError::Magic);
    }
    if u32::from_le_bytes(bytes[8..12].try_into().expect("version field")) != FORMAT {
        return Err(FormatError::Version);
    }
    if bytes.len() < HEADER || bytes.len() > MAX_BYTES {
        return Err(FormatError::Length);
    }
    if crc32c(bytes) != expected_crc {
        return Err(FormatError::Checksum);
    }
    if &bytes[16..32] != database.as_bytes()
        || u64::from_le_bytes(bytes[32..40].try_into().expect("table field")) != table.value()
    {
        return Err(FormatError::Identity);
    }
    if bytes[40..HEADER].iter().any(|byte| *byte != 0) {
        return Err(FormatError::Reserved);
    }
    let count = usize::try_from(u32::from_le_bytes(
        bytes[12..16].try_into().expect("count field"),
    ))
    .map_err(|_| FormatError::Overflow)?;
    if bytes.len() != extent(count)? {
        return Err(FormatError::Length);
    }
    let schema = Schema {
        bytes,
        table,
        columns: count,
    };
    for (index, encoded) in bytes[HEADER..].as_chunks::<COLUMN>().0.iter().enumerate() {
        let column = decode_column(encoded)?;
        for prior in bytes[HEADER..HEADER + index * COLUMN]
            .as_chunks::<COLUMN>()
            .0
        {
            let prior = decode_column(prior)?;
            if column.id == prior.id || column.name.eq_ignore_ascii_case(prior.name) {
                return Err(FormatError::Field);
            }
        }
    }
    Ok(schema)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> DatabaseId {
        DatabaseId::new([7; 16]).unwrap()
    }

    fn column(id: u32, name: &str, kind: DataType, nullable: bool) -> ColumnSpec<'_> {
        ColumnSpec::new(ColumnId::new(id).unwrap(), name, kind, nullable).unwrap()
    }

    #[test]
    fn independent_wire_fixture_matches_producer_and_consumer() {
        let expected = include_bytes!("../../test/data/catalog-schema/columns.bin");
        let table = TableId::new(0x0102_0304_0506_0708).unwrap();
        // Both the retained bytes and this literal CRC come from the independent
        // Python fixture generator, so a shared codec bug cannot set the answer.
        let schema = decode(expected, database(), table, 0x666a_62b7).unwrap();
        assert_eq!(
            schema.column(0).unwrap(),
            column(29, "note", DataType::String, true)
        );
        assert_eq!(
            schema.column(1).unwrap(),
            column(3, "amount", DataType::Double, false)
        );
        let mut encoded = [0; MAX_BYTES];
        let length = encode(
            &mut encoded,
            database(),
            table,
            &[
                column(29, "note", DataType::String, true),
                column(3, "amount", DataType::Double, false),
            ],
        )
        .unwrap();
        assert_eq!(&encoded[..length], expected);
        for prefix in 0..expected.len() {
            assert!(decode(&expected[..prefix], database(), table, 0x666a_62b7).is_err());
        }
        let mut extended = expected.to_vec();
        extended.push(0);
        assert!(matches!(
            decode(&extended, database(), table, crc32c(&extended)),
            Err(FormatError::Length)
        ));
        let mut future = expected.to_vec();
        future[8] = 7;
        for prefix in [12, future.len()] {
            assert!(matches!(
                decode(
                    &future[..prefix],
                    database(),
                    table,
                    crc32c(&future[..prefix])
                ),
                Err(FormatError::Version)
            ));
        }
    }

    #[test]
    fn exact_name_and_column_capacities_preserve_all_fields() {
        let names: Vec<_> = (0..MAX_COLUMNS)
            .map(|index| format!("c{index:031}"))
            .collect();
        let columns: Vec<_> = names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                column(
                    u32::try_from(MAX_COLUMNS - index).unwrap(),
                    name,
                    DataType::String,
                    index % 2 == 0,
                )
            })
            .collect();
        let mut encoded = [0; MAX_BYTES];
        let table = TableId::new(u64::MAX).unwrap();
        assert_eq!(
            encode(&mut encoded, database(), table, &columns).unwrap(),
            MAX_BYTES
        );
        let schema = decode(&encoded, database(), table, crc32c(&encoded)).unwrap();
        for (ordinal, expected) in columns.iter().enumerate() {
            assert_eq!(schema.column(ordinal), Some(*expected));
        }
    }

    #[test]
    fn declared_columns_preserve_identity_across_physical_reordering() {
        let columns = [
            column(19, "description", DataType::String, true),
            column(2, "amount", DataType::Double, true),
            column(7, "count", DataType::Int64, false),
            column(31, "day", DataType::Date, false),
        ];
        let mut out = [0; MAX_BYTES];
        let table = TableId::new(41).unwrap();
        let length = encode(&mut out, database(), table, &columns).unwrap();
        let schema = decode(&out[..length], database(), table, crc32c(&out[..length])).unwrap();
        assert_eq!(schema.table(), table);
        assert_eq!(schema.len(), 4);
        for (index, expected) in columns.into_iter().enumerate() {
            let actual = schema.column(index).unwrap();
            assert_eq!(actual.id(), expected.id());
            assert_eq!(actual.name(), expected.name());
            assert_eq!(actual.data_type(), expected.data_type());
            assert_eq!(actual.nullable(), expected.nullable());
        }
        assert!(schema.column(4).is_none());
        assert!(schema.column(usize::MAX).is_none());
    }

    #[test]
    fn malformed_semantics_are_rejected_even_with_a_matching_checksum() {
        let mut out = [0; MAX_BYTES];
        let table = TableId::new(1).unwrap();
        let length = encode(
            &mut out,
            database(),
            table,
            &[
                column(1, "a", DataType::Int64, false),
                column(2, "b", DataType::String, true),
            ],
        )
        .unwrap();
        // Recompute the CRC after each mutation to exercise semantic validation,
        // including duplicate IDs and case-insensitive duplicate names.
        for (offset, value) in [
            (12, 0),
            (12, 65),
            (16, 8),
            (32, 2),
            (40, 1),
            (64, 0),
            (68, 255),
            (69, 2),
            (70, 33),
            (71, 1),
            (73, 1),
            (104, 1),
            (112, 1),
            (120, b'A'),
        ] {
            let mut changed = out[..length].to_vec();
            changed[offset] = value;
            assert!(
                decode(&changed, database(), table, crc32c(&changed)).is_err(),
                "offset {offset}"
            );
        }
        let checksum = crc32c(&out[..length]);
        out[72] = b'z';
        assert!(matches!(
            decode(&out[..length], database(), table, checksum),
            Err(FormatError::Checksum)
        ));
    }

    #[test]
    fn duplicate_or_over_capacity_input_does_not_mutate_output() {
        let mut out = [0xab; MAX_BYTES];
        let before = out;
        for columns in [
            vec![],
            vec![column(1, "a", DataType::Int64, false); 65],
            vec![
                column(1, "a", DataType::Int64, false),
                column(1, "b", DataType::String, true),
            ],
            vec![
                column(1, "a", DataType::Int64, false),
                column(2, "A", DataType::String, true),
            ],
        ] {
            assert!(encode(&mut out, database(), TableId::new(1).unwrap(), &columns).is_err());
            assert_eq!(out, before);
        }
        assert!(
            encode(
                &mut out[..64],
                database(),
                TableId::new(1).unwrap(),
                &[column(1, "a", DataType::Int64, false)]
            )
            .is_err()
        );
        assert_eq!(out, before);
        assert!(TableId::new(0).is_err());
        assert!(ColumnId::new(0).is_err());
        for name in [
            "",
            "1bad",
            "has space",
            "é",
            "abcdefghijklmnopqrstuvwxyz1234567",
        ] {
            assert!(
                ColumnSpec::new(ColumnId::new(1).unwrap(), name, DataType::String, true).is_err()
            );
        }
    }
}
