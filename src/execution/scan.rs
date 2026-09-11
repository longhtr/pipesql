//! Shared scan scheduling, conditional demand, and batch publication.
//!
//! The cursor fills a supplied batch; the runtime owns that output and decides
//! when to advance or replay. Source modules own stored layouts and decoding.
use crate::batch::{Batch, OwnedBatch};
use crate::effects::Effects;
use crate::execution::computed::BatchScratch;
use crate::execution::planning::Pipeline;
use crate::execution::predicate::{BranchScratch, PhysicalFilter};
use crate::execution::{Advance, BATCH_ROWS, COMPUTE_ROWS};
use crate::frontend::MAX_COLUMNS;
use crate::resources::Reservation;
use crate::{CancellationToken, Error, Value};

pub(super) mod declared;
pub(super) mod legacy;

#[derive(Clone, Copy)]
enum ScanPhase {
    Begin,
    Filter(usize),
    Output(usize),
    Done,
}

pub(super) enum Source {
    Legacy(legacy::Scan),
    Declared(Vec<declared::Scan>),
}

impl Source {
    fn value(&self, column: u8, row: usize) -> Result<Value<'_>, Error> {
        match self {
            Self::Legacy(scan) => scan
                .column(column)
                .and_then(|value| value.value(row))
                .map_err(Error::Corrupt),
            Self::Declared(owner) => owner[0].value(column, row),
        }
    }
}

pub(super) struct AdmittedScan<'db> {
    pub(super) scan: ScanCursor<'db>,
    pub(super) input: OwnedBatch<'db>,
    pub(super) output: OwnedBatch<'db>,
}

pub(super) struct ScanCursor<'db> {
    source: Source,
    computation: BatchScratch,
    selection: Vec<u32>,
    branches: BranchScratch,
    start: usize,
    end: usize,
    phase: ScanPhase,
    // Physical owners drop before their account.
    reservation: Reservation<'db>,
}

#[cfg(test)]
impl AdmittedScan<'_> {
    pub(super) fn advance(
        &mut self,
        plan: &Pipeline,
        cancellation: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Advance, Error> {
        self.scan
            .advance(&mut self.input, plan, cancellation, effects)
    }

    pub(super) fn restart(&mut self, cancellation: &CancellationToken) -> Result<(), Error> {
        self.scan.restart(cancellation)?;
        self.input.clear();
        Ok(())
    }
}

impl ScanCursor<'_> {
    pub(super) fn memory_bytes(&self) -> u64 {
        self.reservation.bytes()
    }

    #[cfg(test)]
    pub(super) fn source_mut(&mut self) -> &mut Source {
        &mut self.source
    }

    pub(super) fn restart(&mut self, cancel: &CancellationToken) -> Result<(), Error> {
        let Source::Declared(scan) = &mut self.source else {
            return Err(Error::Corrupt("source has no replay transition"));
        };
        scan[0].restart_once(cancel)?;
        self.selection.clear();
        self.computation.invalidate();
        self.start = 0;
        self.end = 0;
        self.phase = ScanPhase::Begin;
        Ok(())
    }

    fn block_rows(&self) -> Result<usize, &'static str> {
        match &self.source {
            Source::Legacy(scan) => scan.block_rows(),
            Source::Declared(scan) => Ok(scan[0].rows()),
        }
    }

    fn load_column(
        &mut self,
        column: u8,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<bool, Error> {
        match &mut self.source {
            Source::Legacy(scan) => scan.load_column(column, cancel, effects),
            Source::Declared(scan) => scan[0].load(column, cancel, effects),
        }
    }

    fn finish_block(&mut self) -> Result<(), Error> {
        match &mut self.source {
            Source::Legacy(scan) => scan.finish_block()?,
            Source::Declared(scan) => scan[0].finish_unit(),
        }
        Ok(())
    }

    fn source_finished(&self) -> bool {
        match &self.source {
            Source::Legacy(scan) => scan.is_finished(),
            Source::Declared(_) => false,
        }
    }

    fn filter_source(
        &mut self,
        selection: &mut Vec<u32>,
        filter: PhysicalFilter<'_>,
        plan: &Pipeline,
    ) -> Result<(), Error> {
        if usize::from(filter.column) >= MAX_COLUMNS {
            let source = &self.source;
            self.computation
                .evaluate(plan, &[filter.column], selection, |column, row| {
                    source.value(column, row)
                })?;
            let mut retained = 0;
            for index in 0..selection.len() {
                if filter.matches(self.computation.value(plan, filter.column, index)?)? {
                    selection[retained] = selection[index];
                    retained += 1;
                }
            }
            selection.truncate(retained);
            return Ok(());
        }
        match &self.source {
            Source::Legacy(scan) => scan
                .column(filter.column)
                .and_then(|column| column.filter(selection, filter))
                .map_err(Error::Corrupt),
            Source::Declared(owner) => {
                let scan = &owner[0];
                let mut retained = 0;
                for index in 0..selection.len() {
                    let row = selection[index];
                    if filter.matches(scan.value(filter.column, row as usize)?)? {
                        selection[retained] = row;
                        retained += 1;
                    }
                }
                selection.truncate(retained);
                Ok(())
            }
        }
    }

    fn filter_branch(
        &mut self,
        index: usize,
        plan: &Pipeline<'_>,
        cancellation: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Advance, Error> {
        let filter = plan.filters[index];
        self.branches.selected.clear();
        for (&row, &next) in self.selection.iter().zip(&self.branches.next) {
            if usize::from(next) == index {
                self.branches.selected.push(row);
            }
        }
        if !self.branches.selected.is_empty() {
            let raw = plan.raw_columns(&[filter.column])?;
            for column in 0..MAX_COLUMNS {
                if raw & (1 << column) != 0
                    && self.load_column(column as u8, cancellation, effects)?
                {
                    return Ok(Advance::Progress);
                }
            }
            let mut selected = std::mem::take(&mut self.branches.selected);
            let result = self.filter_source(&mut selected, filter, plan);
            self.branches.selected = selected;
            result?;
            let mut matched = 0;
            for (&row, next) in self.selection.iter().zip(&mut self.branches.next) {
                if usize::from(*next) != index {
                    continue;
                }
                let keep = self.branches.selected.get(matched) == Some(&row);
                if keep {
                    matched += 1;
                }
                let offset = if keep {
                    filter.control.matched
                } else {
                    filter.control.other
                };
                *next = if offset == 0 {
                    u8::MAX
                } else {
                    let target = index + usize::from(offset);
                    if target > plan.filter_count {
                        return Err(Error::Corrupt("scan branch outside pipeline"));
                    }
                    target as u8
                };
            }
            assert_eq!(
                matched,
                self.branches.selected.len(),
                "branch selection preserves row order"
            );
        }
        if index + 1 == plan.filter_count {
            let mut retained = 0;
            for position in 0..self.selection.len() {
                if usize::from(self.branches.next[position]) == plan.filter_count {
                    self.selection[retained] = self.selection[position];
                    retained += 1;
                }
            }
            self.selection.truncate(retained);
            if retained == 0 {
                self.start = self.end;
                self.phase = ScanPhase::Begin;
            } else {
                self.phase = ScanPhase::Output(0);
            }
        } else {
            self.phase = ScanPhase::Filter(index + 1);
        }
        Ok(Advance::Progress)
    }

    fn decode_source(
        &mut self,
        plan: &Pipeline,
        start: usize,
        end: usize,
        input: &mut Batch,
    ) -> Result<usize, Error> {
        if plan.has_computed_outputs() {
            let source = &self.source;
            self.computation.evaluate(
                plan,
                &plan.columns[..plan.column_count],
                &self.selection[start..end],
                |column, row| source.value(column, row),
            )?;
            for index in start..end {
                for (position, &column) in plan.columns[..plan.column_count].iter().enumerate() {
                    let value = if usize::from(column) < MAX_COLUMNS {
                        source.value(column, self.selection[index] as usize)?
                    } else {
                        self.computation.value(plan, column, index - start)?
                    };
                    match input.set(index - start, position, value) {
                        Err(Error::Resource {
                            owner: "batch UTF-8 bytes",
                            ..
                        }) if index > start => return Ok(index),
                        result => result?,
                    }
                }
            }
            return Ok(end);
        }
        match &self.source {
            Source::Legacy(scan) => {
                for (position, column) in plan.columns[..plan.column_count].iter().enumerate() {
                    scan.column(*column)
                        .map_err(Error::Corrupt)?
                        .decode(
                            &self.selection[start..end],
                            input
                                .nonnull_column(position, end - start)
                                .map_err(Error::Corrupt)?,
                        )
                        .map_err(Error::Corrupt)?;
                }
            }
            Source::Declared(owner) => {
                let scan = &owner[0];
                for index in start..end {
                    for (position, column) in plan.columns[..plan.column_count].iter().enumerate() {
                        let value = scan.value(*column, self.selection[index] as usize)?;
                        match input.set(index - start, position, value) {
                            // Publish only the completed prefix. The next step
                            // clears partial bytes and retries this same row.
                            Err(Error::Resource {
                                owner: "batch UTF-8 bytes",
                                ..
                            }) if index > start => return Ok(index),
                            result => result?,
                        }
                    }
                }
            }
        }
        Ok(end)
    }

    pub(super) fn advance(
        &mut self,
        input: &mut Batch,
        plan: &Pipeline,
        cancellation: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Advance, Error> {
        cancellation.check()?;
        input.clear();
        match self.phase {
            ScanPhase::Begin => {
                if let Source::Declared(scan) = &mut self.source
                    && !scan[0].has_unit()
                {
                    return Ok(if scan[0].next_unit(cancellation, effects)? {
                        Advance::Progress
                    } else {
                        self.phase = ScanPhase::Done;
                        Advance::Finished
                    });
                }
                if self.source_finished() {
                    self.phase = ScanPhase::Done;
                    return Ok(Advance::Finished);
                }
                let rows = self.block_rows().map_err(Error::Corrupt)?;
                if self.start == rows {
                    self.finish_block()?;
                    self.start = 0;
                    return Ok(Advance::Progress);
                }
                self.end = self
                    .start
                    .checked_add(if !plan.has_computed_work() {
                        COMPUTE_ROWS
                    } else {
                        BATCH_ROWS
                    })
                    .ok_or(Error::Corrupt("quantum extent overflow"))?
                    .min(rows);
                self.selection.clear();
                for row in self.start..self.end {
                    self.selection
                        .push(u32::try_from(row).expect("block row bound"));
                }
                if self.branches.next.capacity() != 0 {
                    assert!(
                        self.selection.len() <= self.branches.next.capacity(),
                        "branch cursors cover the scan quantum"
                    );
                    self.branches.next.clear();
                    self.branches.next.resize(self.selection.len(), 0);
                }
                self.phase = if plan.filter_count == 0 {
                    ScanPhase::Output(0)
                } else {
                    ScanPhase::Filter(0)
                };
                Ok(Advance::Progress)
            }
            ScanPhase::Filter(index) => {
                if self.branches.next.capacity() != 0 {
                    return self.filter_branch(index, plan, cancellation, effects);
                }
                let filter = plan.filters[index];
                let raw = plan.raw_columns(&[filter.column])?;
                for column in 0..MAX_COLUMNS {
                    if raw & (1 << column) != 0
                        && self.load_column(column as u8, cancellation, effects)?
                    {
                        return Ok(Advance::Progress);
                    }
                }
                let mut selection = std::mem::take(&mut self.selection);
                let filtered = self.filter_source(&mut selection, filter, plan);
                self.selection = selection;
                filtered?;
                let retained = self.selection.len();
                if retained == 0 {
                    self.start = self.end;
                    self.phase = ScanPhase::Begin;
                } else if index + 1 == plan.filter_count {
                    self.phase = ScanPhase::Output(0);
                } else {
                    self.phase = ScanPhase::Filter(index + 1);
                }
                Ok(Advance::Progress)
            }
            ScanPhase::Output(start) => {
                // A call reads at most one checked block, or emits one batch.
                let raw = plan.raw_columns(&plan.columns[..plan.column_count])?;
                for column in 0..MAX_COLUMNS {
                    if raw & (1 << column) != 0
                        && self.load_column(column as u8, cancellation, effects)?
                    {
                        return Ok(Advance::Progress);
                    }
                }
                let end = start
                    .checked_add(BATCH_ROWS)
                    .ok_or(Error::Corrupt("batch extent overflow"))?
                    .min(self.selection.len());
                let end = self.decode_source(plan, start, end, input)?;
                input.publish_rows(end - start);
                if end == self.selection.len() {
                    self.start = self.end;
                    self.phase = ScanPhase::Begin;
                } else {
                    self.phase = ScanPhase::Output(end);
                }
                assert!(!input.is_empty(), "output phase requires selected rows");
                Ok(Advance::Rows)
            }
            ScanPhase::Done => Ok(Advance::Finished),
        }
    }
}
