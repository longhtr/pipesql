//! Admit one stable bounded source, then stream its query through the public API.
use super::output::{output_error, write_query_header, write_value};
use pipesql::{CancellationToken, Database, Error, QueryStep};
use pipesql_filesystem as filesystem;
use std::io::{Read, Write};
use std::path::Path;

const MAX_QUERY_BYTES: usize = 4_096;
const MAX_QUERY_BYTES_U64: u64 = 4_096;

// Metadata checks detect observed source changes; they do not make a mutable
// file an atomic snapshot. The caller must keep the source unchanged during read.
fn validate_query_source(
    expected: &filesystem::Metadata,
    observed: &filesystem::Metadata,
) -> Result<(), Error> {
    if !observed.file_type().is_file()
        || observed.len() > MAX_QUERY_BYTES_U64
        || observed.identity() != expected.identity()
        || observed.len() != expected.len()
        || observed.mtime() != expected.mtime()
        || observed.mtime_nsec() != expected.mtime_nsec()
        || observed.ctime() != expected.ctime()
        || observed.ctime_nsec() != expected.ctime_nsec()
    {
        return Err(Error::Input {
            message: "query file changed during source admission or read",
            byte_offset: 0,
        });
    }
    Ok(())
}

pub(super) fn execute_query_file(
    database: &Database,
    path: &Path,
    output: &mut impl Write,
) -> Result<(), Error> {
    let metadata = filesystem::symlink_metadata(path).map_err(|source| Error::Io {
        operation: "inspect query file",
        source,
    })?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_QUERY_BYTES_U64 {
        return Err(Error::Input {
            message: "query file must be a regular non-symlink file of at most 4096 bytes",
            byte_offset: 0,
        });
    }
    let mut file = filesystem::open_read(path).map_err(|source| Error::Io {
        operation: "open query file",
        source,
    })?;
    let opened = filesystem::file_metadata(&file).map_err(|source| Error::Io {
        operation: "inspect opened query file",
        source,
    })?;
    validate_query_source(&metadata, &opened)?;
    let mut bytes = [0_u8; MAX_QUERY_BYTES + 1];
    let mut length = 0_usize;
    loop {
        let read = file
            .read(&mut bytes[length..])
            .map_err(|source| Error::Io {
                operation: "read query file",
                source,
            })?;
        if read == 0 {
            break;
        }
        length = length.checked_add(read).ok_or(Error::Input {
            message: "query file length overflow",
            byte_offset: 0,
        })?;
        if length > MAX_QUERY_BYTES {
            return Err(Error::Input {
                message: "query file exceeds 4096 bytes",
                byte_offset: MAX_QUERY_BYTES_U64,
            });
        }
    }
    let final_metadata = filesystem::file_metadata(&file).map_err(|source| Error::Io {
        operation: "inspect query file after read",
        source,
    })?;
    validate_query_source(&metadata, &final_metadata)?;
    let named = filesystem::symlink_metadata(path).map_err(|source| Error::Io {
        operation: "inspect query source name after read",
        source,
    })?;
    validate_query_source(&metadata, &named)?;
    if u64::try_from(length).expect("bounded query length fits u64") != metadata.len() {
        return Err(Error::Input {
            message: "query file read disagrees with admitted length",
            byte_offset: 0,
        });
    }
    let source = std::str::from_utf8(&bytes[..length]).map_err(|_| Error::Input {
        message: "query file must be UTF-8",
        byte_offset: 0,
    })?;
    let prepared = database.prepare(source)?;
    let cancellation = CancellationToken::new();
    let mut result = database.execute(&prepared, &cancellation)?;
    write_query_header(output, database, &prepared)?;
    let mut rows = 0_u64;
    loop {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Finished => break,
            QueryStep::Failed(_) => {
                return Err(result.into_error().expect("failed result owns its error"));
            }
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    output
                        .write_all(b"row=")
                        .map_err(|source| output_error("write query row", source))?;
                    for column in 0..batch.column_count() {
                        if column != 0 {
                            output
                                .write_all(b"|")
                                .map_err(|source| output_error("write query value", source))?;
                        }
                        let value = batch
                            .value(row, column)
                            .ok_or(Error::Corrupt("batch value is missing"))?;
                        write_value(output, &value)?;
                    }
                    output
                        .write_all(b"\n")
                        .map_err(|source| output_error("write query row", source))?;
                    rows = rows
                        .checked_add(1)
                        .ok_or(Error::Corrupt("CLI result row count overflow"))?;
                }
            }
        }
    }
    writeln!(output, "row_count={rows}\nstatus=queried")
        .map_err(|source| output_error("write query completion", source))
}

#[cfg(test)]
mod tests {
    use super::{MAX_QUERY_BYTES, validate_query_source};
    use pipesql::Error;
    use pipesql_filesystem as filesystem;
    use std::path::PathBuf;

    #[test]
    fn query_source_requires_matching_regular_bounded_unchanged_metadata() {
        use std::fs;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        struct Temp(PathBuf);

        impl Drop for Temp {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let root = Temp(std::env::temp_dir().join(format!(
            "pipesql-cli-source-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        fs::create_dir(&root.0).unwrap();
        let first = root.0.join("source");
        let second = root.0.join("different-source");
        fs::write(&first, b"FROM lineitem").unwrap();
        fs::write(&second, b"FROM lineitem").unwrap();
        let expected = filesystem::symlink_metadata(&first).unwrap();
        let opened = filesystem::open_read(&first).unwrap();
        validate_query_source(
            &expected,
            &filesystem::Metadata::from_std(&opened.metadata().unwrap()),
        )
        .unwrap();
        for path in [&second, &root.0] {
            assert!(matches!(
                validate_query_source(&expected, &filesystem::symlink_metadata(path).unwrap()),
                Err(Error::Input { .. })
            ));
        }
        let link = root.0.join("link");
        std::os::unix::fs::symlink(&first, &link).unwrap();
        assert!(
            validate_query_source(&expected, &filesystem::symlink_metadata(&link).unwrap())
                .is_err()
        );
        opened
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1))
            .unwrap();
        assert!(
            validate_query_source(
                &expected,
                &filesystem::Metadata::from_std(&opened.metadata().unwrap()),
            )
            .is_err()
        );
        for length in [MAX_QUERY_BYTES, MAX_QUERY_BYTES + 1] {
            fs::write(&first, vec![b' '; length]).unwrap();
            let observed = filesystem::symlink_metadata(&first).unwrap();
            assert!(validate_query_source(&expected, &observed).is_err());
            assert_eq!(
                validate_query_source(&observed, &observed).is_ok(),
                length == MAX_QUERY_BYTES
            );
        }
    }
}
