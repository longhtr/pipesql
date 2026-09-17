//! Read or write a complete buffer, handling short filesystem transfers.
//!
//! A successful OS call may transfer only part of a buffer. These helpers continue
//! from the remaining bytes until the request is complete. Ordinary transfers
//! advance the file's cursor; the `_at` functions use an explicit offset without
//! moving that cursor. Files and buffers belong to the caller.
//!
//! An error may follow a partial transfer. These helpers neither undo written
//! bytes nor synchronize them to durable storage. They return all OS errors,
//! including `Interrupted`, instead of silently retrying them. This lets callers
//! control retries and lets fault-injection tests observe each failed attempt.
//!
//! A zero-byte read before the buffer is full returns `UnexpectedEof`. A zero-byte
//! write returns `WriteZero`. Both stop the loop instead of repeating a transfer
//! that makes no progress.
//!
//! Callers check cancellation around the complete request, not between positive
//! short transfers here. A byte bound does not impose a wall-clock deadline on
//! the OS calls needed to transfer those bytes.

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

// These Unix calls use signed 64-bit offsets, while Rust accepts a u64 here.
// Reject a range whose end exceeds that limit before transferring any bytes;
// conversion to the native type must not turn a large offset into a negative one.
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
