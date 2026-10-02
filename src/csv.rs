//! Decode CSV into bounded batches without writing to a database.
//!
//! A decoder borrows its target schema and owns reusable input and output buffers.
//! The header maps input fields to declaration order. Each successful batch lends
//! typed values until the next call; only a successful end of input validates the
//! entire stream. After an error the decoder cannot continue with a partial row.
//!
//! `docs/formats.md#csv` defines the format, byte offsets and limits. An importer
//! must reserve decoder memory alongside append buffers and abort on failure.

mod import;
mod record;
mod typed;

pub use import::ImportLimits;

use crate::batch::input::Cell;
use crate::resources::{allocate, buffer_charge};
use crate::{CancellationToken, ColumnDeclaration, Error};
use std::io::Read;
use std::mem::size_of;

// Cover native rounding in addition to each buffer's allocation-reuse charge.
// This is a macOS/GNU premise, not a bound for arbitrary global allocators.
const ALLOCATION_ROUNDING_BYTES: u64 = 16_384;
const MAX_COLUMNS: usize = 64;
const READ_BYTES: usize = 4_096;
const MAX_FIELD_BYTES: u32 = 65_536;
const MAX_RECORD_BYTES: u32 = 64 * (2 * MAX_FIELD_BYTES + 2) + 65;

/// Positive input and batch bounds. Record bytes include CSV punctuation and endings.
#[derive(Clone, Copy, Debug)]
pub struct CsvLimits {
    /// Total encoded bytes, including the header.
    pub input_bytes: u64,
    /// Total data rows; the header is excluded.
    pub rows: u64,
    /// Encoded bytes per record, including its ending; at most 8,388,801.
    pub record_bytes: u32,
    /// Decoded bytes per field, including header fields; at most 65,536.
    pub field_bytes: u32,
    /// At most 256 rows per batch.
    pub batch_rows: u16,
    /// Total decoded STRING bytes in one batch, also the maximum for one row.
    pub batch_text_bytes: u32,
}

#[derive(Clone, Copy, Default)]
struct Field {
    offset: u64,
    start: usize,
    end: usize,
    quoted: bool,
}

/// A batch borrowed from a [`CsvDecoder`], in declaration order.
/// A valid batch is a prefix, not confirmation that the entire input is valid.
pub type CsvBatch<'a> = crate::InputBatch<'a>;

/// A streaming decoder with a separate memory limit and no database side effects.
/// The decoder owns its reader. Reader allocations and the borrowed schema are
/// outside this memory limit. See `docs/formats.md#csv`.
pub struct CsvDecoder<'schema, R> {
    reader: R,
    schema: &'schema [ColumnDeclaration<'schema>],
    limits: CsvLimits,
    memory_bytes: u64,
    input: [u8; READ_BYTES],
    input_position: usize,
    input_end: usize,
    input_finished: bool,
    offset: u64,
    record_start: u64,
    raw: Vec<u8>,
    decoded: Vec<u8>,
    fields: [Field; MAX_COLUMNS],
    field_count: usize,
    mapping: [usize; MAX_COLUMNS],
    row: [Cell; MAX_COLUMNS],
    row_text_bytes: usize,
    cells: Vec<Cell>,
    text: Vec<u8>,
    rows: u64,
    header: bool,
    pending: bool,
    finished: bool,
    failed: bool,
}

impl<'schema, R: Read> CsvDecoder<'schema, R> {
    /// Validate configuration and calculate requested memory before reading or allocating.
    /// Includes the decoder object, buffer capacities and native allocator rounding
    /// allowances. Reader allocations and borrowed schema storage are excluded.
    pub fn required_memory(
        schema: &[ColumnDeclaration<'_>],
        limits: CsvLimits,
    ) -> Result<u64, Error> {
        if schema.is_empty() || schema.len() > MAX_COLUMNS {
            return Err(Error::InvalidConfig("CSV requires 1 through 64 columns"));
        }
        for (index, column) in schema.iter().enumerate() {
            if !crate::schema::valid_name(column.name.as_bytes()) {
                return Err(Error::InvalidConfig("invalid CSV column name"));
            }
            if schema[..index]
                .iter()
                .any(|other| other.name.eq_ignore_ascii_case(column.name))
            {
                return Err(Error::InvalidConfig("duplicate CSV column name"));
            }
        }
        if limits.input_bytes == 0
            || limits.rows == 0
            || limits.record_bytes == 0
            || limits.field_bytes == 0
            || limits.field_bytes > MAX_FIELD_BYTES
            || limits.record_bytes > MAX_RECORD_BYTES
            || limits.batch_text_bytes > 64 * MAX_FIELD_BYTES
            || limits.field_bytes > limits.record_bytes
            || !(1..=256).contains(&limits.batch_rows)
            || limits.batch_text_bytes == 0
        {
            return Err(Error::InvalidConfig("invalid CSV limits"));
        }
        let cells = schema.len() * usize::from(limits.batch_rows);
        [
            limits.record_bytes as usize,
            limits.record_bytes as usize,
            cells * size_of::<Cell>(),
            limits.batch_text_bytes as usize,
        ]
        .into_iter()
        .try_fold(
            size_of::<Self>() as u64 + 4 * ALLOCATION_ROUNDING_BYTES,
            |total, capacity| {
                buffer_charge(capacity).and_then(|charge| total.checked_add(charge as u64))
            },
        )
        .ok_or(Error::InvalidConfig("CSV memory size overflow"))
    }

    /// Allocate all reusable buffers within `memory_limit`, without reading input.
    /// An importer must separately hold a shared database reservation for
    /// `required_memory` until this decoder is dropped.
    pub fn new(
        reader: R,
        schema: &'schema [ColumnDeclaration<'schema>],
        limits: CsvLimits,
        memory_limit: u64,
        cancel: &CancellationToken,
    ) -> Result<Self, Error> {
        cancel.check()?;
        let memory_bytes = Self::required_memory(schema, limits)?;
        if memory_bytes > memory_limit {
            return Err(Error::Resource {
                owner: "CSV decoder",
                required: memory_bytes,
                limit: memory_limit,
            });
        }
        let records = limits.record_bytes as usize;
        let cells = schema.len() * usize::from(limits.batch_rows);
        let text = limits.batch_text_bytes as usize;
        let result = Self {
            reader,
            schema,
            limits,
            memory_bytes,
            input: [0; READ_BYTES],
            input_position: 0,
            input_end: 0,
            input_finished: false,
            offset: 0,
            record_start: 0,
            raw: allocate(records, records, "CSV record", memory_limit)?,
            decoded: allocate(records, records, "CSV decoded record", memory_limit)?,
            fields: [Field::default(); MAX_COLUMNS],
            field_count: 0,
            mapping: [0; MAX_COLUMNS],
            row: [Cell::Null; MAX_COLUMNS],
            row_text_bytes: 0,
            cells: allocate(cells, cells, "CSV cells", memory_limit)?,
            text: allocate(text, text, "CSV batch text", memory_limit)?,
            rows: 0,
            header: false,
            pending: false,
            finished: false,
            failed: false,
        };
        cancel.check()?;
        Ok(result)
    }

    /// Memory allowance retained for this decoder's lifetime.
    pub fn memory_bytes(&self) -> u64 {
        self.memory_bytes
    }

    /// Lend the next batch, return `None` at validated EOF, or permanently stop on error.
    pub fn next_batch(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<Option<CsvBatch<'_>>, Error> {
        if self.failed {
            return Err(Error::Unsupported("CSV decoder already failed"));
        }
        // A failure, including cancellation, must never expose a partially filled batch.
        self.failed = true;
        self.fill_batch(cancel)?;
        cancel.check()?;
        self.failed = false;
        Ok((!self.cells.is_empty()).then_some(CsvBatch {
            cells: &self.cells,
            text: &self.text,
            columns: self.schema.len(),
        }))
    }

    fn fill_batch(&mut self, cancel: &CancellationToken) -> Result<(), Error> {
        cancel.check()?;
        self.cells.clear();
        self.text.clear();
        if self.finished {
            return Ok(());
        }
        if !self.header {
            if !self.read_record(cancel)? {
                return Err(input_error("missing CSV header", self.offset));
            }
            self.decode_fields(cancel)?;
            self.map_header()?;
            self.header = true;
        }
        while self.cells.len() / self.schema.len() < usize::from(self.limits.batch_rows) {
            if !self.pending {
                if !self.read_record(cancel)? {
                    self.finished = true;
                    break;
                }
                if self.rows == self.limits.rows {
                    return Err(input_error("CSV row limit", self.record_start));
                }
                self.decode_fields(cancel)?;
                self.type_row()?;
            }
            if self.row_text_bytes > self.limits.batch_text_bytes as usize {
                return Err(input_error(
                    "CSV row exceeds batch text limit",
                    self.record_start,
                ));
            }
            if self.text.len() + self.row_text_bytes > self.limits.batch_text_bytes as usize {
                self.pending = true;
                break;
            }
            for cell in &self.row[..self.schema.len()] {
                self.cells.push(match *cell {
                    Cell::Text { start, end } => {
                        let output_start = self.text.len();
                        self.text.extend_from_slice(&self.decoded[start..end]);
                        Cell::Text {
                            start: output_start,
                            end: self.text.len(),
                        }
                    }
                    other => other,
                });
            }
            self.rows += 1;
            self.pending = false;
        }
        Ok(())
    }
}

fn input_error(message: &'static str, byte_offset: u64) -> Error {
    Error::Input {
        message,
        byte_offset,
    }
}

#[cfg(test)]
mod tests;
