//! Complete flat success history for catalog publication.
//! Private append preserves the validated old prefix. Lookup reports membership,
//! never abort: the live resolver must also account for active writer authority.
use crate::catalog::{self, ObjectId, ObjectRef};
use crate::effects::{Effect, Effects, MetadataKind};
use crate::storage_format::{
    CatalogCommit, Crc32c, DatabaseId, MAX_SUCCESSES, RootState, TransactionId, WalRecord, crc32c,
};
use crate::{CancellationToken, Error};
use pipesql_filesystem as filesystem;
use std::fs::File;
use std::path::Path;
pub(super) const SCRATCH_BYTES: usize = 65_536;
const HEADER: usize = 64;
const MAGIC: &[u8; 8] = b"PSQLSUCC";
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

fn scratch(buffer: &mut [u8]) -> Result<&mut [u8], Error> {
    if buffer.len() < SCRATCH_BYTES {
        return Err(Error::Resource {
            owner: "success-index scratch bytes",
            required: SCRATCH_BYTES as u64,
            limit: buffer.len() as u64,
        });
    }
    Ok(&mut buffer[..SCRATCH_BYTES])
}

fn committed(snapshot: WalRecord) -> Result<Option<CatalogCommit>, Error> {
    let RootState::Catalog(value) = snapshot.state else {
        return Err(Error::Corrupt("success index requires catalog snapshot"));
    };
    if value.is_some_and(|v| {
        !v.transaction().belongs_to(snapshot.database)
            || v.transaction().sequence() > snapshot.issued
    }) {
        return Err(Error::Corrupt("success snapshot identity is invalid"));
    }
    Ok(value)
}

struct Reader {
    file: File,
    expected: u32,
    checksum: Crc32c,
    count: u64,
    seen: u64,
    previous: u64,
    last: u64,
}

impl Reader {
    fn open(
        objects: &Path,
        database: DatabaseId,
        commit: CatalogCommit,
        buffer: &mut [u8],
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Self, Error> {
        let reference = commit.successes();
        let (file, length) = catalog::open_object(objects, reference.object(), cancel, effects)?;
        if length != u64::from(reference.bytes()) {
            return Err(Error::Corrupt("success index extent differs"));
        }
        cancel.check()?;
        crate::effects::read_exact_at(&file, &mut buffer[..HEADER], 0, READ, effects)?;
        let header = &buffer[..HEADER];
        if &header[..8] != MAGIC {
            return Err(Error::Corrupt("success index magic differs"));
        }
        if u32_at(header, 8) != 6 {
            return Err(Error::UnsupportedVersion(u32_at(header, 8)));
        }
        if header[12..16] != [0; 4]
            || header[52..56] != [0; 4]
            || &header[16..32] != database.as_bytes()
            || u64_at(header, 32) != commit.generation()
            || u64_at(header, 40) != reference.object().attempt()
            || u32_at(header, 48) != reference.object().ordinal()
            || u64_at(header, 56) != commit.transaction().sequence()
        {
            return Err(Error::Corrupt(
                "success index header disagrees with snapshot",
            ));
        }
        let mut checksum = Crc32c::new();
        checksum.update(header);
        Ok(Self {
            file,
            expected: reference.checksum(),
            checksum,
            count: commit.generation(),
            seen: 0,
            previous: 0,
            last: commit.transaction().sequence(),
        })
    }
    // Callers cannot publish a result until next returns None and verifies the
    // complete checksum. Partial copied bytes remain private on any failure.
    fn next(
        &mut self,
        buffer: &mut [u8],
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Option<usize>, Error> {
        cancel.check()?;
        if self.seen == self.count {
            if self.previous != self.last || self.checksum.finish() != self.expected {
                return Err(Error::Corrupt(
                    "success index checksum or final sequence differs",
                ));
            }
            return Ok(None);
        }
        let entries = (self.count - self.seen).min((SCRATCH_BYTES / 8) as u64);
        let length = usize::try_from(entries * 8).expect("bounded index block");
        crate::effects::read_exact_at(
            &self.file,
            &mut buffer[..length],
            64 + self.seen * 8,
            READ,
            effects,
        )?;
        for bytes in buffer[..length].as_chunks::<8>().0 {
            let sequence = u64::from_le_bytes(*bytes);
            if sequence <= self.previous || sequence > self.last {
                return Err(Error::Corrupt(
                    "success index is not a strict issued prefix subset",
                ));
            }
            self.previous = sequence;
        }
        self.checksum.update(&buffer[..length]);
        self.seen += entries;
        Ok(Some(length))
    }
}

fn write_verified(
    file: &File,
    buffer: &mut [u8],
    offset: u64,
    cancel: &CancellationToken,
    effects: &mut Effects,
) -> Result<(), Error> {
    cancel.check()?;
    let expected = crc32c(buffer);
    crate::effects::write_all_at(file, buffer, offset, WRITE, effects)?;
    cancel.check()?;
    crate::effects::read_exact_at(file, buffer, offset, READ, effects)?;
    if crc32c(buffer) != expected {
        return Err(Error::Corrupt("success index readback differs"));
    }
    Ok(())
}

pub(super) fn append(
    file: &File,
    objects: &Path,
    prior: WalRecord,
    object: ObjectId,
    buffer: &mut [u8],
    cancel: &CancellationToken,
    effects: &mut Effects,
) -> Result<ObjectRef, Error> {
    cancel.check()?;
    let previous = committed(prior)?;
    let count = previous.map_or(0, CatalogCommit::generation);
    if count == MAX_SUCCESSES {
        return Err(Error::Resource {
            owner: "retained success count",
            required: MAX_SUCCESSES + 1,
            limit: MAX_SUCCESSES,
        });
    }
    if object.attempt() != prior.issued
        || previous.is_some_and(|v| v.transaction().sequence() >= prior.issued)
    {
        return Err(Error::Corrupt("success append needs a new issued attempt"));
    }
    let buffer = scratch(buffer)?;
    let mut reader = previous
        .map(|commit| Reader::open(objects, prior.database, commit, buffer, cancel, effects))
        .transpose()?;
    cancel.check()?;
    effects.before(INSPECT)?;
    let stat = filesystem::file_metadata(file)
        .map_err(|e| crate::error::io_error("inspect private success index", e))?;
    if !stat.file_type().is_file() || !stat.is_empty() || stat.nlink() != 1 {
        return Err(Error::Corrupt(
            "success append requires an empty private file",
        ));
    }
    let mut header = [0; HEADER];
    header[..8].copy_from_slice(MAGIC);
    put_u32(&mut header, 8, 6);
    header[16..32].copy_from_slice(prior.database.as_bytes());
    put_u64(&mut header, 32, count + 1);
    put_u64(&mut header, 40, object.attempt());
    put_u32(&mut header, 48, object.ordinal());
    put_u64(&mut header, 56, prior.issued);
    let mut checksum = Crc32c::new();
    checksum.update(&header);
    let mut offset = 64u64;
    if let Some(reader) = reader.as_mut() {
        while let Some(length) = reader.next(buffer, cancel, effects)? {
            checksum.update(&buffer[..length]);
            write_verified(file, &mut buffer[..length], offset, cancel, effects)?;
            offset = offset
                .checked_add(length as u64)
                .expect("bounded history extent");
        }
    }
    buffer[..8].copy_from_slice(&prior.issued.to_le_bytes());
    checksum.update(&buffer[..8]);
    write_verified(file, &mut buffer[..8], offset, cancel, effects)?;
    let bytes = 64 + (count + 1) * 8;
    assert_eq!(offset + 8, bytes);
    buffer[..HEADER].copy_from_slice(&header);
    write_verified(file, &mut buffer[..HEADER], 0, cancel, effects)?;
    cancel.check()?;
    effects.before(INSPECT)?;
    if filesystem::file_metadata(file)
        .map_err(|e| crate::error::io_error("inspect completed success index", e))?
        .len()
        != bytes
    {
        return Err(Error::Corrupt("success index length changed"));
    }
    cancel.check()?;
    ObjectRef::new(
        object,
        u32::try_from(bytes).expect("bounded success index extent"),
        checksum.finish(),
    )
    .map_err(|_| Error::Corrupt("success reference construction failed"))
}

pub(super) fn find(
    objects: &Path,
    snapshot: WalRecord,
    transaction: TransactionId,
    buffer: &mut [u8],
    cancel: &CancellationToken,
    effects: &mut Effects,
) -> Result<Option<u64>, Error> {
    cancel.check()?;
    let commit = committed(snapshot)?;
    if !transaction.belongs_to(snapshot.database) || transaction.sequence() > snapshot.issued {
        return Err(Error::NotFound);
    }
    let Some(commit) = commit else {
        return Ok(None);
    };
    let buffer = scratch(buffer)?;
    let mut reader = Reader::open(objects, snapshot.database, commit, buffer, cancel, effects)?;
    let mut ordinal = 0;
    let mut found = None;
    while let Some(length) = reader.next(buffer, cancel, effects)? {
        for bytes in buffer[..length].as_chunks::<8>().0 {
            ordinal += 1;
            if u64::from_le_bytes(*bytes) == transaction.sequence() {
                found = Some(ordinal);
            }
        }
    }
    cancel.check()?;
    Ok(found)
}

#[cfg(test)]
mod tests;
