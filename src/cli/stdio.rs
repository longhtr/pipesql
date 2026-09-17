//! Own the CLI's standard descriptors and buffer stdout without heap allocation.
//!
//! Startup duplicates stdout and stderr into descriptors numbered at least 3.
//! CSV import captures stdin the same way before database files can reuse it.
//! Later database files can reuse a closed standard descriptor without becoming
//! an output destination. Each duplicate is owned by a `File` and closes on drop.
//!
//! `Output` holds a 1 KiB buffer. A newline flushes it; a full buffer is flushed
//! before accepting more bytes. The command must explicitly flush the final tail
//! and handle failure before returning success. Dropping Output never flushes.
//!
//! Short writes advance through the remaining bytes. Interrupted writes, zero
//! progress and other errors return immediately. A write can fail after sending
//! a prefix, so the caller must stop the command rather than retry the input.

use std::fs::File;
use std::io::{self, Write};
use std::os::fd::FromRawFd;

fn duplicate(descriptor: libc::c_int) -> io::Result<File> {
    // SAFETY: this fcntl command takes integer arguments and returns a new
    // descriptor owned by this call, or -1. Requiring a number >= 3 leaves the
    // inherited standard slots unchanged, including any that were closed.
    let owned = unsafe { libc::fcntl(descriptor, libc::F_DUPFD_CLOEXEC, 3) };
    if owned < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: owned is a valid new descriptor. Transfer its sole ownership to
    // File so dropping the Rust value closes it exactly once.
    Ok(unsafe { File::from_raw_fd(owned) })
}

pub(super) fn stdin() -> io::Result<File> {
    duplicate(libc::STDIN_FILENO)
}

pub(super) fn stderr() -> io::Result<File> {
    duplicate(libc::STDERR_FILENO)
}

// Stop on interruption instead of Write::write_all's default retry. Each
// successful iteration must consume bytes, so zero progress is also an error.
pub(super) fn write_all(output: &mut impl Write, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let written = output.write(bytes)?;
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        bytes = &bytes[written..];
    }
    Ok(())
}

pub(super) struct Output {
    file: File,
    bytes: [u8; 1024],
    start: usize,
    end: usize,
}

impl Output {
    pub(super) fn stdout() -> io::Result<Self> {
        Ok(Self {
            file: duplicate(libc::STDOUT_FILENO)?,
            bytes: [0; 1024],
            start: 0,
            end: 0,
        })
    }
}

impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.end == self.bytes.len() {
            self.flush()?;
        }
        let length = bytes.len().min(self.bytes.len() - self.end);
        self.bytes[self.end..self.end + length].copy_from_slice(&bytes[..length]);
        self.end += length;
        // Make complete lines visible promptly. This can send a prefix before
        // failing, so the command must not retry the original write on error.
        if bytes[..length].contains(&b'\n') {
            self.flush()?;
        }
        Ok(length)
    }

    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        write_all(self, bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        while self.start < self.end {
            let written = self.file.write(&self.bytes[self.start..self.end])?;
            if written == 0 {
                return Err(io::ErrorKind::WriteZero.into());
            }
            self.start += written;
        }
        self.start = 0;
        self.end = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! Observe real buffered writes through a socket, then use a controlled
    //! writer to check short writes, interruption and refusal to make progress.

    use super::{Output, duplicate, write_all};
    use std::fs::File;
    use std::io::{self, Write};

    #[test]
    fn duplicate_owns_a_separate_close_on_exec_descriptor() {
        use std::io::Read;
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;

        for close_original_first in [false, true] {
            let (mut original, mut receiver) = UnixStream::pair().unwrap();
            receiver
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut copied = duplicate(original.as_raw_fd()).unwrap();
            assert!(copied.as_raw_fd() >= 3);
            assert_ne!(copied.as_raw_fd(), original.as_raw_fd());
            // SAFETY: copied owns a live descriptor; F_GETFD takes no pointer.
            let flags = unsafe { libc::fcntl(copied.as_raw_fd(), libc::F_GETFD) };
            assert!(flags >= 0);
            assert_ne!(flags & libc::FD_CLOEXEC, 0);
            if close_original_first {
                drop(original);
                copied.write_all(b"x").unwrap();
                drop(copied);
            } else {
                drop(copied);
                original.write_all(b"x").unwrap();
                drop(original);
            }
            let mut bytes = Vec::new();
            receiver.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, b"x");
        }
        assert_eq!(duplicate(-1).unwrap_err().raw_os_error(), Some(libc::EBADF));
    }

    #[test]
    fn owned_buffer_flushes_lines_capacity_and_tail() {
        use std::io::Read;
        use std::os::fd::OwnedFd;
        use std::os::unix::net::UnixStream;
        let (sender, mut receiver) = UnixStream::pair().unwrap();
        receiver.set_nonblocking(true).unwrap();
        let mut output = Output {
            file: File::from(OwnedFd::from(sender)),
            bytes: [0; 1024],
            start: 0,
            end: 0,
        };
        output.write_all(b"x").unwrap();
        let mut bytes = [0; 1024];
        assert_eq!(
            receiver.read(&mut bytes).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        output.write_all(b"\n").unwrap();
        assert_eq!(receiver.read(&mut bytes).unwrap(), 2);
        assert_eq!(&bytes[..2], b"x\n");
        output.write_all(&[b'y'; 1025]).unwrap();
        assert_eq!(receiver.read(&mut bytes).unwrap(), 1024);
        assert_eq!(bytes, [b'y'; 1024]);
        output.flush().unwrap();
        assert_eq!(receiver.read(&mut bytes).unwrap(), 1);
        assert_eq!(bytes[0], b'y');
        assert_eq!(output.start, 0);
        assert_eq!(output.end, 0);
    }

    #[test]
    fn finite_writer_preserves_short_writes_and_refuses_no_progress() {
        struct Sink {
            result: io::Result<usize>,
            calls: usize,
        }

        impl Write for Sink {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                self.calls += 1;
                self.result.as_ref().copied().map_err(|e| e.kind().into())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut sink = Sink {
            result: Ok(1),
            calls: 0,
        };
        write_all(&mut sink, b"abc").unwrap();
        assert_eq!(sink.calls, 3);
        for kind in [io::ErrorKind::Interrupted, io::ErrorKind::BrokenPipe] {
            sink = Sink {
                result: Err(kind.into()),
                calls: 0,
            };
            assert_eq!(write_all(&mut sink, b"abc").unwrap_err().kind(), kind);
            assert_eq!(sink.calls, 1);
        }
        sink = Sink {
            result: Ok(0),
            calls: 0,
        };
        assert_eq!(
            write_all(&mut sink, b"abc").unwrap_err().kind(),
            io::ErrorKind::WriteZero
        );
        assert_eq!(sink.calls, 1);
    }
}
