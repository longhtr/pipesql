//! One private controller from source batches through validated grouped results.
mod hash;
mod reduction;
use crate::batch::Batch;
use crate::effects::Effects;
use crate::execution::aggregation::arguments::ArgumentBatch;
use crate::execution::aggregation::numeric::{AggregateLayout, AggregateState};
use crate::execution::blocking::{
    ArgumentShape, Files, IO_BYTES, KeyColumn, MAX_FRAME_BYTES, MAX_KEY_BYTES, MAX_KEYS,
    RECORD_HEADER, ReadAt, RecordSpan, RowLayout, RowSort, SortPhase, SortRecord, append_bytes,
    append_value, read_value,
};
use crate::execution::computed::RowValues;
use crate::execution::planning::Pipeline;
use crate::execution::{BATCH_ROWS, COMPUTE_ROWS, ConsumerInput, ConsumerStep, MAX_AGGREGATE_ROWS};

#[cfg(test)]
use crate::frontend::AggregateKind;
use crate::frontend::{AggregatePlan, DataType, MAX_COLUMNS, SemanticColumn, SourceColumn};
use crate::frontend::{Direction, NullPlacement};
use crate::resources::{Reservation, allocate};
use crate::storage_format;
use crate::{CancellationToken, Database, Error, Value};
use hash::{HashStep, MemoryGroups};
use reduction::Reduction;
use std::mem::size_of;

// Resolve semantic grouping keys before constructing the shared row codec.
pub(super) fn key_layout(
    plan: &AggregatePlan,
    inputs: &[SemanticColumn],
) -> Result<RowLayout, Error> {
    let mut columns = [KeyColumn {
        input: 0,
        kind: DataType::Int64,
        nullable: false,
        direction: Direction::Ascending,
        nulls: NullPlacement::First,
    }; MAX_COLUMNS];
    let count = usize::from(plan.group_count);
    if count == 0 || count > MAX_KEYS {
        return Err(Error::Corrupt("general group key count"));
    }
    let mut max_bytes = 0_usize;
    let mut layout = [0_u8; MAX_KEYS * 2 + 1];
    layout[0] = plan.group_count;
    for (index, source) in plan.group_columns().enumerate() {
        let input = inputs
            .iter()
            .position(|input| *input == source)
            .ok_or(Error::Corrupt("group input is not scanned"))?;
        let kind = source.data_type();
        let nullable = source.nullable();
        columns[index] = KeyColumn {
            input,
            kind,
            nullable,
            direction: Direction::Ascending,
            nulls: NullPlacement::First,
        };
        let (tag, width) = match kind {
            DataType::Int64 => (1, 9),
            DataType::Double => (2, 9),
            DataType::Date => (3, 5),
            DataType::String => (4, 5 + crate::batch::MAX_TEXT_BYTES),
        };
        layout[1 + index * 2] = tag;
        layout[2 + index * 2] = u8::from(nullable);
        max_bytes = max_bytes
            .checked_add(width)
            .ok_or(Error::Corrupt("group key byte bound"))?;
    }
    assert!(max_bytes <= MAX_KEY_BYTES);
    Ok(RowLayout {
        columns,
        count,
        key_count: count,
        max_bytes,
        key_max_bytes: max_bytes,
        layout: storage_format::crc32c(&layout),
    })
}

const RESULT_MAGIC: &[u8; 8] = b"PGOUT001";

struct Limits {
    arguments: usize,
    run_bytes: usize,
    run_rows: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Create,
    Read,
    Await,
    Capture(usize),
    Hash,
    Encode(usize),
    Push(usize),
    Spill(usize),
    Sort,
    Reduce,
    Flush,
    Order,
    Check(usize),
    EmitMemory(usize),
    EmitDisk,
    Done,
    Failed,
}

struct OutputLayout {
    columns: [(DataType, bool); MAX_COLUMNS],
    count: usize,
    max_bytes: usize,
    tag: u32,
}

impl OutputLayout {
    #[cfg(test)]
    fn new(
        plan: &Pipeline,
        keys: &RowLayout,
        aggregate: &AggregateState<'_>,
    ) -> Result<Self, Error> {
        let count = plan.column_count;
        if !(1..=MAX_COLUMNS).contains(&count) {
            return Err(Error::Corrupt("group result width"));
        }
        let mut columns = [(DataType::Int64, false); MAX_COLUMNS];
        for (index, output) in columns[..count].iter_mut().enumerate() {
            let column = usize::from(plan.columns[index]);
            *output = if column >= MAX_COLUMNS {
                let definition = plan
                    .computed
                    .get(column - MAX_COLUMNS)
                    .ok_or(Error::Corrupt("computed group result slot"))?;
                (definition.column.data_type(), definition.column.nullable())
            } else if column < keys.count {
                (keys.columns[column].kind, keys.columns[column].nullable)
            } else {
                let entry = column - keys.count;
                if entry >= aggregate.plan.entries.len() || aggregate.demand & (1 << entry) == 0 {
                    return Err(Error::Corrupt("group result is not demanded"));
                }
                match aggregate.plan.entries[entry].kind {
                    AggregateKind::Count => (DataType::Int64, false),
                    AggregateKind::Avg => (DataType::Double, true),
                    AggregateKind::Sum => (
                        aggregate.inputs[aggregate.entry_states[entry]]
                            .ok_or(Error::Corrupt("group result argument absent"))?
                            .data_type,
                        true,
                    ),
                }
            };
        }
        Self::from_columns(columns, count)
    }

    fn from_columns(columns: [(DataType, bool); MAX_COLUMNS], count: usize) -> Result<Self, Error> {
        if !(1..=MAX_COLUMNS).contains(&count) {
            return Err(Error::Corrupt("group result width"));
        }
        let mut bytes = RECORD_HEADER;
        let mut descriptor = [0_u8; 1 + MAX_COLUMNS * 2];
        descriptor[0] = count as u8;
        for (index, output) in columns[..count].iter().enumerate() {
            let (tag, width) = match output.0 {
                DataType::Int64 => (1, 9),
                DataType::Double => (2, 9),
                DataType::Date => (3, 5),
                DataType::String => (4, 5 + crate::batch::MAX_TEXT_BYTES),
            };
            descriptor[1 + 2 * index] = tag;
            descriptor[2 + 2 * index] = u8::from(output.1);
            bytes = bytes
                .checked_add(width)
                .ok_or(Error::Corrupt("group result byte bound"))?;
        }
        assert!(bytes <= MAX_FRAME_BYTES);
        Ok(Self {
            columns,
            count,
            max_bytes: bytes,
            tag: storage_format::crc32c(&descriptor),
        })
    }

    fn check_payload(&self, bytes: &[u8]) -> Result<(), Error> {
        let mut rest = bytes;
        for &(kind, nullable) in &self.columns[..self.count] {
            read_value(&mut rest, kind, nullable)?;
        }
        if !rest.is_empty() {
            return Err(Error::Corrupt("group result trailing bytes"));
        }
        Ok(())
    }
}

struct Minimum<'db> {
    arguments: ArgumentBatch<'db>,
    sort: RowSort<'db>,
    record: SortRecord,
    files: Files<'db>,
    keys: RowLayout,
    output: OutputLayout,
    reservation: Reservation<'db>,
}

impl<'db> Minimum<'db> {
    fn new(
        database: &'db Database,
        keys: RowLayout,
        output: OutputLayout,
        shape: ArgumentShape,
        limits: &Limits,
    ) -> Result<Self, Error> {
        let frame_bytes = output
            .max_bytes
            .max(RECORD_HEADER + keys.max_bytes + shape.count * 8);
        // AggregateState charges its heap arrays only; this controller owns and
        // charges its inline fields. ArgumentBatch and RowSort include theirs.
        let inline =
            size_of::<General<'_>>() - size_of::<ArgumentBatch<'_>>() - size_of::<RowSort<'_>>();
        let bytes = inline
            .checked_add(frame_bytes)
            .ok_or(Error::Corrupt("group controller bytes"))? as u64;
        let reservation = database.reserve_memory(bytes, "group query controller")?;
        let record = SortRecord::new(frame_bytes, bytes)?;
        let files = Files::Pending(crate::scratch::Creation::reserve(database)?);
        let arguments = ArgumentBatch::new(database, shape, limits.arguments)?;
        let sort = RowSort::new(database, &keys, shape, limits.run_bytes, limits.run_rows)?;
        Ok(Self {
            arguments,
            sort,
            record,
            files,
            keys,
            output,
            reservation,
        })
    }

    fn required(
        keys: &RowLayout,
        output: &OutputLayout,
        shape: ArgumentShape,
    ) -> Result<u64, Error> {
        let record = RECORD_HEADER + keys.max_bytes + shape.count * 8;
        // One captured argument, one maximum run record, two spans, two merge
        // records, three I/O buffers, a prior key, and the reusable final frame.
        [
            size_of::<General<'_>>(),
            record.max(output.max_bytes),
            shape.count * 8,
            record,
            2 * size_of::<RecordSpan>(),
            2 * record,
            3 * IO_BYTES,
            keys.max_bytes,
            crate::scratch::Creation::memory_requirement_bytes() as usize,
        ]
        .into_iter()
        .try_fold(0_usize, usize::checked_add)
        .and_then(|bytes| u64::try_from(bytes).ok())
        .ok_or(Error::Corrupt("group minimum memory"))
    }
}

pub(in crate::execution) struct General<'db> {
    // One optional heap owner keeps its complete charge separate from the
    // controller and fallback. Drop the physical owner before its reservation.
    memory: Vec<MemoryGroups<'db>>,
    aggregate: AggregateState<'db>,
    arguments: ArgumentBatch<'db>,
    sort: RowSort<'db>,
    record: SortRecord,
    files: Files<'db>,
    keys: RowLayout,
    output: OutputLayout,
    phase: Phase,
    replayed: bool,
    reduction: Reduction,
    ordered: bool,
    ordinal: u64,
    result_rows: u64,
    result_end: u64,
    result_cursor: u64,
    emitted: u64,
    reservation: Reservation<'db>,
}

struct Admission<'db> {
    keys: RowLayout,
    layout: AggregateLayout<'db>,
    shape: ArgumentShape,
    output: OutputLayout,
    bytes: u64,
}

impl<'db> General<'db> {
    fn admission(
        semantic: &'db AggregatePlan,
        demand: u16,
        input_columns: impl Iterator<Item = SemanticColumn>,
        output_columns: impl Iterator<Item = SemanticColumn>,
    ) -> Result<Admission<'db>, Error> {
        let mut inputs = [SourceColumn::QUANTITY.semantic(); MAX_COLUMNS];
        let mut count = 0;
        for column in input_columns {
            if count == MAX_COLUMNS {
                return Err(Error::Corrupt("group input width"));
            }
            inputs[count] = column;
            count += 1;
        }
        let keys = key_layout(semantic, &inputs[..count])?;
        let layout = AggregateLayout::new(semantic, demand, inputs[..count].iter().copied());
        let shape = ArgumentShape::from_inputs(&layout.inputs[..layout.states]);
        let mut columns = [(DataType::Int64, false); MAX_COLUMNS];
        let mut output_count = 0;
        for column in output_columns {
            if output_count == MAX_COLUMNS {
                return Err(Error::Corrupt("group result width"));
            }
            columns[output_count] = (column.data_type(), column.nullable());
            output_count += 1;
        }
        let output = OutputLayout::from_columns(columns, output_count)?;
        let bytes = Minimum::required(&keys, &output, shape)?
            .checked_add(layout.required_bytes(1, 1)?)
            .ok_or(Error::Corrupt("complete grouping minimum"))?;
        Ok(Admission {
            keys,
            layout,
            shape,
            output,
            bytes,
        })
    }

    pub(in crate::execution) fn minimum_bytes(
        semantic: &'db AggregatePlan,
        demand: u16,
        input: impl Iterator<Item = SemanticColumn>,
        output: impl Iterator<Item = SemanticColumn>,
    ) -> Result<u64, Error> {
        Ok(Self::admission(semantic, demand, input, output)?.bytes)
    }

    pub(in crate::execution) fn open(
        database: &'db Database,
        semantic: &'db AggregatePlan,
        demand: u16,
        input_columns: impl Iterator<Item = SemanticColumn> + Clone,
        output_columns: impl Iterator<Item = SemanticColumn>,
    ) -> Result<Vec<Self>, Error> {
        let Admission {
            keys,
            layout,
            shape,
            output,
            bytes: minimum_bytes,
        } = Self::admission(semantic, demand, input_columns.clone(), output_columns)?;
        let available = database
            .config()
            .memory_limit_bytes()
            .checked_sub(database.reserved_memory_bytes())
            .ok_or(Error::Corrupt("memory account exceeds configured limit"))?;
        let mut extra = available
            .checked_sub(minimum_bytes)
            .ok_or(Error::Resource {
                owner: "group query minimum",
                required: minimum_bytes,
                limit: available,
            })?;
        let argument_row_bytes = (shape.count * size_of::<u64>()) as u64;
        let arguments = if let Some(affordable) = extra.checked_div(argument_row_bytes) {
            let additional = affordable.min((BATCH_ROWS - 1) as u64);
            extra -= additional * argument_row_bytes;
            1 + additional as usize
        } else {
            BATCH_ROWS
        };
        // Reserve a maximum-width first record. Additional row slots and bytes
        // grow together, with at most 4,096 rows in one initial run.
        let shortest_record = RECORD_HEADER + keys.count + shape.count * 8;
        let extra_rows = (extra / (shortest_record + 2 * size_of::<RecordSpan>()) as u64)
            .min((COMPUTE_ROWS - 1) as u64) as usize;
        let limits = Limits {
            arguments,
            run_rows: 1 + extra_rows,
            run_bytes: RECORD_HEADER
                + keys.max_bytes
                + shape.count * 8
                + extra_rows * shortest_record,
        };
        let minimum = Minimum::new(database, keys, output, shape, &limits)?;
        let aggregate = AggregateState::from_layout(&database.memory, layout, 1)?;
        aggregate.validate_plan(semantic, demand, 1, input_columns)?;
        let general = Self::assemble(database, aggregate, minimum, semantic.ordered, None)?;
        let mut owner = allocate(1, 1, "group query owner", general.reservation.bytes())?;
        owner.push(general);
        Ok(owner)
    }

    #[cfg(test)]
    fn new(
        database: &'db Database,
        aggregate: AggregateState<'db>,
        keys: RowLayout,
        plan: &Pipeline,
        ordered: bool,
        limits: Limits,
        hash: (usize, usize),
    ) -> Result<Self, Error> {
        let output = OutputLayout::new(plan, &keys, &aggregate)?;
        let shape = ArgumentShape::from_aggregate(&aggregate);
        let minimum = Minimum::new(database, keys, output, shape, &limits)?;
        Self::assemble(database, aggregate, minimum, ordered, Some(hash))
    }

    fn assemble(
        database: &'db Database,
        aggregate: AggregateState<'db>,
        minimum: Minimum<'db>,
        ordered: bool,
        hash: Option<(usize, usize)>,
    ) -> Result<Self, Error> {
        let Minimum {
            arguments,
            sort,
            record,
            files,
            keys,
            output,
            reservation,
        } = minimum;
        let (hash_groups, hash_bytes) = match hash {
            Some(limits) => limits,
            None => MemoryGroups::capacities(database, &aggregate, &keys)?,
        };
        // Every fallback allocation above survives optional admission failure.
        let mut memory = Vec::new();
        if hash_groups != 0 {
            match MemoryGroups::new(database, &aggregate, &keys, hash_groups, hash_bytes) {
                Ok(groups) => {
                    memory = allocate(1, 1, "hash group owner", groups.memory_bytes())?;
                    memory.push(groups);
                }
                Err(Error::Resource { .. }) => (),
                Err(error) => return Err(error),
            }
        }
        let phase = if memory.is_empty() {
            Phase::Create
        } else {
            Phase::Read
        };
        Ok(Self {
            memory,
            aggregate,
            arguments,
            sort,
            record,
            files,
            keys,
            output,
            phase,
            replayed: false,
            reduction: Reduction::Start,
            ordered,
            ordinal: 0,
            result_rows: 0,
            result_end: 0,
            result_cursor: 0,
            emitted: 0,
            reservation,
        })
    }

    pub(in crate::execution) fn memory_bytes(&self) -> u64 {
        self.reservation.bytes()
            + self.aggregate.reservation.bytes()
            + self.arguments.reservation.bytes()
            + self.sort.memory_bytes()
            + self.memory.first().map_or(0, MemoryGroups::memory_bytes)
            + if matches!(self.files, Files::Pending(_)) {
                crate::scratch::Creation::memory_requirement_bytes()
            } else {
                0
            }
    }

    fn value(&self, group: usize, column: usize) -> Result<Value<'_>, Error> {
        if let Some(memory) = self.memory.first() {
            if column < self.keys.count {
                memory.key_value(group, column, &self.keys)
            } else {
                self.aggregate.value(0, column - self.keys.count)
            }
        } else {
            self.reduction
                .value(&self.sort, &self.aggregate, &self.keys, column)
        }
    }

    fn retains(&self, group: usize, plan: &Pipeline) -> Result<bool, Error> {
        RowValues::new(plan, |column| self.value(group, usize::from(column))).retains()
    }

    fn check_result(&self, group: usize, plan: &Pipeline) -> Result<bool, Error> {
        if !self.retains(group, plan)? {
            return Ok(false);
        }
        let mut values = RowValues::new(plan, |column| self.value(group, usize::from(column)));
        for &column in &plan.columns[..self.output.count] {
            values.value(column)?;
        }
        Ok(true)
    }

    fn encode_result(&mut self, plan: &Pipeline) -> Result<(), Error> {
        let mut bytes = std::mem::take(&mut self.record.bytes);
        let encoded = (|| {
            bytes.clear();
            append_bytes(&mut bytes, &[0; RECORD_HEADER])?;
            let mut values = RowValues::new(plan, |column| self.value(0, usize::from(column)));
            for (index, &column) in plan.columns[..self.output.count].iter().enumerate() {
                let (kind, nullable) = self.output.columns[index];
                append_value(&mut bytes, values.value(column)?, kind, nullable)?;
            }
            bytes[..8].copy_from_slice(RESULT_MAGIC);
            bytes[8..16].copy_from_slice(&self.result_rows.to_le_bytes());
            let length = u32::try_from(bytes.len()).expect("bounded result frame");
            bytes[16..20].copy_from_slice(&length.to_le_bytes());
            bytes[20..24].copy_from_slice(&self.output.tag.to_le_bytes());
            let crc = storage_format::crc32c(&bytes);
            bytes[28..32].copy_from_slice(&crc.to_le_bytes());
            Ok(())
        })();
        self.record.bytes = bytes;
        encoded
    }

    fn read_result(
        &mut self,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        let slot = self.sort.spool_slot();
        let at = ReadAt {
            slot,
            offset: self.result_cursor,
            limit: self.result_end,
        };
        let mut io = self.files.io(cancel, effects)?;
        let bytes = &mut self.record.bytes;
        bytes.resize(RECORD_HEADER, 0);
        let reader = self.sort.spool_reader();
        reader.read(at, bytes, &mut io)?;
        let length = u32::from_le_bytes(bytes[16..20].try_into().expect("result length")) as usize;
        if &bytes[..8] != RESULT_MAGIC
            || u64::from_le_bytes(bytes[8..16].try_into().expect("result ordinal")) != self.emitted
            || u32::from_le_bytes(bytes[20..24].try_into().expect("result schema"))
                != self.output.tag
            || bytes[24..28] != [0; 4]
            || !(RECORD_HEADER..=self.output.max_bytes).contains(&length)
        {
            return Err(Error::Corrupt("group result header"));
        }
        assert!(
            length <= bytes.capacity(),
            "final row admitted before input"
        );
        bytes.resize(length, 0);
        let offset = self
            .result_cursor
            .checked_add(RECORD_HEADER as u64)
            .ok_or(Error::Corrupt("group result payload offset"))?;
        reader.read(
            ReadAt { offset, ..at },
            &mut bytes[RECORD_HEADER..],
            &mut io,
        )?;
        let expected = u32::from_le_bytes(bytes[28..32].try_into().expect("result checksum"));
        bytes[28..32].fill(0);
        let actual = storage_format::crc32c(bytes);
        bytes[28..32].copy_from_slice(&expected.to_le_bytes());
        if expected != actual {
            return Err(Error::Corrupt("group result checksum"));
        }
        self.output.check_payload(&bytes[RECORD_HEADER..])?;
        Ok(())
    }

    pub(in crate::execution) fn replay(&mut self, cancel: &CancellationToken) -> Result<(), Error> {
        let phase = std::mem::replace(&mut self.phase, Phase::Failed);
        cancel.check()?;
        if self.replayed || !matches!(phase, Phase::EmitMemory(_) | Phase::EmitDisk | Phase::Done) {
            return Err(Error::Corrupt(
                "group replay requires validated retained output",
            ));
        }
        self.replayed = true;
        self.phase = if self.memory.is_empty() {
            // Drop cached bytes so replay validates the retained file again.
            self.sort.spool_reader().clear();
            self.record.bytes.clear();
            self.result_cursor = 0;
            self.emitted = 0;
            Phase::EmitDisk
        } else {
            Phase::EmitMemory(0)
        };
        Ok(())
    }

    pub(in crate::execution) fn step(
        &mut self,
        input: ConsumerInput<'_>,
        output: &mut Batch,
        plan: &Pipeline,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<ConsumerStep, Error> {
        let phase = std::mem::replace(&mut self.phase, Phase::Failed);
        output.clear();
        if phase == Phase::Failed {
            return Err(Error::Corrupt("group query has failed"));
        }
        if phase == Phase::Done {
            self.phase = phase;
            return Ok(ConsumerStep::Finished);
        }
        cancel.check()?;
        let mut advance = ConsumerStep::Progress;
        self.phase = match phase {
            Phase::Create => {
                self.files.create(cancel, effects)?;
                Phase::Read
            }
            Phase::Read => {
                advance = ConsumerStep::Input;
                Phase::Await
            }
            Phase::Await => {
                if input.finished {
                    if !input.batch.is_empty() {
                        return Err(Error::Corrupt("finished grouping input contains rows"));
                    }
                    if let Some(memory) = self.memory.first_mut() {
                        if self.ordered {
                            memory.begin_order()?;
                            Phase::Order
                        } else {
                            Phase::Check(0)
                        }
                    } else {
                        self.sort.finish()?;
                        Phase::Sort
                    }
                } else {
                    if input.batch.is_empty() {
                        return Err(Error::Corrupt("grouping input was not supplied"));
                    }
                    Phase::Capture(0)
                }
            }
            Phase::Capture(start) => {
                if start == input.batch.len() {
                    Phase::Read
                } else {
                    let end = (start + self.arguments.capacity).min(input.batch.len());
                    self.arguments.evaluate(
                        &mut self.aggregate,
                        input.batch,
                        start..end,
                        cancel,
                    )?;
                    if let Some(memory) = self.memory.first_mut() {
                        memory.begin(&self.arguments)?;
                        Phase::Hash
                    } else {
                        Phase::Encode(0)
                    }
                }
            }
            Phase::Hash => {
                match self.memory[0].step(input.batch, &self.arguments, &self.keys, cancel)? {
                    HashStep::Progress => Phase::Hash,
                    HashStep::Complete => Phase::Capture(
                        self.arguments.source_start.expect("captured source") + self.arguments.rows,
                    ),
                    HashStep::Fallback(_) => {
                        drop(std::mem::take(&mut self.memory));
                        advance = ConsumerStep::Replay;
                        self.arguments.clear();
                        Phase::Create
                    }
                }
            }
            Phase::Encode(row) => {
                if row == self.arguments.rows {
                    Phase::Capture(self.arguments.source_start.expect("captured source") + row)
                } else {
                    self.arguments.encode_record(
                        &mut self.record,
                        &self.keys,
                        input.batch,
                        row,
                        self.ordinal,
                    )?;
                    Phase::Push(row)
                }
            }
            Phase::Push(row) => {
                if self.sort.push(&self.record)? {
                    self.ordinal += 1; // RowSort checks the total bound before publication.
                    Phase::Encode(row + 1)
                } else {
                    Phase::Spill(row)
                }
            }
            Phase::Spill(row) => {
                let phase = self
                    .sort
                    .step(&self.keys, &mut self.files.io(cancel, effects)?)?;
                if phase == SortPhase::Collect {
                    Phase::Push(row)
                } else {
                    Phase::Spill(row)
                }
            }
            Phase::Sort => {
                if self
                    .sort
                    .step(&self.keys, &mut self.files.io(cancel, effects)?)?
                    == SortPhase::Done
                {
                    self.sort.spool_writer().begin_file();
                    Phase::Reduce
                } else {
                    Phase::Sort
                }
            }
            Phase::Reduce => {
                match self.reduction.step(
                    &mut self.sort,
                    &mut self.arguments,
                    &mut self.aggregate,
                    &self.keys,
                    &mut self.files.io(cancel, effects)?,
                )? {
                    Reduction::Group => {
                        if self.retains(0, plan)? {
                            let next_rows = self
                                .result_rows
                                .checked_add(1)
                                .filter(|rows| *rows <= MAX_AGGREGATE_ROWS)
                                .ok_or(Error::Corrupt("group result row bound"))?;
                            self.encode_result(plan)?;
                            let slot = self.sort.spool_slot();
                            self.sort.spool_writer().append(
                                slot,
                                &self.record.bytes,
                                &mut self.files.io(cancel, effects)?,
                            )?;
                            self.result_rows = next_rows;
                        }
                        Phase::Reduce
                    }
                    Reduction::Done => Phase::Flush,
                    _ => Phase::Reduce,
                }
            }
            Phase::Flush => {
                let slot = self.sort.spool_slot();
                self.sort
                    .spool_writer()
                    .flush(slot, &mut self.files.io(cancel, effects)?)?;
                self.result_end = self.sort.spool_writer().position()?;
                self.sort.spool_reader().clear();
                Phase::EmitDisk
            }
            Phase::Order => {
                if self.memory[0].order_step(&self.keys, cancel)? {
                    Phase::Check(0)
                } else {
                    Phase::Order
                }
            }
            Phase::Check(group) => {
                if group == self.memory[0].len() {
                    Phase::EmitMemory(0)
                } else {
                    self.memory[0].load_group(group, &mut self.aggregate)?;
                    self.check_result(group, plan)?;
                    Phase::Check(group + 1)
                }
            }
            Phase::EmitMemory(index) => {
                if index == self.memory[0].len() {
                    Phase::Done
                } else {
                    let group = if self.ordered {
                        self.memory[0].ordered_group(index)?
                    } else {
                        index
                    };
                    self.memory[0].load_group(group, &mut self.aggregate)?;
                    if self.retains(group, plan)? {
                        let mut values =
                            RowValues::new(plan, |column| self.value(group, usize::from(column)));
                        for (position, &column) in
                            plan.columns[..self.output.count].iter().enumerate()
                        {
                            output.set(0, position, values.value(column)?)?;
                        }
                        output.publish_rows(1);
                        advance = ConsumerStep::Rows;
                    }
                    Phase::EmitMemory(index + 1)
                }
            }
            Phase::EmitDisk => {
                if self.emitted == self.result_rows {
                    if self.result_cursor != self.result_end {
                        return Err(Error::Corrupt("group result extent differs"));
                    }
                    Phase::Done
                } else {
                    self.read_result(cancel, effects)?;
                    let mut payload = &self.record.bytes[RECORD_HEADER..];
                    for (index, &(kind, nullable)) in
                        self.output.columns[..self.output.count].iter().enumerate()
                    {
                        output.set(0, index, read_value(&mut payload, kind, nullable)?)?;
                    }
                    self.result_cursor = self
                        .result_cursor
                        .checked_add(self.record.bytes.len() as u64)
                        .ok_or(Error::Corrupt("group result read cursor"))?;
                    self.emitted += 1;
                    output.publish_rows(1);
                    advance = ConsumerStep::Rows;
                    Phase::EmitDisk
                }
            }
            Phase::Done | Phase::Failed => unreachable!("terminal states handled before work"),
        };
        Ok(advance)
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod admission_tests;

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod reduction_tests;
