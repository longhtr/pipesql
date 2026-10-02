//! Provide the native filesystem and mutex operations used by the database engine.
//!
//! Callers pass paths and borrowed files through safe Rust functions. Native
//! pointers and platform-specific layouts stay inside `syscall` and `mutex`;
//! successful opens return owned `File` handles. `metadata` gives pathname and
//! open-file observations a common representation for identity checks.
//!
//! Path resolution limits native calls and uses caller-accounted overflow storage.
//! Directory traversal returns one borrowed name at a time from a fixed buffer.
//! Each operation reports failure to its caller; it does not decide whether a
//! database transaction committed or which files recovery may remove.
//!
//! The engine chooses when to synchronize, where to check cancellation and how
//! to recover. This crate implements those operations on macOS and Linux. It is
//! private to PipeSQL and does not promise a stable API for other applications.

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("the bounded filesystem boundary supports macOS and Linux only");

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::ops::Range;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

mod metadata;

#[allow(unsafe_code)]
mod mutex;
pub use mutex::{LockError, Mutex, MutexGuard};

#[allow(unsafe_code)]
mod syscall;
pub use metadata::{FileIdentity, FileType, Metadata};

/// Return the native-reported stack extent of the current thread, in bytes.
/// This test-only observation does not measure live frames or resident memory.
#[cfg(feature = "test-stack-observation")]
pub fn test_current_thread_stack_bytes() -> usize {
    syscall::test_current_thread_stack_bytes()
}

/// Request used by bounded-thread regressions; the runtime may allocate more.
#[cfg(feature = "test-stack-observation")]
pub const TEST_SMALL_STACK_REQUEST_BYTES: usize = 48 * 1024;

/// Largest native-reported stack extent accepted by the small-stack tests.
/// The tested GNU arm64 runtime needs 128 KiB plus thread-local/runtime storage;
/// allow up to 16 KiB for that overhead. Other targets use a 64-KiB ceiling.
/// A runtime that exceeds its ceiling fails the check rather than widening it.
#[cfg(feature = "test-stack-observation")]
pub const TEST_SMALL_STACK_LIMIT_BYTES: usize = if cfg!(all(
    target_os = "linux",
    target_arch = "aarch64",
    target_env = "gnu"
)) {
    144 * 1024
} else {
    64 * 1024
};

/// Check the current thread's stack extent before running a small-stack test.
/// The scenario must still run successfully; this assertion alone proves only
/// that the reported extent fits the test's ceiling.
#[cfg(feature = "test-stack-observation")]
pub fn test_assert_small_stack() {
    let reported = test_current_thread_stack_bytes();
    assert!(
        reported > 0 && reported <= TEST_SMALL_STACK_LIMIT_BYTES,
        "reported thread stack {reported} outside 1..={TEST_SMALL_STACK_LIMIT_BYTES}"
    );
}

/// Inspect a borrowed open file in the same representation as pathname metadata.
/// The descriptor remains owned by the caller; this does not follow its old name.
pub fn file_metadata(file: &File) -> io::Result<Metadata> {
    file.metadata().map(|raw| Metadata::from_std(&raw))
}

/// Request synchronization once: F_FULLFSYNC on macOS, fsync on Linux.
/// Return interrupted or unsupported errors without retrying or choosing a weaker
/// operation. The caller keeps the file open and decides when the database protocol
/// requires this call. Success is not, by itself, evidence for an entire platform's
/// durability under failure.
pub fn sync_all(file: &File) -> io::Result<()> {
    syscall::sync_all(file)
}

/// Inspect the named object; if its final component is a symlink, inspect the
/// link itself. This neither opens the object for data I/O nor changes it.
pub fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    syscall::metadata(path.as_ref(), false)
}

/// Inspect the final target, following symlinks, without opening it for data I/O.
pub fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    syscall::metadata(path.as_ref(), true)
}

/// Open for reading and transfer the new descriptor to the caller.
/// Use nonblocking open so a substituted FIFO cannot wait for a writer. Regular
/// file I/O still has its ordinary behavior, and the caller must check the opened
/// object's type and identity before trusting it as database data.
pub fn open_read(path: impl AsRef<Path>) -> io::Result<File> {
    syscall::open_read(path.as_ref())
}

/// Open an existing file for in-place reads/writes, without creating/truncating it.
/// As with open_read, the caller owns descriptor validation and synchronization.
pub fn open_read_write(path: impl AsRef<Path>) -> io::Result<File> {
    syscall::open_read_write(path.as_ref())
}

/// Maximum native helper calls during one path resolution. Refuse the next call
/// after this limit; it bounds call count, not kernel work or elapsed time.
pub const MAX_NATIVE_PATH_CALLS: u32 = 65_536;

/// Extra storage supplied by the caller when path traversal outgrows its inline
/// buffer. Implementations must reserve memory before growing and preserve the
/// old bytes even if growth fails.
pub trait PathScratch {
    type Error;

    /// Initialized bytes available to the traversal; the caller retains ownership.
    fn bytes(&mut self) -> &mut [u8];
    /// Grow beyond the current length, preserving bytes on success and failure.
    /// `required` never exceeds MAX_CANONICALIZE_SCRATCH_BYTES. Success must make
    /// at least that many initialized bytes available through `bytes`.
    fn grow(&mut self, required: usize) -> Result<(), Self::Error>;
}

const LINUX_SYMLINKS: usize = 40;
/// A bounded input plus at most forty bounded Linux symlink targets.
pub const MAX_CANONICALIZE_SCRATCH_BYTES: usize = (LINUX_SYMLINKS + 1) * MAX_PATH_BYTES;

#[derive(Debug)]
pub enum CanonicalizeError<ScratchError = std::convert::Infallible> {
    Io(io::Error),
    WorkLimit,
    Scratch(ScratchError),
}

impl<E> From<io::Error> for CanonicalizeError<E> {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Resolve an absolute path, including symlinks, and return its canonical name.
/// Linux may request scratch growth to hold unresolved path suffixes; macOS uses
/// fixed storage. The native-call limit applies separately to each resolution.
/// Check cancellation around this synchronous call and validate opened-file
/// identity afterward: the returned name cannot prevent later filesystem changes.
/// Failure returns no partial name.
pub fn canonicalize<S: PathScratch>(
    path: impl AsRef<Path>,
    scratch: &mut S,
) -> Result<std::path::PathBuf, CanonicalizeError<S::Error>> {
    let path = path.as_ref();
    if !path.is_absolute() {
        return Err(CanonicalizeError::Io(io::ErrorKind::InvalidInput.into()));
    }
    let mut resolved = [0; MAX_PATH_BYTES + 1];

    #[cfg(target_os = "macos")]
    let _ = scratch;
    #[cfg(target_os = "macos")]
    let length = syscall::canonicalize(path, &mut resolved).map_err(|error| match error {
        CanonicalizeError::Io(error) => CanonicalizeError::Io(error),
        CanonicalizeError::WorkLimit => CanonicalizeError::WorkLimit,
        CanonicalizeError::Scratch(never) => match never {},
    })?;

    #[cfg(target_os = "linux")]
    let length = syscall::canonicalize(path, &mut resolved, scratch)?;
    let mut owned = std::path::PathBuf::new();
    owned
        .try_reserve_exact(length)
        .map_err(|_| CanonicalizeError::Io(io::ErrorKind::OutOfMemory.into()))?;
    if owned.capacity() > MAX_PATH_BYTES {
        return Err(CanonicalizeError::Io(io::ErrorKind::OutOfMemory.into()));
    }
    owned.push(OsStr::from_bytes(&resolved[..length]));
    Ok(owned)
}

/// Exclusive creation, mode 0666 subject to the process umask. Existing objects
/// are never opened or truncated. The caller owns writes and required flushes.
pub fn create_new_read_write(path: impl AsRef<Path>) -> io::Result<File> {
    syscall::create_new_read_write(path.as_ref())
}

pub fn create_dir(path: impl AsRef<Path>) -> io::Result<()> {
    syscall::create_dir(path.as_ref())
}

pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    syscall::remove_file(path.as_ref())
}

pub fn remove_dir(path: impl AsRef<Path>) -> io::Result<()> {
    syscall::remove_dir(path.as_ref())
}

/// Rename with one native call after checking both path lengths and NUL bytes.
/// This can replace an existing destination. It performs no synchronization;
/// the caller must complete any durability protocol after the rename.
pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    syscall::rename(from.as_ref(), to.as_ref())
}

pub fn hard_link(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    syscall::hard_link(from.as_ref(), to.as_ref())
}

pub const MAX_PATH_BYTES: usize = 4_096;
const BUFFER_BYTES: usize = 8_192;
const MAX_RAW_ENTRIES: usize = 64;
// Missing in pinned libc; independently checked against sys/attr.h in the gate.
#[cfg(target_os = "macos")]
const ATTR_CMN_ERROR: libc::attrgroup_t = 0x2000_0000;

/// Reusable 8-KiB directory-read buffer supplied by the caller.
#[repr(align(8))]
pub struct DirectoryBuffer {
    bytes: [u8; BUFFER_BYTES],
}

impl Default for DirectoryBuffer {
    fn default() -> Self {
        Self {
            bytes: [0; BUFFER_BYTES],
        }
    }
}

/// Read directory names using a private cursor and the caller's fixed buffer.
///
/// Each name borrows the buffer until the next mutable call. The descriptor stays
/// private so no other reader can advance the cursor. Each call examines at most
/// 65 native records and performs at most 65 reads; even skipped dot entries and
/// vacant records count toward these limits.
///
/// End-of-directory and errors stop the cursor permanently. Propagate the first
/// error: later calls return `None`, not that error again. The caller checks
/// cancellation between names and decides which names the database permits.
pub struct Directory<'buffer> {
    file: File,
    buffer: &'buffer mut DirectoryBuffer,
    cursor: usize,
    end: usize,
    raw_entries: usize,
    raw_limit: usize,
    done: bool,
}

impl<'buffer> Directory<'buffer> {
    /// Open a new cursor with a total limit of 64 native records, including dot
    /// and vacant entries. Use `open_bounded` when a larger inventory is expected.
    pub fn open(path: &Path, buffer: &'buffer mut DirectoryBuffer) -> io::Result<Self> {
        Self::open_bounded(path, buffer, MAX_RAW_ENTRIES)
    }

    /// Open with a nonzero total record limit. Dot and vacant entries count too.
    /// The per-call limit still applies; exceeding either returns an error and
    /// permanently stops the cursor.
    pub fn open_bounded(
        path: &Path,
        buffer: &'buffer mut DirectoryBuffer,
        raw_limit: usize,
    ) -> io::Result<Self> {
        if raw_limit == 0 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(Self {
            file: syscall::open_directory(path)?,
            buffer,
            cursor: 0,
            end: 0,
            raw_entries: 0,
            raw_limit,
            done: false,
        })
    }

    pub fn next_name(&mut self) -> io::Result<Option<&OsStr>> {
        if self.done {
            return Ok(None);
        }
        // Stop on any error, including one after partial buffer consumption.
        // `advance` returns offsets so we can update state before lending the name.
        self.done = true;
        match self.advance()? {
            None => Ok(None),
            Some(range) => {
                self.done = false;
                Ok(Some(OsStr::from_bytes(&self.buffer.bytes[range])))
            }
        }
    }

    fn advance(&mut self) -> io::Result<Option<Range<usize>>> {
        // Count skipped records as work too. A directory full of dot or vacant
        // entries must not keep one call running without a bound.
        for _ in 0..=MAX_RAW_ENTRIES {
            if self.cursor == self.end {
                self.end = syscall::read_directory(&self.file, self.buffer)?;
                self.cursor = 0;
                if self.end == 0 {
                    return Ok(None);
                }
            }
            if self.raw_entries == self.raw_limit {
                return Err(io::ErrorKind::InvalidData.into());
            }
            let record = decode(&self.buffer.bytes[self.cursor..self.end])?;
            let start = self
                .cursor
                .checked_add(record.name.start)
                .ok_or(io::ErrorKind::InvalidData)?;
            let end = self
                .cursor
                .checked_add(record.name.end)
                .ok_or(io::ErrorKind::InvalidData)?;
            self.cursor = self
                .cursor
                .checked_add(record.length)
                .ok_or(io::ErrorKind::InvalidData)?;
            self.raw_entries += 1;
            let name = &self.buffer.bytes[start..end];
            if !record.vacant && name != b"." && name != b".." {
                return Ok(Some(start..end));
            }
        }
        Err(io::ErrorKind::InvalidData.into())
    }
}

struct Record {
    length: usize,
    name: Range<usize>,
    vacant: bool,
}

#[cfg(target_os = "macos")]
fn u32_at(bytes: &[u8], start: usize) -> io::Result<u32> {
    let end = start.checked_add(4).ok_or(io::ErrorKind::InvalidData)?;
    let word = bytes.get(start..end).ok_or(io::ErrorKind::InvalidData)?;
    Ok(u32::from_ne_bytes(
        word.try_into().expect("validated four-byte word"),
    ))
}

#[cfg(target_os = "macos")]
fn record_length(bytes: &[u8]) -> io::Result<usize> {
    let length = usize::try_from(u32_at(bytes, 0)?).map_err(|_| io::ErrorKind::InvalidData)?;
    if length < 40 || !length.is_multiple_of(8) || length > bytes.len() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(length)
}

#[cfg(target_os = "macos")]
fn decode(bytes: &[u8]) -> io::Result<Record> {
    let length = record_length(bytes)?;
    let bytes = &bytes[..length];
    let flags = u32_at(bytes, 4)?;
    let required = libc::ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_ERROR;
    if flags & required != required
        || flags & !(required | libc::ATTR_CMN_NAME) != 0
        || bytes[8..24].iter().any(|byte| *byte != 0)
    {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let error = i32::try_from(u32_at(bytes, 24)?).map_err(|_| io::ErrorKind::InvalidData)?;
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error));
    }
    if flags & libc::ATTR_CMN_NAME == 0 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let offset = i32::from_ne_bytes(bytes[28..32].try_into().expect("fixed name reference"));
    let offset = usize::try_from(offset).map_err(|_| io::ErrorKind::InvalidData)?;
    if offset < 8 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    // Darwin stores the name offset relative to the attribute reference at byte
    // 28, not to the start of the record. Keep both the reference and name in bounds.
    let start = 28_usize
        .checked_add(offset)
        .ok_or(io::ErrorKind::InvalidData)?;
    let size = usize::try_from(u32_at(bytes, 32)?).map_err(|_| io::ErrorKind::InvalidData)?;
    let end = start.checked_add(size).ok_or(io::ErrorKind::InvalidData)?;
    if !(2..=1_024).contains(&size) {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let name = bytes.get(start..end).ok_or(io::ErrorKind::InvalidData)?;
    validate_name(name)?;
    Ok(Record {
        length,
        name: start..end - 1,
        vacant: false,
    })
}

#[cfg(target_os = "linux")]
fn decode(bytes: &[u8]) -> io::Result<Record> {
    // Linux getdents64 UAPI: ino64, off64, reclen16, type8, NUL-terminated name.
    const NAME: usize = 19;
    if bytes.len() < NAME + 1 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let length = usize::from(u16::from_ne_bytes(
        bytes[16..18].try_into().expect("fixed record length"),
    ));
    if length < NAME + 1 || !length.is_multiple_of(8) || length > bytes.len() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let bytes = &bytes[..length];
    let nul = bytes[NAME..]
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(io::ErrorKind::InvalidData)?;
    let end = NAME
        .checked_add(nul)
        .and_then(|n| n.checked_add(1))
        .ok_or(io::ErrorKind::InvalidData)?;
    validate_name(&bytes[NAME..end])?;
    Ok(Record {
        length,
        name: NAME..end - 1,
        vacant: bytes[..8].iter().all(|byte| *byte == 0),
    })
}

fn validate_name(name: &[u8]) -> io::Result<()> {
    if name.len() < 2
        || name.last() != Some(&0)
        || name[..name.len() - 1]
            .iter()
            .any(|byte| *byte == 0 || *byte == b'/')
    {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "../../test/support/cleanup.rs"]
mod test_cleanup;
