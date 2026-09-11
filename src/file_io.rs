//! Exact concrete-file I/O without std's hidden Interrupted retry loops.
//! Every successful native call consumes bytes; zero progress and every error
//! terminate. Callers own effect placement, buffer bounds and publication state.
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::fs::FileExt;

pub(crate) fn write_all(file: &mut File, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let written = file.write(bytes)?;
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        bytes = bytes.get(written..).ok_or(io::ErrorKind::InvalidData)?;
    }
    Ok(())
}

pub(crate) fn read_exact(file: &mut File, mut bytes: &mut [u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let read = file.read(bytes)?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        bytes = bytes.get_mut(read..).ok_or(io::ErrorKind::InvalidData)?;
    }
    Ok(())
}

// Unix pread/pwrite offsets and file lengths have a signed 64-bit ceiling on
// supported targets. Validate the complete byte extent before the first effect;
// never let std's u64-to-native conversion choose a negative offset for us.
fn validate_range(offset: u64, length: usize) -> io::Result<()> {
    let bytes = u64::try_from(length).map_err(|_| io::ErrorKind::InvalidInput)?;
    if offset
        .checked_add(bytes)
        .is_none_or(|end| end > i64::MAX.unsigned_abs())
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(())
}

pub(crate) fn write_all_at(file: &File, mut bytes: &[u8], mut offset: u64) -> io::Result<()> {
    validate_range(offset, bytes.len())?;
    while !bytes.is_empty() {
        let written = file.write_at(bytes, offset)?;
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        bytes = bytes.get(written..).ok_or(io::ErrorKind::InvalidData)?;
        offset = offset
            .checked_add(u64::try_from(written).map_err(|_| io::ErrorKind::InvalidData)?)
            .ok_or(io::ErrorKind::InvalidData)?;
    }
    Ok(())
}

pub(crate) fn read_exact_at(file: &File, mut bytes: &mut [u8], mut offset: u64) -> io::Result<()> {
    validate_range(offset, bytes.len())?;
    while !bytes.is_empty() {
        let read = file.read_at(bytes, offset)?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        bytes = bytes.get_mut(read..).ok_or(io::ErrorKind::InvalidData)?;
        offset = offset
            .checked_add(u64::try_from(read).map_err(|_| io::ErrorKind::InvalidData)?)
            .ok_or(io::ErrorKind::InvalidData)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positional_extent_refuses_before_native_narrowing() {
        let maximum = i64::MAX.unsigned_abs();
        for (offset, length) in [(0, 0), (0, 1), (maximum, 0), (maximum - 1, 1)] {
            validate_range(offset, length).unwrap();
        }
        for (offset, length) in [
            (maximum, 1),
            (maximum + 1, 0),
            (u64::MAX, 1),
            (maximum, usize::MAX),
        ] {
            assert_eq!(
                validate_range(offset, length).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
}
