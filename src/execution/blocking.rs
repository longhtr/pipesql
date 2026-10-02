//! Sort more rows than fit in memory by writing and merging sorted chunks.
//!
//! A sorted chunk is a run. `RowSort` collects encoded rows in a bounded buffer,
//! sorts their byte ranges without moving the payloads, and writes each run to
//! scratch. After input ends, `MergePasses` combines pairs of runs until one
//! remains. Each pass reads one file and writes the other. Streaming run headers
//! avoids a growing in-memory list of every run on disk.
//!
//! Callers push one record at a time. If `push` returns false, keep that record
//! and step the sorter until it can collect again, then retry. After `finish`,
//! step to Done and borrow the sorted cursor. Equal keys use the original row
//! number to keep their input order, which matters for floating aggregates.
//!
//! `SortedInput` connects this machinery to ORDER/DISTINCT, joins and set
//! operations. Grouping uses `RowSort` directly to carry evaluated arguments.
//! The record module owns typed encoding and comparison; the I/O module owns
//! buffered transfers. Callers reserve these owners before requesting source
//! rows and keep scratch files alive while the sorted output is needed.
//!
//! Readers check record bytes and run extents; merging also checks output order
//! and row counts. An error leaves the enclosing sorter failed, so partial disk
//! output cannot be retried as completed work. Scratch is temporary query data
//! and is not part of the durable database format.

use crate::effects::Effects;
use crate::execution::{BATCH_ROWS, MAX_AGGREGATE_ROWS};
use crate::query::{MAX_AGGREGATE_COLUMNS, MAX_ROW_VALUES};
use crate::resources::{Reservation, allocate};
use crate::storage::format;
use crate::value::Value;
use crate::{CancellationToken, Database, Error};
use std::cmp::Ordering;
use std::mem::size_of;

mod io;
mod record;

#[cfg(test)]
pub(super) use io::IO_BYTES;
pub(super) use io::{IO_MEMORY_BYTES, Io, ReadAt, ReadBuffer, WriteBuffer};
use record::compare_values;
pub(super) use record::{
    ArgumentShape, KeyColumn, RowLayout, SortRecord, append_bytes, append_value, read_value,
};

pub(super) mod join;
pub(super) mod order;
pub(super) mod sorted_set;

// Reserve creation memory before input begins, but create files only if needed.
// Taking Pending sets Failed first so a failed creation cannot be retried here.
pub(super) enum Files<'db> {
    Pending(crate::storage::scratch::Creation<'db>),
    Open(crate::storage::scratch::Scratch<'db>),
    Failed,
}

impl<'db> Files<'db> {
    pub(super) fn create(
        &mut self,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        let Self::Pending(creation) = std::mem::replace(self, Self::Failed) else {
            return Err(Error::Corrupt("group scratch creation state"));
        };
        *self = Self::Open(creation.create(cancel, effects)?);
        Ok(())
    }

    pub(super) fn io<'a>(
        &'a mut self,
        cancel: &'a CancellationToken,
        effects: &'a mut Effects,
    ) -> Result<Io<'a, 'db>, Error> {
        match self {
            Self::Open(scratch) => Ok(Io::new(scratch, cancel, effects)),
            _ => Err(Error::Corrupt("group scratch is not open")),
        }
    }
}

const ROW_ARGUMENTS: ArgumentShape = ArgumentShape {
    count: 0,
    nonnull: 0,
    integers: 0,
    presence: 0,
    dates: 0,
    text: 0,
};

struct SortedInput<'db> {
    layout: RowLayout,
    sort: RowSort<'db>,
    record: SortRecord,
    files: Files<'db>,
    ordinal: u64,
    reservation: Reservation<'db>,
}

impl<'db> SortedInput<'db> {
    fn new(database: &'db Database, layout: RowLayout) -> Result<Self, Error> {
        let record_bytes = RECORD_HEADER + layout.max_bytes;
        let inline = size_of::<Self>() - size_of::<RowSort<'_>>();
        let reservation = database.reserve_memory(
            (inline + buffer_memory_bytes(record_bytes)?) as u64,
            "sorted input",
        )?;
        let record = SortRecord::new(record_bytes, reservation.bytes())?;
        // One maximum row always fits. More short rows share the remaining run
        // space; row count and bytes independently bound sorting work.
        let run_bytes = record_bytes + (BATCH_ROWS - 1) * (RECORD_HEADER + layout.count);
        let sort = RowSort::new(database, &layout, ROW_ARGUMENTS, run_bytes, BATCH_ROWS)?;
        let files = Files::Pending(crate::storage::scratch::Creation::reserve(database)?);
        Ok(Self {
            layout,
            sort,
            record,
            files,
            ordinal: 0,
            reservation,
        })
    }

    fn start(&mut self, cancel: &CancellationToken, effects: &mut Effects) -> Result<(), Error> {
        // Execution begins only after every runtime owner has been admitted.
        // Optional replacement buffers coexist with the empty minimum buffers;
        // refusal leaves those buffers usable without changing query admission.
        cancel.check()?;
        self.files.create(cancel, effects)?;
        match self
            .sort
            .buffer
            .grow_empty(RECORD_HEADER + self.layout.count, cancel)
        {
            Ok(()) | Err(Error::Resource { .. } | Error::Contention(_)) => (),
            Err(error) => return Err(error),
        }
        Ok(())
    }

    // Join, Order and SortedSet live in one-element vectors for fallible heap
    // construction. Vec drops their fields before freeing their inline storage.
    // Move only that inline charge to the runtime, which outlives the vector;
    // separately allocated payloads retain their charges here until freed.
    fn transfer_inline_to(&mut self, destination: &mut Reservation<'db>) -> Result<(), Error> {
        self.reservation.transfer_to(
            destination,
            (size_of::<Self>() - size_of::<RowSort<'_>>()) as u64,
        )?;
        self.sort.transfer_inline_to(destination)
    }

    fn memory_bytes(&self) -> u64 {
        self.reservation.bytes()
            + self.sort.memory_bytes()
            + if matches!(self.files, Files::Pending(_)) {
                crate::storage::scratch::Creation::memory_requirement_bytes()
            } else {
                0
            }
    }

    fn sort_step(
        &mut self,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<bool, Error> {
        // The last write clears the run's rows and bytes. Retain its observed
        // width first, then grow only after the arena is empty. Final runs are
        // about to release their arena, so they need no replacement.
        let growth = if !self.sort.finishing {
            self.sort.buffer.growth_after_write()?
        } else {
            None
        };
        let phase = self
            .sort
            .step(&self.layout, &mut self.files.io(cancel, effects)?)?;
        if let Some(growth) = growth {
            assert_eq!(phase, SortPhase::Collect);
            let result = match growth {
                RunGrowth::Bytes(bytes) => self.sort.buffer.grow_bytes_empty(bytes, cancel),
                RunGrowth::Rows { bytes, rows } => {
                    self.sort.buffer.replace_empty(bytes, rows, cancel)
                }
            };
            match result {
                Ok(()) | Err(Error::Resource { .. } | Error::Contention(_)) => (),
                Err(error) => return Err(error),
            }
        }
        Ok(phase == SortPhase::Done)
    }

    fn release_merge_right(&mut self) {
        assert_eq!(self.sort.phase, SortPhase::Done);
        let pair = &mut self.sort.merge.pair;
        assert!(pair.right.finished() && !pair.right.loaded);
        // Join input 0 needs only the left cursor for output and replay. Free
        // the unused merge storage before admitting input 1's optional growth.
        // Completed final validation and synchronous I/O leave no buffer user.
        let record_capacity = pair.right.record.bytes.capacity();
        drop(std::mem::take(&mut pair.right.record.bytes));
        let read_capacity = pair.right.reader.release();
        let released = crate::resources::buffer_charge(record_capacity)
            .expect("admitted merge record charge")
            + crate::resources::buffer_charge(read_capacity).expect("admitted merge reader charge");
        pair.reservation
            .shrink_to(pair.reservation.bytes() - released as u64);
    }

    fn release_staging_record(&mut self) {
        assert_eq!(self.sort.phase, SortPhase::Done);
        // Join input 0 cannot capture again after sorting. Output and replay
        // borrow the sorted cursor's record, so free this separate allocation
        // before returning its charge for input 1's optional growth.
        let capacity = self.record.bytes.capacity();
        drop(std::mem::take(&mut self.record.bytes));
        let released =
            crate::resources::buffer_charge(capacity).expect("admitted sorted input record charge");
        self.reservation
            .shrink_to(self.reservation.bytes() - released as u64);
    }

    fn begin_read(&mut self) {
        let rows = self.sort.sorted_rows();
        rows.cursor.begin(rows.run);
    }

    fn finished(&self) -> bool {
        self.sort.sorted_cursor().finished()
    }

    // True means a record was loaded on this call. The consumer can yield before
    // processing it; false means the record is already loaded or input has ended.
    fn load(&mut self, cancel: &CancellationToken, effects: &mut Effects) -> Result<bool, Error> {
        let rows = self.sort.sorted_rows();
        if !rows.cursor.needs_load()? {
            return Ok(false);
        }
        rows.cursor.load(
            rows.slot,
            &self.layout,
            ROW_ARGUMENTS,
            &mut self.files.io(cancel, effects)?,
        )?;
        Ok(true)
    }

    // Fill the caller's row directly: returning a maximum-width array copies
    // every cell even for narrow inputs. Strings borrow this loaded record;
    // callers must finish using the row before advancing its cursor.
    fn read_values<'row>(&'row self, values: &mut [Value<'row>]) -> Result<(), Error> {
        assert_eq!(values.len(), self.layout.count);
        let record = self
            .sort
            .sorted_cursor()
            .record()
            .ok_or(Error::Corrupt("sorted payload requires a loaded row"))?;
        let mut remaining = record.key();
        for field in &self.layout.columns[..self.layout.count] {
            values[field.input] = read_value(&mut remaining, field.kind, field.nullable)?;
        }
        if !remaining.is_empty() {
            return Err(Error::Corrupt("sorted row trailing bytes"));
        }
        Ok(())
    }

    fn consume(&mut self) -> Result<(), Error> {
        self.sort.sorted_rows().cursor.consume()
    }
}

pub(super) const MAX_KEYS: usize = MAX_AGGREGATE_COLUMNS - 1;
pub(super) const RECORD_HEADER: usize = 32;

const RECORD_MAGIC: &[u8; 8] = b"PGRP0001";
pub(super) const MAX_KEY_BYTES: usize = MAX_KEYS * (5 + crate::batch::MAX_TEXT_BYTES);
pub(super) const MAX_ARGUMENT_RECORD_BYTES: usize =
    RECORD_HEADER + MAX_KEY_BYTES + MAX_AGGREGATE_COLUMNS * (8 + crate::batch::MAX_TEXT_BYTES);
const MAX_RECORD_BYTES: usize = MAX_FRAME_BYTES;
// A projection may repeat a text key in every result column.
pub(super) const MAX_FRAME_BYTES: usize =
    RECORD_HEADER + MAX_ROW_VALUES * (5 + crate::batch::MAX_TEXT_BYTES);
const _: () = assert!(MAX_ARGUMENT_RECORD_BYTES <= MAX_RECORD_BYTES);

pub(super) use crate::resources::BUFFER_ALLOCATION_UNIT;

pub(super) fn buffer_capacity(bytes: usize) -> Result<usize, Error> {
    crate::resources::buffer_capacity(bytes)
        .ok_or(Error::Corrupt("blocking buffer allocation capacity"))
}

/// Charge the chosen capacity without changing the encoded-byte limit.
pub(super) fn buffer_memory_bytes(bytes: usize) -> Result<usize, Error> {
    crate::resources::buffer_charge(buffer_capacity(bytes)?)
        .ok_or(Error::Corrupt("blocking buffer memory charge"))
}

const _: () = assert!((BUFFER_ALLOCATION_UNIT - 32).is_multiple_of(size_of::<RecordSpan>()));

fn span_capacity(rows: usize) -> Result<usize, Error> {
    let bytes = rows
        .checked_mul(size_of::<RecordSpan>())
        .ok_or(Error::Corrupt("run span allocation capacity"))?;
    Ok(buffer_capacity(bytes)? / size_of::<RecordSpan>())
}

pub(super) fn run_allocation_bytes(bytes: usize, rows: usize) -> Result<usize, Error> {
    let bytes = buffer_memory_bytes(bytes)?;
    crate::resources::buffer_charge(span_capacity(rows)? * size_of::<RecordSpan>())
        .and_then(|spans| spans.checked_mul(2))
        .and_then(|spans| spans.checked_add(bytes))
        .ok_or(Error::Corrupt("run allocation bytes"))
}

#[derive(Clone, Copy)]
struct RunId {
    pass: u32,
    index: u32,
}

const MAX_MERGE_PASSES: u32 = u64::BITS - (MAX_AGGREGATE_ROWS - 1).leading_zeros();
const RUN_HEADER: usize = 40;
const RUN_MAGIC: &[u8; 8] = b"PGRUN001";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Run {
    start: u64,
    end: u64,
    rows: u64,
}

impl Run {
    fn header(id: RunId, rows: u64, bytes: u64) -> Result<[u8; RUN_HEADER], Error> {
        let RunId { pass, index } = id;

        if pass > MAX_MERGE_PASSES
            || u64::from(index) >= MAX_AGGREGATE_ROWS
            || rows == 0
            || rows > MAX_AGGREGATE_ROWS
            || bytes
                < rows
                    .checked_mul(RECORD_HEADER as u64)
                    .ok_or(Error::Corrupt("run byte lower bound"))?
            || bytes
                > rows
                    .checked_mul(MAX_RECORD_BYTES as u64)
                    .ok_or(Error::Corrupt("run byte upper bound"))?
        {
            return Err(Error::Corrupt("group run shape"));
        }
        let mut header = [0_u8; RUN_HEADER];
        header[..8].copy_from_slice(RUN_MAGIC);
        header[8..12].copy_from_slice(&pass.to_le_bytes());
        header[12..16].copy_from_slice(&index.to_le_bytes());
        header[16..24].copy_from_slice(&rows.to_le_bytes());
        header[24..32].copy_from_slice(&bytes.to_le_bytes());
        let crc = format::crc32c(&header);
        header[32..36].copy_from_slice(&crc.to_le_bytes());
        Ok(header)
    }

    fn read(
        reader: &mut ReadBuffer,
        at: ReadAt,
        id: RunId,
        io: &mut Io<'_, '_>,
    ) -> Result<Self, Error> {
        let ReadAt {
            slot,
            offset,
            limit,
        } = at;
        let RunId { pass, index } = id;

        let mut header = [0_u8; RUN_HEADER];
        reader.read(
            ReadAt {
                slot,
                offset,
                limit,
            },
            &mut header,
            io,
        )?;
        let rows = u64::from_le_bytes(header[16..24].try_into().expect("row count"));
        let bytes = u64::from_le_bytes(header[24..32].try_into().expect("run byte count"));
        if header != Self::header(RunId { pass, index }, rows, bytes)? {
            return Err(Error::Corrupt("group run identity or checksum"));
        }
        let start = offset
            .checked_add(RUN_HEADER as u64)
            .ok_or(Error::Corrupt("run start offset"))?;
        let end = start
            .checked_add(bytes)
            .ok_or(Error::Corrupt("run end offset"))?;
        if end > limit {
            return Err(Error::Corrupt("group run exceeds file"));
        }
        Ok(Self { start, end, rows })
    }
}

#[derive(Clone, Copy)]
struct CursorPosition {
    offset: u64,
    remaining: u64,
}

pub(super) struct RunCursor {
    run: Option<Run>,
    offset: u64,
    remaining: u64,
    loaded: bool,
    record: SortRecord,
    reader: ReadBuffer,
}

impl RunCursor {
    fn finished(&self) -> bool {
        self.remaining == 0
    }

    // Check completion before asking for file access. A loaded record remains
    // available without another read until the caller consumes it.
    fn needs_load(&self) -> Result<bool, Error> {
        if self.remaining == 0 {
            if self.run.is_some_and(|run| self.offset != run.end) {
                return Err(Error::Corrupt("sorted run row count disagrees with bytes"));
            }
            return Ok(false);
        }
        Ok(!self.loaded)
    }

    fn position(&self) -> Option<CursorPosition> {
        (self.loaded && self.remaining != 0).then_some(CursorPosition {
            offset: self.offset,
            remaining: self.remaining,
        })
    }

    fn limit_read_ahead(&mut self, bytes: u64) {
        // The original run end still bounds every read, including a hint that
        // extends beyond it. This changes prefetch, not the cursor's valid range.
        self.reader
            .limit_read_ahead(self.offset.saturating_add(bytes));
    }

    fn rewind(&mut self, position: CursorPosition) -> Result<(), Error> {
        let run = self.run.ok_or(Error::Corrupt("join rewind has no run"))?;
        if !(run.start..run.end).contains(&position.offset)
            || position.remaining == 0
            || position.remaining > run.rows
        {
            return Err(Error::Corrupt("join rewind is outside its run"));
        }
        // The position comes from this cursor after record validation. Keep
        // the original run end so replay still enforces its byte bound.
        // Include the loaded boundary row: joins compare it again after each
        // duplicate group. Clearing the cache still detects changed records.
        let read_ahead_end = if self.loaded {
            self.offset
                .checked_add(self.record.bytes.len() as u64)
                .filter(|end| *end <= run.end)
                .ok_or(Error::Corrupt("replayed row exceeds its run"))?
        } else {
            self.offset
        };
        self.offset = position.offset;
        self.remaining = position.remaining;
        self.loaded = false;
        self.reader.reread_until(read_ahead_end);
        Ok(())
    }

    pub(super) fn record(&self) -> Option<&SortRecord> {
        self.loaded.then_some(&self.record)
    }

    fn new(record_bytes: usize, charge: u64) -> Result<Self, Error> {
        Ok(Self {
            run: None,
            offset: 0,
            remaining: 0,
            loaded: false,
            record: SortRecord::new(record_bytes, charge)?,
            reader: ReadBuffer::new(charge)?,
        })
    }

    pub(super) fn begin(&mut self, run: Option<Run>) {
        // An exhausted cursor cannot read. Restart must reject retired storage
        // before load could resize the record outside its former reservation.
        assert!(
            self.record.bytes.capacity() >= RECORD_HEADER,
            "restart owns record storage"
        );
        self.run = run;
        self.offset = run.map_or(0, |run| run.start);
        self.remaining = run.map_or(0, |run| run.rows);
        self.loaded = false;
        self.reader.clear();
    }

    pub(super) fn load(
        &mut self,
        slot: usize,
        keys: &RowLayout,
        arguments: ArgumentShape,
        io: &mut Io<'_, '_>,
    ) -> Result<(), Error> {
        if self.remaining == 0 {
            if self.run.is_some_and(|run| self.offset != run.end) {
                return Err(Error::Corrupt("group run row count disagrees with bytes"));
            }
            return Ok(());
        }
        if !self.loaded {
            let run = self.run.expect("remaining rows own a run");
            self.record.read(
                &mut self.reader,
                ReadAt {
                    slot,
                    offset: self.offset,
                    limit: run.end,
                },
                keys,
                arguments,
                io,
            )?;
            self.loaded = true;
        }
        Ok(())
    }

    pub(super) fn consume(&mut self) -> Result<(), Error> {
        assert!(
            self.loaded && self.remaining != 0,
            "consume one loaded record"
        );
        self.offset = self
            .offset
            .checked_add(self.record.bytes.len() as u64)
            .ok_or(Error::Corrupt("group record cursor"))?;
        self.remaining -= 1;
        self.loaded = false;
        Ok(())
    }
}

// Keep one checked record from each run. Each comparison writes the lesser
// record and advances only that cursor. The previous output key detects reversed or repeated
// key/row-number pairs; the enclosing MergePasses owns terminal failure state.
struct PairMerge<'db> {
    left: RunCursor,
    right: RunCursor,
    previous_key: Vec<u8>,
    previous_ordinal: Option<u64>,
    expected_end: u64,
    input_slot: usize,
    finished: bool,
    remaining: u64,
    arguments: ArgumentShape,
    reservation: Reservation<'db>,
}

impl<'db> PairMerge<'db> {
    fn new(
        database: &'db Database,
        keys: &RowLayout,
        arguments: ArgumentShape,
    ) -> Result<Self, Error> {
        let states = arguments.count;
        if states > MAX_AGGREGATE_COLUMNS
            || (arguments.nonnull
                | arguments.integers
                | arguments.presence
                | arguments.dates
                | arguments.text)
                >> states
                != 0
            || arguments.dates & (!arguments.integers | arguments.presence) != 0
            || arguments.text & (arguments.integers | arguments.presence | arguments.dates) != 0
        {
            return Err(Error::Corrupt("merge argument width"));
        }
        let record_bytes = RECORD_HEADER
            .checked_add(keys.max_bytes)
            .and_then(|n| n.checked_add(arguments.max_payload_bytes()))
            .ok_or(Error::Corrupt("merge record capacity"))?;
        let record_charge = buffer_memory_bytes(record_bytes)?;
        let key_capacity = buffer_capacity(keys.key_max_bytes)?;
        let key_charge = buffer_memory_bytes(keys.key_max_bytes)?;
        let bytes = record_charge
            .checked_mul(2)
            .and_then(|n| n.checked_add(2 * IO_MEMORY_BYTES))
            .and_then(|n| n.checked_add(key_charge))
            .and_then(|n| n.checked_add(size_of::<Self>()))
            .ok_or(Error::Corrupt("merge memory requirement"))? as u64;
        let reservation = database.reserve_memory(bytes, "group merge minimum")?;
        Ok(Self {
            left: RunCursor::new(record_bytes, bytes)?,
            right: RunCursor::new(record_bytes, bytes)?,
            previous_key: allocate(key_capacity, key_capacity, "merge previous key", bytes)?,
            previous_ordinal: None,
            expected_end: 0,
            input_slot: 0,
            finished: true,
            remaining: 0,
            arguments,
            reservation,
        })
    }

    fn begin(
        &mut self,
        left: Run,
        right: Option<Run>,
        id: RunId,
        writer: &mut WriteBuffer,
        input_slot: usize,
        io: &mut Io<'_, '_>,
    ) -> Result<(), Error> {
        io.cancel.check()?;
        let RunId { pass, index } = id;
        if input_slot >= 2 {
            return Err(Error::Corrupt("merge input file slot"));
        }
        let output_slot = input_slot ^ 1;
        assert!(
            self.finished,
            "check completion before beginning another pair"
        );

        let left_bytes = left
            .end
            .checked_sub(left.start)
            .ok_or(Error::Corrupt("left run extent"))?;
        let right_bytes = right
            .map(|run| {
                run.end
                    .checked_sub(run.start)
                    .ok_or(Error::Corrupt("right run extent"))
            })
            .transpose()?
            .unwrap_or(0);
        let rows = left
            .rows
            .checked_add(right.map_or(0, |run| run.rows))
            .ok_or(Error::Corrupt("merged row count"))?;
        let bytes = left_bytes
            .checked_add(right_bytes)
            .ok_or(Error::Corrupt("merged byte count"))?;
        let header = Run::header(RunId { pass, index }, rows, bytes)?;
        self.expected_end = writer
            .position()?
            .checked_add(RUN_HEADER as u64)
            .and_then(|n| n.checked_add(bytes))
            .ok_or(Error::Corrupt("merged output end"))?;
        writer.append(output_slot, &header, io)?;
        self.left.begin(Some(left));
        self.right.begin(right);
        self.remaining = rows;
        self.input_slot = input_slot;
        self.finished = false;
        self.previous_ordinal = None;
        self.previous_key.clear();
        Ok(())
    }

    // Keep the batched loop out of the per-row sort controller. Inlining it
    // enlarged that controller's frame even for inputs needing no merge.
    #[inline(never)]
    fn step(
        &mut self,
        writer: &mut WriteBuffer,
        keys: &RowLayout,
        io: &mut Io<'_, '_>,
    ) -> Result<bool, Error> {
        // Amortize controller dispatch while bounding comparisons and copying.
        // Merge at most 256 records and 64 KiB of encoded comparison inputs;
        // one oversized pair is always allowed. Check cancellation per record.
        let input_slot = self.input_slot;
        let mut remaining_bytes = 65_536;
        for moved in 0..BATCH_ROWS {
            io.cancel.check()?;

            self.left.load(input_slot, keys, self.arguments, io)?;
            self.right.load(input_slot, keys, self.arguments, io)?;
            if self.remaining == 0 {
                if self.left.remaining != 0
                    || self.right.remaining != 0
                    || writer.position()? != self.expected_end
                {
                    return Err(Error::Corrupt("merged run completion differs"));
                }
                self.finished = true;
                return Ok(true);
            }
            // Loaded cursors may retain the next pair across a byte-budget yield.
            // Count full records: merging copies payloads as well as comparing keys.
            let bytes = if self.left.loaded {
                self.left.record.bytes.len()
            } else {
                0
            } + if self.right.loaded {
                self.right.record.bytes.len()
            } else {
                0
            };
            if bytes > remaining_bytes && moved != 0 {
                return Ok(false);
            }
            remaining_bytes = remaining_bytes.saturating_sub(bytes);
            let take_left = self.left.loaded
                && (!self.right.loaded
                    || self.left.record.compare(&self.right.record, keys)? != Ordering::Greater);
            let selected = if take_left {
                &mut self.left
            } else {
                &mut self.right
            };
            if !selected.loaded {
                return Err(Error::Corrupt("merge exhausted before declared rows"));
            }
            let record = &selected.record;
            if let Some(previous) = self.previous_ordinal {
                let order = keys
                    .compare(&self.previous_key, record.key())?
                    .then_with(|| previous.cmp(&record.ordinal()));
                if order != Ordering::Less {
                    return Err(Error::Corrupt("group run is not strictly ordered"));
                }
            }
            writer.append(input_slot ^ 1, &record.bytes, io)?;
            self.previous_key.clear();
            append_bytes(&mut self.previous_key, keys.sort_prefix(record.key())?)?;
            self.previous_ordinal = Some(record.ordinal());
            selected.consume()?;
            self.remaining -= 1;
            if remaining_bytes == 0 {
                return Ok(false);
            }
        }
        Ok(false)
    }
}

#[derive(Clone, Copy)]
pub(super) struct RecordSpan {
    start: usize,
    end: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RunPhase {
    Collect,
    Sort,
    Header,
    Write,
    Done,
    Released,
    Failed,
}

// Store encoded bytes once and merge-sort two arrays of ranges into those bytes.
// Sorting changes range order; writing follows the final ranges. Row and byte
// limits are separate because a few wide rows can fill the byte buffer first.
struct RunBuffer<'db> {
    bytes: Vec<u8>,
    spans: Vec<RecordSpan>,
    work: Vec<RecordSpan>,
    byte_limit: usize,
    row_limit: usize,
    last_ordinal: Option<u64>,
    phase: RunPhase,
    id: RunId,
    width: usize,
    left: usize,
    middle: usize,
    right: usize,
    end: usize,
    output: usize,
    reservation: Reservation<'db>,
}

#[derive(Debug, PartialEq, Eq)]
enum RunGrowth {
    Bytes(usize),
    Rows { bytes: usize, rows: usize },
}

impl<'db> RunBuffer<'db> {
    fn new(database: &'db Database, byte_limit: usize, row_limit: usize) -> Result<Self, Error> {
        if byte_limit < RECORD_HEADER || row_limit == 0 || row_limit as u64 > MAX_AGGREGATE_ROWS {
            return Err(Error::Corrupt("group run buffer bounds"));
        }
        let charge = run_allocation_bytes(byte_limit, row_limit)?
            .checked_add(size_of::<Self>())
            .ok_or(Error::Corrupt("group run buffer memory"))? as u64;
        let byte_capacity = buffer_capacity(byte_limit)?;
        let spans = span_capacity(row_limit)?;
        let reservation = database.reserve_memory(charge, "group run buffer")?;
        Ok(Self {
            bytes: allocate(byte_capacity, byte_capacity, "group run bytes", charge)?,
            spans: allocate(spans, spans, "group run spans", charge)?,
            work: allocate(spans, spans, "group run sort workspace", charge)?,
            byte_limit,
            row_limit,
            last_ordinal: None,
            phase: RunPhase::Collect,
            id: RunId { pass: 0, index: 0 },
            width: 1,
            left: 0,
            middle: 0,
            right: 0,
            end: 0,
            output: 0,
            reservation,
        })
    }

    fn grow_empty(
        &mut self,
        shortest_record: usize,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        cancel.check()?;
        assert_eq!(self.phase, RunPhase::Collect);
        assert!(self.bytes.is_empty() && self.spans.is_empty() && self.work.is_empty());
        let authority = self.reservation.authority();
        let available = authority
            .limit()
            .checked_sub(authority.reserved())
            .ok_or(Error::Corrupt("memory account exceeds configured limit"))?;
        // Row count and bytes grow together, capped at 4,096 rows. Retain the
        // maximum-width first row admitted by the original buffer. These sums
        // fit usize: the maximum frame plus 4,096 minimum-width rows is bounded.
        let mut rows = self.row_limit;
        let mut upper = 4096;
        while rows < upper {
            let candidate = rows + (upper - rows).div_ceil(2);
            let bytes = self.byte_limit + (candidate - self.row_limit) * shortest_record;
            if run_allocation_bytes(bytes, candidate)? as u64 <= available {
                rows = candidate;
            } else {
                upper = candidate - 1;
            }
        }
        if rows == self.row_limit {
            return Ok(());
        }
        let bytes = self.byte_limit + (rows - self.row_limit) * shortest_record;
        self.replace_empty(bytes, rows, cancel)
    }

    fn replace_empty(
        &mut self,
        bytes: usize,
        rows: usize,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        assert_eq!(self.phase, RunPhase::Collect);
        assert!(self.bytes.is_empty() && self.spans.is_empty() && self.work.is_empty());
        assert!(bytes >= self.byte_limit && rows >= self.row_limit);
        cancel.check()?;
        let authority = self.reservation.authority();
        let charge = run_allocation_bytes(bytes, rows)? as u64;
        let old_charge = run_allocation_bytes(self.byte_limit, self.row_limit)? as u64;
        // Reserve the full replacement before allocating, including allocator
        // rounding. A concurrent reservation can still refuse this attempt.
        let mut replacement = authority.reserve(charge, "optional sort run")?;
        let byte_capacity = buffer_capacity(bytes)?;
        let spans = span_capacity(rows)?;
        let arena = allocate(byte_capacity, byte_capacity, "optional run bytes", charge)?;
        let positions = allocate(spans, spans, "optional run spans", charge)?;
        let work = allocate(spans, spans, "optional run workspace", charge)?;
        cancel.check()?;
        // Transfer before installing buffers, then free every old allocation
        // before releasing its charge. Any inline charge remains with its owner.
        replacement.transfer_to(&mut self.reservation, charge)?;
        drop(std::mem::replace(&mut self.bytes, arena));
        drop(std::mem::replace(&mut self.spans, positions));
        drop(std::mem::replace(&mut self.work, work));
        self.reservation
            .shrink_to(self.reservation.bytes() - old_charge);
        self.byte_limit = bytes;
        self.row_limit = rows;
        Ok(())
    }

    fn growth_after_write(&self) -> Result<Option<RunGrowth>, Error> {
        if self.phase != RunPhase::Write || self.output + 1 != self.spans.len() {
            return Ok(None);
        }
        let width = self.bytes.len().div_ceil(self.spans.len());
        if self.spans.len() > self.row_limit / 2 {
            // A useful nonfinal run can justify more row workspace. Keep small
            // inputs at the initial ceiling and leave pressured initial buffers
            // alone. Require a complete doubling: marginal growth can consume
            // memory without removing a merge pass.
            if self.row_limit < 4096 || self.row_limit >= 16_384 {
                return Ok(None);
            }
            let rows = self.row_limit * 2;
            let bytes = width
                .checked_mul(rows)
                .ok_or(Error::Corrupt("optional run row target"))?
                .max(self.byte_limit);
            return Ok(Some(RunGrowth::Rows { bytes, rows }));
        }
        // Byte-limited runs keep their row workspace. Observed widths avoid
        // reserving schema maxima; optional replacement may use a partial fit.
        let target = width
            .checked_mul(self.row_limit)
            .ok_or(Error::Corrupt("optional run byte target"))?;
        Ok((target > self.byte_limit).then_some(RunGrowth::Bytes(target)))
    }

    // Replacement is rare; keep allocation temporaries off the per-record
    // sorting step.
    #[inline(never)]
    fn grow_bytes_empty(
        &mut self,
        mut upper: usize,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        assert_eq!(self.phase, RunPhase::Collect);
        assert!(self.bytes.is_empty() && self.spans.is_empty() && self.work.is_empty());
        cancel.check()?;
        let authority = self.reservation.authority();
        let available = authority
            .limit()
            .checked_sub(authority.reserved())
            .ok_or(Error::Corrupt("memory account exceeds configured limit"))?;
        let mut bytes = self.byte_limit;
        while bytes < upper {
            let candidate = bytes + (upper - bytes).div_ceil(2);
            if buffer_memory_bytes(candidate)? as u64 <= available {
                bytes = candidate;
            } else {
                upper = candidate - 1;
            }
        }
        if bytes == self.byte_limit {
            return Ok(());
        }
        // Source payloads, batches and the other branch's minimum buffers are
        // already admitted. Charge both arenas during replacement; a racing
        // reservation or failed allocation leaves the old arena usable.
        let charge = buffer_memory_bytes(bytes)? as u64;
        let old_charge = buffer_memory_bytes(self.byte_limit)? as u64;
        let mut replacement = authority.reserve(charge, "optional sort run bytes")?;
        let capacity = buffer_capacity(bytes)?;
        let arena = allocate(capacity, capacity, "optional run bytes", charge)?;
        cancel.check()?;
        replacement.transfer_to(&mut self.reservation, charge)?;
        drop(std::mem::replace(&mut self.bytes, arena));
        self.reservation
            .shrink_to(self.reservation.bytes() - old_charge);
        self.byte_limit = bytes;
        Ok(())
    }

    fn push(&mut self, record: &SortRecord) -> Result<bool, Error> {
        assert_eq!(
            self.phase,
            RunPhase::Collect,
            "only collecting runs admit rows"
        );
        let end = self
            .bytes
            .len()
            .checked_add(record.bytes.len())
            .ok_or(Error::Corrupt("group run byte growth"))?;
        if self.spans.len() == self.row_limit || end > self.byte_limit {
            return Ok(false);
        }
        let ordinal = record.ordinal();
        if self
            .last_ordinal
            .is_some_and(|previous| previous >= ordinal)
        {
            return Err(Error::Corrupt("group input ordinal does not advance"));
        }
        let span = RecordSpan {
            start: self.bytes.len(),
            end,
        };
        append_bytes(&mut self.bytes, &record.bytes)?;
        self.spans.push(span);
        self.work.push(span);
        self.last_ordinal = Some(ordinal);
        Ok(true)
    }

    fn begin(&mut self, index: u32) -> Result<(), Error> {
        assert_eq!(self.phase, RunPhase::Collect);
        self.phase = RunPhase::Failed;
        self.id = RunId { pass: 0, index };
        Run::header(self.id, self.spans.len() as u64, self.bytes.len() as u64)?;
        self.width = 1;
        self.start_pair(0);
        self.phase = if self.spans.len() == 1 {
            RunPhase::Header
        } else {
            RunPhase::Sort
        };
        Ok(())
    }

    fn start_pair(&mut self, start: usize) {
        // Row count and width are at most MAX_AGGREGATE_ROWS; these sums fit
        // usize on supported 64-bit targets. Each pair covers a disjoint range.
        self.left = start;
        self.middle = (start + self.width).min(self.spans.len());
        self.right = self.middle;
        self.end = (self.middle + self.width).min(self.spans.len());
        self.output = start;
    }

    fn compare(
        &self,
        left: RecordSpan,
        right: RecordSpan,
        keys: &RowLayout,
    ) -> Result<Ordering, Error> {
        SortRecord::compare_encoded(
            &self.bytes[left.start..left.end],
            &self.bytes[right.start..right.end],
            keys,
        )
    }

    fn step(
        &mut self,
        keys: &RowLayout,
        writer: &mut WriteBuffer,
        io: &mut Io<'_, '_>,
    ) -> Result<bool, Error> {
        let phase = std::mem::replace(&mut self.phase, RunPhase::Failed);
        if phase == RunPhase::Done {
            self.phase = RunPhase::Done;
            return Ok(true);
        }
        io.cancel.check()?;
        self.phase = match phase {
            RunPhase::Collect | RunPhase::Released | RunPhase::Failed => {
                return Err(Error::Corrupt("run sorting is not active"));
            }
            RunPhase::Sort => {
                // Amortize cursor dispatch for narrow keys without allocating
                // or changing the run size. Limit a call to 256 span moves and
                // 64 KiB of encoded comparison inputs; one wide comparison is
                // always allowed. Check cancellation between moves. Payloads
                // stay in place and equal keys still use their input ordinals.
                let input_limit = RECORD_HEADER + keys.key_max_bytes;
                let mut remaining = 65_536;
                for _ in 0..BATCH_ROWS {
                    io.cancel.check()?;
                    let take_left = if self.left < self.middle && self.right < self.end {
                        let left = self.spans[self.left];
                        let right = self.spans[self.right];
                        // A complete encoded record bounds its key bytes. The
                        // schema ceiling excludes unrelated wide payloads.
                        let bytes = input_limit.min(left.end - left.start)
                            + input_limit.min(right.end - right.start);
                        if bytes > remaining && remaining != 65_536 {
                            break;
                        }
                        remaining = remaining.saturating_sub(bytes);
                        self.compare(left, right, keys)? != Ordering::Greater
                    } else {
                        self.left < self.middle
                    };
                    self.work[self.output] = if take_left {
                        let span = self.spans[self.left];
                        self.left += 1;
                        span
                    } else {
                        let span = self.spans[self.right];
                        self.right += 1;
                        span
                    };
                    self.output += 1;
                    if self.output == self.end {
                        if self.end == self.spans.len() {
                            std::mem::swap(&mut self.spans, &mut self.work);
                            self.width *= 2;
                            self.start_pair(0);
                        } else {
                            self.start_pair(self.end);
                        }
                    }
                    if self.width >= self.spans.len() {
                        break;
                    }
                }
                if self.width >= self.spans.len() {
                    RunPhase::Header
                } else {
                    RunPhase::Sort
                }
            }
            RunPhase::Header => {
                let header =
                    Run::header(self.id, self.spans.len() as u64, self.bytes.len() as u64)?;
                writer.append(0, &header, io)?;
                self.output = 0;
                RunPhase::Write
            }
            RunPhase::Write => {
                let span = self.spans[self.output];
                writer.append(0, &self.bytes[span.start..span.end], io)?;
                self.output += 1;
                if self.output == self.spans.len() {
                    RunPhase::Done
                } else {
                    RunPhase::Write
                }
            }
            RunPhase::Done => unreachable!("handled before cancellation"),
        };
        Ok(self.phase == RunPhase::Done)
    }

    fn clear(&mut self) {
        assert_eq!(
            self.phase,
            RunPhase::Done,
            "finish output before reusing run storage"
        );
        self.bytes.clear();
        self.spans.clear();
        self.work.clear();
        self.last_ordinal = None;
        self.phase = RunPhase::Collect;
    }

    fn release(&mut self) {
        assert_eq!(self.phase, RunPhase::Collect);
        assert!(
            self.spans.is_empty(),
            "flush the final run before releasing its arena"
        );
        let payload_bytes = crate::resources::buffer_charge(self.bytes.capacity())
            .expect("admitted run buffer charge")
            + crate::resources::buffer_charge(self.spans.capacity() * size_of::<RecordSpan>())
                .expect("admitted run span charge")
            + crate::resources::buffer_charge(self.work.capacity() * size_of::<RecordSpan>())
                .expect("admitted run workspace charge");
        drop(std::mem::take(&mut self.bytes));
        drop(std::mem::take(&mut self.spans));
        drop(std::mem::take(&mut self.work));
        // Release the buffers' reservation only after freeing them. The struct
        // itself still occupies memory, charged here or by its runtime owner.
        self.reservation
            .shrink_to(self.reservation.bytes() - payload_bytes as u64);
        self.phase = RunPhase::Released;
    }
}

// Describe a whole run file with counts and an end offset. Individual run bounds
// are read from headers as needed, instead of retaining one entry per run.
#[derive(Clone, Copy)]
struct RunFile {
    slot: usize,
    pass: u32,
    runs: u32,
    rows: u64,
    end: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MergePhase {
    Idle,
    Prepare,
    NextPair,
    Pair,
    Flush,
    Final,
    Done,
    Failed,
}

struct MergePasses<'db> {
    pair: PairMerge<'db>,
    writer: WriteBuffer,
    input: RunFile,
    phase: MergePhase,
    input_index: u32,
    input_offset: u64,
    checked_rows: u64,
    output_runs: u32,
    result: Option<Run>,
    reservation: Reservation<'db>,
}

impl<'db> MergePasses<'db> {
    fn new(
        database: &'db Database,
        keys: &RowLayout,
        arguments: ArgumentShape,
    ) -> Result<Self, Error> {
        // PairMerge accounts its own inline fields and buffers. This charge owns
        // the remaining inline state and the one shared output buffer.
        let bytes = (size_of::<Self>() - size_of::<PairMerge<'_>>() + IO_MEMORY_BYTES) as u64;
        let reservation = database.reserve_memory(bytes, "group merge controller")?;
        Ok(Self {
            pair: PairMerge::new(database, keys, arguments)?,
            writer: WriteBuffer::new(bytes)?,
            input: RunFile {
                slot: 0,
                pass: 0,
                runs: 0,
                rows: 0,
                end: 0,
            },
            phase: MergePhase::Idle,
            input_index: 0,
            input_offset: 0,
            checked_rows: 0,
            output_runs: 0,
            result: None,
            reservation,
        })
    }

    fn begin(&mut self, runs: u32, rows: u64, end: u64) -> Result<(), Error> {
        assert_eq!(
            self.phase,
            MergePhase::Idle,
            "one run stream per merge owner"
        );
        self.phase = MergePhase::Failed;
        let headers = u64::from(runs) * RUN_HEADER as u64;
        let body = end
            .checked_sub(headers)
            .ok_or(Error::Corrupt("initial run file extent"))?;
        if rows > MAX_AGGREGATE_ROWS
            || u64::from(runs) > rows
            || (runs == 0) != (rows == 0)
            || body < rows * RECORD_HEADER as u64
            || body > rows * MAX_RECORD_BYTES as u64
        {
            return Err(Error::Corrupt("initial run file shape"));
        }
        assert!(self.writer.is_empty(), "initial runs must be flushed");
        self.input = RunFile {
            slot: 0,
            pass: 0,
            runs,
            rows,
            end,
        };
        self.phase = if runs <= 1 {
            MergePhase::Final
        } else {
            MergePhase::Prepare
        };
        Ok(())
    }

    fn next_run(&mut self, io: &mut Io<'_, '_>) -> Result<Run, Error> {
        assert!(self.input_index < self.input.runs);
        let run = Run::read(
            &mut self.pair.left.reader,
            ReadAt {
                slot: self.input.slot,
                offset: self.input_offset,
                limit: self.input.end,
            },
            RunId {
                pass: self.input.pass,
                index: self.input_index,
            },
            io,
        )?;
        self.checked_rows = self
            .checked_rows
            .checked_add(run.rows)
            .ok_or(Error::Corrupt("merge pass row count"))?;
        if self.checked_rows > self.input.rows {
            return Err(Error::Corrupt("merge pass exceeds input rows"));
        }
        self.input_offset = run.end;
        self.input_index += 1;
        Ok(run)
    }

    fn step(&mut self, keys: &RowLayout, io: &mut Io<'_, '_>) -> Result<bool, Error> {
        // Any failure, including cancellation after partial I/O, is terminal.
        // The surrounding query drops the files; it cannot retry a partial pair.
        let phase = std::mem::replace(&mut self.phase, MergePhase::Failed);
        if phase == MergePhase::Done {
            self.phase = MergePhase::Done;
            return Ok(true);
        }
        io.cancel.check()?;
        self.phase = match phase {
            MergePhase::Idle | MergePhase::Failed => {
                return Err(Error::Corrupt("merge controller is not active"));
            }
            MergePhase::Prepare => {
                if self.input.pass >= MAX_MERGE_PASSES || self.input.runs <= 1 {
                    return Err(Error::Corrupt("merge pass bound"));
                }
                io.reset(self.input.slot ^ 1)?;
                self.writer.begin_file();
                self.pair.left.reader.clear();
                self.pair.right.reader.clear();
                self.input_index = 0;
                self.input_offset = 0;
                self.checked_rows = 0;
                self.output_runs = 0;
                MergePhase::NextPair
            }
            MergePhase::NextPair => {
                if self.input_index == self.input.runs {
                    if self.input_offset != self.input.end
                        || self.checked_rows != self.input.rows
                        || self.output_runs != self.input.runs.div_ceil(2)
                    {
                        return Err(Error::Corrupt("merge pass completion differs"));
                    }
                    MergePhase::Flush
                } else {
                    let left = self.next_run(io)?;
                    let right = if self.input_index < self.input.runs {
                        Some(self.next_run(io)?)
                    } else {
                        None
                    };
                    self.pair.begin(
                        left,
                        right,
                        RunId {
                            pass: self.input.pass + 1,
                            index: self.output_runs,
                        },
                        &mut self.writer,
                        self.input.slot,
                        io,
                    )?;
                    MergePhase::Pair
                }
            }
            MergePhase::Pair => {
                if self.pair.step(&mut self.writer, keys, io)? {
                    self.output_runs += 1;
                    MergePhase::NextPair
                } else {
                    MergePhase::Pair
                }
            }
            MergePhase::Flush => {
                self.writer.flush(self.input.slot ^ 1, io)?;
                assert!(
                    self.output_runs < self.input.runs,
                    "each pass reduces run count"
                );
                self.input = RunFile {
                    slot: self.input.slot ^ 1,
                    pass: self.input.pass + 1,
                    runs: self.output_runs,
                    rows: self.input.rows,
                    end: self.writer.position()?,
                };
                if self.input.runs == 1 {
                    MergePhase::Final
                } else {
                    MergePhase::Prepare
                }
            }
            MergePhase::Final => {
                // Empty the obsolete file so grouping can reuse it for results.
                // Check the final run header now; consumers still validate each
                // record when reading the run.
                io.reset(self.input.slot ^ 1)?;
                self.pair.left.reader.clear();
                self.input_index = 0;
                self.input_offset = 0;
                self.checked_rows = 0;
                self.result = if self.input.runs == 0 {
                    None
                } else {
                    Some(self.next_run(io)?)
                };
                if self.input_offset != self.input.end || self.checked_rows != self.input.rows {
                    return Err(Error::Corrupt("final run file completion differs"));
                }
                MergePhase::Done
            }
            MergePhase::Done => unreachable!("handled before cancellation"),
        };
        Ok(self.phase == MergePhase::Done)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SortPhase {
    Collect,
    Run,
    Flush,
    Merge,
    Done,
    Failed,
}

/// Borrow the completed run without allocating a new reader. Output and grouping
/// reduction reuse the merge cursor and prior-key buffer; the sorter continues
/// to own their storage and memory reservations.
pub(super) struct SortedRows<'a> {
    pub(super) cursor: &'a mut RunCursor,
    pub(super) previous_key: &'a mut Vec<u8>,
    pub(super) previous_ordinal: &'a mut Option<u64>,
    pub(super) arguments: ArgumentShape,
    pub(super) run: Option<Run>,
    pub(super) slot: usize,
}

pub(super) struct RowSort<'db> {
    buffer: RunBuffer<'db>,
    merge: MergePasses<'db>,
    phase: SortPhase,
    finishing: bool,
    rows: u64,
    runs: u32,
    last_ordinal: Option<u64>,
    reservation: Reservation<'db>,
}

impl<'db> RowSort<'db> {
    fn transfer_inline_to(&mut self, destination: &mut Reservation<'db>) -> Result<(), Error> {
        self.reservation.transfer_to(
            destination,
            (size_of::<Self>() - size_of::<RunBuffer<'_>>() - size_of::<MergePasses<'_>>()) as u64,
        )?;
        self.buffer
            .reservation
            .transfer_to(destination, size_of::<RunBuffer<'_>>() as u64)?;
        self.merge.reservation.transfer_to(
            destination,
            (size_of::<MergePasses<'_>>() - size_of::<PairMerge<'_>>()) as u64,
        )?;
        self.merge
            .pair
            .reservation
            .transfer_to(destination, size_of::<PairMerge<'_>>() as u64)
    }

    pub(super) fn memory_bytes(&self) -> u64 {
        self.reservation.bytes()
            + self.buffer.reservation.bytes()
            + self.merge.reservation.bytes()
            + self.merge.pair.reservation.bytes()
    }

    pub(super) fn sorted_rows(&mut self) -> SortedRows<'_> {
        assert_eq!(self.phase, SortPhase::Done, "output owns completed sorting");
        let pair = &mut self.merge.pair;
        SortedRows {
            cursor: &mut pair.left,
            previous_key: &mut pair.previous_key,
            previous_ordinal: &mut pair.previous_ordinal,
            arguments: pair.arguments,
            run: self.merge.result,
            slot: self.merge.input.slot,
        }
    }

    fn sorted_cursor(&self) -> &RunCursor {
        &self.merge.pair.left
    }

    pub(super) fn previous_key(&self) -> &[u8] {
        &self.merge.pair.previous_key
    }

    // After sorting, grouping can spool results to the other disposable file.
    // The former right-input reader and merge writer retain their existing charges.
    pub(super) fn spool_slot(&self) -> usize {
        self.merge.input.slot ^ 1
    }

    pub(super) fn spool_reader(&mut self) -> &mut ReadBuffer {
        &mut self.merge.pair.right.reader
    }

    pub(super) fn spool_writer(&mut self) -> &mut WriteBuffer {
        &mut self.merge.writer
    }

    pub(super) fn phase(&self) -> SortPhase {
        self.phase
    }

    #[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
    pub(super) fn reduction_progress(&self) -> (u64, usize) {
        (
            self.merge.pair.left.remaining,
            self.merge.pair.left.reader.buffered_bytes(),
        )
    }

    #[cfg(test)]
    pub(super) fn run_limits(&self) -> (usize, usize) {
        (self.buffer.byte_limit, self.buffer.row_limit)
    }

    #[cfg(test)]
    pub(super) fn run_allocation_capacities(&self) -> [usize; 3] {
        [
            self.buffer.bytes.capacity(),
            self.buffer.spans.capacity() * size_of::<RecordSpan>(),
            self.buffer.work.capacity() * size_of::<RecordSpan>(),
        ]
    }

    #[cfg(test)]
    pub(super) fn allocation_capacities(&self) -> [usize; 9] {
        // Expose each live capacity; tests calculate expected charges separately.
        let [bytes, spans, work] = self.run_allocation_capacities();
        [
            bytes,
            spans,
            work,
            self.merge.writer.allocated_bytes(),
            self.merge.pair.previous_key.capacity(),
            self.merge.pair.left.record.bytes.capacity(),
            self.merge.pair.right.record.bytes.capacity(),
            self.merge.pair.left.reader.allocated_bytes(),
            self.merge.pair.right.reader.allocated_bytes(),
        ]
    }

    pub(super) fn new(
        database: &'db Database,
        keys: &RowLayout,
        arguments: ArgumentShape,
        bytes: usize,
        rows: usize,
    ) -> Result<Self, Error> {
        let minimum = RECORD_HEADER
            .checked_add(keys.max_bytes)
            .and_then(|n| n.checked_add(arguments.max_payload_bytes()))
            .ok_or(Error::Corrupt("sort record minimum"))?;
        if bytes < minimum {
            return Err(Error::Resource {
                owner: "group sort record",
                required: minimum as u64,
                limit: bytes as u64,
            });
        }
        let inline = size_of::<Self>() - size_of::<RunBuffer<'_>>() - size_of::<MergePasses<'_>>();
        let reservation = database.reserve_memory(inline as u64, "group sort controller")?;
        Ok(Self {
            buffer: RunBuffer::new(database, bytes, rows)?,
            merge: MergePasses::new(database, keys, arguments)?,
            phase: SortPhase::Collect,
            finishing: false,
            rows: 0,
            runs: 0,
            last_ordinal: None,
            reservation,
        })
    }

    // A false result preserves the caller's row. Step until Collect, then retry
    // that same record; no additional source row may be consumed meanwhile.
    pub(super) fn push(&mut self, record: &SortRecord) -> Result<bool, Error> {
        assert_eq!(self.phase, SortPhase::Collect);
        self.phase = SortPhase::Failed;
        if self.rows == MAX_AGGREGATE_ROWS {
            return Err(Error::Resource {
                owner: "aggregate input rows",
                required: self.rows + 1,
                limit: MAX_AGGREGATE_ROWS,
            });
        }
        if self
            .last_ordinal
            .is_some_and(|last| last >= record.ordinal())
        {
            return Err(Error::Corrupt("sort input ordinal does not advance"));
        }
        if self.buffer.push(record)? {
            self.rows += 1;
            self.last_ordinal = Some(record.ordinal());
            self.phase = SortPhase::Collect;
            Ok(true)
        } else {
            assert!(
                !self.buffer.spans.is_empty(),
                "admission holds one maximum record"
            );
            self.buffer.begin(self.runs)?;
            self.phase = SortPhase::Run;
            Ok(false)
        }
    }

    pub(super) fn finish(&mut self) -> Result<(), Error> {
        assert_eq!(self.phase, SortPhase::Collect);
        self.phase = SortPhase::Failed;
        self.finishing = true;
        if self.buffer.spans.is_empty() {
            self.phase = SortPhase::Flush;
        } else {
            self.buffer.begin(self.runs)?;
            self.phase = SortPhase::Run;
        }
        Ok(())
    }

    pub(super) fn step(
        &mut self,
        keys: &RowLayout,
        io: &mut Io<'_, '_>,
    ) -> Result<SortPhase, Error> {
        let phase = std::mem::replace(&mut self.phase, SortPhase::Failed);
        if phase == SortPhase::Done {
            self.phase = SortPhase::Done;
            return Ok(SortPhase::Done);
        }
        io.cancel.check()?;
        self.phase = match phase {
            SortPhase::Collect => SortPhase::Collect,
            SortPhase::Run => {
                if self.buffer.step(keys, &mut self.merge.writer, io)? {
                    self.runs = self
                        .runs
                        .checked_add(1)
                        .ok_or(Error::Corrupt("initial run count"))?;
                    self.buffer.clear();
                    if self.finishing {
                        SortPhase::Flush
                    } else {
                        SortPhase::Collect
                    }
                } else {
                    SortPhase::Run
                }
            }
            SortPhase::Flush => {
                self.merge.writer.flush(0, io)?;
                self.merge
                    .begin(self.runs, self.rows, self.merge.writer.position()?)?;
                self.buffer.release();
                SortPhase::Merge
            }
            SortPhase::Merge => {
                if self.merge.step(keys, io)? {
                    SortPhase::Done
                } else {
                    SortPhase::Merge
                }
            }
            SortPhase::Failed => return Err(Error::Corrupt("argument sort is not active")),
            SortPhase::Done => unreachable!("handled before cancellation"),
        };
        Ok(self.phase)
    }
}

#[cfg(test)]
pub(super) mod test_support;

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
#[path = "blocking/sorting_tests.rs"]
mod tests;

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
#[path = "blocking/byte_growth_tests.rs"]
mod byte_growth_tests;
