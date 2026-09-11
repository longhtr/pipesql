//! Declared-table native units. Construction writes an admitted, empty private file;
//! the caller owns reservations, cleanup, synchronization and publication.
use crate::catalog::{self, ObjectId};
use crate::catalog_schema::{self, ColumnId, ColumnSpec, Schema};
use crate::date::DateValue;
use crate::effects::{Effect, Effects, MetadataKind};
use crate::frontend::DataType;
use crate::storage_format::{DatabaseId, FormatError, crc32c};
use crate::{CancellationToken, Error};
use pipesql_filesystem as filesystem;
use std::fs::File;
use std::path::Path;

const HEADER: usize = 64;
const DESCRIPTOR: usize = 32;
const MAGIC: &[u8; 8] = b"PSQLDATA";
const FORMAT: u32 = 6;
pub(super) const MAX_ROWS: usize = 32_768;
pub(super) const MAX_COLUMN_BYTES: usize = 524_288;
pub(super) const MAX_TEXT_BYTES: usize = 65_536;
pub(super) const MAX_METADATA_BYTES: usize = HEADER + DESCRIPTOR * catalog_schema::MAX_COLUMNS;
pub(super) const MAX_UNIT_BYTES: usize =
    MAX_METADATA_BYTES + catalog_schema::MAX_COLUMNS * MAX_COLUMN_BYTES;
const READ: Effect = Effect::ReadMetadata(MetadataKind::CatalogObject);
const WRITE: Effect = Effect::WriteMetadata(MetadataKind::CatalogObject);
const INSPECT: Effect = Effect::InspectOpenMetadata(MetadataKind::CatalogObject);

// Unlike ObjectRef, this checksum covers metadata only. Demanded payload CRCs
// are anchored there; opening a projected scan never reads unrelated payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct UnitRef {
    object: ObjectId,
    rows: u32,
    bytes: u32,
    metadata_crc: u32,
}

impl UnitRef {
    pub(super) fn object(self) -> ObjectId {
        self.object
    }

    pub(super) fn rows(self) -> u32 {
        self.rows
    }

    pub(super) fn bytes(self) -> u32 {
        self.bytes
    }

    pub(super) fn metadata_checksum(self) -> u32 {
        self.metadata_crc
    }

    pub(super) fn new(
        object: ObjectId,
        rows: u32,
        bytes: u32,
        metadata_crc: u32,
    ) -> Result<Self, FormatError> {
        if rows == 0
            || u64::from(rows) > MAX_ROWS as u64
            || bytes < (HEADER + DESCRIPTOR) as u32
            || u64::from(bytes) > MAX_UNIT_BYTES as u64
        {
            return Err(FormatError::Field);
        }
        Ok(Self {
            object,
            rows,
            bytes,
            metadata_crc,
        })
    }
}

/// Typed values borrowed for one [`crate::Append::write`] call.
///
/// Exported as [`crate::ColumnValues`]. All columns in a batch must have equal
/// row counts. A separate [`crate::ColumnInput::validity`] bitmap marks NULLs;
/// the caller may reuse the input buffers after `write` returns.
#[derive(Clone, Copy)]
pub enum InputValues<'a> {
    Int64(&'a [i64]),
    Double(&'a [f64]),
    String(&'a [&'a str]),
    Date(&'a [DateValue]),
}

impl InputValues<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Int64(v) => v.len(),
            Self::Double(v) => v.len(),
            Self::String(v) => v.len(),
            Self::Date(v) => v.len(),
        }
    }

    fn kind(&self) -> DataType {
        match self {
            Self::Int64(_) => DataType::Int64,
            Self::Double(_) => DataType::Double,
            Self::String(_) => DataType::String,
            Self::Date(_) => DataType::Date,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct InputColumn<'a> {
    pub(super) id: ColumnId,
    pub(super) values: InputValues<'a>,
    // Low bit is the first row. Unused high bits must be zero.
    pub(super) validity: &'a [u8],
}

fn bitmap_bytes(rows: usize) -> usize {
    rows.div_ceil(8)
}

fn valid(bits: &[u8], row: usize) -> bool {
    bits[row / 8] & (1 << (row % 8)) != 0
}

fn check_bitmap(bits: &[u8], rows: usize, nullable: bool) -> Result<(), FormatError> {
    if bits.len() != bitmap_bytes(rows) {
        return Err(FormatError::Length);
    }
    if !rows.is_multiple_of(8) && bits[bits.len() - 1] >> (rows % 8) != 0 {
        return Err(FormatError::Reserved);
    }
    if !nullable && (0..rows).any(|row| !valid(bits, row)) {
        return Err(FormatError::Field);
    }
    Ok(())
}

fn spec<'a>(schema: &Schema<'a>, id: ColumnId) -> Result<ColumnSpec<'a>, FormatError> {
    (0..schema.len())
        .filter_map(|i| schema.column(i))
        .find(|column| column.id() == id)
        .ok_or(FormatError::Identity)
}

fn fixed_width(kind: DataType) -> Option<usize> {
    match kind {
        DataType::Int64 | DataType::Double => Some(8),
        DataType::Date => Some(4),
        DataType::String => None,
    }
}
// Maximum payload for one admitted unit, including its validity bitmap.
pub(super) fn column_capacity(kind: DataType) -> usize {
    fixed_width(kind).map_or(MAX_COLUMN_BYTES, |width| {
        bitmap_bytes(MAX_ROWS) + MAX_ROWS * width
    })
}

fn payload_length(
    column: &InputColumn<'_>,
    spec: ColumnSpec<'_>,
    rows: usize,
) -> Result<usize, Error> {
    if column.values.len() != rows || column.values.kind() != spec.data_type() {
        return Err(invalid_input(FormatError::Field));
    }
    check_bitmap(column.validity, rows, spec.nullable()).map_err(invalid_input)?;
    let values = if let Some(width) = fixed_width(spec.data_type()) {
        rows.checked_mul(width)
            .ok_or_else(|| invalid_input(FormatError::Overflow))?
    } else {
        let InputValues::String(strings) = &column.values else {
            unreachable!("matching input kind");
        };
        let mut bytes = rows
            .checked_add(1)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| invalid_input(FormatError::Overflow))?;
        for (row, text) in strings.iter().enumerate() {
            if valid(column.validity, row) {
                buffer_capacity(text.len(), MAX_TEXT_BYTES, "native text value bytes")?;
                bytes = bytes
                    .checked_add(text.len())
                    .ok_or_else(|| invalid_input(FormatError::Overflow))?;
                buffer_capacity(
                    bitmap_bytes(rows) + bytes,
                    MAX_COLUMN_BYTES,
                    "native column bytes",
                )?;
            }
        }
        bytes
    };
    let length = bitmap_bytes(rows)
        .checked_add(values)
        .ok_or_else(|| invalid_input(FormatError::Overflow))?;
    buffer_capacity(length, MAX_COLUMN_BYTES, "native column bytes")?;
    Ok(length)
}

fn encode_payload(out: &mut [u8], input: &InputColumn<'_>, rows: usize) {
    out.fill(0);
    let validity = bitmap_bytes(rows);
    out[..validity].copy_from_slice(input.validity);
    if let InputValues::String(strings) = &input.values {
        let base = validity + (rows + 1) * 4;
        let mut offset = 0;
        for (row, text) in strings.iter().enumerate() {
            if valid(input.validity, row) {
                out[base + offset..base + offset + text.len()].copy_from_slice(text.as_bytes());
                offset += text.len();
            }
            put_u32(
                out,
                validity + (row + 1) * 4,
                u32::try_from(offset).expect("admitted text bytes"),
            );
        }
        assert_eq!(base + offset, out.len());
    } else {
        let width = fixed_width(input.values.kind()).expect("fixed input kind");
        for row in 0..rows {
            if valid(input.validity, row) {
                let start = validity + row * width;
                match &input.values {
                    InputValues::Int64(v) => {
                        out[start..start + 8].copy_from_slice(&v[row].to_le_bytes())
                    }
                    InputValues::Double(v) => {
                        out[start..start + 8].copy_from_slice(&v[row].to_bits().to_le_bytes())
                    }
                    InputValues::Date(v) => out[start..start + 4]
                        .copy_from_slice(&v[row].days_since_unix_epoch().to_le_bytes()),
                    InputValues::String(_) => unreachable!("fixed input kind"),
                }
            }
        }
    }
}

fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("fixed field"))
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("fixed field"))
}

fn zero(bytes: &[u8]) -> Result<(), FormatError> {
    if bytes.iter().any(|b| *b != 0) {
        Err(FormatError::Reserved)
    } else {
        Ok(())
    }
}

fn buffer_capacity(required: usize, limit: usize, owner: &'static str) -> Result<(), Error> {
    if required > limit {
        return Err(Error::Resource {
            owner,
            required: u64::try_from(required).expect("bounded buffer"),
            limit: u64::try_from(limit).expect("native buffer"),
        });
    }
    Ok(())
}

fn invalid_input(_: FormatError) -> Error {
    Error::Corrupt("native unit construction input is invalid")
}

pub(super) struct WriteBuffers<'a> {
    pub(super) metadata: &'a mut [u8],
    pub(super) column: &'a mut [u8],
}

// Logical requirements for caller admission before file creation or allocation.
// They do not grant memory/temp authority; write rechecks inputs and buffers.
pub(super) struct Requirements {
    pub(super) rows: usize,
    pub(super) metadata_bytes: usize,
    pub(super) column_bytes: usize,
    pub(super) unit_bytes: usize,
}

pub(super) fn requirements(
    schema: &Schema<'_>,
    columns: &[InputColumn<'_>],
    cancel: &CancellationToken,
) -> Result<Requirements, Error> {
    cancel.check()?;
    if columns.len() != schema.len() {
        return Err(invalid_input(FormatError::Field));
    }
    let rows = columns
        .first()
        .ok_or_else(|| invalid_input(FormatError::Field))?
        .values
        .len();
    if rows == 0 {
        return Err(invalid_input(FormatError::Field));
    }
    buffer_capacity(rows, MAX_ROWS, "native unit rows")?;
    let metadata_bytes = HEADER + DESCRIPTOR * columns.len();
    let mut unit_bytes = metadata_bytes;
    let mut column_bytes = 0;
    for (index, column) in columns.iter().enumerate() {
        cancel.check()?;
        if columns[..index].iter().any(|prior| prior.id == column.id) {
            return Err(invalid_input(FormatError::Identity));
        }
        let length = payload_length(
            column,
            spec(schema, column.id).map_err(invalid_input)?,
            rows,
        )?;
        column_bytes = column_bytes.max(length);
        unit_bytes = unit_bytes
            .checked_add(length)
            .ok_or_else(|| invalid_input(FormatError::Overflow))?;
    }
    cancel.check()?;
    Ok(Requirements {
        rows,
        metadata_bytes,
        column_bytes,
        unit_bytes,
    })
}

// A single private construction operation, not a transaction or durability ack.
// Input and workspace admission finish before any file effect. A failure after
// writing begins leaves cleanup ownership with the caller; no root names change.
pub(super) fn write(
    file: &File,
    object: ObjectId,
    schema: &Schema<'_>,
    columns: &[InputColumn<'_>],
    buffers: WriteBuffers<'_>,
    cancel: &CancellationToken,
    effects: &mut Effects,
) -> Result<UnitRef, Error> {
    let WriteBuffers {
        metadata,
        column: scratch,
    } = buffers;
    let database = schema.database();
    let Requirements {
        rows,
        metadata_bytes,
        column_bytes,
        unit_bytes,
    } = requirements(schema, columns, cancel)?;
    buffer_capacity(metadata_bytes, metadata.len(), "unit metadata buffer")?;
    buffer_capacity(column_bytes, scratch.len(), "unit column buffer")?;
    cancel.check()?;
    effects.before(INSPECT)?;
    let stat = filesystem::file_metadata(file)
        .map_err(|e| crate::error::io_error("inspect private native unit", e))?;
    if !stat.file_type().is_file() || !stat.is_empty() || stat.nlink() != 1 {
        return Err(Error::Corrupt(
            "native unit construction requires an empty private file",
        ));
    }
    metadata[..metadata_bytes].fill(0);
    metadata[..8].copy_from_slice(MAGIC);
    put_u32(metadata, 8, FORMAT);
    put_u32(
        metadata,
        12,
        u32::try_from(columns.len()).expect("column capacity"),
    );
    metadata[16..32].copy_from_slice(database.as_bytes());
    put_u64(metadata, 32, schema.table().value());
    put_u64(metadata, 40, object.attempt());
    put_u32(metadata, 48, object.ordinal());
    put_u32(metadata, 52, u32::try_from(rows).expect("row capacity"));
    put_u32(
        metadata,
        56,
        u32::try_from(metadata_bytes).expect("metadata capacity"),
    );
    let mut offset = metadata_bytes;
    for (index, input) in columns.iter().enumerate() {
        cancel.check()?;
        let column = spec(schema, input.id).expect("admitted column");
        let length = payload_length(input, column, rows).expect("admitted payload");
        encode_payload(&mut scratch[..length], input, rows);
        let checksum = crc32c(&scratch[..length]);
        crate::effects::write_all_at(
            file,
            &scratch[..length],
            u64::try_from(offset).expect("unit extent"),
            WRITE,
            effects,
        )?;
        cancel.check()?;
        crate::effects::read_exact_at(
            file,
            &mut scratch[..length],
            u64::try_from(offset).expect("unit extent"),
            READ,
            effects,
        )?;
        if crc32c(&scratch[..length]) != checksum {
            return Err(Error::Corrupt("native column readback differs"));
        }
        let entry = &mut metadata[HEADER + index * DESCRIPTOR..HEADER + (index + 1) * DESCRIPTOR];
        put_u32(entry, 0, input.id.value());
        entry[4] = catalog_schema::tag(column.data_type());
        entry[5] = u8::from(column.nullable());
        put_u64(entry, 8, u64::try_from(offset).expect("unit extent"));
        put_u32(entry, 16, u32::try_from(length).expect("column extent"));
        put_u32(entry, 20, checksum);
        offset = offset.checked_add(length).expect("admitted unit extent");
    }
    assert_eq!(offset, unit_bytes);
    let reference = UnitRef::new(
        object,
        u32::try_from(rows).expect("row capacity"),
        u32::try_from(unit_bytes).expect("unit capacity"),
        crc32c(&metadata[..metadata_bytes]),
    )
    .map_err(invalid_input)?;
    cancel.check()?;
    crate::effects::write_all_at(file, &metadata[..metadata_bytes], 0, WRITE, effects)?;
    cancel.check()?;
    crate::effects::read_exact_at(file, &mut metadata[..metadata_bytes], 0, READ, effects)?;
    validate_metadata(&metadata[..metadata_bytes], database, reference, schema)
        .map_err(|e| crate::error::map_format_error(e, metadata))?;
    cancel.check()?;
    effects.before(INSPECT)?;
    if filesystem::file_metadata(file)
        .map_err(|e| crate::error::io_error("inspect constructed unit", e))?
        .len()
        != u64::from(reference.bytes)
    {
        return Err(Error::Corrupt("constructed unit extent differs"));
    }
    cancel.check()?;
    Ok(reference)
}

fn validate_metadata(
    bytes: &[u8],
    database: DatabaseId,
    reference: UnitRef,
    schema: &Schema<'_>,
) -> Result<(), FormatError> {
    if schema.database() != database {
        return Err(FormatError::Identity);
    }
    if bytes.len() < 12 {
        return Err(FormatError::Length);
    }
    if &bytes[..8] != MAGIC {
        return Err(FormatError::Magic);
    }
    if u32_at(bytes, 8) != FORMAT {
        return Err(FormatError::Version);
    }
    if bytes.len() != HEADER + schema.len() * DESCRIPTOR {
        return Err(FormatError::Length);
    }
    if crc32c(bytes) != reference.metadata_crc {
        return Err(FormatError::Checksum);
    }
    if &bytes[16..32] != database.as_bytes()
        || u64_at(bytes, 32) != schema.table().value()
        || ObjectId::new(u64_at(bytes, 40), u32_at(bytes, 48))? != reference.object
    {
        return Err(FormatError::Identity);
    }
    if u32_at(bytes, 12) as usize != schema.len()
        || u32_at(bytes, 52) != reference.rows
        || u32_at(bytes, 56) as usize != bytes.len()
    {
        return Err(FormatError::Field);
    }
    zero(&bytes[60..64])?;
    let entries = bytes[HEADER..].as_chunks::<DESCRIPTOR>().0;
    let mut offset = bytes.len() as u64;
    for (index, entry) in entries.iter().enumerate() {
        let id = ColumnId::new(u32_at(entry.as_slice(), 0))?;
        if entries[..index]
            .iter()
            .any(|prior| u32_at(prior.as_slice(), 0) == id.value())
        {
            return Err(FormatError::Identity);
        }
        let column = spec(schema, id)?;
        if entry[4] != catalog_schema::tag(column.data_type())
            || entry[5] != u8::from(column.nullable())
            || u64_at(entry.as_slice(), 8) != offset
        {
            return Err(FormatError::Field);
        }
        zero(&entry[6..8])?;
        zero(&entry[24..])?;
        let length = u32_at(entry.as_slice(), 16) as usize;
        let rows = reference.rows as usize;
        let minimum = bitmap_bytes(rows)
            + if let Some(width) = fixed_width(column.data_type()) {
                rows * width
            } else {
                (rows + 1) * 4
            };
        if length < minimum
            || length > MAX_COLUMN_BYTES
            || (column.data_type() != DataType::String && length != minimum)
        {
            return Err(FormatError::Length);
        }
        offset = offset
            .checked_add(length as u64)
            .ok_or(FormatError::Overflow)?;
    }
    if offset != u64::from(reference.bytes) {
        return Err(FormatError::Length);
    }
    Ok(())
}

// Own fixed metadata so a resumable reader can move without borrowing its
// enclosing workspace. A retaining owner accounts for this complete value.
pub(super) struct Unit {
    file: File,
    metadata: [u8; MAX_METADATA_BYTES],
    metadata_len: usize,
    rows: usize,
}

pub(super) fn read(
    objects: &Path,
    database: DatabaseId,
    reference: UnitRef,
    schema: &Schema<'_>,
    cancel: &CancellationToken,
    effects: &mut Effects,
) -> Result<Unit, Error> {
    cancel.check()?;
    let length = HEADER + schema.len() * DESCRIPTOR;
    let (file, bytes) = catalog::open_object(objects, reference.object, cancel, effects)?;
    if bytes != u64::from(reference.bytes) {
        return Err(Error::Corrupt("native unit extent differs"));
    }
    let mut unit = Unit {
        file,
        metadata: [0; MAX_METADATA_BYTES],
        metadata_len: length,
        rows: reference.rows as usize,
    };
    cancel.check()?;
    crate::effects::read_exact_at(&unit.file, &mut unit.metadata[..length], 0, READ, effects)?;
    validate_metadata(&unit.metadata[..length], database, reference, schema)
        .map_err(|e| crate::error::map_format_error(e, &unit.metadata[..length]))?;
    cancel.check()?;
    Ok(unit)
}

impl Unit {
    pub(super) fn read_column<'a>(
        &self,
        id: ColumnId,
        buffer: &'a mut [u8],
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Column<'a>, Error> {
        cancel.check()?;
        let entry = self.metadata[HEADER..self.metadata_len]
            .as_chunks::<DESCRIPTOR>()
            .0
            .iter()
            .find(|entry| u32_at(entry.as_slice(), 0) == id.value())
            .ok_or(Error::Corrupt("unit column identity absent"))?;
        let length = u32_at(entry.as_slice(), 16) as usize;
        buffer_capacity(length, buffer.len(), "unit column buffer")?;
        crate::effects::read_exact_at(
            &self.file,
            &mut buffer[..length],
            u64_at(entry.as_slice(), 8),
            READ,
            effects,
        )?;
        cancel.check()?;
        if crc32c(&buffer[..length]) != u32_at(entry.as_slice(), 20) {
            return Err(Error::Corrupt("native column checksum differs"));
        }
        let kind = catalog_schema::data_type(entry[4]).expect("validated descriptor kind");
        let column = Column {
            bytes: &buffer[..length],
            rows: self.rows,
            kind,
        };
        column
            .validate(entry[5] != 0)
            .map_err(|_| Error::Corrupt("native column values are invalid"))?;
        cancel.check()?;
        Ok(column)
    }
}
// A query reserves the full buffer capacity and this handle before construction.
// Mutation is private: a failed refill cannot expose stale or partially read data.
pub(super) struct ColumnBuffer {
    bytes: Vec<u8>,
    validated: Option<(usize, usize, DataType)>,
}

impl ColumnBuffer {
    pub(super) fn new(bytes: Vec<u8>) -> Result<Self, Error> {
        buffer_capacity(bytes.capacity(), MAX_COLUMN_BYTES, "native column capacity")?;
        Ok(Self {
            bytes,
            validated: None,
        })
    }

    pub(super) fn read(
        &mut self,
        unit: &Unit,
        id: ColumnId,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        self.validated = None;
        let column = unit.read_column(id, &mut self.bytes, cancel, effects)?;
        self.validated = Some((column.bytes.len(), column.rows, column.kind));
        Ok(())
    }

    pub(super) fn column(&self) -> Option<Column<'_>> {
        self.validated.map(|(length, rows, kind)| Column {
            bytes: &self.bytes[..length],
            rows,
            kind,
        })
    }
}

pub(super) struct Column<'a> {
    bytes: &'a [u8],
    rows: usize,
    kind: DataType,
}

impl<'a> Column<'a> {
    fn validate(&self, nullable: bool) -> Result<(), FormatError> {
        let validity = bitmap_bytes(self.rows);
        check_bitmap(&self.bytes[..validity], self.rows, nullable)?;
        if self.kind == DataType::String {
            let base = validity + (self.rows + 1) * 4;
            let strings = &self.bytes[base..];
            if u32_at(self.bytes, validity) != 0 {
                return Err(FormatError::Field);
            }
            let mut prior = 0;
            for row in 0..self.rows {
                let end = u32_at(self.bytes, validity + (row + 1) * 4) as usize;
                if end < prior || end > strings.len() || end - prior > MAX_TEXT_BYTES {
                    return Err(FormatError::Field);
                }
                if !valid(self.bytes, row) && end != prior {
                    return Err(FormatError::Field);
                }
                std::str::from_utf8(&strings[prior..end]).map_err(|_| FormatError::Field)?;
                prior = end;
            }
            if prior != strings.len() {
                return Err(FormatError::Length);
            }
        } else {
            let width = fixed_width(self.kind).expect("fixed descriptor kind");
            for row in 0..self.rows {
                let bytes = &self.bytes[validity + row * width..validity + (row + 1) * width];
                if !valid(self.bytes, row) {
                    zero(bytes)?;
                } else if self.kind == DataType::Date
                    && DateValue::from_days(i32::from_le_bytes(
                        bytes.try_into().expect("DATE width"),
                    ))
                    .is_none()
                {
                    return Err(FormatError::Field);
                }
            }
        }
        Ok(())
    }

    fn present(&self, row: usize, kind: DataType) -> Result<bool, FormatError> {
        if self.kind != kind || row >= self.rows {
            return Err(FormatError::Field);
        }
        Ok(valid(self.bytes, row))
    }

    pub(super) fn int64(&self, row: usize) -> Result<Option<i64>, FormatError> {
        if !self.present(row, DataType::Int64)? {
            return Ok(None);
        }
        let at = bitmap_bytes(self.rows) + row * 8;
        Ok(Some(i64::from_le_bytes(
            self.bytes[at..at + 8].try_into().expect("INT64 width"),
        )))
    }

    pub(super) fn double(&self, row: usize) -> Result<Option<f64>, FormatError> {
        if !self.present(row, DataType::Double)? {
            return Ok(None);
        }
        Ok(Some(f64::from_bits(u64_at(
            self.bytes,
            bitmap_bytes(self.rows) + row * 8,
        ))))
    }

    pub(super) fn date(&self, row: usize) -> Result<Option<DateValue>, FormatError> {
        if !self.present(row, DataType::Date)? {
            return Ok(None);
        }
        let at = bitmap_bytes(self.rows) + row * 4;
        Ok(Some(
            DateValue::from_days(i32::from_le_bytes(
                self.bytes[at..at + 4].try_into().expect("DATE width"),
            ))
            .expect("validated DATE"),
        ))
    }

    pub(super) fn string(&self, row: usize) -> Result<Option<&'a str>, FormatError> {
        if !self.present(row, DataType::String)? {
            return Ok(None);
        }
        let validity = bitmap_bytes(self.rows);
        let base = validity + (self.rows + 1) * 4;
        let start = u32_at(self.bytes, validity + row * 4) as usize;
        let end = u32_at(self.bytes, validity + (row + 1) * 4) as usize;
        Ok(Some(
            std::str::from_utf8(&self.bytes[base + start..base + end]).expect("validated UTF-8"),
        ))
    }
}

#[cfg(test)]
mod tests;
