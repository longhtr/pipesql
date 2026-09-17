//! Write export bytes with explicit progress and cancellation checks.
//!
//! Both formats keep completion and byte limits with their encoders. This shared
//! loop handles caller writers: short writes advance, while no progress, I/O
//! errors and invalid returned counts fail without an unbounded retry.
//!
//! A writer error is preserved as an output cause. Cancellation is checked before
//! each further write, so already accepted bytes remain the caller's responsibility.
//! This helper neither flushes the stream nor appends a format completion marker.

use crate::{CancellationToken, Error};
use std::io::Write;

pub(crate) fn write_all(
    output: &mut dyn Write,
    mut bytes: &[u8],
    cancel: &CancellationToken,
) -> Result<(), Error> {
    while !bytes.is_empty() {
        cancel.check()?;
        let written = output.write(bytes).map_err(output_error)?;
        if written == 0 {
            return Err(output_error(std::io::ErrorKind::WriteZero.into()));
        }
        if written > bytes.len() {
            return Err(output_error(std::io::ErrorKind::InvalidData.into()));
        }
        bytes = &bytes[written..];
    }
    Ok(())
}

pub(crate) fn output_error(source: std::io::Error) -> Error {
    Error::Io {
        operation: "write result export",
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impossible_writer_count_fails_instead_of_claiming_completion() {
        struct Invalid;
        impl Write for Invalid {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                Ok(bytes.len() + 1)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                panic!("invalid writer must not flush")
            }
        }
        assert!(
            matches!(write_all(&mut Invalid, b"test", &CancellationToken::new()),
            Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::InvalidData)
        );
    }
}
