//! Optional hash grouping. Its allocations never own the disk fallback's charge.
use crate::batch::Batch;
use crate::execution::aggregation::arguments::ArgumentBatch;
use crate::execution::aggregation::numeric::{AggregateCells, AggregateState};
use crate::execution::blocking::{ArgumentShape, MAX_KEY_BYTES, RowLayout, append_bytes};
use crate::execution::{BATCH_ROWS, MAX_AGGREGATE_ROWS};
use crate::resources::{Reservation, allocate};
use crate::{CancellationToken, Database, Error, Value};
use std::cmp::Ordering;
use std::mem::size_of;

const MAX_HASH_PROBES: usize = 64;
const MAX_LOOKUP_BYTES: usize = 8 * MAX_KEY_BYTES;
const EMPTY: u32 = u32::MAX;

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
            },
            key: allocate(keys.max_bytes, keys.max_bytes, "hash lookup key", bytes)?,
            arena: allocate(key_limit, key_limit, "group key arena", bytes)?,
            entries: allocate(capacity, capacity, "group key slots", bytes)?,
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
        let bytes = [
            (count(base.values.len())?, size_of::<f64>()),
            (count(base.integers.len())?, size_of::<i128>()),
            (count(base.nonnull_counts.len())?, size_of::<u32>()),
            (capacity, size_of::<u32>()),
            (count(base.flags.len())?, size_of::<u32>()),
            (capacity, size_of::<KeySlot>()),
            (bucket_count, size_of::<u32>()),
            (BATCH_ROWS, size_of::<(usize, u32)>()),
        ]
        .into_iter()
        .try_fold(size_of::<Self>(), |total, (count, width)| {
            count
                .checked_mul(width)
                .and_then(|bytes| total.checked_add(bytes))
        })
        .and_then(|bytes| bytes.checked_add(key_limit))
        .and_then(|bytes| bytes.checked_add(keys.max_bytes))
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
            + base.nonnull_counts.len() * size_of::<u32>()
            + size_of::<u32>()
            + base.flags.len() * size_of::<u32>();
        // Share the remaining budget between key bytes and group slots. Four
        // buckets per group bounds power-of-two rounding; lookup stays <= 50% full.
        let per_group = cell_bytes + size_of::<KeySlot>() + 4 * size_of::<u32>();
        let capacity = usize::try_from((extra / 2 / per_group as u64).clamp(1, MAX_AGGREGATE_ROWS))
            .map_err(|_| Error::Corrupt("hash capacity does not fit"))?;
        let (_, arrays) = Self::requirement(aggregate, keys, capacity, 0)?;
        let key_bytes = available
            .checked_sub(arrays)
            .ok_or(Error::Corrupt("hash sizing exceeds available memory"))?;
        Ok((
            capacity,
            usize::try_from(key_bytes)
                .map_err(|_| Error::Corrupt("hash key arena does not fit"))?,
        ))
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
        self.reservation.bytes()
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
        copy_group(&self.cells.counts, &mut target.counts, self.capacity, group)?;
        copy_group(&self.cells.flags, &mut target.flags, self.capacity, group)
    }
}

fn filled<T: Copy>(count: usize, value: T, charge: u64) -> Result<Vec<T>, Error> {
    let mut output = allocate(count, count, "hash group arrays", charge)?;
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
