//! Disposable external inventory. The maintenance controller supplies exclusive,
//! empty, unlinked scratch files after synchronizing their name removal.
//! A complete merge validates every protected name's presence before any garbage
//! name is exposed. This module neither deletes objects nor publishes roots.
use super::{Maintenance, Reachable};
use crate::catalog::{self, ObjectId};
use crate::catalog_snapshot::SLOTS;
use crate::effects::{Effect, Effects};
use crate::error::io_error;
use crate::scratch::Scratch;
use crate::storage_format::{Crc32c, crc32c};
use crate::{CancellationToken, Error};

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
use std::fs::File;

const RECORD_BYTES: usize = 16;
const CHUNK_RECORDS: usize = 8192;
const PAGE_BYTES: usize = 4096;
const FAN_IN: usize = 8;
const MAX_REFERENCES: usize = SLOTS * (2 + catalog::MAX_TABLES * (2 + catalog::MAX_UNITS as usize));
const MAX_RECORDS: usize = crate::namespace::MAX_CATALOG_OBJECTS + MAX_REFERENCES;
const MAX_RUNS: usize = MAX_RECORDS.div_ceil(CHUNK_RECORDS);
const MAX_PAGES: usize = (MAX_RECORDS * RECORD_BYTES).div_ceil(PAGE_BYTES);

// The first 12 big-endian bytes sort by attempt and object ordinal. Three
// reserved zero bytes precede membership flags: a directory name, a protected
// graph reference, or both. Repeated references are legal; repeated names are not.
const ID_BYTES: usize = 12;
const FLAGS_OFFSET: usize = 15;
const NAME_PRESENT: u8 = 1;
const REFERENCE_PRESENT: u8 = 2;
const KNOWN_FLAGS: u8 = NAME_PRESENT | REFERENCE_PRESENT;

type Record = [u8; RECORD_BYTES];

#[derive(Clone, Copy)]
struct Run {
    offset: u64,
    records: usize,
    checksum: u32,
}

struct Cursor {
    run: Run,
    read: usize,
    at: usize,
    used: usize,
    checksum: Crc32c,
    previous: Option<Record>,
}

impl Cursor {
    fn new(run: Run) -> Self {
        Self {
            run,
            read: 0,
            at: 0,
            used: 0,
            checksum: Crc32c::new(),
            previous: None,
        }
    }

    fn next(
        &mut self,
        scratch: &Scratch<'_>,
        slot: usize,
        buffer: &mut [u8],
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Option<Record>, Error> {
        cancel.check()?;
        if self.at == self.used {
            if self.read == self.run.records {
                if self.checksum.finish() != self.run.checksum {
                    return Err(Error::Corrupt("reclamation run checksum differs"));
                }
                return Ok(None);
            }
            self.used =
                (self.run.records - self.read).min(PAGE_BYTES / RECORD_BYTES) * RECORD_BYTES;
            scratch.read(
                slot,
                &mut buffer[..self.used],
                self.run.offset + (self.read * RECORD_BYTES) as u64,
                cancel,
                effects,
            )?;
            self.checksum.update(&buffer[..self.used]);
            self.read += self.used / RECORD_BYTES;
            self.at = 0;
        }
        let record: Record = buffer[self.at..self.at + RECORD_BYTES]
            .try_into()
            .expect("record page geometry");
        self.at += RECORD_BYTES;
        validate(record)?;
        if self
            .previous
            .is_some_and(|last| last[..ID_BYTES] >= record[..ID_BYTES])
        {
            return Err(Error::Corrupt("reclamation run order differs"));
        }
        self.previous = Some(record);
        Ok(Some(record))
    }
}

fn validate(record: Record) -> Result<(), Error> {
    object(record)?;
    if record[ID_BYTES..FLAGS_OFFSET] != [0; 3]
        || record[FLAGS_OFFSET] == 0
        || record[FLAGS_OFFSET] & !KNOWN_FLAGS != 0
    {
        return Err(Error::Corrupt("reclamation record flags differ"));
    }
    Ok(())
}

fn object(record: Record) -> Result<ObjectId, Error> {
    ObjectId::new(
        u64::from_be_bytes(record[..8].try_into().expect("attempt bytes")),
        u32::from_be_bytes(record[8..ID_BYTES].try_into().expect("ordinal bytes")),
    )
    .map_err(|_| Error::Corrupt("reclamation object identity differs"))
}

fn record(object: ObjectId, protected: bool) -> Record {
    let mut bytes = [0; RECORD_BYTES];
    bytes[..8].copy_from_slice(&object.attempt().to_be_bytes());
    bytes[8..ID_BYTES].copy_from_slice(&object.ordinal().to_be_bytes());
    bytes[FLAGS_OFFSET] = if protected {
        REFERENCE_PRESENT
    } else {
        NAME_PRESENT
    };
    bytes
}

fn combine(left: &mut Record, right: Record) -> Result<(), Error> {
    assert_eq!(left[..ID_BYTES], right[..ID_BYTES]);
    if left[FLAGS_OFFSET] & right[FLAGS_OFFSET] & NAME_PRESENT != 0 {
        return Err(Error::Corrupt("duplicate reclamation namespace entry"));
    }
    left[FLAGS_OFFSET] |= right[FLAGS_OFFSET];
    Ok(())
}

fn allocate<T>(count: usize, memory_limit: u64) -> Result<Vec<T>, Error> {
    let bytes = count
        .checked_mul(std::mem::size_of::<T>())
        .expect("fixed scratch size");
    let mut result = Vec::new();
    result
        .try_reserve_exact(count)
        .map_err(|_| Error::Resource {
            owner: "reclamation scratch allocation",
            required: bytes as u64,
            limit: memory_limit,
        })?;
    if result.capacity() != count {
        return Err(Error::Resource {
            owner: "reclamation scratch capacity",
            required: result.capacity() as u64,
            limit: count as u64,
        });
    }
    Ok(result)
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Building,
    Ready,
    Finished,
    Failed,
}

pub(super) struct Inventory<'a, 'db> {
    writer: &'a Maintenance<'db>,
    scratch: Scratch<'a>,
    chunk: Vec<Record>,
    runs: Vec<Run>,
    io: Vec<u8>,
    page_checksums: Vec<u32>,
    input_records: usize,
    source: usize,
    state: State,
    next_record: usize,
    loaded_page: Option<usize>,
    merge_passes: usize,
    _charge: crate::resources::Reservation<'a>,
}

impl<'a, 'db> Inventory<'a, 'db> {
    const MEMORY_BYTES: u64 = (CHUNK_RECORDS * RECORD_BYTES
        + MAX_RUNS * std::mem::size_of::<Run>()
        + (FAN_IN + 1) * PAGE_BYTES
        + MAX_PAGES * 4) as u64;
    #[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
    fn new(
        writer: &'a Maintenance<'db>,
        files: [File; 2],
        effects: &mut Effects,
    ) -> Result<Self, Error> {
        let scratch = Scratch::from_files(writer.database, files, effects)?;
        Self::from_scratch(writer, scratch)
    }

    pub(super) fn from_scratch(
        writer: &'a Maintenance<'db>,
        scratch: Scratch<'a>,
    ) -> Result<Self, Error> {
        let database = writer.database;
        let charge = database
            .memory
            .reserve(Self::MEMORY_BYTES, "reclamation inventory")?;
        let chunk = allocate(CHUNK_RECORDS, database.memory.limit())?;
        let runs = allocate(MAX_RUNS, database.memory.limit())?;
        let mut io = allocate((FAN_IN + 1) * PAGE_BYTES, database.memory.limit())?;
        io.resize((FAN_IN + 1) * PAGE_BYTES, 0);
        let page_checksums = allocate(MAX_PAGES, database.memory.limit())?;
        Ok(Self {
            writer,
            scratch,
            chunk,
            runs,
            io,
            page_checksums,
            input_records: 0,
            source: 0,
            state: State::Building,
            next_record: 0,
            loaded_page: None,
            merge_passes: 0,
            _charge: charge,
        })
    }

    fn push(
        &mut self,
        id: ObjectId,
        protected: bool,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        assert!(self.state == State::Building);
        cancel.check()?;
        if self.input_records == MAX_RECORDS {
            return Err(Error::Resource {
                owner: "reclamation input records",
                required: MAX_RECORDS as u64 + 1,
                limit: MAX_RECORDS as u64,
            });
        }
        if self.chunk.len() == CHUNK_RECORDS {
            self.flush(cancel, effects)?;
        }
        self.chunk.push(record(id, protected));
        self.input_records += 1;
        Ok(())
    }

    fn flush(&mut self, cancel: &CancellationToken, effects: &mut Effects) -> Result<(), Error> {
        if self.chunk.is_empty() {
            return Ok(());
        }
        cancel.check()?;
        self.chunk.sort_unstable();
        let mut count = 0;
        for read in 0..self.chunk.len() {
            let entry = self.chunk[read];
            if count != 0 && self.chunk[count - 1][..ID_BYTES] == entry[..ID_BYTES] {
                combine(&mut self.chunk[count - 1], entry)?;
            } else {
                self.chunk[count] = entry;
                count += 1;
            }
        }
        self.chunk.truncate(count);
        let bytes = self.chunk.as_flattened();
        let offset = self
            .runs
            .last()
            .map_or(0, |run| run.offset + (run.records * RECORD_BYTES) as u64);
        self.scratch.write(0, offset, bytes, cancel, effects)?;
        assert!(self.runs.len() < MAX_RUNS);
        self.runs.push(Run {
            offset,
            records: count,
            checksum: crc32c(bytes),
        });
        self.chunk.clear();
        Ok(())
    }

    // No object namespace changes while enumeration is live. Scratch writes
    // affect only the caller's unlinked files in a different namespace.
    pub(super) fn scan(
        &mut self,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        if self.state != State::Building || self.input_records != 0 {
            return Err(Error::Unsupported("inventory scan already started"));
        }
        let result = self
            .scan_inner(cancel, effects)
            .and_then(|()| self.finish(cancel, effects));
        if result.is_err() {
            self.state = State::Failed;
        }
        result
    }

    fn scan_inner(
        &mut self,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        let mut reachable = Reachable::open(self.writer)?;
        {
            cancel.check()?;
            effects.before(Effect::ListDatabaseDirectory)?;
            let mut buffer = pipesql_filesystem::DirectoryBuffer::default();
            let mut directory = pipesql_filesystem::Directory::open_bounded(
                &reachable.objects,
                &mut buffer,
                crate::namespace::MAX_CATALOG_OBJECTS + 3,
            )
            .map_err(|e| io_error("list reclamation inventory", e))?;
            let mut count = 0;
            while let Some(name) = directory
                .next_name()
                .map_err(|e| io_error("read reclamation entry", e))?
            {
                effects.before(Effect::ReadDatabaseEntry)?;
                if count == crate::namespace::MAX_CATALOG_OBJECTS {
                    return Err(Error::Resource {
                        owner: "catalog namespace objects",
                        required: count as u64 + 1,
                        limit: count as u64,
                    });
                }
                count += 1;
                let id = ObjectId::from_name(name.as_encoded_bytes())
                    .map_err(|_| Error::Corrupt("unknown reclamation object name"))?;
                if id.attempt() > self.writer.prior.issued {
                    return Err(Error::Corrupt("reclamation object exceeds issuance"));
                }
                self.push(id, false, cancel, effects)?;
            }
        }
        while let Some(id) = reachable.next(cancel, effects)? {
            self.push(id, true, cancel, effects)?;
        }
        Ok(())
    }

    fn merge(&mut self, cancel: &CancellationToken, effects: &mut Effects) -> Result<(), Error> {
        let target = 1 - self.source;
        let mut offset = 0;
        let mut groups = 0;
        for start in (0..self.runs.len()).step_by(FAN_IN) {
            let end = (start + FAN_IN).min(self.runs.len());
            let mut cursors: [Option<Cursor>; FAN_IN] = std::array::from_fn(|i| {
                self.runs
                    .get(start + i)
                    .filter(|_| start + i < end)
                    .copied()
                    .map(Cursor::new)
            });
            let mut heads: [Option<Record>; FAN_IN] = [None; FAN_IN];
            for i in 0..end - start {
                heads[i] = cursors[i].as_mut().expect("run exists").next(
                    &self.scratch,
                    self.source,
                    &mut self.io[i * PAGE_BYTES..(i + 1) * PAGE_BYTES],
                    cancel,
                    effects,
                )?;
            }
            let mut count = 0;
            let mut used = 0;
            let mut crc = Crc32c::new();
            let mut pending: Option<Record> = None;
            // Each transition consumes one head from finite admitted runs.
            while let Some(i) = (0..FAN_IN)
                .filter(|&i| heads[i].is_some())
                .min_by_key(|&i| heads[i])
            {
                let entry = heads[i].take().expect("selected head");
                if let Some(previous) = pending.as_mut()
                    && previous[..ID_BYTES] == entry[..ID_BYTES]
                {
                    combine(previous, entry)?;
                } else {
                    if let Some(previous) = pending {
                        self.io
                            [FAN_IN * PAGE_BYTES + used..FAN_IN * PAGE_BYTES + used + RECORD_BYTES]
                            .copy_from_slice(&previous);
                        used += RECORD_BYTES;
                        count += 1;
                        if used == PAGE_BYTES {
                            let page = &self.io[FAN_IN * PAGE_BYTES..];
                            self.scratch.write(
                                target,
                                offset + (count * RECORD_BYTES - used) as u64,
                                page,
                                cancel,
                                effects,
                            )?;
                            crc.update(page);
                            used = 0;
                        }
                    }
                    pending = Some(entry);
                }
                heads[i] = cursors[i].as_mut().expect("selected cursor").next(
                    &self.scratch,
                    self.source,
                    &mut self.io[i * PAGE_BYTES..(i + 1) * PAGE_BYTES],
                    cancel,
                    effects,
                )?;
            }
            if let Some(previous) = pending {
                self.io[FAN_IN * PAGE_BYTES + used..FAN_IN * PAGE_BYTES + used + RECORD_BYTES]
                    .copy_from_slice(&previous);
                used += RECORD_BYTES;
                count += 1;
            }
            if used != 0 {
                let page = &self.io[FAN_IN * PAGE_BYTES..FAN_IN * PAGE_BYTES + used];
                self.scratch.write(
                    target,
                    offset + (count * RECORD_BYTES - used) as u64,
                    page,
                    cancel,
                    effects,
                )?;
                crc.update(page);
            }
            self.runs[groups] = Run {
                offset,
                records: count,
                checksum: crc.finish(),
            };
            groups += 1;
            offset += (count * RECORD_BYTES) as u64;
        }
        self.runs.truncate(groups);
        self.source = target;
        self.merge_passes += 1;
        Ok(())
    }

    fn finish(&mut self, cancel: &CancellationToken, effects: &mut Effects) -> Result<(), Error> {
        self.flush(cancel, effects)?;
        while self.runs.len() > 1 {
            self.merge(cancel, effects)?;
        }
        if let Some(run) = self.runs.first().copied() {
            let mut cursor = Cursor::new(run);
            // Validate the entire result, including references without a name,
            // before permitting any absence result. Cache page CRCs for rereads.
            while let Some(record) = cursor.next(
                &self.scratch,
                self.source,
                &mut self.io[..PAGE_BYTES],
                cancel,
                effects,
            )? {
                if cursor.at == RECORD_BYTES {
                    self.page_checksums.push(crc32c(&self.io[..cursor.used]));
                }
                if record[FLAGS_OFFSET] == REFERENCE_PRESENT {
                    return Err(Error::Corrupt("protected reclamation object is missing"));
                }
            }
        }
        self.state = State::Ready;
        Ok(())
    }

    pub(super) fn next(
        &mut self,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Option<ObjectId>, Error> {
        if self.state == State::Finished {
            return Ok(None);
        }
        if self.state != State::Ready {
            return Err(Error::Unsupported("reclamation inventory is incomplete"));
        }
        let result = self.next_inner(cancel, effects);
        if result.is_err() {
            self.state = State::Failed;
        }
        result
    }

    fn next_inner(
        &mut self,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Option<ObjectId>, Error> {
        let count = self.runs.first().map_or(0, |run| run.records);
        while self.next_record < count {
            cancel.check()?;
            let page = self.next_record / (PAGE_BYTES / RECORD_BYTES);
            if self.loaded_page != Some(page) {
                let offset = page * PAGE_BYTES;
                let used = (count * RECORD_BYTES - offset).min(PAGE_BYTES);
                self.scratch.read(
                    self.source,
                    &mut self.io[..used],
                    offset as u64,
                    cancel,
                    effects,
                )?;
                if crc32c(&self.io[..used]) != self.page_checksums[page] {
                    return Err(Error::Corrupt("reclamation inventory page changed"));
                }
                self.loaded_page = Some(page);
            }
            let offset = self.next_record % (PAGE_BYTES / RECORD_BYTES) * RECORD_BYTES;
            let entry: Record = self.io[offset..offset + RECORD_BYTES]
                .try_into()
                .expect("record page");
            self.next_record += 1;
            if entry[FLAGS_OFFSET] == NAME_PRESENT {
                return object(entry).map(Some);
            }
        }
        self.state = State::Finished;
        Ok(None)
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;
