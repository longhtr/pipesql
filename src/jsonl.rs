//! Stream a query as typed JSON Lines, preserving its schema and complete values.
//!
//! The result cursor owns execution; the encoder borrows each batch and retains
//! only a fixed output buffer. Only Finished permits the completion record.
//! Errors leave any written prefix with the caller and release execution owners.
//!
//! The schema record describes positional columns, including duplicate names, and
//! DOUBLE values preserve their bits. Row and byte limits apply to the stream as it
//! is produced; a caller must check the returned result even if it received rows
//! or a completion record before a final output error.

use crate::export_io::{output_error, write_all};
use crate::{CancellationToken, DataType, Database, Error, PreparedQuery, QueryStep, Value};
use std::io::{Cursor, Write};

/// Maximum complete rows and encoded bytes, including schema and completion.
/// Zero rows permits an empty result. A byte limit may leave a partial record.
#[derive(Clone, Copy, Debug)]
pub struct ExportLimits {
    pub rows: u64,
    pub bytes: u64,
}

impl Database {
    /// Stream the prepared query using the typed JSON Lines profile in docs/formats.md#json-lines.
    ///
    /// Success means execution finished and the supplied writer flushed. Failure
    /// may leave partial output, including a completion record if its flush failed.
    /// The caller owns the writer, its allocations and publication of any file.
    /// Engine buffers share the database budget; no result rows are retained here.
    /// Cancellation cannot interrupt a blocked writer. Query errors retain their
    /// source spans. Returns the complete row count on success.
    pub fn export_jsonl(
        &self,
        query: &PreparedQuery<'_>,
        output: &mut impl Write,
        limits: ExportLimits,
        cancel: &CancellationToken,
    ) -> Result<u64, Error> {
        cancel.check()?;
        let _reservation = self.reserve_memory(
            (std::mem::size_of::<Encoder<'_, '_>>() + 64) as u64,
            "result export buffer",
        )?;
        let mut result = self.execute(query, cancel)?;
        let mut encoder = Encoder {
            output,
            cancel,
            buffer: [0; 1024],
            length: 0,
            bytes: 0,
            limit: limits.bytes,
        };
        encoder.schema(query)?;
        let mut rows = 0_u64;
        loop {
            match result.step() {
                QueryStep::Progress => {}
                QueryStep::Finished => break,
                QueryStep::Failed(_) => {
                    return Err(result.into_error().expect("failed query owns its error"));
                }
                QueryStep::Rows(batch) => {
                    for row in 0..batch.len() {
                        let next = rows.checked_add(1).ok_or(Error::Unsupported(
                            "export row count exceeds unsigned 64-bit range",
                        ))?;
                        if next > limits.rows {
                            return Err(Error::Resource {
                                owner: "result export rows",
                                required: next,
                                limit: limits.rows,
                            });
                        }
                        encoder.bytes(b"{\"row\":[")?;
                        for column in 0..batch.column_count() {
                            if column != 0 {
                                encoder.bytes(b",")?;
                            }
                            encoder.value(
                                batch
                                    .value(row, column)
                                    .ok_or(Error::Corrupt("export result cell is missing"))?,
                            )?;
                        }
                        encoder.bytes(b"]}\n")?;
                        rows = next;
                    }
                }
            }
        }
        encoder.bytes(b"{\"complete\":true,\"rows\":")?;
        encoder.formatted(format_args!("{rows}"))?;
        encoder.bytes(b"}\n")?;
        encoder.flush_buffer()?;
        cancel.check()?;
        encoder.output.flush().map_err(output_error)?;
        cancel.check()?;
        Ok(rows)
    }
}

// The byte count includes bytes buffered but not yet sent. Drop never flushes:
// a failed query must not publish a tail merely because its encoder is released.
struct Encoder<'writer, 'cancel> {
    output: &'writer mut dyn Write,
    cancel: &'cancel CancellationToken,
    buffer: [u8; 1024],
    length: usize,
    bytes: u64,
    limit: u64,
}

impl Encoder<'_, '_> {
    fn bytes(&mut self, mut bytes: &[u8]) -> Result<(), Error> {
        self.cancel.check()?;
        let required = self
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or(Error::Unsupported(
                "export byte count exceeds unsigned 64-bit range",
            ))?;
        if required > self.limit {
            return Err(Error::Resource {
                owner: "result export bytes",
                required,
                limit: self.limit,
            });
        }
        self.bytes = required;
        while !bytes.is_empty() {
            let length = bytes.len().min(self.buffer.len() - self.length);
            self.buffer[self.length..self.length + length].copy_from_slice(&bytes[..length]);
            self.length += length;
            bytes = &bytes[length..];
            if self.length == self.buffer.len() {
                self.flush_buffer()?;
            }
        }
        Ok(())
    }

    fn flush_buffer(&mut self) -> Result<(), Error> {
        write_all(self.output, &self.buffer[..self.length], self.cancel)?;
        self.length = 0;
        Ok(())
    }

    fn formatted(&mut self, arguments: std::fmt::Arguments<'_>) -> Result<(), Error> {
        // Integers, bit patterns and ISO dates fit in 64 bytes. Formatting into
        // this local buffer avoids allocating a String for each scalar value.
        let mut buffer = Cursor::new([0; 64]);
        buffer
            .write_fmt(arguments)
            .map_err(|_| Error::Corrupt("export scalar exceeds formatting buffer"))?;
        self.bytes(&buffer.get_ref()[..buffer.position() as usize])
    }

    fn string(&mut self, text: &str) -> Result<(), Error> {
        self.bytes(b"\"")?;
        let mut start = 0;
        for (index, byte) in text.bytes().enumerate() {
            if byte < 0x20 || byte == b'"' || byte == b'\\' {
                self.bytes(&text.as_bytes()[start..index])?;
                match byte {
                    b'"' => self.bytes(b"\\\"")?,
                    b'\\' => self.bytes(b"\\\\")?,
                    _ => self.formatted(format_args!("\\u{byte:04x}"))?,
                }
                start = index + 1;
            }
        }
        self.bytes(&text.as_bytes()[start..])?;
        self.bytes(b"\"")
    }

    fn schema(&mut self, query: &PreparedQuery<'_>) -> Result<(), Error> {
        self.bytes(b"{\"format\":\"pipesql-jsonl\",\"version\":1,\"columns\":[")?;
        for index in 0..query.result_column_count() {
            if index != 0 {
                self.bytes(b",")?;
            }
            let column = query
                .result_column(index)
                .ok_or(Error::Corrupt("export result column is missing"))?;
            self.bytes(b"{\"name\":")?;
            match column.name {
                Some(name) => self.string(name)?,
                None => self.bytes(b"null")?,
            }
            self.bytes(b",\"type\":")?;
            self.string(match column.data_type {
                DataType::Int64 => "int64",
                DataType::Double => "double",
                DataType::Date => "date",
                DataType::String => "string",
            })?;
            self.bytes(b",\"nullable\":")?;
            self.bytes(if column.nullable { b"true" } else { b"false" })?;
            self.bytes(b"}")?;
        }
        self.bytes(b"]}\n")
    }

    fn value(&mut self, value: Value<'_>) -> Result<(), Error> {
        match value {
            Value::Null => self.bytes(b"null"),
            Value::Int64(value) => self.formatted(format_args!("\"{value}\"")),
            Value::Double(value) => self.formatted(format_args!("\"{:016x}\"", value.to_bits())),
            Value::Date(value) => self.formatted(format_args!("\"{value}\"")),
            Value::String(value) => self.string(value.as_str()),
        }
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;
