//! Read a small SQL or schema file into the caller's buffer.
//!
//! Before reading, require a regular, non-symlink file of at most 4096 bytes and
//! check that the opened descriptor identifies the inspected file. After reading,
//! check the descriptor and pathname again, along with the consumed byte count.
//! Only then return a UTF-8 slice. The file closes when this function returns;
//! the text borrows the caller's buffer.
//!
//! These checks detect changed identity, size and timestamps. They do not freeze
//! an externally writable file, so callers must keep the source unchanged during
//! the read. Parsing and database changes belong to the command that uses it.

use pipesql::Error;
use pipesql_filesystem as filesystem;
use std::io::Read;
use std::path::Path;

pub(super) const MAX_SOURCE_BYTES: usize = 4_096;
const MAX_SOURCE_BYTES_U64: u64 = 4_096;

fn validate_source(
    expected: &filesystem::Metadata,
    observed: &filesystem::Metadata,
) -> Result<(), Error> {
    if !observed.file_type().is_file()
        || observed.len() > MAX_SOURCE_BYTES_U64
        || observed.identity() != expected.identity()
        || observed.len() != expected.len()
        || observed.mtime() != expected.mtime()
        || observed.mtime_nsec() != expected.mtime_nsec()
        || observed.ctime() != expected.ctime()
        || observed.ctime_nsec() != expected.ctime_nsec()
    {
        return Err(Error::Input {
            message: "command source file changed during source admission or read",
            byte_offset: 0,
        });
    }
    Ok(())
}

pub(super) fn read<'buffer>(
    path: &Path,
    bytes: &'buffer mut [u8; MAX_SOURCE_BYTES + 1],
) -> Result<&'buffer str, Error> {
    let metadata = filesystem::symlink_metadata(path).map_err(|source| Error::Io {
        operation: "inspect command source file",
        source,
    })?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_SOURCE_BYTES_U64 {
        return Err(Error::Input {
            message: "command source file must be a regular non-symlink file of at most 4096 bytes",
            byte_offset: 0,
        });
    }
    let mut file = filesystem::open_read(path).map_err(|source| Error::Io {
        operation: "open command source file",
        source,
    })?;
    let opened = filesystem::file_metadata(&file).map_err(|source| Error::Io {
        operation: "inspect opened command source file",
        source,
    })?;
    validate_source(&metadata, &opened)?;
    // The extra byte detects growth beyond the admitted limit. At exactly
    // 4096 bytes, one more read must establish EOF before the text is accepted.
    let mut length = 0_usize;
    loop {
        let read = file
            .read(&mut bytes[length..])
            .map_err(|source| Error::Io {
                operation: "read command source file",
                source,
            })?;
        if read == 0 {
            break;
        }
        length = length.checked_add(read).ok_or(Error::Input {
            message: "command source file length overflow",
            byte_offset: 0,
        })?;
        if length > MAX_SOURCE_BYTES {
            return Err(Error::Input {
                message: "command source file exceeds 4096 bytes",
                byte_offset: MAX_SOURCE_BYTES_U64,
            });
        }
    }
    let final_metadata = filesystem::file_metadata(&file).map_err(|source| Error::Io {
        operation: "inspect command source file after read",
        source,
    })?;
    validate_source(&metadata, &final_metadata)?;
    // An unchanged open file does not prove that its pathname still names it.
    let named = filesystem::symlink_metadata(path).map_err(|source| Error::Io {
        operation: "inspect command source name after read",
        source,
    })?;
    validate_source(&metadata, &named)?;
    if u64::try_from(length).expect("bounded source length fits u64") != metadata.len() {
        return Err(Error::Input {
            message: "command source file read disagrees with admitted length",
            byte_offset: 0,
        });
    }
    let source = std::str::from_utf8(&bytes[..length]).map_err(|_| Error::Input {
        message: "command source file must be UTF-8",
        byte_offset: 0,
    })?;
    Ok(source)
}

#[cfg(test)]
mod tests {
    //! Exercise metadata validation with real files, another identity, a
    //! directory, a symlink, changed timestamps and the exact size boundary.
    //! These checks call the validator directly; they do not simulate a read race.

    use super::{MAX_SOURCE_BYTES, validate_source};
    use crate::test_support::Directory;
    use pipesql::Error;
    use pipesql_filesystem as filesystem;

    #[test]
    fn source_requires_matching_regular_bounded_unchanged_metadata() {
        use std::fs;
        let root = Directory::new();
        let first = root.0.join("source");
        let second = root.0.join("different-source");
        fs::write(&first, b"FROM lineitem").unwrap();
        fs::write(&second, b"FROM lineitem").unwrap();
        let expected = filesystem::symlink_metadata(&first).unwrap();
        let opened = filesystem::open_read(&first).unwrap();
        validate_source(
            &expected,
            &filesystem::Metadata::from_std(&opened.metadata().unwrap()),
        )
        .unwrap();
        for path in [&second, &root.0] {
            assert!(matches!(
                validate_source(&expected, &filesystem::symlink_metadata(path).unwrap()),
                Err(Error::Input { .. })
            ));
        }
        let link = root.0.join("link");
        std::os::unix::fs::symlink(&first, &link).unwrap();
        assert!(validate_source(&expected, &filesystem::symlink_metadata(&link).unwrap()).is_err());
        opened
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1))
            .unwrap();
        assert!(
            validate_source(
                &expected,
                &filesystem::Metadata::from_std(&opened.metadata().unwrap()),
            )
            .is_err()
        );
        for length in [MAX_SOURCE_BYTES, MAX_SOURCE_BYTES + 1] {
            fs::write(&first, vec![b' '; length]).unwrap();
            let observed = filesystem::symlink_metadata(&first).unwrap();
            assert!(validate_source(&expected, &observed).is_err());
            assert_eq!(
                validate_source(&observed, &observed).is_ok(),
                length == MAX_SOURCE_BYTES
            );
        }
    }
}
