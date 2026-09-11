//! Allocation-free, UTF-8-safe diagnostics with a fixed byte bound.
use std::io::{self, Write};

const MAX_DIAGNOSTIC_BYTES: usize = 4_096;

pub(super) struct Diagnostic {
    bytes: [u8; MAX_DIAGNOSTIC_BYTES],
    length: usize,
}

impl Diagnostic {
    pub(super) fn new(prefix: &str, message: &impl std::fmt::Display) -> Self {
        use std::fmt::Write;
        let mut buffer = Diagnostic {
            bytes: [0; MAX_DIAGNOSTIC_BYTES],
            length: 0,
        };
        // A capacity refusal (or a failing Display) deliberately retains the bounded
        // prefix already formatted. Diagnostic failure must not replace the exit code.
        let _ = write!(&mut buffer, "{prefix}: {message}");
        buffer.bytes[buffer.length] = b'\n';
        buffer.length += 1;
        buffer
    }

    pub(super) fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}

impl std::fmt::Write for Diagnostic {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        // Reserve one byte for LF; truncate only at a UTF-8 boundary. Returning
        // fmt::Error stops further formatting rather than allocating then cutting.
        let available = self.bytes.len() - 1 - self.length;
        let mut end = value.len().min(available);
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        self.bytes[self.length..self.length + end].copy_from_slice(&value.as_bytes()[..end]);
        self.length += end;
        if end < value.len() {
            Err(std::fmt::Error)
        } else {
            Ok(())
        }
    }
}

fn write_diagnostic(
    output: &mut impl Write,
    prefix: &str,
    message: &impl std::fmt::Display,
) -> io::Result<()> {
    let buffer = Diagnostic::new(prefix, message);
    super::sink::write_all(output, buffer.as_bytes())
}

pub(super) fn print_error(
    output: &mut Option<std::fs::File>,
    prefix: &str,
    message: &impl std::fmt::Display,
) {
    // There is no secondary diagnostic sink. A missing or broken stderr cannot
    // replace the command's nonzero exit status, panic, or reopen descriptor 2.
    if let Some(output) = output {
        let _ = write_diagnostic(output, prefix, message);
    }
}

#[cfg(test)]
mod tests {
    use super::{Diagnostic, MAX_DIAGNOSTIC_BYTES, write_diagnostic};
    use std::io::{self, Write};

    #[test]
    fn broken_diagnostic_sink_is_an_error_not_a_panic() {
        struct Broken;

        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let error = write_diagnostic(&mut Broken, "database error", &"failed").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        let mut output = Vec::new();
        write_diagnostic(&mut output, "database error", &"failed").unwrap();
        assert_eq!(output, b"database error: failed\n");
    }

    #[test]
    fn diagnostics_are_byte_bounded() {
        let message = "é".repeat(MAX_DIAGNOSTIC_BYTES);
        let rendered = Diagnostic::new("database error", &message);
        assert!(rendered.as_bytes().len() <= MAX_DIAGNOSTIC_BYTES);
        assert!(std::str::from_utf8(rendered.as_bytes()).is_ok());
        let rendered = Diagnostic::new(&message, &message);
        assert!(rendered.as_bytes().len() <= MAX_DIAGNOSTIC_BYTES);
        assert!(std::str::from_utf8(rendered.as_bytes()).is_ok());
    }
}
