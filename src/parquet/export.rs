//! Collect bounded query groups and stream their Parquet column pages.
//!
//! Group boundaries follow row/text limits, independently of cursor batches. The
//! encoder retains cells, text, one page/footer buffer and completed chunk ranges.
//! Only query completion permits the final group and footer; drop writes nothing.
//!
//! Earlier row groups may already be in the caller's output when a later query,
//! limit or writer error occurs. The error releases retained buffers and execution
//! owners, but cannot undo those bytes. Only a successful return establishes a
//! complete export, including the final output flush.

use super::{
    bound,
    footer_write::{self, Chunk, Group},
    page_write,
};
use crate::batch::input::Cell;
use crate::export_io::{output_error, write_all};
use crate::resources::{allocate, buffer_charge};
use crate::{
    CancellationToken, ColumnDeclaration, DataType, Database, Error, PreparedQuery, QueryStep,
    ResultBatch, Value,
};
use std::io::Write;
use std::mem::size_of;

/// Bounds for streamed Parquet output. See `docs/formats.md#parquet` for the flat profile.
#[derive(Clone, Copy, Debug)]
pub struct ParquetExportLimits {
    /// Total complete rows; zero permits an empty result.
    pub rows: u64,
    /// Total output bytes, including magic bytes and the footer.
    pub bytes: u64,
    /// Rows retained per group; positive and at most 65,536.
    pub row_group_rows: u32,
    /// Total STRING bytes retained per group; positive and at most 8 MiB.
    /// A single row must fit; each STRING remains limited to 65,536 bytes.
    pub row_group_text_bytes: u32,
    /// Group count; positive and at most 4,096.
    pub row_groups: u32,
    /// Footer bytes; positive and at most 16 MiB.
    pub metadata_bytes: u32,
}

impl ParquetExportLimits {
    fn validate(self) -> Result<(), Error> {
        if !(1..=65_536).contains(&self.row_group_rows)
            || !(1..=8_388_608).contains(&self.row_group_text_bytes)
            || !(1..=4_096).contains(&self.row_groups)
            || !(1..=16_777_216).contains(&self.metadata_bytes)
        {
            return Err(Error::InvalidConfig("invalid Parquet output limits"));
        }
        Ok(())
    }

    fn scratch_bytes(self) -> usize {
        let rows = self.row_group_rows as usize;
        let values = (8 * rows).max(4 * rows + self.row_group_text_bytes as usize);
        // Optional v1 levels have a four-byte length, at most five run-header
        // bytes, and one packed bit per row. All columns fit the 16 MiB page cap.
        (page_write::HEADER_BYTES + values + 9 + rows.div_ceil(8)).max(self.metadata_bytes as usize)
    }
}

impl Database {
    /// Export a complete query as flat, uncompressed PLAIN Parquet.
    ///
    /// Columns need unique declared-table names; alias unnamed or repeated outputs.
    /// All scalar bits and NULLs are preserved. Groups are bounded independently
    /// of query batches, so output can exceed retained memory. Encoder buffers
    /// share the database budget; the writer's allocations remain caller costs.
    ///
    /// Success requires query completion and final flush, and returns the row
    /// count. An error may leave any output prefix, even a valid footer if flush
    /// failed. The caller owns file publication and must check the returned result.
    /// Drop does not flush. Cancellation cannot interrupt a blocked writer.
    pub fn export_parquet(
        &self,
        query: &PreparedQuery<'_>,
        output: &mut impl Write,
        limits: ParquetExportLimits,
        cancel: &CancellationToken,
    ) -> Result<u64, Error> {
        cancel.check()?;
        limits.validate()?;
        let count = query.result_column_count();
        if count == 0 || count > 64 {
            return Err(Error::Unsupported(
                "Parquet output requires 1 through 64 columns",
            ));
        }
        let mut schema = [ColumnDeclaration {
            name: "",
            data_type: DataType::Int64,
            nullable: false,
        }; 64];
        for index in 0..count {
            let column = query
                .result_column(index)
                .ok_or(Error::Corrupt("Parquet result column is missing"))?;
            let name = column
                .name
                .ok_or(Error::Unsupported("Parquet output columns require names"))?;
            if !crate::schema::valid_name(name.as_bytes()) {
                return Err(Error::Unsupported(
                    "Parquet output name is not a declared-table identifier",
                ));
            }
            if schema[..index]
                .iter()
                .any(|previous| previous.name.eq_ignore_ascii_case(name))
            {
                return Err(Error::Unsupported(
                    "Parquet output column names must be unique",
                ));
            }
            schema[index] = ColumnDeclaration {
                name,
                data_type: column.data_type,
                nullable: column.nullable,
            };
        }
        let schema = &schema[..count];
        let required = Encoder::required_memory(count, limits);
        let _reservation = self.reserve_memory(required, "Parquet export buffers")?;
        let mut encoder = Encoder::new(schema, output, limits, required, cancel)?;
        let mut result = self.execute(query, cancel)?;
        encoder.output.write(b"PAR1")?;
        loop {
            match result.step() {
                QueryStep::Progress => {}
                QueryStep::Finished => break,
                QueryStep::Failed(_) => {
                    return Err(result.into_error().expect("failed query owns its error"));
                }
                QueryStep::Rows(batch) => {
                    if batch.column_count() != count {
                        return Err(Error::Corrupt("Parquet result column count differs"));
                    }
                    for row in 0..batch.len() {
                        encoder.row(&batch, row)?;
                    }
                }
            }
        }
        encoder.finish()
    }
}

struct Output<'writer, 'cancel> {
    writer: &'writer mut dyn Write,
    cancel: &'cancel CancellationToken,
    bytes: u64,
    limit: u64,
}

impl Output<'_, '_> {
    fn write(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.cancel.check()?;
        let next = self
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or(Error::Unsupported("Parquet output byte count overflows"))?;
        bound(next, self.limit, "Parquet output bytes")?;
        write_all(self.writer, bytes, self.cancel)?;
        self.bytes = next;
        Ok(())
    }
}

struct Encoder<'schema, 'writer, 'cancel> {
    schema: &'schema [ColumnDeclaration<'schema>],
    limits: ParquetExportLimits,
    cells: Vec<Cell>,
    text: Vec<u8>,
    scratch: Vec<u8>,
    groups: Vec<Group>,
    chunks: Vec<Chunk>,
    rows: u64,
    output: Output<'writer, 'cancel>,
}

impl<'schema, 'writer, 'cancel> Encoder<'schema, 'writer, 'cancel> {
    fn required_memory(columns: usize, limits: ParquetExportLimits) -> u64 {
        // Validated maxima bound both writable capacities and native reuse
        // charges. Groups and chunks occupy separate physical allocations.
        let buffers = [
            columns * limits.row_group_rows as usize * size_of::<Cell>(),
            limits.row_group_text_bytes as usize,
            limits.scratch_bytes(),
            limits.row_groups as usize * size_of::<Group>(),
            limits.row_groups as usize * columns * size_of::<Chunk>(),
        ];
        let retained: usize = buffers
            .into_iter()
            .map(|capacity| buffer_charge(capacity).expect("validated Parquet buffer bound"))
            .sum();
        (size_of::<Self>() + size_of::<[ColumnDeclaration<'_>; 64]>() + 5 * 16_384 + retained)
            as u64
    }

    fn new(
        schema: &'schema [ColumnDeclaration<'schema>],
        output: &'writer mut dyn Write,
        limits: ParquetExportLimits,
        memory: u64,
        cancel: &'cancel CancellationToken,
    ) -> Result<Self, Error> {
        let cells = schema.len() * limits.row_group_rows as usize;
        let groups = limits.row_groups as usize;
        let chunks = groups * schema.len();
        let scratch_bytes = limits.scratch_bytes();
        let mut scratch = allocate(
            scratch_bytes,
            scratch_bytes,
            "Parquet encoding buffer",
            memory,
        )?;
        scratch.resize(scratch_bytes, 0);
        Ok(Self {
            schema,
            limits,
            cells: allocate(cells, cells, "Parquet output cells", memory)?,
            text: allocate(
                limits.row_group_text_bytes as usize,
                limits.row_group_text_bytes as usize,
                "Parquet output text",
                memory,
            )?,
            scratch,
            groups: allocate(groups, groups, "Parquet output groups", memory)?,
            chunks: allocate(chunks, chunks, "Parquet output chunks", memory)?,
            rows: 0,
            output: Output {
                writer: output,
                cancel,
                bytes: 0,
                limit: limits.bytes,
            },
        })
    }

    fn row(&mut self, batch: &ResultBatch<'_>, row: usize) -> Result<(), Error> {
        self.output.cancel.check()?;
        let next = self
            .rows
            .checked_add(1)
            .ok_or(Error::Unsupported("Parquet output row count overflows"))?;
        bound(next, self.limits.rows, "Parquet output rows")?;
        let mut text_bytes = 0;
        for (ordinal, column) in self.schema.iter().enumerate() {
            let value = batch
                .value(row, ordinal)
                .ok_or(Error::Corrupt("Parquet result cell is missing"))?;
            match (value, column.data_type) {
                (Value::Null, _) if column.nullable => {}
                (Value::Int64(_), DataType::Int64)
                | (Value::Double(_), DataType::Double)
                | (Value::Date(_), DataType::Date) => {}
                (Value::String(value), DataType::String) => {
                    bound(value.as_str().len() as u64, 65_536, "Parquet STRING bytes")?;
                    text_bytes += value.as_str().len();
                }
                _ => {
                    return Err(Error::Corrupt(
                        "Parquet result value differs from its schema",
                    ));
                }
            }
        }
        bound(
            text_bytes as u64,
            u64::from(self.limits.row_group_text_bytes),
            "Parquet row text bytes",
        )?;
        if self.cells.len() / self.schema.len() == self.limits.row_group_rows as usize
            || self.text.len() + text_bytes > self.limits.row_group_text_bytes as usize
        {
            self.flush_group()?;
        }
        bound(
            self.groups.len() as u64 + 1,
            u64::from(self.limits.row_groups),
            "Parquet output row groups",
        )?;
        for ordinal in 0..self.schema.len() {
            let cell = match batch.value(row, ordinal).expect("validated result cell") {
                Value::Null => Cell::Null,
                Value::Int64(value) => Cell::Int64(value),
                Value::Double(value) => Cell::Double(value),
                Value::Date(value) => Cell::Date(value),
                Value::String(value) => {
                    let start = self.text.len();
                    self.text.extend_from_slice(value.as_str().as_bytes());
                    Cell::Text {
                        start,
                        end: self.text.len(),
                    }
                }
            };
            self.cells.push(cell);
        }
        self.rows = next;
        Ok(())
    }

    fn flush_group(&mut self) -> Result<(), Error> {
        if self.cells.is_empty() {
            return Ok(());
        }
        let rows = (self.cells.len() / self.schema.len()) as u32;
        let start = self.output.bytes;
        for (ordinal, column) in self.schema.iter().enumerate() {
            let page = page_write::encode(
                &mut self.scratch,
                &self.cells,
                &self.text,
                column,
                ordinal,
                self.schema.len(),
                self.output.cancel,
            )?;
            let start = self.output.bytes;
            self.output.write(&self.scratch[page.header])?;
            self.output.write(&self.scratch[page.payload])?;
            self.chunks.push(Chunk {
                start,
                bytes: self.output.bytes - start,
            });
        }
        self.groups.push(Group {
            rows,
            bytes: self.output.bytes - start,
        });
        self.cells.clear();
        self.text.clear();
        Ok(())
    }

    fn finish(mut self) -> Result<u64, Error> {
        self.flush_group()?;
        self.output.cancel.check()?;
        let length = footer_write::encode(
            &mut self.scratch[..self.limits.metadata_bytes as usize],
            self.schema,
            &self.groups,
            &self.chunks,
            self.rows,
            self.output.cancel,
        )?;
        self.output.write(&self.scratch[..length])?;
        self.output.write(&(length as u32).to_le_bytes())?;
        self.output.write(b"PAR1")?;
        self.output.cancel.check()?;
        self.output.writer.flush().map_err(output_error)?;
        self.output.cancel.check()?;
        Ok(self.rows)
    }
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
