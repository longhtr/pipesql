//! Own the admitted input buffers and lend bounded batches from one row group.
//!
//! Construction validates the footer before transaction issuance. Advancing then
//! reads and validates complete row groups, lending at most 256 rows at a time.
//! A final length/EOF check precedes completion. After any error, no partial group
//! can be exposed and the decoder cannot resume.

use super::{
    ParquetReadLimits, bound,
    metadata::{Chunk, Group, Metadata},
    rows,
};
use crate::batch::input::Cell;
use crate::resources::{allocate, buffer_charge};
use crate::{CancellationToken, ColumnDeclaration, Error, InputBatch};
use std::io::{Read, Seek, SeekFrom};
use std::mem::size_of;

pub(super) struct Decoder<'schema, R> {
    reader: R,
    schema: &'schema [ColumnDeclaration<'schema>],
    limits: ParquetReadLimits,
    length: u64,
    _footer: Vec<u8>,
    metadata: Metadata,
    encoded: Vec<u8>,
    cells: Vec<Cell>,
    group: usize,
    row: usize,
    finished: bool,
    failed: bool,
}

impl<'schema, R: Read + Seek> Decoder<'schema, R> {
    pub(super) fn row_count(&self) -> u64 {
        self.metadata.rows
    }

    pub(super) fn required_memory(
        schema: &[ColumnDeclaration<'_>],
        limits: ParquetReadLimits,
    ) -> Result<u64, Error> {
        limits.validate()?;
        if schema.is_empty() || schema.len() > 64 {
            return Err(Error::InvalidConfig(
                "Parquet requires 1 through 64 target columns",
            ));
        }
        for (index, column) in schema.iter().enumerate() {
            crate::schema::valid_name(column.name.as_bytes())
                .then_some(())
                .ok_or(())
                .map_err(|_| Error::InvalidConfig("invalid Parquet target column name"))?;
            if schema[..index]
                .iter()
                .any(|other| other.name.eq_ignore_ascii_case(column.name))
            {
                return Err(Error::InvalidConfig("duplicate Parquet target column name"));
            }
        }
        // Each physical buffer needs its own native reuse charge. Validated
        // maxima bound these products and their sum on the supported 64-bit hosts.
        let buffers = [
            limits.metadata_bytes as usize,
            limits.row_group_bytes as usize,
            limits.row_group_rows as usize * schema.len() * size_of::<Cell>(),
            limits.row_groups as usize * size_of::<Group>(),
            limits.row_groups as usize * schema.len() * size_of::<Chunk>(),
        ];
        let retained: usize = buffers
            .into_iter()
            .map(|capacity| buffer_charge(capacity).expect("validated Parquet buffer bound"))
            .sum();
        Ok((size_of::<Self>() + 5 * 16_384 + retained) as u64)
    }

    pub(super) fn new(
        mut reader: R,
        schema: &'schema [ColumnDeclaration<'schema>],
        limits: ParquetReadLimits,
        memory: u64,
        cancel: &CancellationToken,
    ) -> Result<Self, Error> {
        cancel.check()?;
        bound(
            Self::required_memory(schema, limits)?,
            memory,
            "Parquet decoder",
        )?;
        let length = seek(&mut reader, SeekFrom::End(0), cancel)?;
        bound(length, limits.input_bytes, "Parquet input bytes")?;
        if length < 12 {
            return Err(input(
                "Parquet file is shorter than its header and trailer",
                0,
            ));
        }
        let mut magic = [0; 4];
        seek(&mut reader, SeekFrom::Start(0), cancel)?;
        read_exact(&mut reader, &mut magic, 0, cancel)?;
        if &magic != b"PAR1" {
            return Err(input("invalid Parquet file header", 0));
        }
        let mut trailer = [0; 8];
        seek(&mut reader, SeekFrom::Start(length - 8), cancel)?;
        read_exact(&mut reader, &mut trailer, length - 8, cancel)?;
        if &trailer[4..] != b"PAR1" {
            return Err(input("invalid Parquet file trailer", length - 4));
        }
        let footer_bytes = u32::from_le_bytes(trailer[..4].try_into().expect("four bytes"));
        bound(
            u64::from(footer_bytes),
            u64::from(limits.metadata_bytes),
            "Parquet footer bytes",
        )?;
        if u64::from(footer_bytes) > length - 12 {
            return Err(input("Parquet footer overlaps its file header", length - 8));
        }
        let base = length - 8 - u64::from(footer_bytes);
        let mut footer = allocate(
            limits.metadata_bytes as usize,
            limits.metadata_bytes as usize,
            "Parquet footer",
            memory,
        )?;
        footer.resize(footer_bytes as usize, 0);
        seek(&mut reader, SeekFrom::Start(base), cancel)?;
        read_exact(&mut reader, &mut footer, base, cancel)?;
        let metadata = Metadata::read(&footer, base, schema, limits, memory, cancel)?;
        let encoded = allocate(
            limits.row_group_bytes as usize,
            limits.row_group_bytes as usize,
            "Parquet row group",
            memory,
        )?;
        let cell_count = limits.row_group_rows as usize * schema.len();
        let cells = allocate(cell_count, cell_count, "Parquet cells", memory)?;
        cancel.check()?;
        Ok(Self {
            reader,
            schema,
            limits,
            length,
            _footer: footer,
            metadata,
            encoded,
            cells,
            group: 0,
            row: 0,
            finished: false,
            failed: false,
        })
    }

    pub(super) fn next_batch(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<Option<InputBatch<'_>>, Error> {
        if self.failed {
            return Err(Error::Unsupported("Parquet decoder already failed"));
        }
        self.failed = true;
        self.advance(cancel)?;
        cancel.check()?;
        self.failed = false;
        if self.finished {
            return Ok(None);
        }
        let start = self.row;
        let end = (start + 256).min(self.cells.len() / self.schema.len());
        self.row = end;
        Ok(Some(InputBatch {
            cells: &self.cells[start * self.schema.len()..end * self.schema.len()],
            text: &self.encoded,
            columns: self.schema.len(),
        }))
    }

    fn advance(&mut self, cancel: &CancellationToken) -> Result<(), Error> {
        cancel.check()?;
        if self.finished || self.row < self.cells.len() / self.schema.len() {
            return Ok(());
        }
        if self.group == self.metadata.groups.len() {
            if seek(&mut self.reader, SeekFrom::End(0), cancel)? != self.length {
                return Err(input("Parquet input length changed", self.length));
            }
            let mut extra = [0];
            let count = self.reader.read(&mut extra).map_err(|source| Error::Io {
                operation: "check Parquet input end",
                source,
            })?;
            cancel.check()?;
            if count != 0 {
                return Err(input("Parquet input grew after its footer", self.length));
            }
            self.finished = true;
            return Ok(());
        }
        let group = &self.metadata.groups[self.group];
        self.encoded.resize((group.end - group.start) as usize, 0);
        self.cells
            .resize(group.rows * self.schema.len(), Cell::Null);
        seek(&mut self.reader, SeekFrom::Start(group.start), cancel)?;
        read_exact(&mut self.reader, &mut self.encoded, group.start, cancel)?;
        rows::decode(
            &self.encoded,
            &self.metadata,
            self.group,
            self.schema,
            &mut self.cells,
            self.limits,
            cancel,
        )?;
        self.group += 1;
        self.row = 0;
        Ok(())
    }
}

fn seek(reader: &mut impl Seek, to: SeekFrom, cancel: &CancellationToken) -> Result<u64, Error> {
    cancel.check()?;
    let position = reader.seek(to).map_err(|source| Error::Io {
        operation: "seek Parquet input",
        source,
    })?;
    cancel.check()?;
    if let SeekFrom::Start(expected) = to
        && position != expected
    {
        return Err(input("Parquet seek returned the wrong position", position));
    }
    Ok(position)
}

fn read_exact(
    reader: &mut impl Read,
    mut bytes: &mut [u8],
    mut offset: u64,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    while !bytes.is_empty() {
        cancel.check()?;
        // Propagate Interrupted instead of retrying without a progress bound.
        let count = reader.read(bytes).map_err(|source| Error::Io {
            operation: "read Parquet input",
            source,
        })?;
        if count == 0 {
            return Err(input("truncated Parquet input", offset));
        }
        if count > bytes.len() {
            return Err(input(
                "Parquet reader returned an invalid byte count",
                offset,
            ));
        }
        bytes = &mut bytes[count..];
        offset += count as u64;
    }
    cancel.check()
}

fn input(message: &'static str, byte_offset: u64) -> Error {
    Error::Input {
        message,
        byte_offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const BATCHES: &[u8] = include_bytes!("../../test/data/parquet/plain-batches.parquet");
    const ONE_COLUMN: [ColumnDeclaration<'static>; 1] = [ColumnDeclaration {
        name: "id",
        data_type: crate::DataType::Int64,
        nullable: false,
    }];

    fn batch_limits() -> ParquetReadLimits {
        ParquetReadLimits {
            input_bytes: 10_000,
            metadata_bytes: 2048,
            row_groups: 1,
            row_group_rows: 600,
            row_group_bytes: 6000,
            page_bytes: 1024,
            rows: 600,
        }
    }

    struct Controlled {
        bytes: Cursor<&'static [u8]>,
        read_error: bool,
        short: usize,
        zero: bool,
        bad_count: bool,
        seek_error: bool,
        wrong_position: bool,
    }

    impl Controlled {
        fn new() -> Self {
            Self {
                bytes: Cursor::new(BATCHES),
                read_error: false,
                short: 7,
                zero: false,
                bad_count: false,
                seek_error: false,
                wrong_position: false,
            }
        }
    }

    impl Read for Controlled {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            if self.read_error {
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            if self.zero {
                return Ok(0);
            }
            if self.bad_count {
                return Ok(bytes.len() + 1);
            }
            let count = bytes.len().min(self.short);
            self.bytes.read(&mut bytes[..count])
        }
    }

    impl Seek for Controlled {
        fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
            if self.seek_error {
                return Err(std::io::ErrorKind::Other.into());
            }
            self.bytes
                .seek(position)
                .map(|n| n + u64::from(self.wrong_position))
        }
    }

    #[test]
    fn short_reads_multiple_pages_and_batch_boundaries_preserve_all_rows() {
        let cancel = CancellationToken::new();
        let limits = batch_limits();
        let memory = Decoder::<Controlled>::required_memory(&ONE_COLUMN, limits).unwrap();
        let mut decoder =
            Decoder::new(Controlled::new(), &ONE_COLUMN, limits, memory, &cancel).unwrap();
        let mut expected = 0;
        for count in [256, 256, 88] {
            let batch = decoder.next_batch(&cancel).unwrap().unwrap();
            assert_eq!(batch.row_count(), count);
            for row in 0..count {
                assert_eq!(batch.value(row, 0), Some(crate::Value::Int64(expected)));
                expected += 1;
            }
        }
        assert_eq!(expected, 600);
        assert!(decoder.next_batch(&cancel).unwrap().is_none());
    }

    #[test]
    fn read_seek_and_completion_failures_stop_without_exposing_partial_batches() {
        let limits = batch_limits();
        let memory = Decoder::<Controlled>::required_memory(&ONE_COLUMN, limits).unwrap();
        for fault in 0..6 {
            let cancel = CancellationToken::new();
            let mut decoder =
                Decoder::new(Controlled::new(), &ONE_COLUMN, limits, memory, &cancel).unwrap();
            match fault {
                0 => decoder.reader.read_error = true,
                1 => decoder.reader.zero = true,
                2 => decoder.reader.bad_count = true,
                3 => decoder.reader.seek_error = true,
                4 => decoder.reader.wrong_position = true,
                5 => cancel.cancel(),
                _ => unreachable!(),
            }
            let error = decoder
                .next_batch(&cancel)
                .err()
                .expect("failure returns no batch");
            match fault {
                0 | 3 => assert!(matches!(error, Error::Io { .. })),
                5 => assert!(matches!(error, Error::Cancelled)),
                _ => assert!(matches!(error, Error::Input { .. })),
            }
            assert!(matches!(
                decoder.next_batch(&CancellationToken::new()),
                Err(Error::Unsupported("Parquet decoder already failed"))
            ));
        }
        for fault in 0..3 {
            let cancel = CancellationToken::new();
            let mut decoder =
                Decoder::new(Controlled::new(), &ONE_COLUMN, limits, memory, &cancel).unwrap();
            for _ in 0..3 {
                decoder.next_batch(&cancel).unwrap().unwrap();
            }
            match fault {
                0 => decoder.reader.wrong_position = true,
                1 => decoder.reader.seek_error = true,
                2 => decoder.reader.read_error = true,
                _ => unreachable!(),
            }
            assert!(decoder.next_batch(&cancel).is_err());
        }
    }

    #[test]
    fn admitted_decoder_finishes_both_independent_files_and_stops_after_error() {
        let schema = [
            ColumnDeclaration {
                name: "id",
                data_type: crate::DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "amount",
                data_type: crate::DataType::Int64,
                nullable: true,
            },
            ColumnDeclaration {
                name: "number",
                data_type: crate::DataType::Double,
                nullable: true,
            },
            ColumnDeclaration {
                name: "day",
                data_type: crate::DataType::Date,
                nullable: true,
            },
            ColumnDeclaration {
                name: "note",
                data_type: crate::DataType::String,
                nullable: true,
            },
        ];
        let limits = ParquetReadLimits {
            input_bytes: 100_000,
            metadata_bytes: 16_384,
            row_groups: 3,
            row_group_rows: 3,
            row_group_bytes: 70_000,
            page_bytes: 70_000,
            rows: 8,
        };
        let cancel = CancellationToken::new();
        for bytes in [
            &include_bytes!("../../test/data/parquet/plain-v1.parquet")[..],
            &include_bytes!("../../test/data/parquet/plain-v2.parquet")[..],
        ] {
            let required = Decoder::<Cursor<&[u8]>>::required_memory(&schema, limits).unwrap();
            assert!(matches!(
                Decoder::new(Cursor::new(bytes), &schema, limits, required - 1, &cancel),
                Err(Error::Resource { .. })
            ));
            let mut decoder =
                Decoder::new(Cursor::new(bytes), &schema, limits, required, &cancel).unwrap();
            for rows in [3, 3, 2] {
                assert_eq!(
                    decoder.next_batch(&cancel).unwrap().unwrap().row_count(),
                    rows
                );
            }
            assert!(decoder.next_batch(&cancel).unwrap().is_none());
            assert!(decoder.next_batch(&cancel).unwrap().is_none());
            let mut damaged = bytes.to_vec();
            damaged[90] ^= 1;
            let mut decoder = Decoder::new(
                Cursor::new(damaged.as_slice()),
                &schema,
                limits,
                required,
                &cancel,
            )
            .unwrap();
            assert!(matches!(
                decoder.next_batch(&cancel),
                Err(Error::Input { .. })
            ));
            assert!(matches!(
                decoder.next_batch(&cancel),
                Err(Error::Unsupported("Parquet decoder already failed"))
            ));
        }
    }
}
