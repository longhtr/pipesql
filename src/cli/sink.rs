//! Owned CLI sinks. Duplicate before engine entry so a closed inherited sink
//! cannot later alias a database descriptor. No lazy stdio heap allocation.
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::FromRawFd;

fn duplicate(descriptor: libc::c_int) -> io::Result<File> {
    // SAFETY: fcntl has no pointer argument here. It either refuses or returns
    // one new descriptor owned exclusively by this call, at least 3 (never an
    // inherited standard slot). The original descriptor remains untouched.
    let owned = unsafe { libc::fcntl(descriptor, libc::F_DUPFD_CLOEXEC, 3) };
    if owned < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: ownership transfers exactly once; File closes it on every exit.
    Ok(unsafe { File::from_raw_fd(owned) })
}

pub(super) fn stderr() -> io::Result<File> {
    duplicate(libc::STDERR_FILENO)
}

// Unlike Write::write_all's default, interruption is terminal. Each successful
// iteration decreases remaining bytes; no hidden EINTR retry or drop flush.
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
        // Preserve the CLI's line-buffered progress without allocator-owned
        // stdio state. A sink failure is terminal for the command; no drop retry.
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
    use super::{Output, write_all};
    use std::fs::File;
    use std::io::{self, Write};

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
