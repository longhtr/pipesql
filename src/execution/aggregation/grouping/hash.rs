//! Try bounded in-memory grouping without taking ownership of the disk fallback.
//!
//! Encoded keys live in an arena; hash buckets locate group IDs and typed cells
//! hold each group's aggregates. A batch first resolves positions, then folds its
//! captured arguments. Limits on groups, key bytes, collision work and text growth
//! return `Fallback`, which asks the controller to discard this state and replay.
//!
//! Text growth charges both old and replacement buffers until copying finishes.
//! Cancellation or an error leaves the operation failed. Once input ends, ordered
//! output reuses the hash buckets as merge-sort index arrays; keys and aggregate
//! cells remain in place. Final values are read through the shared accumulator.

use crate::batch::Batch;
use crate::execution::aggregation::accumulator::{AggregateCells, AggregateState, TextSpan};
use crate::execution::aggregation::arguments::ArgumentBatch;
use crate::execution::blocking::{
    ArgumentShape, BUFFER_ALLOCATION_UNIT, MAX_KEY_BYTES, RowLayout, append_bytes, buffer_capacity,
};
use crate::execution::{BATCH_ROWS, MAX_AGGREGATE_ROWS};
use crate::resources::{MemoryAuthority, Reservation, allocate};
use crate::{CancellationToken, Database, Error, Value};
use std::cmp::Ordering;
use std::mem::size_of;

const MAX_HASH_PROBES: usize = 64;
const MAX_LOOKUP_BYTES: usize = 8 * MAX_KEY_BYTES;
const EMPTY: u32 = u32::MAX;

// A power-of-two slot width keeps group arrays on the same capacity geometry
// as numeric state arrays. The padding is owned and charged before allocation.
#[repr(align(16))]
#[derive(Clone, Copy)]
struct KeySlot {
    start: usize,
    end: usize,
    hash: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HashLimit {
    Groups,
    KeyBytes,
    ProbeWork,
    TextBytes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HashStep {
    Progress,
    Complete,
    Fallback(HashLimit),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Input,
    GrowText,
    Complete,
    OrderInit(usize),
    Order(OrderCursor),
    Ordered,
    Fallback(HashLimit),
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OrderCursor {
    source: usize,
    target: usize,
    width: usize,
    left: usize,
    middle: usize,
    right: usize,
    end: usize,
    output: usize,
}

impl OrderCursor {
    fn pair(&mut self, start: usize, count: usize) {
        // All positions and widths are bounded by MAX_AGGREGATE_ROWS.
        self.left = start;
        self.middle = (start + self.width).min(count);
        self.right = self.middle;
        self.end = (self.middle + self.width).min(count);
        self.output = start;
    }
}

// Growth owns its new allocation and charge while the old arena remains live.
// Copying advances by at most one maximum-width text value per public step.
struct TextGrowth<'db> {
    bytes: Vec<u8>,
    reservation: Reservation<'db>,
}

pub(super) struct MemoryGroups<'db> {
    cells: AggregateCells,
    key: Vec<u8>,
    arena: Vec<u8>,
    entries: Vec<KeySlot>,
    buckets: Vec<u32>,
    positions: Vec<(usize, u32)>,
    shape: ArgumentShape,
    key_layout: u32,
    capacity: usize,
    key_limit: usize,
    phase: Phase,
    source_start: usize,
    batch_rows: usize,
    cursor: usize,
    input_rows: u64,
    order_start: usize,
    text_growth: Option<TextGrowth<'db>>,
    text_reservation: Option<Reservation<'db>>,
    authority: &'db MemoryAuthority,
    reservation: Reservation<'db>,
}

impl<'db> MemoryGroups<'db> {
    pub(super) fn new(
        database: &'db Database,
        aggregate: &AggregateState<'_>,
        keys: &RowLayout,
        capacity: usize,
        key_limit: usize,
    ) -> Result<Self, Error> {
        let (bucket_count, bytes) = Self::requirement(aggregate, keys, capacity, key_limit)?;
        let base = &aggregate.cells;
        let key_capacity = buffer_capacity(keys.max_bytes)?;
        let arena_capacity = array_capacity(key_limit, 1)
            .ok_or(Error::Corrupt("hash key arena allocation capacity"))?;
        let entry_capacity = array_capacity(capacity, size_of::<KeySlot>())
            .ok_or(Error::Corrupt("hash key slot capacity"))?;
        let reservation = database.reserve_memory(bytes, "optional hash groups")?;
        Ok(Self {
            cells: AggregateCells {
                values: repeated(&base.values, capacity, 0.0, bytes)?,
                integers: repeated(&base.integers, capacity, 0, bytes)?,
                nonnull_counts: repeated(&base.nonnull_counts, capacity, 0, bytes)?,
                counts: repeated(&base.counts, capacity, 0, bytes)?,
                flags: repeated(&base.flags, capacity, 0, bytes)?,
                value_slots: base.value_slots,
                count_slots: base.count_slots,
                sum_states: base.sum_states,
                extrema: repeated(&base.extrema, capacity, 0, bytes)?,
                extrema_slots: base.extrema_slots,
                text: Vec::new(),
                text_spans: if base.text.is_empty() {
                    Vec::new()
                } else {
                    filled(base.extrema.len() * capacity, TextSpan::default(), bytes)?
                },
                text_offsets: base.text_offsets,
            },
            key: allocate(key_capacity, key_capacity, "hash lookup key", bytes)?,
            arena: allocate(arena_capacity, arena_capacity, "group key arena", bytes)?,
            entries: allocate(entry_capacity, entry_capacity, "group key slots", bytes)?,
            buckets: filled(bucket_count, EMPTY, bytes)?,
            positions: filled(BATCH_ROWS, (0, 0), bytes)?,
            shape: ArgumentShape::from_aggregate(aggregate),
            key_layout: keys.layout,
            capacity,
            key_limit,
            phase: Phase::Idle,
            source_start: 0,
            batch_rows: 0,
            cursor: 0,
            input_rows: 0,
            order_start: 0,
            text_growth: None,
            text_reservation: None,
            authority: &database.memory,
            reservation,
        })
    }

    pub(super) fn requirement(
        aggregate: &AggregateState<'_>,
        keys: &RowLayout,
        capacity: usize,
        key_limit: usize,
    ) -> Result<(usize, u64), Error> {
        if aggregate.cells.counts.len() != 1
            || capacity == 0
            || capacity as u64 > MAX_AGGREGATE_ROWS
        {
            return Err(Error::Corrupt("memory grouping capacity"));
        }
        let bucket_count = capacity
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)
            .ok_or(Error::Corrupt("group bucket capacity"))?;
        let base = &aggregate.cells;
        let count = |width: usize| {
            width
                .checked_mul(capacity)
                .ok_or(Error::Corrupt("hash cell count"))
        };
        let arena_capacity = array_capacity(key_limit, 1)
            .ok_or(Error::Corrupt("hash key arena allocation capacity"))?;
        let key_capacity = buffer_capacity(keys.max_bytes)?;
        let bytes = [
            (count(base.values.len())?, size_of::<f64>()),
            (count(base.integers.len())?, size_of::<i128>()),
            (count(base.extrema.len())?, size_of::<u64>()),
            (
                if base.text.is_empty() {
                    0
                } else {
                    count(base.extrema.len())?
                },
                size_of::<TextSpan>(),
            ),
            (count(base.nonnull_counts.len())?, size_of::<u32>()),
            (capacity, size_of::<u32>()),
            (count(base.flags.len())?, size_of::<u32>()),
            (capacity, size_of::<KeySlot>()),
            (bucket_count, size_of::<u32>()),
            (BATCH_ROWS, size_of::<(usize, u32)>()),
        ]
        .into_iter()
        .try_fold(size_of::<Self>(), |total, (count, width)| {
            array_capacity(count, width)
                .and_then(|count| count.checked_mul(width))
                .and_then(|bytes| total.checked_add(bytes))
        })
        .and_then(|bytes| bytes.checked_add(arena_capacity))
        .and_then(|bytes| bytes.checked_add(key_capacity))
        .ok_or(Error::Corrupt("hash grouping memory"))? as u64;
        Ok((bucket_count, bytes))
    }

    pub(super) fn capacities(
        database: &Database,
        aggregate: &AggregateState<'_>,
        keys: &RowLayout,
    ) -> Result<(usize, usize), Error> {
        let available = database
            .config()
            .memory_limit_bytes()
            .checked_sub(database.reserved_memory_bytes())
            .ok_or(Error::Corrupt("memory account exceeds configured limit"))?;
        let (_, first) = Self::requirement(aggregate, keys, 1, 0)?;
        let Some(extra) = available.checked_sub(first) else {
            return Ok((0, 0));
        };
        if extra == 0 {
            return Ok((0, 0));
        }
        let base = &aggregate.cells;
        let cell_bytes = base.values.len() * size_of::<f64>()
            + base.integers.len() * size_of::<i128>()
            + base.extrema.len() * size_of::<u64>()
            + if base.text.is_empty() {
                0
            } else {
                base.extrema.len() * size_of::<TextSpan>()
            }
            + base.nonnull_counts.len() * size_of::<u32>()
            + size_of::<u32>()
            + base.flags.len() * size_of::<u32>();
        // Share the remaining budget between key bytes and group slots. Four
        // buckets per group bounds power-of-two rounding; lookup stays <= 50% full.
        let per_group = cell_bytes + size_of::<KeySlot>() + 4 * size_of::<u32>();
        // Keep STRING metadata bounded independently of the maximum text width.
        // Larger cardinalities retain the already-admitted external path.
        let maximum_groups = if base.text.is_empty() {
            MAX_AGGREGATE_ROWS
        } else {
            crate::execution::COMPUTE_ROWS as u64
        };
        let capacity = usize::try_from((extra / 2 / per_group as u64).clamp(1, maximum_groups))
            .map_err(|_| Error::Corrupt("hash capacity does not fit"))?;
        // Keep cell and slot arrays on power-of-two group capacities. Their
        // element widths then avoid the large partial allocation classes of
        // arbitrary row counts. Key bytes use the remaining budget below.
        let mut capacity = 1 << capacity.ilog2();
        let (_, mut arrays) = Self::requirement(aggregate, keys, capacity, 0)?;
        // Physical cell capacities can exceed their logical state counts.
        // Keep the metadata half-budget before assigning the key arena.
        while capacity > 1 && arrays > first + extra / 2 {
            capacity /= 2;
            arrays = Self::requirement(aggregate, keys, capacity, 0)?.1;
        }
        let key_bytes = available
            .checked_sub(arrays)
            .ok_or(Error::Corrupt("hash sizing exceeds available memory"))?;
        // Each occupied slot stores one encoded key. Bytes beyond this bound
        // cannot be used before the group limit forces the existing spill path.
        let maximum_key_bytes = capacity
            .checked_mul(keys.max_bytes)
            .ok_or(Error::Corrupt("hash key capacity overflow"))?;
        let key_bytes = usize::try_from(key_bytes)
            .map_err(|_| Error::Corrupt("hash key arena does not fit"))?;
        // Large key arenas use power-of-two capacities. They otherwise consume
        // arbitrary remainders that may reuse much larger native allocations
        // after previous queries. Round down within the remaining admission;
        // an exhausted arena still uses the independently admitted fallback.
        let key_bytes = if key_bytes > BUFFER_ALLOCATION_UNIT {
            1 << key_bytes.ilog2()
        } else {
            key_bytes
        };
        // Keep the full encoded-key limit when its rounded allocation fits.
        // Clamping before rounding down would discard keys despite available space.
        Ok((capacity, key_bytes.min(maximum_key_bytes)))
    }

    pub(super) fn begin(&mut self, arguments: &ArgumentBatch<'_>) -> Result<(), Error> {
        assert!(
            matches!(self.phase, Phase::Idle | Phase::Complete),
            "finish the prior hash batch"
        );
        self.phase = Phase::Failed;
        if arguments.shape != self.shape {
            return Err(Error::Corrupt("hash argument plan differs"));
        }
        let rows = self
            .input_rows
            .checked_add(arguments.rows as u64)
            .ok_or(Error::Corrupt("hash input row count"))?;
        if rows > MAX_AGGREGATE_ROWS {
            return Err(Error::Resource {
                owner: "aggregate input rows",
                required: rows,
                limit: MAX_AGGREGATE_ROWS,
            });
        }
        self.source_start = arguments.source_start.unwrap_or(0);
        if arguments.rows != 0 && arguments.source_start.is_none() {
            return Err(Error::Corrupt("hash input requires captured source rows"));
        }
        self.batch_rows = arguments.rows;
        self.cursor = 0;
        self.phase = Phase::Input;
        Ok(())
    }

    pub(super) fn step(
        &mut self,
        source: &Batch,
        arguments: &ArgumentBatch<'_>,
        keys: &RowLayout,
        cancel: &CancellationToken,
    ) -> Result<HashStep, Error> {
        let phase = std::mem::replace(&mut self.phase, Phase::Failed);
        match phase {
            Phase::Complete => {
                self.phase = phase;
                return Ok(HashStep::Complete);
            }
            Phase::Fallback(limit) => {
                self.phase = phase;
                return Ok(HashStep::Fallback(limit));
            }
            Phase::GrowText => {
                cancel.check()?;
                let growth = self
                    .text_growth
                    .as_mut()
                    .ok_or(Error::Corrupt("missing text growth"))?;
                let start = growth.bytes.len();
                let end = (start + crate::batch::MAX_TEXT_BYTES).min(self.cells.text.len());
                growth.bytes.extend_from_slice(&self.cells.text[start..end]);
                if end == self.cells.text.len() {
                    let growth = self.text_growth.take().expect("active text growth");
                    drop(std::mem::replace(&mut self.cells.text, growth.bytes));
                    // The old allocation is gone before its reservation is released.
                    self.text_reservation = Some(growth.reservation);
                    self.phase = Phase::Input;
                } else {
                    self.phase = Phase::GrowText;
                }
                return Ok(HashStep::Progress);
            }
            Phase::Input => {}
            Phase::Idle
            | Phase::OrderInit(_)
            | Phase::Order(_)
            | Phase::Ordered
            | Phase::Failed => {
                return Err(Error::Corrupt("hash grouping is not active"));
            }
        }
        cancel.check()?;
        if keys.layout != self.key_layout
            || arguments.shape != self.shape
            || arguments.rows != self.batch_rows
            || arguments.source_start.unwrap_or(0) != self.source_start
            || self
                .source_start
                .checked_add(self.batch_rows)
                .is_none_or(|end| end > source.len())
        {
            return Err(Error::Corrupt("hash input changed during a batch"));
        }
        if self.cursor == self.batch_rows {
            let required = self.text_requirement(arguments)?;
            if required > self.cells.text.capacity() {
                let capacity = required
                    .checked_next_power_of_two()
                    .ok_or(Error::Corrupt("hash text growth capacity"))?;
                match self.start_text_growth(capacity) {
                    Ok(()) => {
                        self.phase = Phase::GrowText;
                        return Ok(HashStep::Progress);
                    }
                    Err(Error::Resource { .. }) => {
                        self.phase = Phase::Fallback(HashLimit::TextBytes);
                        return Ok(HashStep::Fallback(HashLimit::TextBytes));
                    }
                    Err(error) => return Err(error),
                }
            }
            arguments.fold_positions(&mut self.cells, &self.positions[..self.batch_rows])?;
            self.phase = Phase::Complete;
            return Ok(HashStep::Complete);
        }
        keys.encode(source, self.source_start + self.cursor, &mut self.key)?;
        let hash = keys.hash(&self.key)?;
        let group = match self.lookup(keys, hash)? {
            Ok(group) => group,
            Err(limit) => {
                self.phase = Phase::Fallback(limit);
                return Ok(HashStep::Fallback(limit));
            }
        };
        self.positions[self.cursor] = (group, self.cells.count_row(group)?);
        self.cursor += 1;
        self.input_rows += 1; // begin checked the complete batch before admission.
        self.phase = Phase::Input;
        Ok(HashStep::Progress)
    }

    fn start_text_growth(&mut self, capacity: usize) -> Result<(), Error> {
        let reservation = self
            .authority
            .reserve(capacity as u64, "hash text growth")?;
        let bytes = allocate(capacity, capacity, "hash text growth", reservation.bytes())?;
        self.text_growth = Some(TextGrowth { bytes, reservation });
        Ok(())
    }

    fn text_requirement(&self, arguments: &ArgumentBatch<'_>) -> Result<usize, Error> {
        let mut required = self.cells.text.len();
        if self.cells.text_spans.is_empty() {
            return Ok(required);
        }
        let slots = self.cells.extrema.len() / self.capacity;
        // A conservative batch bound: only values wider than the current slot
        // can append a region. Repeated groups may overestimate growth, but row
        // folding never allocates and the bound is independent of prior batches.
        for state in 0..self.shape.count {
            if self.shape.text & (1 << state) == 0 {
                continue;
            }
            for (row, &(group, _)) in self.positions[..self.batch_rows].iter().enumerate() {
                let Some(value) = arguments.text_value(state, row)? else {
                    continue;
                };
                for direction in 0..2 {
                    let slot = self.cells.extrema_slots[direction][state];
                    if slot == u8::MAX {
                        continue;
                    }
                    let span = self.cells.text_spans[group * slots + usize::from(slot)];
                    if value.len() > span.capacity {
                        required = required
                            .checked_add(value.len().next_power_of_two())
                            .ok_or(Error::Corrupt("hash text batch requirement"))?;
                    }
                }
            }
        }
        Ok(required)
    }

    fn lookup(&mut self, keys: &RowLayout, hash: u64) -> Result<Result<usize, HashLimit>, Error> {
        let mask = self.buckets.len() - 1;
        let first = usize::try_from(hash & mask as u64).expect("masked bucket index");
        // Reserve encoding, UTF-8 validation, hashing and a possible key copy,
        // then validation and comparison of both keys for each matching hash.
        // Even one short input must not hide repeated scans of wide stored keys.
        let mut bytes = self.key.len() * 4;
        for probe in 0..MAX_HASH_PROBES.min(self.buckets.len()) {
            let bucket = (first + probe) & mask;
            let entry = self.buckets[bucket];
            if entry == EMPTY {
                if self.entries.len() == self.capacity {
                    return Ok(Err(HashLimit::Groups));
                }
                let end = self
                    .arena
                    .len()
                    .checked_add(self.key.len())
                    .ok_or(Error::Corrupt("group key growth"))?;
                if end > self.key_limit {
                    return Ok(Err(HashLimit::KeyBytes));
                }
                let group = self.entries.len();
                let start = self.arena.len();
                append_bytes(&mut self.arena, &self.key)?;
                self.entries.push(KeySlot { start, end, hash });
                self.buckets[bucket] = u32::try_from(group).expect("bounded group identity");
                return Ok(Ok(group));
            }
            let group = entry as usize;
            let slot = self.entries[group];
            if slot.hash != hash {
                continue;
            }
            bytes = self
                .key
                .len()
                .checked_add(slot.end - slot.start)
                .and_then(|work| work.checked_mul(2))
                .and_then(|work| bytes.checked_add(work))
                .ok_or(Error::Corrupt("group lookup work"))?;
            if bytes > MAX_LOOKUP_BYTES {
                return Ok(Err(HashLimit::ProbeWork));
            }
            if keys.compare(&self.key, &self.arena[slot.start..slot.end])? == Ordering::Equal {
                return Ok(Ok(group));
            }
        }
        Ok(Err(HashLimit::ProbeWork))
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn memory_bytes(&self) -> u64 {
        // Growth owns a separate reservation and keeps the old arena alive
        // until copying finishes. Report both throughout that overlap.
        self.reservation.bytes()
            + self.text_reservation.as_ref().map_or(0, Reservation::bytes)
            + self
                .text_growth
                .as_ref()
                .map_or(0, |growth| growth.reservation.bytes())
    }

    pub(super) fn begin_order(&mut self) -> Result<(), Error> {
        if !matches!(self.phase, Phase::Idle | Phase::Complete) {
            self.phase = Phase::Failed;
            return Err(Error::Corrupt("hash input must finish before ordering"));
        }
        // Input is finished. Reuse the no-longer-needed hash buckets as two
        // index arrays, preserving key bytes and cells in their original slots.
        assert!(self.buckets.len() >= 2 * self.entries.len());
        self.phase = Phase::OrderInit(0);
        Ok(())
    }

    pub(super) fn order_step(
        &mut self,
        keys: &RowLayout,
        cancel: &CancellationToken,
    ) -> Result<bool, Error> {
        let phase = std::mem::replace(&mut self.phase, Phase::Failed);
        cancel.check()?;
        if keys.layout != self.key_layout {
            return Err(Error::Corrupt("hash ordering key layout"));
        }
        let count = self.entries.len();
        self.phase = match phase {
            Phase::OrderInit(start) => {
                let end = (start + BATCH_ROWS).min(count);
                for index in start..end {
                    self.buckets[index] = u32::try_from(index).expect("bounded group identity");
                }
                if end != count {
                    Phase::OrderInit(end)
                } else if count <= 1 {
                    Phase::Ordered
                } else {
                    let mut cursor = OrderCursor {
                        source: 0,
                        target: count,
                        width: 1,
                        left: 0,
                        middle: 0,
                        right: 0,
                        end: 0,
                        output: 0,
                    };
                    cursor.pair(0, count);
                    Phase::Order(cursor)
                }
            }
            Phase::Order(mut cursor) => {
                // One step compares at most two bounded keys and moves one ID.
                let left_first = if cursor.left == cursor.middle {
                    false
                } else if cursor.right == cursor.end {
                    true
                } else {
                    let left = self.entries[self.buckets[cursor.source + cursor.left] as usize];
                    let right = self.entries[self.buckets[cursor.source + cursor.right] as usize];
                    keys.compare(
                        &self.arena[left.start..left.end],
                        &self.arena[right.start..right.end],
                    )? != Ordering::Greater
                };
                let input = if left_first {
                    &mut cursor.left
                } else {
                    &mut cursor.right
                };
                self.buckets[cursor.target + cursor.output] = self.buckets[cursor.source + *input];
                *input += 1;
                cursor.output += 1;
                if cursor.output == cursor.end {
                    if cursor.end == count {
                        std::mem::swap(&mut cursor.source, &mut cursor.target);
                        cursor.width *= 2;
                        cursor.pair(0, count);
                    } else {
                        cursor.pair(cursor.end, count);
                    }
                }
                if cursor.width >= count {
                    self.order_start = cursor.source;
                    Phase::Ordered
                } else {
                    Phase::Order(cursor)
                }
            }
            Phase::Ordered => Phase::Ordered,
            _ => return Err(Error::Corrupt("hash ordering is not active")),
        };
        Ok(self.phase == Phase::Ordered)
    }

    pub(super) fn ordered_group(&self, index: usize) -> Result<usize, Error> {
        if self.phase != Phase::Ordered || index >= self.entries.len() {
            return Err(Error::Corrupt("ordered group position"));
        }
        Ok(self.buckets[self.order_start + index] as usize)
    }

    pub(super) fn key_value<'a>(
        &'a self,
        group: usize,
        column: usize,
        keys: &RowLayout,
    ) -> Result<Value<'a>, Error> {
        let slot = self
            .entries
            .get(group)
            .ok_or(Error::Corrupt("hash group identity"))?;
        if keys.layout != self.key_layout {
            return Err(Error::Corrupt("hash key layout differs"));
        }
        keys.value(&self.arena[slot.start..slot.end], column)
    }

    pub(super) fn load_group(
        &self,
        group: usize,
        aggregate: &mut AggregateState<'_>,
    ) -> Result<(), Error> {
        if !matches!(self.phase, Phase::Complete | Phase::Ordered)
            || group >= self.entries.len()
            || aggregate.cells.counts.len() != 1
            || ArgumentShape::from_aggregate(aggregate) != self.shape
            || aggregate.cells.value_slots != self.cells.value_slots
            || aggregate.cells.count_slots != self.cells.count_slots
            || aggregate.cells.extrema_slots != self.cells.extrema_slots
            || aggregate.cells.text_offsets != self.cells.text_offsets
            || aggregate.cells.sum_states != self.cells.sum_states
        {
            return Err(Error::Corrupt("hash finalization state differs"));
        }
        let target = &mut aggregate.cells;
        copy_group(&self.cells.values, &mut target.values, self.capacity, group)?;
        copy_group(
            &self.cells.integers,
            &mut target.integers,
            self.capacity,
            group,
        )?;
        copy_group(
            &self.cells.nonnull_counts,
            &mut target.nonnull_counts,
            self.capacity,
            group,
        )?;
        copy_group(
            &self.cells.extrema,
            &mut target.extrema,
            self.capacity,
            group,
        )?;
        let slots = target.extrema.len();
        if !self.cells.text_spans.is_empty()
            && self.cells.text_spans.len() != self.cells.extrema.len()
        {
            return Err(Error::Corrupt("hash text slot extent"));
        }
        for slot in 0..slots {
            if target.text_offsets[slot + 1] != target.text_offsets[slot] {
                let value = self.cells.text_value(group, slot)?;
                let start = target.text_offsets[slot];
                target.text[start..start + value.len()].copy_from_slice(value.as_bytes());
            }
        }
        copy_group(&self.cells.counts, &mut target.counts, self.capacity, group)?;
        copy_group(&self.cells.flags, &mut target.flags, self.capacity, group)
    }
}

// Large arrays own a power-of-two physical extent. Vec length still describes
// exactly the logical groups and state lanes consumed by folding and replay.
fn array_capacity(count: usize, width: usize) -> Option<usize> {
    if !width.is_power_of_two() {
        return None;
    }
    if count.checked_mul(width)? > BUFFER_ALLOCATION_UNIT {
        count.checked_next_power_of_two()
    } else {
        Some(count)
    }
}

fn filled<T: Copy>(count: usize, value: T, charge: u64) -> Result<Vec<T>, Error> {
    let capacity = array_capacity(count, size_of::<T>())
        .ok_or(Error::Corrupt("hash array allocation capacity"))?;
    let mut output = allocate(capacity, capacity, "hash group arrays", charge)?;
    output.resize(count, value);
    Ok(output)
}

fn repeated<T: Copy>(base: &[T], groups: usize, value: T, charge: u64) -> Result<Vec<T>, Error> {
    let count = base
        .len()
        .checked_mul(groups)
        .ok_or(Error::Corrupt("hash numeric cells"))?;
    filled(count, value, charge)
}

fn copy_group<T: Copy>(
    source: &[T],
    target: &mut [T],
    groups: usize,
    group: usize,
) -> Result<(), Error> {
    if target.len().checked_mul(groups) != Some(source.len()) {
        return Err(Error::Corrupt("hash group cell shape"));
    }
    let start = group
        .checked_mul(target.len())
        .ok_or(Error::Corrupt("hash group cell offset"))?;
    target.copy_from_slice(&source[start..start + target.len()]);
    Ok(())
}

#[cfg(test)]
mod tests;
