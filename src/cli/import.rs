//! Capture an import source before opening the database, then publish through the
//! library importer. A file must retain its identity, size and timestamps through
//! completion. CSV stdin has no expected length; Parquet requires a seekable file.
//!
//! The token is flushed before CSV rows or Parquet pages are read. A receipt failure aborts the attempt;
//! failure writing the final status cannot undo an already committed transaction.

use super::{command::ImportFormat, output::output_error, stdio};
use pipesql::{CancellationToken, Database, Error};
use pipesql_filesystem as filesystem;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

pub(super) struct Input<'a> {
    file: File,
    expected: Option<(&'a Path, filesystem::Metadata)>,
    position: u64,
}

impl<'a> Input<'a> {
    pub(super) fn open(path: &'a Path, limit: u64) -> Result<Self, Error> {
        if path == Path::new("-") {
            return Ok(Self {
                file: stdio::stdin().map_err(|source| Error::Io {
                    operation: "capture CSV stdin",
                    source,
                })?,
                expected: None,
                position: 0,
            });
        }
        Self::open_file(path, limit)
    }

    pub(super) fn open_parquet(path: &'a Path, limit: u64) -> Result<Self, Error> {
        if path == Path::new("-") {
            return Err(Error::Input {
                message: "Parquet input requires a seekable regular file",
                byte_offset: 0,
            });
        }
        Self::open_file(path, limit)
    }

    fn open_file(path: &'a Path, limit: u64) -> Result<Self, Error> {
        if !path.is_absolute() {
            return Err(Error::Input {
                message: "import input path must be absolute",
                byte_offset: 0,
            });
        }
        let expected = filesystem::symlink_metadata(path).map_err(|source| Error::Io {
            operation: "inspect import input",
            source,
        })?;
        if !expected.file_type().is_file() || expected.len() > limit {
            return Err(Error::Input {
                message: "import input must be a regular non-symlink file within the input limit",
                byte_offset: 0,
            });
        }
        let file = filesystem::open_read(path).map_err(|source| Error::Io {
            operation: "open import input",
            source,
        })?;
        let observed = filesystem::file_metadata(&file).map_err(|source| Error::Io {
            operation: "inspect opened import input",
            source,
        })?;
        unchanged(&expected, &observed).map_err(|source| Error::Io {
            operation: "validate opened import input",
            source,
        })?;
        Ok(Self {
            file,
            expected: Some((path, expected)),
            position: 0,
        })
    }
}

fn unchanged(expected: &filesystem::Metadata, observed: &filesystem::Metadata) -> io::Result<()> {
    if !observed.file_type().is_file()
        || expected.identity() != observed.identity()
        || expected.len() != observed.len()
        || expected.mtime() != observed.mtime()
        || expected.mtime_nsec() != observed.mtime_nsec()
        || expected.ctime() != observed.ctime()
        || expected.ctime_nsec() != observed.ctime_nsec()
    {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(())
}

impl Read for Input<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let count = self.file.read(bytes)?;
        self.position = self
            .position
            .checked_add(count as u64)
            .ok_or(io::ErrorKind::InvalidData)?;
        if count == 0
            && let Some((path, expected)) = &self.expected
        {
            unchanged(expected, &filesystem::file_metadata(&self.file)?)?;
            unchanged(expected, &filesystem::symlink_metadata(path)?)?;
            if self.position != expected.len() {
                return Err(io::ErrorKind::InvalidData.into());
            }
        }
        Ok(count)
    }
}

impl Seek for Input<'_> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let Some((path, expected)) = &self.expected else {
            return Err(io::ErrorKind::Unsupported.into());
        };
        let position = self.file.seek(to)?;
        self.position = position;
        // The Parquet decoder asks for the end before reading its footer and
        // again before commit. Check both the open file and its pathname there.
        if matches!(to, SeekFrom::End(0)) {
            unchanged(expected, &filesystem::file_metadata(&self.file)?)?;
            unchanged(expected, &filesystem::symlink_metadata(path)?)?;
            if position != expected.len() {
                return Err(io::ErrorKind::InvalidData.into());
            }
        }
        Ok(position)
    }
}

pub(super) fn execute(
    database: &Database,
    table: &str,
    input: Input<'_>,
    limits: ImportFormat,
    output: &mut impl Write,
) -> Result<(), Error> {
    let issued = |token| writeln!(output, "transaction={token}").and_then(|()| output.flush());
    let cancel = CancellationToken::new();
    let commit = match limits {
        ImportFormat::Csv(limits) => database.import_csv(table, input, limits, &cancel, issued)?,
        ImportFormat::Parquet(limits) => {
            database.import_parquet(table, input, limits, &cancel, issued)?
        }
    };
    writeln!(
        output,
        "status=imported\ngeneration={}",
        commit.generation()
    )
    .map_err(|source| output_error("write import status", source))
}

#[cfg(test)]
mod tests {
    //! Mutate real file identity and contents between admission and EOF. The
    //! reader must fail before the importer could accept end of input.

    use super::*;
    use crate::test_support::Directory;

    #[test]
    fn source_checks_final_identity_length_and_times() {
        for (parquet, mutation) in [false, true].into_iter().flat_map(|parquet| {
            ["replace", "grow", "shorten", "timestamp", "symlink"]
                .map(move |mutation| (parquet, mutation))
        }) {
            let directory = Directory::new();
            let path = directory.0.join("input.csv");
            std::fs::write(&path, b"id\n1\n").unwrap();
            let mut input = if parquet {
                Input::open_parquet(&path, 5)
            } else {
                Input::open(&path, 5)
            }
            .unwrap();
            if parquet {
                assert_eq!(input.seek(SeekFrom::End(0)).unwrap(), 5);
                input.seek(SeekFrom::Start(0)).unwrap();
            }
            let mut bytes = [0; 5];
            assert_eq!(input.read(&mut bytes).unwrap(), 5);
            assert_eq!(&bytes, b"id\n1\n");
            match mutation {
                "replace" => {
                    let other = directory.0.join("other.csv");
                    std::fs::write(&other, b"id\n1\n").unwrap();
                    std::fs::rename(other, &path).unwrap();
                }
                "grow" => std::fs::write(&path, b"id\n1\n2\n").unwrap(),
                "shorten" => std::fs::write(&path, b"id\n").unwrap(),
                "timestamp" => input.file.set_modified(std::time::UNIX_EPOCH).unwrap(),
                "symlink" => {
                    let old = directory.0.join("old.csv");
                    std::fs::rename(&path, &old).unwrap();
                    std::os::unix::fs::symlink(old, &path).unwrap();
                }
                _ => unreachable!(),
            }
            if parquet {
                assert_eq!(
                    input.seek(SeekFrom::End(0)).unwrap_err().kind(),
                    io::ErrorKind::InvalidData
                );
                continue;
            }
            let mut reached_error = false;
            for _ in 0..3 {
                match input.read(&mut bytes) {
                    Err(error) => {
                        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                        reached_error = true;
                        break;
                    }
                    Ok(0) => panic!("accepted changed file: {mutation}"),
                    Ok(_) => {}
                }
            }
            assert!(reached_error, "{mutation}");
        }
    }

    #[test]
    fn source_admission_and_unchanged_eof() {
        let directory = Directory::new();
        let path = directory.0.join("input.csv");
        std::fs::write(&path, b"id\n1\n").unwrap();
        assert!(Input::open(&path, 4).is_err());
        assert!(Input::open(&directory.0, 100).is_err());
        assert!(Input::open(Path::new("relative.csv"), 100).is_err());
        let link = directory.0.join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(Input::open(&link, 100).is_err());
        let mut input = Input::open(&path, 5).unwrap();
        let mut bytes = [0; 8];
        assert_eq!(input.read(&mut []).unwrap(), 0);
        assert_eq!(input.read(&mut bytes).unwrap(), 5);
        assert_eq!(input.read(&mut bytes).unwrap(), 0);
    }

    #[test]
    fn parquet_source_requires_a_file_and_checks_position_after_seeking() {
        assert!(Input::open_parquet(Path::new("-"), 100).is_err());
        let directory = Directory::new();
        let path = directory.0.join("input.parquet");
        std::fs::write(&path, b"0123456789").unwrap();
        let mut input = Input::open_parquet(&path, 10).unwrap();
        assert_eq!(input.seek(SeekFrom::End(0)).unwrap(), 10);
        input.seek(SeekFrom::Start(8)).unwrap();
        let mut bytes = [0; 2];
        assert_eq!(input.read(&mut bytes).unwrap(), 2);
        assert_eq!(bytes, *b"89");
        input.seek(SeekFrom::Start(0)).unwrap();
        assert_eq!(input.read(&mut bytes).unwrap(), 2);
        assert_eq!(bytes, *b"01");
        assert_eq!(input.seek(SeekFrom::End(0)).unwrap(), 10);
        assert_eq!(input.read(&mut bytes).unwrap(), 0);
    }

    #[test]
    fn receipt_failure_aborts_but_final_output_failure_preserves_commit() {
        use pipesql::{
            AppendLimits, ColumnDeclaration, Config, CsvLimits, DataType, ImportLimits,
            ParquetImportLimits, ParquetReadLimits,
        };
        struct FailingOutput {
            bytes: Vec<u8>,
            flushed: bool,
            fail_flush: bool,
        }
        impl Write for FailingOutput {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.flushed {
                    return Err(io::ErrorKind::BrokenPipe.into());
                }
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                self.flushed = true;
                if self.fail_flush {
                    Err(io::ErrorKind::BrokenPipe.into())
                } else {
                    Ok(())
                }
            }
        }
        for (parquet, fail_flush) in [(false, true), (false, false), (true, true), (true, false)] {
            let directory = Directory::new();
            let db = Database::create_empty(
                &directory.0.join("db"),
                Config::new(4_000_000, 4_000_000).unwrap(),
            )
            .unwrap();
            db.declare_table(
                "facts",
                &[ColumnDeclaration {
                    name: "id",
                    data_type: DataType::Int64,
                    nullable: false,
                }],
                &CancellationToken::new(),
            )
            .unwrap();
            let path = directory.0.join("input.csv");
            if parquet {
                std::fs::write(
                    &path,
                    include_bytes!("../../test/data/parquet/plain-batches.parquet"),
                )
                .unwrap();
            } else {
                std::fs::write(&path, b"id\n1\n2\n3\n").unwrap();
            }
            let bounds = ImportLimits {
                csv: CsvLimits {
                    input_bytes: 1000,
                    rows: 10,
                    record_bytes: 128,
                    field_bytes: 64,
                    batch_rows: 2,
                    batch_text_bytes: 128,
                },
                append: AppendLimits {
                    batches: 4,
                    encoded_bytes: 100_000,
                },
            };
            let bounds = if parquet {
                ImportFormat::Parquet(ParquetImportLimits {
                    parquet: ParquetReadLimits {
                        input_bytes: 10_000,
                        rows: 600,
                        metadata_bytes: 2048,
                        row_groups: 1,
                        row_group_rows: 600,
                        row_group_bytes: 6000,
                        page_bytes: 1024,
                    },
                    append: AppendLimits {
                        batches: 4,
                        encoded_bytes: 100_000,
                    },
                })
            } else {
                ImportFormat::Csv(bounds)
            };
            let input = if parquet {
                Input::open_parquet(&path, 10_000)
            } else {
                Input::open(&path, 1000)
            }
            .unwrap();
            let mut output = FailingOutput {
                bytes: Vec::new(),
                flushed: false,
                fail_flush,
            };
            let error = execute(&db, "facts", input, bounds, &mut output).unwrap_err();
            assert!(
                matches!(error, Error::Io { source, .. } if source.kind() == io::ErrorKind::BrokenPipe)
            );
            let text = std::str::from_utf8(&output.bytes).unwrap();
            let hex = text
                .strip_prefix("transaction=")
                .unwrap()
                .strip_suffix('\n')
                .unwrap();
            assert_eq!(hex.len(), 48);
            let mut bytes = [0; 24];
            for (index, byte) in bytes.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).unwrap();
            }
            let token = pipesql::TransactionId::from_bytes(bytes).unwrap();
            assert_eq!(db.generation(), if fail_flush { 1 } else { 2 });
            let resolution = db.resolve_commit(token).unwrap();
            assert_eq!(
                matches!(resolution, pipesql::CommitResolution::Aborted),
                fail_flush
            );
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
    }
}
