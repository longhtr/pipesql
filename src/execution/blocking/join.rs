//! Join two sorted inputs on one equality key, retaining every matching pair.
//!
//! Each `SortedInput` owns one side's row buffers, sorter and temporary files.
//! Both sides finish collection and sorting before matching starts. After sorting
//! the first side, free its staging record and unused merge-right buffers before
//! collecting the second side; output and replay retain the left cursor and files.
//! The join advances whichever key is smaller until the keys match or an input ends.
//!
//! For equal keys, it remembers the first right row's file position and key.
//! It scans that right group again for each equal left row: two left rows and
//! three right rows produce six pairs. This avoids holding the whole group in
//! memory. Sorting groups NULLs and NaNs together, but join equality rejects them.
//!
//! A LEFT JOIN fills right fields with NULL when a left row has no match.
//! Downstream filters run after matching; rejecting every pair does not make
//! that left row unmatched. Expressions and file reads can fail during emission.
//!
//! Replay restarts the retained inputs once. Any failed step stops further work;
//! the enclosing query releases both sides' buffers and temporary files.

use super::{
    CursorPosition as Bookmark, RowLayout, SortPhase, SortedInput, append_bytes, compare_values,
};
use crate::batch::Batch;
use crate::effects::Effects;
use crate::execution::ConsumerInput;
use crate::execution::computed::RowValues;
use crate::execution::planning::Pipeline;
use crate::query::{JoinKind, MAX_ROW_VALUES, SemanticColumn};
use crate::resources::{Reservation, allocate};
use crate::value::{DataType, Value};
use crate::{CancellationToken, Database, Error};
use std::cmp::Ordering;
use std::mem::size_of;

impl SortedInput<'_> {
    fn key(&self) -> Result<Value<'_>, Error> {
        let record = self
            .sort
            .sorted_cursor()
            .record()
            .ok_or(Error::Corrupt("join key requires a loaded row"))?;
        self.layout.value(record.key(), 0)
    }

    // Preserve the group's key as well as its position. The right cursor can
    // move past this group before the next left row arrives for comparison.
    fn bookmark(&mut self) -> Result<Bookmark, Error> {
        let rows = self.sort.sorted_rows();
        let position = rows
            .cursor
            .position()
            .ok_or(Error::Corrupt("join bookmark requires a loaded row"))?;
        rows.previous_key.clear();
        append_bytes(
            rows.previous_key,
            self.layout.sort_prefix(
                rows.cursor
                    .record()
                    .expect("bookmark has a loaded row")
                    .key(),
            )?,
        )?;
        // A preceding replay may leave only this group's first row buffered.
        // Bound its initial prefetch before the next rewind discards excess rows.
        // Larger groups can continue reading beyond the hint; rewinds still
        // clear the buffer and validate every record again.
        rows.cursor.limit_read_ahead(16_384);
        Ok(position)
    }

    fn rewind(&mut self, bookmark: Bookmark) -> Result<(), Error> {
        self.sort.sorted_rows().cursor.rewind(bookmark)
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Create(usize),
    Read(usize),
    Await(usize),
    Capture(usize, usize),
    Push(usize, usize),
    Spill(usize, usize),
    Sort(usize),
    Seek,
    Emit,
    Unmatched,
    Right,
    Left,
    Done,
    Failed,
}

pub(in crate::execution) enum Step {
    Input(usize),
    Progress,
    Rows,
    Finished,
}

pub(in crate::execution) struct Join<'db> {
    sides: [SortedInput<'db>; 2],
    phase: Phase,
    group: Option<Bookmark>,
    kind: JoinKind,
    batch_matches: bool,
    replayed: bool,
    reservation: Reservation<'db>,
}

impl<'db> Join<'db> {
    pub(in crate::execution) fn new(
        database: &'db Database,
        left: impl Iterator<Item = SemanticColumn>,
        right: impl Iterator<Item = SemanticColumn>,
        keys: (u8, u8),
        kind: JoinKind,
        direct_output: bool,
        runtime: &mut Reservation<'db>,
    ) -> Result<Vec<Self>, Error> {
        let left = RowLayout::for_join(left, usize::from(keys.0))?;
        let right = RowLayout::for_join(right, usize::from(keys.1))?;
        if left.columns[0].kind != right.columns[0].kind
            || left.count + right.count > MAX_ROW_VALUES
        {
            return Err(Error::Corrupt("join input type or width"));
        }
        let batch_matches = direct_output
            && [&left, &right].iter().all(|layout| {
                layout.columns[..layout.count]
                    .iter()
                    .all(|column| column.kind != DataType::String)
            });
        let reservation = database.reserve_memory(
            (size_of::<Self>() - 2 * size_of::<SortedInput<'_>>()) as u64,
            "join controller",
        )?;
        let sides = [
            SortedInput::new(database, left)?,
            SortedInput::new(database, right)?,
        ];
        let mut owner = allocate(
            1,
            1,
            "join owner",
            reservation.bytes() + sides.iter().map(SortedInput::memory_bytes).sum::<u64>(),
        )?;
        owner.push(Self {
            sides,
            phase: Phase::Create(0),
            group: None,
            kind,
            batch_matches,
            replayed: false,
            reservation,
        });
        // The controller's Vec remains allocated while its fields are dropped.
        // Runtime keeps its charge until that allocation is freed, including
        // when opening a later source fails before the join can start.
        let join = &mut owner[0];
        join.reservation.transfer_to(
            runtime,
            (size_of::<Self>() - 2 * size_of::<SortedInput<'_>>()) as u64,
        )?;
        for side in &mut join.sides {
            side.transfer_inline_to(runtime)?;
        }
        Ok(owner)
    }

    pub(in crate::execution) fn memory_bytes(&self) -> u64 {
        self.reservation.bytes()
            + self
                .sides
                .iter()
                .map(SortedInput::memory_bytes)
                .sum::<u64>()
    }

    #[cfg(test)]
    pub(in crate::execution) fn was_replayed(&self) -> bool {
        self.replayed
    }

    pub(in crate::execution) fn replay(&mut self, cancel: &CancellationToken) -> Result<(), Error> {
        cancel.check()?;
        if self.replayed {
            return Err(Error::Resource {
                owner: "join replays",
                required: 2,
                limit: 1,
            });
        }
        if !matches!(
            self.phase,
            Phase::Seek | Phase::Emit | Phase::Right | Phase::Left | Phase::Done
        ) {
            self.phase = Phase::Failed;
            return Err(Error::Corrupt("join replay precedes sorted inputs"));
        }
        for side in &mut self.sides {
            side.begin_read();
        }
        self.group = None;
        self.replayed = true;
        self.phase = Phase::Seek;
        Ok(())
    }

    fn emit_fixed(
        &mut self,
        output: &mut Batch,
        plan: &Pipeline,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        let left_width = self.sides[0].layout.count;
        let width = left_width + self.sides[1].layout.count;
        let mut rows = 0;
        loop {
            cancel.check()?;
            let mut values = [Value::Null; MAX_ROW_VALUES];
            self.sides[0].read_values(&mut values[..left_width])?;
            self.sides[1].read_values(&mut values[left_width..width])?;
            for (position, column) in plan.columns[..plan.column_count].iter().enumerate() {
                let value = values[..width]
                    .get(usize::from(*column))
                    .copied()
                    .ok_or(Error::Corrupt("join output column bound"))?;
                output.set(rows, position, value)?;
            }
            rows += 1;
            self.sides[1].consume()?;
            if rows == crate::batch::ROWS {
                break;
            }
            self.sides[1].load(cancel, effects)?;
            if self.sides[1].finished()
                || compare_values(self.sides[0].key()?, self.sides[1].key()?) != Ordering::Equal
            {
                break;
            }
        }
        // A later read can fail after earlier cells were filled. Those cells
        // stay private until every pair in this batch has succeeded.
        output.publish_rows(rows);
        Ok(())
    }

    pub(in crate::execution) fn step(
        &mut self,
        inputs: [ConsumerInput<'_>; 2],
        output: &mut Batch,
        plan: &Pipeline,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Step, Error> {
        output.clear();
        // Install Failed before any fallible work so an error cannot resume
        // matching with only one of the two cursors advanced.
        let phase = std::mem::replace(&mut self.phase, Phase::Failed);
        if matches!(phase, Phase::Failed) {
            return Err(Error::Corrupt("join has failed"));
        }
        if matches!(phase, Phase::Done) {
            self.phase = Phase::Done;
            return Ok(Step::Finished);
        }
        cancel.check()?;
        let mut step = Step::Progress;
        self.phase = match phase {
            Phase::Create(side) => {
                self.sides[side].start(cancel, effects)?;
                Phase::Read(side)
            }
            Phase::Read(side) => {
                step = Step::Input(side);
                Phase::Await(side)
            }
            Phase::Await(side) => {
                if inputs[side].finished {
                    if !inputs[side].batch.is_empty() {
                        return Err(Error::Corrupt("finished join input contains rows"));
                    }
                    self.sides[side].sort.finish()?;
                    Phase::Sort(side)
                } else {
                    if inputs[side].batch.is_empty() {
                        return Err(Error::Corrupt("join input was not supplied"));
                    }
                    Phase::Capture(side, 0)
                }
            }
            Phase::Capture(side, row) => {
                if row == inputs[side].batch.len() {
                    Phase::Read(side)
                } else {
                    let input = &mut self.sides[side];
                    input.record.encode_row(
                        &input.layout,
                        inputs[side].batch,
                        row,
                        input.ordinal,
                        None,
                    )?;
                    Phase::Push(side, row)
                }
            }
            // Keep a record rejected by a full run while that run spills.
            // Retry the same record before advancing its input row number.
            Phase::Push(side, row) => {
                let input = &mut self.sides[side];
                if input.sort.push(&input.record)? {
                    input.ordinal = input
                        .ordinal
                        .checked_add(1)
                        .ok_or(Error::Corrupt("join input ordinal overflow"))?;
                    Phase::Capture(side, row + 1)
                } else {
                    Phase::Spill(side, row)
                }
            }
            Phase::Spill(side, row) => {
                let input = &mut self.sides[side];
                input.sort_step(cancel, effects)?;
                if input.sort.phase() == SortPhase::Collect {
                    Phase::Push(side, row)
                } else {
                    phase
                }
            }
            Phase::Sort(side) => {
                if self.sides[side].sort_step(cancel, effects)? {
                    self.sides[side].begin_read();
                    if side == 0 {
                        self.sides[0].release_merge_right();
                        self.sides[0].release_staging_record();
                        Phase::Create(1)
                    } else {
                        Phase::Seek
                    }
                } else {
                    phase
                }
            }
            Phase::Seek => {
                if self.sides[0].load(cancel, effects)? || self.sides[1].load(cancel, effects)? {
                    phase
                } else if self.sides[0].finished() {
                    Phase::Done
                } else if self.sides[1].finished() {
                    if self.kind == JoinKind::Left {
                        Phase::Unmatched
                    } else {
                        Phase::Done
                    }
                } else {
                    let (left, right) = (self.sides[0].key()?, self.sides[1].key()?);
                    match compare_values(left, right) {
                        Ordering::Less => {
                            if self.kind == JoinKind::Left {
                                Phase::Unmatched
                            } else {
                                self.sides[0].consume()?;
                                phase
                            }
                        }
                        Ordering::Greater => {
                            self.sides[1].consume()?;
                            phase
                        }
                        Ordering::Equal if left == Value::Null || left != right => {
                            // Equal sort keys can still be two NULLs or two
                            // NaNs. Neither pair satisfies join equality.
                            if self.kind == JoinKind::Left {
                                Phase::Unmatched
                            } else {
                                self.sides[0].consume()?;
                                phase
                            }
                        }
                        Ordering::Equal => {
                            self.group = Some(self.sides[1].bookmark()?);
                            Phase::Emit
                        }
                    }
                }
            }
            // Direct fixed-width values fit the admitted output slots. Filters
            // and expressions retain one-row demand so LIMIT can stop before
            // a later pair's arithmetic error.
            Phase::Emit if self.batch_matches => {
                self.emit_fixed(output, plan, cancel, effects)?;
                step = Step::Rows;
                Phase::Right
            }
            Phase::Emit | Phase::Unmatched => {
                let unmatched = matches!(phase, Phase::Unmatched);
                let left_width = self.sides[0].layout.count;
                let width = left_width + self.sides[1].layout.count;
                let mut values = [Value::Null; MAX_ROW_VALUES];
                self.sides[0].read_values(&mut values[..left_width])?;
                if !unmatched {
                    self.sides[1].read_values(&mut values[left_width..width])?;
                }
                let value = |column: u8| -> Result<Value<'_>, Error> {
                    values[..width]
                        .get(usize::from(column))
                        .copied()
                        .ok_or(Error::Corrupt("join output column bound"))
                };
                let mut evaluated = RowValues::new(plan, value);
                let retained = evaluated.retains()?;
                if retained {
                    for (position, column) in plan.columns[..plan.column_count].iter().enumerate() {
                        output.set(0, position, evaluated.value(*column)?)?;
                    }
                    output.publish_rows(1);
                    step = Step::Rows;
                }
                // A following WHERE can discard a matched pair, but cannot
                // turn it into an unmatched left row. Advance by match status,
                // regardless of whether this pair produced an output row.
                if unmatched {
                    self.sides[0].consume()?;
                    Phase::Seek
                } else {
                    self.sides[1].consume()?;
                    Phase::Right
                }
            }
            // Walk the remaining right rows with this key for the current left
            // row. Only after that group ends may the left cursor advance.
            Phase::Right => {
                if self.sides[1].load(cancel, effects)? {
                    phase
                } else if !self.sides[1].finished()
                    && compare_values(self.sides[0].key()?, self.sides[1].key()?) == Ordering::Equal
                {
                    Phase::Emit
                } else {
                    self.sides[0].consume()?;
                    Phase::Left
                }
            }
            Phase::Left => {
                if self.sides[0].load(cancel, effects)? {
                    phase
                } else if self.sides[0].finished() {
                    Phase::Done
                } else {
                    let left = &self.sides[0];
                    let same = left.layout.compare(
                        left.sort
                            .sorted_cursor()
                            .record()
                            .expect("loaded left join row")
                            .key(),
                        self.sides[1].sort.previous_key(),
                    )? == Ordering::Equal;
                    if same {
                        // Another left row needs every right match again.
                        // Rewind rereads and validates records from the bookmark.
                        self.sides[1].rewind(
                            self.group
                                .ok_or(Error::Corrupt("join group bookmark absent"))?,
                        )?;
                        Phase::Right
                    } else {
                        self.group = None;
                        Phase::Seek
                    }
                }
            }
            Phase::Done | Phase::Failed => unreachable!("terminal join phases handled before work"),
        };
        Ok(step)
    }
}

#[cfg(test)]
#[path = "join_tests.rs"]
mod tests;
