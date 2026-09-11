//! Flat immutable table/unit index. The catalog anchors the complete file; a
//! cursor retains derived page CRCs so later reads keep that integrity boundary.
use crate::catalog::{self, MAX_UNITS, ObjectId, ObjectRef, TableEntry};
use crate::catalog_schema::Schema;
use crate::effects::{Effect, Effects, MetadataKind};
use crate::native_unit::UnitRef;
use crate::storage_format::{Crc32c, DatabaseId, FormatError, crc32c};
use crate::{CancellationToken, Error};
use pipesql_filesystem as filesystem;
use std::fs::File;
use std::path::Path;

pub(super) const HEADER: u32 = 64;
pub(super) const ENTRY_BYTES: u32 = 48;
const PAGE_ENTRIES: usize = 64;
pub(super) const SCRATCH_BYTES: usize = PAGE_ENTRIES * ENTRY_BYTES as usize;
const MAX_PAGES: usize = (MAX_UNITS as usize).div_ceil(PAGE_ENTRIES);
const MAGIC: &[u8; 8] = b"PSQLTBLD";
const FORMAT: u32 = 6;
const READ: Effect = Effect::ReadMetadata(MetadataKind::CatalogObject);
const WRITE: Effect = Effect::WriteMetadata(MetadataKind::CatalogObject);
const INSPECT: Effect = Effect::InspectOpenMetadata(MetadataKind::CatalogObject);

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("fixed field"))
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("fixed field"))
}

fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn corrupt(_: FormatError) -> Error {
    Error::Corrupt("table-data index validation failed")
}

fn scratch(buffer: &mut [u8]) -> Result<&mut [u8], Error> {
    if buffer.len() < SCRATCH_BYTES {
        return Err(Error::Resource {
            owner: "table-data scratch bytes",
            required: SCRATCH_BYTES as u64,
            limit: buffer.len() as u64,
        });
    }
    Ok(&mut buffer[..SCRATCH_BYTES])
}

fn length(count: usize) -> Result<u32, Error> {
    if count == 0 {
        return Err(corrupt(FormatError::Field));
    }
    if count > MAX_UNITS as usize {
        return Err(Error::Resource {
            owner: "table-data units",
            required: count as u64,
            limit: u64::from(MAX_UNITS),
        });
    }
    let count = u32::try_from(count).map_err(|_| corrupt(FormatError::Overflow))?;
    count
        .checked_mul(ENTRY_BYTES)
        .and_then(|n| n.checked_add(HEADER))
        .ok_or_else(|| corrupt(FormatError::Overflow))
}

fn check_unit(unit: UnitRef, previous: Option<ObjectId>, object: ObjectId) -> Result<(), Error> {
    if unit.object() == object
        || unit.object().attempt() > object.attempt()
        || previous.is_some_and(|prior| prior >= unit.object())
    {
        return Err(corrupt(FormatError::Identity));
    }
    Ok(())
}

fn encode_entry(bytes: &mut [u8; ENTRY_BYTES as usize], unit: UnitRef, row_start: u64) {
    bytes.fill(0);
    put_u64(bytes, 0, unit.object().attempt());
    put_u32(bytes, 8, unit.object().ordinal());
    put_u32(bytes, 12, unit.rows());
    put_u32(bytes, 16, unit.bytes());
    put_u32(bytes, 20, unit.metadata_checksum());
    put_u64(bytes, 24, row_start);
}

fn decode_entry(bytes: &[u8; ENTRY_BYTES as usize]) -> Result<(UnitRef, u64), Error> {
    if bytes[32..].iter().any(|b| *b != 0) {
        return Err(corrupt(FormatError::Reserved));
    }
    let object = ObjectId::new(u64_at(bytes, 0), u32_at(bytes, 8)).map_err(corrupt)?;
    Ok((
        UnitRef::new(
            object,
            u32_at(bytes, 12),
            u32_at(bytes, 16),
            u32_at(bytes, 20),
        )
        .map_err(corrupt)?,
        u64_at(bytes, 24),
    ))
}

// Caller owns admission, empty private file, scratch, cleanup and later sync.
// No unit payload is copied; old units can be shared with a new immutable index.
pub(super) fn write(
    file: &File,
    object: ObjectId,
    schema: &Schema<'_>,
    units: &[UnitRef],
    buffer: &mut [u8],
    cancel: &CancellationToken,
    effects: &mut Effects,
) -> Result<ObjectRef, Error> {
    cancel.check()?;
    let bytes = length(units.len())?;
    let buffer = scratch(buffer)?;
    let mut rows = 0u64;
    let mut previous = None;
    for unit in units {
        check_unit(*unit, previous, object)?;
        rows = rows
            .checked_add(u64::from(unit.rows()))
            .ok_or_else(|| corrupt(FormatError::Overflow))?;
        previous = Some(unit.object());
    }
    cancel.check()?;
    effects.before(INSPECT)?;
    let stat = filesystem::file_metadata(file)
        .map_err(|e| crate::error::io_error("inspect private table-data index", e))?;
    if !stat.file_type().is_file() || !stat.is_empty() || stat.nlink() != 1 {
        return Err(Error::Corrupt(
            "table-data construction requires an empty private file",
        ));
    }
    let mut header = [0; HEADER as usize];
    header[..8].copy_from_slice(MAGIC);
    put_u32(&mut header, 8, FORMAT);
    put_u32(&mut header, 12, units.len() as u32);
    header[16..32].copy_from_slice(schema.database().as_bytes());
    put_u64(&mut header, 32, schema.table().value());
    put_u64(&mut header, 40, object.attempt());
    put_u32(&mut header, 48, object.ordinal());
    put_u64(&mut header, 56, rows);
    let mut whole = Crc32c::new();
    whole.update(&header);
    let mut row_start = 0u64;
    for (page, entries) in units.chunks(PAGE_ENTRIES).enumerate() {
        cancel.check()?;
        let used = entries.len() * ENTRY_BYTES as usize;
        for (encoded, unit) in buffer[..used]
            .as_chunks_mut::<{ ENTRY_BYTES as usize }>()
            .0
            .iter_mut()
            .zip(entries)
        {
            encode_entry(encoded, *unit, row_start);
            row_start = row_start
                .checked_add(u64::from(unit.rows()))
                .expect("admitted row count");
        }
        whole.update(&buffer[..used]);
        let checksum = crc32c(&buffer[..used]);
        let offset = u64::from(HEADER) + (page * SCRATCH_BYTES) as u64;
        crate::effects::write_all_at(file, &buffer[..used], offset, WRITE, effects)?;
        cancel.check()?;
        crate::effects::read_exact_at(file, &mut buffer[..used], offset, READ, effects)?;
        if crc32c(&buffer[..used]) != checksum {
            return Err(Error::Corrupt("table-data page readback differs"));
        }
    }
    assert_eq!(row_start, rows);
    cancel.check()?;
    crate::effects::write_all_at(file, &header, 0, WRITE, effects)?;
    cancel.check()?;
    crate::effects::read_exact_at(file, &mut buffer[..HEADER as usize], 0, READ, effects)?;
    if buffer[..HEADER as usize] != header {
        return Err(Error::Corrupt("table-data header readback differs"));
    }
    cancel.check()?;
    effects.before(INSPECT)?;
    if filesystem::file_metadata(file)
        .map_err(|e| crate::error::io_error("inspect constructed table-data index", e))?
        .len()
        != u64::from(bytes)
    {
        return Err(Error::Corrupt("table-data extent differs"));
    }
    cancel.check()?;
    ObjectRef::new(object, bytes, whole.finish()).map_err(corrupt)
}

#[derive(Clone, Copy)]
enum State {
    Active,
    Finished,
    Failed,
}
// Cached page ownership stays with the cursor across moves and yields. There
// is no per-page allocation; a retaining owner accounts for the complete value.
pub(super) struct Cursor {
    file: File,
    buffer: [u8; SCRATCH_BYTES],
    checksums: [u32; MAX_PAGES],
    count: usize,
    index: usize,
    loaded_page: Option<usize>,
    state: State,
}

pub(super) fn open(
    objects: &Path,
    database: DatabaseId,
    table: TableEntry<'_>,
    cancel: &CancellationToken,
    effects: &mut Effects,
) -> Result<Option<Cursor>, Error> {
    cancel.check()?;
    let Some(reference) = table.data() else {
        return Ok(None);
    };
    let mut buffer = [0; SCRATCH_BYTES];
    let (file, bytes) = catalog::open_object(objects, reference.object(), cancel, effects)?;
    if bytes != u64::from(reference.bytes()) {
        return Err(Error::Corrupt("table-data file extent differs"));
    }
    cancel.check()?;
    crate::effects::read_exact_at(&file, &mut buffer[..HEADER as usize], 0, READ, effects)?;
    let header = &buffer[..HEADER as usize];
    if &header[..8] != MAGIC {
        return Err(corrupt(FormatError::Magic));
    }
    if u32_at(header, 8) != FORMAT {
        return Err(Error::UnsupportedVersion(u32_at(header, 8)));
    }
    if &header[16..32] != database.as_bytes()
        || u64_at(header, 32) != table.id().value()
        || ObjectId::new(u64_at(header, 40), u32_at(header, 48)).map_err(corrupt)?
            != reference.object()
    {
        return Err(corrupt(FormatError::Identity));
    }
    if u32_at(header, 12) != table.units()
        || u64_at(header, 56) != table.rows()
        || header[52..56] != [0; 4]
    {
        return Err(corrupt(FormatError::Field));
    }
    let count = table.units() as usize;
    if reference.bytes() != length(count)? {
        return Err(corrupt(FormatError::Length));
    }
    let mut whole = Crc32c::new();
    whole.update(header);
    let mut checksums = [0; MAX_PAGES];
    let mut rows = 0u64;
    let mut previous = None;
    for (page, checksum) in checksums
        .iter_mut()
        .enumerate()
        .take(count.div_ceil(PAGE_ENTRIES))
    {
        cancel.check()?;
        let used = (count - page * PAGE_ENTRIES).min(PAGE_ENTRIES) * ENTRY_BYTES as usize;
        crate::effects::read_exact_at(
            &file,
            &mut buffer[..used],
            u64::from(HEADER) + (page * SCRATCH_BYTES) as u64,
            READ,
            effects,
        )?;
        whole.update(&buffer[..used]);
        *checksum = crc32c(&buffer[..used]);
        for encoded in buffer[..used].as_chunks::<{ ENTRY_BYTES as usize }>().0 {
            let (unit, start) = decode_entry(encoded)?;
            check_unit(unit, previous, reference.object())?;
            if start != rows {
                return Err(corrupt(FormatError::Field));
            }
            rows = rows
                .checked_add(u64::from(unit.rows()))
                .ok_or_else(|| corrupt(FormatError::Overflow))?;
            previous = Some(unit.object());
        }
    }
    if rows != table.rows() || whole.finish() != reference.checksum() {
        return Err(corrupt(FormatError::Checksum));
    }
    cancel.check()?;
    Ok(Some(Cursor {
        file,
        buffer,
        checksums,
        count,
        index: 0,
        loaded_page: None,
        state: State::Active,
    }))
}

impl Cursor {
    pub(super) fn rewind(&mut self) -> Result<(), Error> {
        if matches!(self.state, State::Failed) {
            return Err(Error::Corrupt("failed table-data cursor cannot rewind"));
        }
        // Retain the admitted descriptor and page checksums. Re-read pages after
        // rewind so mutation cannot hide behind a cache from the first pass.
        self.index = 0;
        self.loaded_page = None;
        self.state = State::Active;
        Ok(())
    }

    pub(super) fn next(
        &mut self,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Option<UnitRef>, Error> {
        match self.state {
            State::Finished => return Ok(None),
            State::Failed => return Err(Error::Corrupt("table-data cursor has failed")),
            State::Active => {}
        }
        self.state = State::Failed;
        cancel.check()?;
        if self.index == self.count {
            self.state = State::Finished;
            return Ok(None);
        }
        let page = self.index / PAGE_ENTRIES;
        if self.loaded_page != Some(page) {
            let used = (self.count - page * PAGE_ENTRIES).min(PAGE_ENTRIES) * ENTRY_BYTES as usize;
            crate::effects::read_exact_at(
                &self.file,
                &mut self.buffer[..used],
                u64::from(HEADER) + (page * SCRATCH_BYTES) as u64,
                READ,
                effects,
            )?;
            if crc32c(&self.buffer[..used]) != self.checksums[page] {
                return Err(Error::Corrupt("table-data page changed after admission"));
            }
            self.loaded_page = Some(page);
        }
        let at = (self.index % PAGE_ENTRIES) * ENTRY_BYTES as usize;
        let (unit, _) = decode_entry(
            self.buffer[at..at + ENTRY_BYTES as usize]
                .try_into()
                .expect("validated entry extent"),
        )?;
        cancel.check()?;
        self.index += 1;
        self.state = State::Active;
        Ok(Some(unit))
    }
}

#[cfg(test)]
mod tests;
