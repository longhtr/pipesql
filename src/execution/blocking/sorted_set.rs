//! Compare sorted rows to compute EXCEPT and INTERSECT.
//!
//! Each `SortedInput` owns one branch's buffers, sorter and temporary files.
//! The controller collects and sorts the entire left branch, then the right,
//! before comparing their rows. Even an empty left branch requires the right
//! branch to finish, so its demanded errors cannot disappear.
//!
//! EXCEPT selects left rows absent from the right; INTERSECT selects matches.
//! DISTINCT returns one representative of each selected row value. ALL matches
//! occurrences one-to-one: three left copies and two right copies produce one
//! EXCEPT ALL row or two INTERSECT ALL rows. Selected values come from the left
//! and retain their bits. Comparison uses grouping equality, including NULLs,
//! NaNs and signed zeros.
//!
//! Downstream expressions run only for selected rows. Replay restarts both
//! retained cursors once, without reopening the sources. A failed step prevents
//! further work; the enclosing query releases both inputs and their scratch files.

use super::{RowLayout, SortPhase, SortedInput, append_bytes, compare_values, read_value};
use crate::batch::Batch;
use crate::effects::Effects;
use crate::execution::computed::RowValues;
use crate::execution::planning::{OrderColumn, Pipeline};
use crate::execution::{ConsumerInput, SetStep};
use crate::query::{Direction, MAX_COLUMNS, NullPlacement, SetKind, SetPlan};
use crate::resources::{Reservation, allocate};
use crate::{CancellationToken, Database, Error};
use std::cmp::Ordering;
use std::mem::size_of;

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
    Done,
    Failed,
}

pub(in crate::execution) struct SortedSet<'db> {
    sides: [SortedInput<'db>; 2],
    // A child may store SELECT k, k in one slot. Map each set column back to
    // that slot so both positions are present in the row used for comparison.
    positions: [[u8; MAX_COLUMNS]; 2],
    kind: SetKind,
    phase: Phase,
    replayed: bool,
    reservation: Reservation<'db>,
}

impl<'db> SortedSet<'db> {
    pub(in crate::execution) fn new(
        database: &'db Database,
        bound: &SetPlan,
        inputs: [&Pipeline<'_>; 2],
        runtime: &mut Reservation<'db>,
    ) -> Result<Vec<Self>, Error> {
        let width = bound.width();
        if bound.kind() == SetKind::UnionAll || width == 0 || width > MAX_COLUMNS {
            return Err(Error::Corrupt("sorted set descriptor shape"));
        }
        let mut positions = [[0; MAX_COLUMNS]; 2];
        for side in 0..2 {
            for (position, slot) in positions[side][..width].iter_mut().enumerate() {
                let column = bound.inputs(position).expect("validated set width")[side];
                *slot = u8::try_from(
                    inputs[side]
                        .position(column.identity())
                        .ok_or(Error::Corrupt("sorted set comparison input absent"))?,
                )
                .map_err(|_| Error::Corrupt("sorted set comparison input position"))?;
            }
        }
        let keys: [OrderColumn; MAX_COLUMNS] = std::array::from_fn(|column| OrderColumn {
            column: column as u8,
            direction: Direction::Ascending,
            nulls: NullPlacement::First,
        });
        let layout = |side| {
            RowLayout::for_order(
                (0..width)
                    .map(|position| bound.inputs(position).expect("validated set width")[side]),
                &keys[..width],
            )
        };
        let reservation = database.reserve_memory(
            (size_of::<Self>() - 2 * size_of::<SortedInput<'_>>()) as u64,
            "sorted set controller",
        )?;
        let sides = [
            SortedInput::new(database, layout(0)?)?,
            SortedInput::new(database, layout(1)?)?,
        ];
        let mut owner = allocate(
            1,
            1,
            "sorted set owner",
            reservation.bytes() + sides.iter().map(SortedInput::memory_bytes).sum::<u64>(),
        )?;
        owner.push(Self {
            sides,
            positions,
            kind: bound.kind(),
            phase: Phase::Create(0),
            replayed: false,
            reservation,
        });
        // The containing Vec stays allocated while its fields are dropped.
        // Keep its charge in the runtime until the allocation itself is freed.
        let sorted_set = &mut owner[0];
        sorted_set.reservation.transfer_to(
            runtime,
            (size_of::<Self>() - 2 * size_of::<SortedInput<'_>>()) as u64,
        )?;
        for side in &mut sorted_set.sides {
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

    fn begin_read(&mut self) {
        for input in &mut self.sides {
            input.begin_read();
            let rows = input.sort.sorted_rows();
            rows.previous_key.clear();
            *rows.previous_ordinal = None;
        }
    }

    pub(in crate::execution) fn replay(&mut self, cancel: &CancellationToken) -> Result<(), Error> {
        cancel.check()?;
        if self.replayed || !matches!(self.phase, Phase::Seek | Phase::Emit | Phase::Done) {
            return Err(Error::Corrupt(
                "sorted set replay requires completed sorted inputs",
            ));
        }
        self.begin_read();
        self.replayed = true;
        self.phase = Phase::Seek;
        Ok(())
    }

    // Types match across branches, but NULLability can differ. Decode each
    // side with its own layout or a valid right-side NULL could be rejected.
    fn compare(&self) -> Result<Ordering, Error> {
        let [left, right] = &self.sides;
        let mut left_bytes = left
            .sort
            .sorted_cursor()
            .record()
            .expect("loaded left row")
            .key();
        let mut right_bytes = right
            .sort
            .sorted_cursor()
            .record()
            .expect("loaded right row")
            .key();
        for (a, b) in left.layout.columns[..left.layout.count]
            .iter()
            .zip(&right.layout.columns[..right.layout.count])
        {
            let order = compare_values(
                read_value(&mut left_bytes, a.kind, a.nullable)?,
                read_value(&mut right_bytes, b.kind, b.nullable)?,
            );
            if order != Ordering::Equal {
                return Ok(order);
            }
        }
        Ok(Ordering::Equal)
    }

    // Check order against the last consumed row, whether or not it was emitted.
    // Equal keys are duplicates; equal or reversed key/ordinal pairs are damage.
    fn duplicate(&mut self, side: usize) -> Result<bool, Error> {
        let input = &mut self.sides[side];
        let rows = input.sort.sorted_rows();
        let record = rows.cursor.record().expect("loaded set row");
        let Some(previous) = *rows.previous_ordinal else {
            return Ok(false);
        };
        let order = input.layout.compare(rows.previous_key, record.key())?;
        if order.then(previous.cmp(&record.ordinal())) != Ordering::Less {
            return Err(Error::Corrupt("sorted set input is not monotonic"));
        }
        Ok(order == Ordering::Equal)
    }

    fn consume(&mut self, side: usize) -> Result<(), Error> {
        let rows = self.sides[side].sort.sorted_rows();
        let record = rows.cursor.record().expect("consume loaded set row");
        rows.previous_key.clear();
        append_bytes(rows.previous_key, record.key())?;
        *rows.previous_ordinal = Some(record.ordinal());
        rows.cursor.consume()
    }

    pub(in crate::execution) fn step(
        &mut self,
        inputs: [ConsumerInput<'_>; 2],
        output: &mut Batch,
        plan: &Pipeline,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<SetStep, Error> {
        output.clear();
        // An early error must not resume with partly advanced input cursors.
        let phase = std::mem::replace(&mut self.phase, Phase::Failed);
        match phase {
            Phase::Failed => return Err(Error::Corrupt("sorted set has failed")),
            Phase::Done => {
                self.phase = Phase::Done;
                return Ok(SetStep::Finished);
            }
            _ => (),
        }
        cancel.check()?;
        let mut step = SetStep::Progress;
        self.phase = match phase {
            Phase::Create(side) => {
                self.sides[side].files.create(cancel, effects)?;
                Phase::Read(side)
            }
            Phase::Read(side) => {
                step = SetStep::Input(side);
                Phase::Await(side)
            }
            Phase::Await(side) => {
                if inputs[side].finished {
                    if !inputs[side].batch.is_empty() {
                        return Err(Error::Corrupt("finished sorted set input contains rows"));
                    }
                    self.sides[side].sort.finish()?;
                    Phase::Sort(side)
                } else if inputs[side].batch.is_empty() {
                    return Err(Error::Corrupt("sorted set input was not supplied"));
                } else {
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
                        Some(&self.positions[side][..input.layout.count]),
                    )?;
                    Phase::Push(side, row)
                }
            }
            // Keep an unaccepted record while the full run spills. Retrying
            // Push must not advance either the batch row or its input ordinal.
            Phase::Push(side, row) => {
                let input = &mut self.sides[side];
                if input.sort.push(&input.record)? {
                    input.ordinal = input
                        .ordinal
                        .checked_add(1)
                        .ok_or(Error::Corrupt("sorted set input ordinal overflow"))?;
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
                    if side == 0 {
                        Phase::Create(1)
                    } else {
                        self.begin_read();
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
                } else if self.duplicate(0)?
                    && matches!(
                        self.kind,
                        SetKind::ExceptDistinct | SetKind::IntersectDistinct
                    )
                {
                    self.consume(0)?;
                    phase
                } else if self.sides[1].finished() {
                    if matches!(
                        self.kind,
                        SetKind::IntersectDistinct | SetKind::IntersectAll
                    ) {
                        Phase::Done
                    } else {
                        Phase::Emit
                    }
                } else {
                    self.duplicate(1)?;
                    match self.compare()? {
                        Ordering::Less
                            if matches!(
                                self.kind,
                                SetKind::ExceptDistinct | SetKind::ExceptAll
                            ) =>
                        {
                            Phase::Emit
                        }
                        Ordering::Less => {
                            self.consume(0)?;
                            phase
                        }
                        Ordering::Equal => match self.kind {
                            SetKind::ExceptDistinct => {
                                self.consume(0)?;
                                phase
                            }
                            SetKind::IntersectDistinct => Phase::Emit,
                            // ALL spends one right occurrence for each match.
                            // DISTINCT leaves it available for later left copies;
                            // duplicate() skips those copies before comparison.
                            SetKind::ExceptAll => {
                                self.consume(0)?;
                                self.consume(1)?;
                                phase
                            }
                            SetKind::IntersectAll => {
                                self.consume(1)?;
                                Phase::Emit
                            }
                            SetKind::UnionAll => {
                                return Err(Error::Corrupt("union in sorted set owner"));
                            }
                        },
                        Ordering::Greater => {
                            self.consume(1)?;
                            phase
                        }
                    }
                }
            }
            Phase::Emit => {
                let values = self.sides[0].values()?;
                let mut evaluated = RowValues::new(plan, |position: u8| {
                    values[..self.sides[0].layout.count]
                        .get(usize::from(position))
                        .copied()
                        .ok_or(Error::Corrupt("sorted set output position"))
                });
                if evaluated.retains()? {
                    for (position, column) in plan.columns[..plan.column_count].iter().enumerate() {
                        output.set(0, position, evaluated.value(*column)?)?;
                    }
                    output.publish_rows(1);
                    step = SetStep::Rows;
                }
                self.consume(0)?;
                Phase::Seek
            }
            Phase::Done | Phase::Failed => unreachable!("terminal phases handled before effects"),
        };
        Ok(step)
    }
}

#[cfg(test)]
#[path = "sorted_set_tests.rs"]
mod tests;
