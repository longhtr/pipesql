//! Immutable declared-table catalog references. Callers own the
//! admitted namespace, buffers and snapshot lifetime; this module cannot publish.
use crate::catalog_schema::{self, Schema, TableId};
use crate::effects::{Effect, Effects, MetadataKind};
use crate::storage_format::{DatabaseId, FormatError, crc32c};
use crate::{CancellationToken, Error};
use std::path::Path;

pub(super) const MAX_TABLES: usize = 64;
pub(super) const MAX_UNITS: u32 = 4_096;
const HEADER: usize = 64;
const TABLE: usize = 128;
pub(super) const MAX_BYTES: usize = HEADER + TABLE * MAX_TABLES;
const MAGIC: &[u8; 8] = b"PSQLCATL";
const FORMAT: u32 = 6;
const REFERENCE_BYTES: usize = 24;
use crate::table_data::{ENTRY_BYTES as DATA_ENTRY, HEADER as DATA_HEADER};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct ObjectId {
    attempt: u64,
    ordinal: u32,
}

impl ObjectId {
    pub(super) fn new(attempt: u64, ordinal: u32) -> Result<Self, FormatError> {
        if attempt == 0 || ordinal == 0 {
            return Err(FormatError::Identity);
        }
        Ok(Self { attempt, ordinal })
    }

    pub(super) fn attempt(self) -> u64 {
        self.attempt
    }

    pub(super) fn ordinal(self) -> u32 {
        self.ordinal
    }
    // Accept only the fixed ASCII spelling. Directory callers can inspect
    // OsStr::as_encoded_bytes without allocating or decoding a native pathname.
    pub(super) fn from_name(name: &[u8]) -> Result<Self, FormatError> {
        if name.len() != 29 || name[16] != b'-' || &name[25..] != b".obj" {
            return Err(FormatError::Identity);
        }

        fn hex(bytes: &[u8]) -> Result<u64, FormatError> {
            let mut value = 0u64;
            for byte in bytes {
                let digit = match byte {
                    b'0'..=b'9' => byte - b'0',
                    b'a'..=b'f' => byte - b'a' + 10,
                    _ => return Err(FormatError::Identity),
                };
                value = value
                    .checked_mul(16)
                    .and_then(|v| v.checked_add(u64::from(digit)))
                    .ok_or(FormatError::Overflow)?;
            }
            Ok(value)
        }
        Self::new(
            hex(&name[..16])?,
            u32::try_from(hex(&name[17..25])?).map_err(|_| FormatError::Overflow)?,
        )
    }
    // A fixed ASCII filename keeps persisted references from becoming paths.
    pub(super) fn name(self) -> [u8; 29] {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut name = *b"0000000000000000-00000000.obj";
        for (index, byte) in self.attempt.to_be_bytes().iter().enumerate() {
            name[index * 2] = HEX[usize::from(byte >> 4)];
            name[index * 2 + 1] = HEX[usize::from(byte & 15)];
        }
        for (index, byte) in self.ordinal.to_be_bytes().iter().enumerate() {
            name[17 + index * 2] = HEX[usize::from(byte >> 4)];
            name[18 + index * 2] = HEX[usize::from(byte & 15)];
        }
        name
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ObjectRef {
    id: ObjectId,
    bytes: u32,
    crc: u32,
}

impl ObjectRef {
    pub(super) fn new(id: ObjectId, bytes: u32, crc: u32) -> Result<Self, FormatError> {
        // Every object contains at least its format/identity header. The parent
        // role imposes the tighter maximum before any allocation or I/O.
        if bytes < HEADER as u32 {
            return Err(FormatError::Length);
        }
        Ok(Self { id, bytes, crc })
    }

    pub(super) fn object(self) -> ObjectId {
        self.id
    }

    pub(super) fn bytes(self) -> u32 {
        self.bytes
    }

    pub(super) fn checksum(self) -> u32 {
        self.crc
    }

    pub(super) fn encode(self, out: &mut [u8; REFERENCE_BYTES]) {
        out.fill(0);
        out[..8].copy_from_slice(&self.id.attempt.to_le_bytes());
        out[8..12].copy_from_slice(&self.id.ordinal.to_le_bytes());
        out[12..16].copy_from_slice(&self.bytes.to_le_bytes());
        out[16..20].copy_from_slice(&self.crc.to_le_bytes());
    }

    pub(super) fn decode(bytes: &[u8; REFERENCE_BYTES]) -> Result<Self, FormatError> {
        zero(&bytes[20..])?;
        Self::new(
            ObjectId::new(u64_at(bytes, 0), u32_at(bytes, 8))?,
            u32_at(bytes, 12),
            u32_at(bytes, 16),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TableEntry<'a> {
    id: TableId,
    name: &'a str,
    schema: ObjectRef,
    data: Option<ObjectRef>,
    rows: u64,
    units: u32,
}

impl<'a> TableEntry<'a> {
    pub(super) fn new(
        id: TableId,
        name: &'a str,
        schema: ObjectRef,
        data: Option<ObjectRef>,
        rows: u64,
        units: u32,
    ) -> Result<Self, FormatError> {
        catalog_schema::validate_name(name.as_bytes())?;
        if !catalog_schema::valid_extent(schema.bytes) {
            return Err(FormatError::Length);
        }
        if units > MAX_UNITS || rows < u64::from(units) {
            return Err(FormatError::Field);
        }
        let maximum_rows = u64::from(units)
            .checked_mul(crate::native_unit::MAX_ROWS as u64)
            .ok_or(FormatError::Overflow)?;
        if rows > maximum_rows {
            return Err(FormatError::Field);
        }
        match data {
            None if rows == 0 && units == 0 => {}
            Some(reference) if units != 0 && reference.id != schema.id => {
                let expected = units
                    .checked_mul(DATA_ENTRY)
                    .and_then(|n| n.checked_add(DATA_HEADER))
                    .ok_or(FormatError::Overflow)?;
                if reference.bytes != expected {
                    return Err(FormatError::Length);
                }
            }
            _ => return Err(FormatError::Field),
        }
        Ok(Self {
            id,
            name,
            schema,
            data,
            rows,
            units,
        })
    }

    pub(super) fn with_data(
        self,
        data: ObjectRef,
        rows: u64,
        units: u32,
    ) -> Result<Self, FormatError> {
        Self::new(self.id, self.name, self.schema, Some(data), rows, units)
    }

    pub(super) fn id(self) -> TableId {
        self.id
    }

    pub(super) fn name(self) -> &'a str {
        self.name
    }

    pub(super) fn rows(self) -> u64 {
        self.rows
    }

    pub(super) fn units(self) -> u32 {
        self.units
    }

    pub(super) fn schema_object(self) -> ObjectId {
        self.schema.object()
    }

    pub(super) fn data(self) -> Option<ObjectRef> {
        self.data
    }
}

pub(super) struct Catalog<'a> {
    bytes: &'a [u8],
    database: DatabaseId,
    tables: usize,
}

impl<'a> Catalog<'a> {
    pub(super) fn len(&self) -> usize {
        self.tables
    }

    pub(super) fn table(&self, ordinal: usize) -> Option<TableEntry<'a>> {
        let entry = self.bytes[HEADER..].as_chunks::<TABLE>().0.get(ordinal)?;
        Some(decode_table(entry).expect("validated immutable catalog"))
    }

    #[cfg(test)]
    pub(super) fn find(&self, name: &str) -> Option<TableEntry<'a>> {
        (0..self.tables)
            .filter_map(|index| self.table(index))
            .find(|entry| entry.name.eq_ignore_ascii_case(name))
    }

    pub(super) fn open_data(
        &self,
        objects: &Path,
        ordinal: usize,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Option<crate::table_data::Cursor>, Error> {
        let table = self
            .table(ordinal)
            .ok_or(Error::Corrupt("catalog table ordinal is invalid"))?;
        crate::table_data::open(objects, self.database, table, cancel, effects)
    }
    // Only a table from this catalog can choose the schema reference. Callers
    // cannot substitute an unrelated name, checksum or table identity.
    pub(super) fn read_schema<'buffer>(
        &self,
        objects: &Path,
        ordinal: usize,
        buffer: &'buffer mut [u8],
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Schema<'buffer>, Error> {
        let entry = self
            .table(ordinal)
            .ok_or(Error::Corrupt("catalog table ordinal is invalid"))?;
        let bytes = read_object(
            objects,
            entry.schema,
            catalog_schema::MAX_BYTES,
            buffer,
            cancel,
            effects,
        )?;
        catalog_schema::decode(bytes, self.database, entry.id, entry.schema.crc)
            .map_err(|error| crate::error::map_format_error(error, bytes))
    }
}

fn extent(count: usize) -> Result<usize, FormatError> {
    if count > MAX_TABLES {
        return Err(FormatError::Field);
    }
    count
        .checked_mul(TABLE)
        .and_then(|n| n.checked_add(HEADER))
        .ok_or(FormatError::Overflow)
}

fn zero(bytes: &[u8]) -> Result<(), FormatError> {
    if bytes.iter().any(|byte| *byte != 0) {
        Err(FormatError::Reserved)
    } else {
        Ok(())
    }
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("validated fixed field"))
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("validated fixed field"))
}

fn decode_table(bytes: &[u8; TABLE]) -> Result<TableEntry<'_>, FormatError> {
    zero(&bytes[9..16])?;
    zero(&bytes[108..])?;
    let length = usize::from(bytes[8]);
    if !(1..=32).contains(&length) {
        return Err(FormatError::Field);
    }
    zero(&bytes[16 + length..48])?;
    let name = std::str::from_utf8(&bytes[16..16 + length]).map_err(|_| FormatError::Field)?;
    let schema = ObjectRef::decode(bytes[48..72].try_into().expect("reference field"))?;
    let data_bytes = &bytes[72..96];
    let data = if data_bytes == [0; REFERENCE_BYTES] {
        None
    } else {
        Some(ObjectRef::decode(
            data_bytes.try_into().expect("reference field"),
        )?)
    };
    TableEntry::new(
        TableId::new(u64_at(bytes, 0))?,
        name,
        schema,
        data,
        u64_at(bytes, 96),
        u32_at(bytes, 104),
    )
}

fn validate_entry(
    entry: TableEntry<'_>,
    prior: Option<TableEntry<'_>>,
    object: ObjectId,
) -> Result<(), FormatError> {
    // Monotonic IDs permit bounded merge/reclamation walks without sorting.
    if prior.is_some_and(|prior| prior.id.value() >= entry.id.value())
        || entry.schema.id == object
        || entry.schema.id.attempt > object.attempt
        || entry
            .data
            .is_some_and(|data| data.id == object || data.id.attempt > object.attempt)
    {
        return Err(FormatError::Identity);
    }
    Ok(())
}

fn no_alias(left: TableEntry<'_>, right: TableEntry<'_>) -> Result<(), FormatError> {
    if left.name.eq_ignore_ascii_case(right.name)
        || left.schema.id == right.schema.id
        || left
            .data
            .is_some_and(|d| d.id == right.schema.id || right.data.is_some_and(|r| r.id == d.id))
        || right.data.is_some_and(|d| d.id == left.schema.id)
    {
        return Err(FormatError::Field);
    }
    Ok(())
}

pub(super) fn encode(
    out: &mut [u8],
    database: DatabaseId,
    object: ObjectId,
    tables: &[TableEntry<'_>],
) -> Result<ObjectRef, FormatError> {
    let length = extent(tables.len())?;
    if out.len() < length {
        return Err(FormatError::Length);
    }
    for (index, entry) in tables.iter().copied().enumerate() {
        validate_entry(entry, index.checked_sub(1).map(|i| tables[i]), object)?;
        for prior in &tables[..index] {
            no_alias(entry, *prior)?;
        }
    }
    // All fallible admission precedes mutation of the caller's private buffer.
    let count = u32::try_from(tables.len()).map_err(|_| FormatError::Overflow)?;
    let length_u32 = u32::try_from(length).map_err(|_| FormatError::Overflow)?;
    out[..length].fill(0);
    out[..8].copy_from_slice(MAGIC);
    out[8..12].copy_from_slice(&FORMAT.to_le_bytes());
    out[12..16].copy_from_slice(&count.to_le_bytes());
    out[16..32].copy_from_slice(database.as_bytes());
    out[32..40].copy_from_slice(&object.attempt.to_le_bytes());
    out[40..44].copy_from_slice(&object.ordinal.to_le_bytes());
    for (entry, bytes) in tables
        .iter()
        .zip(out[HEADER..length].as_chunks_mut::<TABLE>().0)
    {
        bytes[..8].copy_from_slice(&entry.id.value().to_le_bytes());
        bytes[8] = u8::try_from(entry.name.len()).expect("validated name");
        bytes[16..16 + entry.name.len()].copy_from_slice(entry.name.as_bytes());
        entry
            .schema
            .encode((&mut bytes[48..72]).try_into().expect("reference field"));
        if let Some(data) = entry.data {
            data.encode((&mut bytes[72..96]).try_into().expect("reference field"));
        }
        bytes[96..104].copy_from_slice(&entry.rows.to_le_bytes());
        bytes[104..108].copy_from_slice(&entry.units.to_le_bytes());
    }
    Ok(ObjectRef {
        id: object,
        bytes: length_u32,
        crc: crc32c(&out[..length]),
    })
}

pub(super) fn decode(
    bytes: &[u8],
    database: DatabaseId,
    reference: ObjectRef,
) -> Result<Catalog<'_>, FormatError> {
    if bytes.len() < 12 {
        return Err(FormatError::Length);
    }
    if &bytes[..8] != MAGIC {
        return Err(FormatError::Magic);
    }
    if u32_at(bytes, 8) != FORMAT {
        return Err(FormatError::Version);
    }
    if bytes.len() < HEADER
        || bytes.len() > MAX_BYTES
        || u64::try_from(bytes.len()).map_err(|_| FormatError::Overflow)?
            != u64::from(reference.bytes)
    {
        return Err(FormatError::Length);
    }
    if crc32c(bytes) != reference.crc {
        return Err(FormatError::Checksum);
    }
    if &bytes[16..32] != database.as_bytes()
        || ObjectId::new(u64_at(bytes, 32), u32_at(bytes, 40))? != reference.id
    {
        return Err(FormatError::Identity);
    }
    zero(&bytes[44..HEADER])?;
    let count = usize::try_from(u32_at(bytes, 12)).map_err(|_| FormatError::Overflow)?;
    if bytes.len() != extent(count)? {
        return Err(FormatError::Length);
    }
    let entries = bytes[HEADER..].as_chunks::<TABLE>().0;
    let mut previous = None;
    for (index, bytes) in entries.iter().enumerate() {
        let entry = decode_table(bytes)?;
        validate_entry(entry, previous, reference.id)?;
        for prior in &entries[..index] {
            no_alias(entry, decode_table(prior)?)?;
        }
        previous = Some(entry);
    }
    Ok(Catalog {
        bytes,
        database,
        tables: count,
    })
}

fn read_object<'a>(
    objects: &Path,
    reference: ObjectRef,
    maximum: usize,
    buffer: &'a mut [u8],
    cancel: &CancellationToken,
    effects: &mut Effects,
) -> Result<&'a [u8], Error> {
    cancel.check()?;
    let length = usize::try_from(reference.bytes)
        .map_err(|_| Error::Corrupt("object extent exceeds native range"))?;
    if length > maximum {
        return Err(Error::Corrupt("catalog object exceeds role capacity"));
    }
    if length > buffer.len() {
        return Err(Error::Resource {
            owner: "catalog read buffer",
            required: u64::from(reference.bytes),
            limit: u64::try_from(buffer.len()).expect("buffer length fits u64"),
        });
    }
    let (file, observed) = open_object(objects, reference.id, cancel, effects)?;
    if observed != u64::from(reference.bytes) {
        return Err(Error::Corrupt("catalog object has incorrect extent"));
    }
    cancel.check()?;
    crate::effects::read_exact_at(
        &file,
        &mut buffer[..length],
        0,
        Effect::ReadMetadata(MetadataKind::CatalogObject),
        effects,
    )?;
    cancel.check()?;
    // Decode owns magic/version precedence and validates the parent's CRC.
    Ok(&buffer[..length])
}

pub(super) fn open_object(
    objects: &Path,
    object: ObjectId,
    cancel: &CancellationToken,
    effects: &mut Effects,
) -> Result<(std::fs::File, u64), Error> {
    cancel.check()?;
    let name = object.name();
    let path = crate::path::joined_path(
        objects,
        std::str::from_utf8(&name).expect("hex object name"),
    )?;
    let identity = crate::namespace::validate_regular_file_type(
        &path,
        "catalog object is not a regular file",
        effects,
    )?;
    cancel.check()?;
    crate::namespace::open_metadata(&path, identity, effects, MetadataKind::CatalogObject)
}

pub(super) fn read<'a>(
    objects: &Path,
    database: DatabaseId,
    reference: ObjectRef,
    buffer: &'a mut [u8],
    cancel: &CancellationToken,
    effects: &mut Effects,
) -> Result<Catalog<'a>, Error> {
    let bytes = read_object(objects, reference, MAX_BYTES, buffer, cancel, effects)?;
    decode(bytes, database, reference).map_err(|error| crate::error::map_format_error(error, bytes))
}

// Field order releases the physical buffer before its accounting reservation.
pub(super) struct Scratch<'db> {
    bytes: Vec<u8>,
    _reservation: crate::resources::Reservation<'db>,
}

impl<'db> Scratch<'db> {
    pub(super) fn new(
        memory: &'db crate::resources::MemoryAuthority,
        owner: &'static str,
    ) -> Result<Self, Error> {
        Self::sized(memory, SNAPSHOT_SCRATCH_BYTES, owner)
    }

    pub(super) fn sized(
        memory: &'db crate::resources::MemoryAuthority,
        size: usize,
        owner: &'static str,
    ) -> Result<Self, Error> {
        let reservation = memory.reserve(size as u64, owner)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(size).map_err(|_| Error::Resource {
            owner,
            required: size as u64,
            limit: memory.limit(),
        })?;
        if bytes.capacity() != size {
            return Err(Error::Resource {
                owner,
                required: bytes.capacity() as u64,
                limit: size as u64,
            });
        }
        bytes.resize(size, 0);
        Ok(Self {
            bytes,
            _reservation: reservation,
        })
    }

    pub(super) fn bytes(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}

// Recovery admission shares scratch between history validation and graph walking.
// It retains no graph or file handle after returning. Payload integrity remains
// the demanded-column reader's responsibility; all referenced metadata is checked.
pub(super) const SNAPSHOT_SCRATCH_BYTES: usize = crate::success_index::SCRATCH_BYTES;

pub(super) fn validate_snapshot(
    objects: &Path,
    snapshot: crate::storage_format::WalRecord,
    buffer: &mut [u8],
    cancel: &CancellationToken,
    effects: &mut Effects,
) -> Result<(), Error> {
    cancel.check()?;
    let crate::storage_format::RootState::Catalog(commit) = snapshot.state else {
        return Err(Error::Corrupt(
            "catalog admission requires catalog snapshot",
        ));
    };
    let Some(commit) = commit else {
        return Ok(());
    };
    if buffer.len() < SNAPSHOT_SCRATCH_BYTES {
        return Err(Error::Resource {
            owner: "catalog snapshot scratch bytes",
            required: SNAPSHOT_SCRATCH_BYTES as u64,
            limit: buffer.len() as u64,
        });
    }
    // Lookup validates the entire history before exposing even the last entry.
    if crate::success_index::find(
        objects,
        snapshot,
        commit.transaction(),
        buffer,
        cancel,
        effects,
    )? != Some(commit.generation())
    {
        return Err(Error::Corrupt("catalog snapshot success history differs"));
    }
    // Unit metadata and the index page now live in their reader values.
    const GRAPH_BYTES: usize = MAX_BYTES + catalog_schema::MAX_BYTES;
    const { assert!(GRAPH_BYTES <= SNAPSHOT_SCRATCH_BYTES) };
    let (catalog_bytes, rest) = buffer.split_at_mut(MAX_BYTES);
    let schema_bytes = rest;
    let catalog = read(
        objects,
        snapshot.database,
        commit.catalog(),
        catalog_bytes,
        cancel,
        effects,
    )?;
    for table in 0..catalog.len() {
        let schema = catalog.read_schema(objects, table, schema_bytes, cancel, effects)?;
        if let Some(mut units) = catalog.open_data(objects, table, cancel, effects)? {
            while let Some(reference) = units.next(cancel, effects)? {
                // Drop the validated file owner before advancing the next unit.
                crate::native_unit::read(
                    objects,
                    snapshot.database,
                    reference,
                    &schema,
                    cancel,
                    effects,
                )?;
            }
        }
    }
    cancel.check()
}

#[cfg(test)]
mod tests;
