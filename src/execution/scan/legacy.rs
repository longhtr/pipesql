//! Legacy fixed-schema source: demanded buffers, checked stored values, and unit admission.
//! Metadata is validated before payload; a column is loaded only when demanded.
use super::{AdmittedScan, ScanCursor, ScanPhase, Source};
use crate::batch::{Batch, ColumnMut, OwnedBatch};
use crate::effects::{Effect, Effects, QueryEffect};
use crate::error::io_error;
use crate::execution::COMPUTE_ROWS;
use crate::execution::computed::{BatchLayout, BatchScratch};
use crate::execution::planning::PhysicalPlan;
use crate::execution::predicate::{BranchScratch, PhysicalFilter};
use crate::fixed_text::StringValue as FixedKey;
use crate::frontend::{DataType, FilterLiteral, MAX_COLUMNS, Predicate, PreparedQuery};
use crate::namespace::{UNIT_NAME, UNITS_NAME};
use crate::resources::{Reservation, allocate};
use crate::storage_format::{self, BlockDescriptor, UnitMetadata};
use crate::{CancellationToken, DatabaseId, DateValue, Error, StringValue, Value};
use pipesql_filesystem as filesystem;
use std::fs::File;
use std::io;
use std::path::Path;

const BLOCK_BYTES: usize = 262_144;
pub(in crate::execution) const BLOCK_ROWS: usize = 32_768;
pub(in crate::execution) const MAX_WORKSPACE_BYTES: u64 = WORKSPACE_FIXED_BYTES
    + ARENA_BYTES as u64
    + 2 * crate::batch::MAX_BYTES
    + BranchScratch::MAX_BYTES;
const ARENA_BYTES: usize = 5 * BLOCK_BYTES + 2 * BLOCK_ROWS;
const WORKSPACE_FIXED_BYTES: u64 = 106_496;
const METADATA_BYTES: usize = storage_format::HEADER_BYTES
    + storage_format::DESCRIPTOR_BYTES
    + storage_format::UNIT_PADDING_BYTES;
const DESCRIPTOR_CEILING: usize = 1344;
const SELECTION_CEILING: usize = COMPUTE_ROWS;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::execution) struct UnitExpectation {
    pub(in crate::execution) database_id: DatabaseId,
    pub(in crate::execution) rows: u64,
    pub(in crate::execution) unit_bytes: u64,
    pub(in crate::execution) unit_metadata_crc32c: u32,
    pub(in crate::execution) projected_crc32c: u32,
    pub(in crate::execution) descriptor_count: u32,
}

pub(in crate::execution) struct Scan {
    file: Option<File>,
    arena: Vec<u8>,
    buffers: [usize; 8],
    descriptors: Vec<BlockDescriptor>,
    rows: usize,
    blocks: usize,
    block: usize,
    loaded: u8,
    loaded_date_block: Option<usize>,
}

impl Scan {
    pub(super) fn finish_block(&mut self) -> Result<(), Error> {
        self.block = self
            .block
            .checked_add(1)
            .ok_or(Error::Corrupt("block cursor overflow"))?;
        self.loaded = 0;
        Ok(())
    }

    pub(super) fn is_finished(&self) -> bool {
        self.block == self.blocks
    }

    pub(super) fn block_rows(&self) -> Result<usize, &'static str> {
        let first = self
            .block
            .checked_mul(BLOCK_ROWS)
            .ok_or("block row offset overflow")?;
        self.rows
            .checked_sub(first)
            .map(|remaining| remaining.min(BLOCK_ROWS))
            .ok_or("block exceeds rows")
    }

    pub(super) fn load_column(
        &mut self,
        column: u8,
        cancellation: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<bool, Error> {
        if column >= 7 {
            return Err(Error::Corrupt("physical column is invalid"));
        }
        let mask = 1_u8 << column;
        let date_block = self.block / 2;
        if (column == 6 && self.loaded_date_block == Some(date_block))
            || (column != 6 && self.loaded & mask != 0)
        {
            return Ok(false);
        }
        let block = if column == 6 { date_block } else { self.block };
        let rows_per_block = if column == 6 {
            2 * BLOCK_ROWS
        } else {
            BLOCK_ROWS
        };
        let width = match column {
            0..=3 => 8,
            4..=5 => 1,
            6 => 4,
            _ => unreachable!(),
        };
        let index = usize::from(column)
            .checked_mul(self.blocks)
            .and_then(|first| first.checked_add(block))
            .ok_or(Error::Corrupt("descriptor index overflow"))?;
        let descriptor = *self
            .descriptors
            .get(index)
            .ok_or(Error::Corrupt("descriptor is missing"))?;
        let (start, capacity) = buffer_range(&self.buffers, column).map_err(Error::Corrupt)?;
        let bytes = read_demanded_block(
            self.file
                .as_ref()
                .ok_or(Error::Corrupt("empty source attempted a read"))?,
            descriptor,
            &mut self.arena[start..start + capacity],
            cancellation,
            effects,
        )?;
        let first_row = block
            .checked_mul(rows_per_block)
            .ok_or(Error::Corrupt("column row offset overflow"))?;
        let rows = self
            .rows
            .checked_sub(first_row)
            .ok_or(Error::Corrupt("column block exceeds rows"))?
            .min(rows_per_block);
        if bytes
            != rows
                .checked_mul(width)
                .ok_or(Error::Corrupt("column byte overflow"))?
        {
            return Err(Error::Corrupt("column extent disagrees with rows"));
        }
        if column == 6 {
            self.loaded_date_block = Some(date_block);
        } else {
            self.loaded |= mask;
        }
        Ok(true)
    }

    pub(super) fn column(&self, column: u8) -> Result<StoredColumn<'_>, &'static str> {
        if column >= 7 {
            return Err("column index is invalid");
        }
        let rows = self.block_rows()?;
        let loaded = if column == 6 {
            self.loaded_date_block == Some(self.block / 2)
        } else {
            self.loaded & (1_u8 << column) != 0
        };
        if !loaded {
            return Err("unloaded column");
        }
        let (first, capacity) = buffer_range(&self.buffers, column)?;
        let buffer = &self.arena[first..first + capacity];
        Ok(match column {
            0..=3 => StoredColumn::Double(&buffer[..rows * 8]),
            4..=5 => StoredColumn::String(&buffer[..rows]),
            6 => {
                let first = (self.block % 2) * BLOCK_ROWS * 4;
                StoredColumn::Date(&buffer[first..first + rows * 4])
            }
            _ => unreachable!("validated column"),
        })
    }
}

#[derive(Clone, Copy)]
pub(super) enum StoredColumn<'a> {
    Double(&'a [u8]),
    String(&'a [u8]),
    Date(&'a [u8]),
}

impl StoredColumn<'_> {
    pub(super) fn value(self, row: usize) -> Result<Value<'static>, &'static str> {
        Ok(match self {
            Self::Double(bytes) => Value::Double(read_f64(bytes, row)?),
            Self::Date(bytes) => Value::Date(read_date(bytes, row)?),
            Self::String(bytes) => {
                let key = FixedKey::from_byte(*bytes.get(row).ok_or("stored key row bound")?)
                    .ok_or("stored key is outside printable ASCII domain")?;
                Value::String(StringValue::new(key.as_str()))
            }
        })
    }

    pub(super) fn decode(
        self,
        selection: &[u32],
        output: ColumnMut<'_>,
    ) -> Result<(), &'static str> {
        match (self, output) {
            (Self::Double(bytes), ColumnMut::Double(output)) => {
                assert_eq!(output.len(), selection.len(), "decoded column extent");
                for (out, row) in output.iter_mut().zip(selection) {
                    *out = read_f64(bytes, usize::try_from(*row).expect("block row fits"))?;
                }
            }
            (Self::String(bytes), ColumnMut::String(output)) => {
                assert_eq!(output.len(), selection.len(), "decoded column extent");
                for (out, row) in output.iter_mut().zip(selection) {
                    *out = FixedKey::from_byte(
                        *bytes
                            .get(usize::try_from(*row).expect("block row fits"))
                            .ok_or("key row exceeds block")?,
                    )
                    .ok_or("stored key is outside printable ASCII domain")?;
                }
            }
            (Self::Date(bytes), ColumnMut::Date(output)) => {
                assert_eq!(output.len(), selection.len(), "decoded column extent");
                for (out, row) in output.iter_mut().zip(selection) {
                    *out = read_date(bytes, usize::try_from(*row).expect("block row fits"))?;
                }
            }
            _ => return Err("stored and batch column types disagree"),
        }
        Ok(())
    }

    pub(super) fn filter(
        self,
        selection: &mut Vec<u32>,
        filter: PhysicalFilter<'_>,
    ) -> Result<(), &'static str> {
        let mut retained = 0;
        match (self, *filter.predicate) {
            (
                Self::Double(bytes),
                Predicate::Compare {
                    comparison,
                    literal: FilterLiteral::Double(bits),
                },
            ) => {
                let literal = f64::from_bits(bits);
                for index in 0..selection.len() {
                    let row = selection[index];
                    if comparison.test(
                        read_f64(bytes, usize::try_from(row).expect("block row fits"))?,
                        literal,
                    ) != filter.control.negated
                    {
                        selection[retained] = row;
                        retained += 1;
                    }
                }
            }
            (
                Self::Date(bytes),
                Predicate::Compare {
                    comparison,
                    literal: FilterLiteral::Date(literal),
                },
            ) => {
                for index in 0..selection.len() {
                    let row = selection[index];
                    if comparison.test_order(
                        read_date(bytes, usize::try_from(row).expect("block row fits"))?
                            .cmp(&literal),
                    ) != filter.control.negated
                    {
                        selection[retained] = row;
                        retained += 1;
                    }
                }
            }
            (
                column @ Self::String(_),
                Predicate::Compare {
                    literal: FilterLiteral::String(_),
                    ..
                },
            )
            | (column, Predicate::IsNull { .. }) => {
                for index in 0..selection.len() {
                    let row = selection[index];
                    let value = column.value(usize::try_from(row).expect("block row fits"))?;
                    if filter
                        .matches(value)
                        .map_err(|_| "stored filter type disagrees")?
                    {
                        selection[retained] = row;
                        retained += 1;
                    }
                }
            }
            _ => return Err("stored filter type disagrees"),
        }
        selection.truncate(retained);
        Ok(())
    }
}

fn read_date(bytes: &[u8], row: usize) -> Result<DateValue, &'static str> {
    let first = row.checked_mul(4).ok_or("DATE row extent overflow")?;
    let end = first.checked_add(4).ok_or("DATE value extent overflow")?;
    let value = i32::from_le_bytes(
        bytes
            .get(first..end)
            .ok_or("DATE row exceeds block")?
            .try_into()
            .expect("DATE width"),
    );
    DateValue::from_days(value).ok_or("stored DATE is outside supported calendar")
}

// Four 256-KiB DOUBLE buffers, two 32-KiB key buffers, one 256-KiB DATE buffer.
// Prefix offsets pack the demanded source buffers. Missing columns occupy no
// bytes. Metadata validation reuses the arena before any payload is loaded.
#[derive(Clone, Copy)]
pub(in crate::execution) struct Layout {
    computation: BatchLayout,
    branch_rows: usize,
    offsets: [usize; 8],
    input_types: [DataType; MAX_COLUMNS],
    input_count: usize,
    output_types: [DataType; MAX_COLUMNS],
    output_count: usize,
}

impl Layout {
    pub(in crate::execution) fn open_empty(
        self,
        mut reservation: Reservation<'_>,
    ) -> Result<AdmittedScan<'_>, Error> {
        Ok(AdmittedScan {
            input: OwnedBatch::new(&[], &mut reservation)?,
            output: OwnedBatch::new(&self.output_types[..self.output_count], &mut reservation)?,
            scan: ScanCursor {
                computation: BatchScratch::new(self.computation)?,
                source: Source::Legacy(Scan {
                    file: None,
                    arena: Vec::new(),
                    buffers: self.offsets,
                    descriptors: Vec::new(),
                    rows: 0,
                    blocks: 0,
                    block: 0,
                    loaded: 0,
                    loaded_date_block: None,
                }),
                selection: Vec::new(),
                branches: BranchScratch::default(),
                start: 0,
                end: 0,
                phase: ScanPhase::Begin,
                reservation,
            },
        })
    }

    pub(in crate::execution) fn new(
        query: &PreparedQuery<'_>,
        physical: &PhysicalPlan<'_>,
    ) -> Self {
        let demand = physical
            .scan()
            .raw_demand()
            .expect("validated scan dependencies");
        let mut offsets = [0_usize; 8];
        for column in 0..7 {
            let bytes = if demand & (1 << column) == 0 {
                0
            } else {
                column_capacity(column)
            };
            offsets[column + 1] = offsets[column]
                .checked_add(bytes)
                .expect("seven bounded column buffers");
        }
        let mut input_types = [DataType::Double; MAX_COLUMNS];
        let mut input_count = 0;
        for column in physical.scan().output_columns(&query.plan) {
            input_types[input_count] = column.data_type();
            input_count += 1;
        }
        let mut output_types = [DataType::Double; MAX_COLUMNS];
        let output_count = physical
            .aggregate_pipeline()
            .map_or(0, |pipeline| pipeline.column_count);
        if let Some(pipeline) = physical.aggregate_pipeline() {
            for (kind, column) in output_types
                .iter_mut()
                .zip(pipeline.output_columns(&query.plan))
            {
                *kind = column.data_type();
            }
        }
        Self {
            computation: BatchLayout::new(physical.scan()).expect("validated computation layout"),
            branch_rows: BranchScratch::rows(physical.scan()),
            offsets,
            input_types,
            input_count,
            output_types,
            output_count,
        }
    }

    fn arena_bytes(&self) -> usize {
        self.offsets[7].max(METADATA_BYTES)
    }

    pub(in crate::execution) fn workspace_bytes(&self) -> Result<u64, Error> {
        let input = Batch::required_bytes(&self.input_types[..self.input_count])?;
        let output = Batch::required_bytes(&self.output_types[..self.output_count])?;
        (WORKSPACE_FIXED_BYTES + self.arena_bytes() as u64)
            .checked_add(input)
            .and_then(|bytes| bytes.checked_add(output))
            .and_then(|bytes| bytes.checked_add(self.computation.bytes()))
            .and_then(|bytes| bytes.checked_add(BranchScratch::bytes(self.branch_rows)))
            .ok_or(Error::Corrupt("scan workspace extent"))
    }

    pub(in crate::execution) fn validate(
        &self,
        plan: &PhysicalPlan,
        query: &PreparedQuery<'_>,
    ) -> Result<(), Error> {
        if self.branch_rows != BranchScratch::rows(plan.scan())
            || self.computation != BatchLayout::new(plan.scan())?
            || self.input_count != plan.scan().column_count
            || self.output_count
                != plan
                    .aggregate_pipeline()
                    .map_or(0, |pipeline| pipeline.column_count)
        {
            return Err(Error::Corrupt("batch column count disagrees with plan"));
        }
        for (kind, column) in self.input_types[..self.input_count]
            .iter()
            .zip(plan.scan().output_columns(&query.plan))
        {
            if *kind != column.data_type() {
                return Err(Error::Corrupt("batch source type disagrees"));
            }
        }
        if let Some(pipeline) = plan.aggregate_pipeline() {
            for (kind, column) in self.output_types[..self.output_count]
                .iter()
                .zip(pipeline.output_columns(&query.plan))
            {
                if column.data_type() != *kind {
                    return Err(Error::Corrupt("batch output type disagrees"));
                }
            }
        }
        if self.offsets[0] != 0 || self.offsets[7] > ARENA_BYTES {
            return Err(Error::Corrupt("column buffer extent"));
        }
        let demand = plan.scan().raw_demand()?;
        for column in 0..7 {
            let demanded = demand & (1 << column) != 0;
            let expected = if demanded { column_capacity(column) } else { 0 };
            if self.offsets[column + 1].checked_sub(self.offsets[column]) != Some(expected) {
                return Err(Error::Corrupt(
                    "column buffer disagrees with physical demand",
                ));
            }
        }
        Ok(())
    }
}

fn buffer_range(offsets: &[usize; 8], column: u8) -> Result<(usize, usize), &'static str> {
    let column = usize::from(column);
    let start = offsets[column];
    let bytes = offsets[column + 1] - start;
    if bytes == 0 {
        return Err("column has no admitted buffer");
    }
    Ok((start, bytes))
}

fn column_capacity(column: usize) -> usize {
    match column {
        0..=3 | 6 => BLOCK_BYTES,
        4..=5 => BLOCK_ROWS,
        _ => unreachable!("validated storage column"),
    }
}

// Namespace inspection and unit admission have separate large scratch owners.
// Keep the decoder slot out of the caller's namespace-inspection frame.
#[inline(never)]
pub(in crate::execution) fn open_workspace<'db>(
    root: &Path,
    expected: UnitExpectation,
    double_blocks: u32,
    buffers: Layout,
    mut reservation: Reservation<'db>,
    cancellation: &CancellationToken,
    effects: &mut Effects,
) -> Result<AdmittedScan<'db>, Error> {
    let rows = expected.rows;
    let arena_bytes = buffers.arena_bytes();
    let mut arena = allocate::<u8>(arena_bytes, arena_bytes, "scan arena", reservation.bytes())?;
    arena.resize(arena_bytes, 0);
    let mut descriptors = allocate::<BlockDescriptor>(
        DESCRIPTOR_CEILING,
        DESCRIPTOR_CEILING,
        "scan descriptors",
        reservation.bytes(),
    )?;
    let selection = allocate::<u32>(
        COMPUTE_ROWS,
        SELECTION_CEILING,
        "scan selection",
        reservation.bytes(),
    )?;
    let input = OwnedBatch::new(
        &buffers.input_types[..buffers.input_count],
        &mut reservation,
    )?;
    let output = OwnedBatch::new(
        &buffers.output_types[..buffers.output_count],
        &mut reservation,
    )?;
    let computation = BatchScratch::new(buffers.computation)?;
    let mut slot = std::mem::MaybeUninit::uninit();
    let (file, metadata) = open_unit(
        root,
        expected,
        &mut slot,
        &mut arena[..METADATA_BYTES],
        cancellation,
        effects,
    )?;
    // Copy validated small records into their retained owner. The full
    // metadata value never moves across the opener/decoder boundary.
    descriptors.extend_from_slice(&metadata.descriptors[..metadata.descriptor_count()]);
    Ok(AdmittedScan {
        input,
        output,
        scan: ScanCursor {
            computation,
            source: Source::Legacy(Scan {
                file: Some(file),
                arena,
                buffers: buffers.offsets,
                descriptors,
                rows: usize::try_from(rows)
                    .map_err(|_| Error::Corrupt("row count does not fit"))?,
                blocks: usize::try_from(double_blocks)
                    .map_err(|_| Error::Corrupt("block count does not fit"))?,
                block: 0,
                loaded: 0,
                loaded_date_block: None,
            }),
            selection,
            branches: BranchScratch::new(buffers.branch_rows, reservation.bytes())?,
            start: 0,
            end: 0,
            phase: ScanPhase::Begin,
            reservation,
        },
    })
}

fn open_unit<'out>(
    root: &Path,
    expected_plan: UnitExpectation,
    decoded_slot: &'out mut std::mem::MaybeUninit<UnitMetadata>,
    buffer: &mut [u8],
    cancellation: &CancellationToken,
    effects: &mut Effects,
) -> Result<(File, &'out UnitMetadata), Error> {
    let path = crate::path::joined_path(&crate::path::joined_path(root, UNITS_NAME)?, UNIT_NAME)?;
    cancellation.check()?;
    effects.before(Effect::Query(QueryEffect::InspectUnit))?;
    let expected = filesystem::symlink_metadata(&path)
        .map_err(|source| io_error("inspect query unit", source))?;
    if !expected.file_type().is_file() || expected.len() != expected_plan.unit_bytes {
        return Err(Error::Corrupt("query unit has invalid type or length"));
    }
    cancellation.check()?;
    effects.before(Effect::Query(QueryEffect::OpenUnit))?;
    let file = filesystem::open_read(path).map_err(|source| io_error("open query unit", source))?;
    cancellation.check()?;
    effects.before(Effect::Query(QueryEffect::InspectOpenUnit))?;
    let opened = filesystem::file_metadata(&file)
        .map_err(|source| io_error("inspect open query unit", source))?;
    if opened.identity() != expected.identity() || opened.len() != expected_plan.unit_bytes {
        return Err(Error::Corrupt("query unit changed while opening"));
    }

    // Metadata I/O precedes all payload reads. The caller can reuse this charged
    // byte buffer after decoding; only its separate typed slot remains borrowed.
    let (header, rest) = buffer.split_at_mut(storage_format::HEADER_BYTES);
    let (descriptors, rest) = rest.split_at_mut(storage_format::DESCRIPTOR_BYTES);
    let padding = &mut rest[..storage_format::UNIT_PADDING_BYTES];
    read_exact_at(
        &file,
        header,
        0,
        QueryEffect::ReadHeader,
        cancellation,
        effects,
    )?;
    read_exact_at(
        &file,
        descriptors,
        u64::try_from(storage_format::HEADER_BYTES).expect("header bytes fit u64"),
        QueryEffect::ReadDescriptors,
        cancellation,
        effects,
    )?;
    read_exact_at(
        &file,
        padding,
        u64::try_from(storage_format::HEADER_BYTES + storage_format::DESCRIPTOR_BYTES)
            .expect("unit metadata prefix fits u64"),
        QueryEffect::ReadPadding,
        cancellation,
        effects,
    )?;
    if padding.iter().any(|byte| *byte != 0) {
        return Err(Error::Corrupt("query unit padding is nonzero"));
    }
    let (metadata, checksum) =
        storage_format::decode_unit_metadata_into(header, descriptors, decoded_slot)
            .map_err(|_| Error::Corrupt("query unit metadata is invalid"))?;
    if metadata.database != expected_plan.database_id
        || metadata.rows != expected_plan.rows
        || metadata.projected_crc32c != expected_plan.projected_crc32c
        || checksum != expected_plan.unit_metadata_crc32c
        || metadata.descriptor_count()
            != usize::try_from(expected_plan.descriptor_count)
                .map_err(|_| Error::Corrupt("descriptor count does not fit usize"))?
    {
        return Err(Error::Corrupt(
            "query unit metadata disagrees with physical plan",
        ));
    }
    Ok((file, metadata))
}

fn read_demanded_block(
    file: &File,
    descriptor: BlockDescriptor,
    buffer: &mut [u8],
    cancellation: &CancellationToken,
    effects: &mut Effects,
) -> Result<usize, Error> {
    let bytes = usize::try_from(descriptor.bytes)
        .map_err(|_| Error::Corrupt("demanded block length does not fit usize"))?;
    if bytes == 0 || bytes > buffer.len() {
        return Err(Error::Corrupt("demanded block length is invalid"));
    }
    read_exact_at(
        file,
        &mut buffer[..bytes],
        descriptor.offset,
        QueryEffect::ReadPayload,
        cancellation,
        effects,
    )?;
    if storage_format::crc32c(&buffer[..bytes]) != descriptor.crc32c {
        return Err(Error::Corrupt("demanded unit payload checksum failed"));
    }
    Ok(bytes)
}

fn read_exact_at(
    file: &File,
    buffer: &mut [u8],
    offset: u64,
    effect: QueryEffect,
    cancellation: &CancellationToken,
    effects: &mut Effects,
) -> Result<(), Error> {
    cancellation.check()?;
    let short = effects.before(Effect::Query(effect))?;
    let length = if short {
        buffer
            .len()
            .checked_sub(1)
            .expect("fixed query reads are nonempty")
    } else {
        buffer.len()
    };
    crate::file_io::read_exact_at(file, &mut buffer[..length], offset)
        .map_err(|source| io_error(effect.name(), source))?;
    if short {
        return Err(io_error(
            effect.name(),
            io::Error::from(io::ErrorKind::UnexpectedEof),
        ));
    }
    Ok(())
}

fn read_f64(buffer: &[u8], index: usize) -> Result<f64, &'static str> {
    let Some(start) = index.checked_mul(size_of::<f64>()) else {
        return Err("DOUBLE value offset overflow");
    };
    let Some(end) = start.checked_add(size_of::<f64>()) else {
        return Err("DOUBLE value extent overflow");
    };
    let Some(bytes) = buffer.get(start..end) else {
        return Err("DOUBLE value is outside demanded block");
    };
    Ok(f64::from_bits(u64::from_le_bytes(
        bytes
            .try_into()
            .map_err(|_| "DOUBLE value width is invalid")?,
    )))
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;
