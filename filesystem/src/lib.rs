//! Private bounded OS effects, not a stable API. Engine semantics stay in PipeSQL.
//!
//! Directory owns a freshly opened, unshared cursor and borrows one 8-KiB buffer.
//! Names borrow that buffer until the next mutable call. Neither descriptor nor
//! mutable bytes escape. EOF/error is terminal; drop closes the sole descriptor.
//! Each call either yields a name, finishes, or fails within 65 raw records and
//! at most 65 bounded reads per call. Callers set the finite total raw-record
//! limit; the default namespace reader permits 64 records. No retries, callbacks, locks,
//! background work, or implicit directory-sized caches. Callers own cancellation
//! checks between names and their stricter semantic entry limits.

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

/// Test-only current-thread observation in bytes, not VM mapping or residency.
/// No thread handle or pointer escapes. Ordinary builds omit this entry point.
#[cfg(feature = "test-stack-observation")]
pub fn test_current_thread_stack_bytes() -> usize {
    syscall::test_current_thread_stack_bytes()
}

/// Inspect a borrowed open file in the same representation as pathname metadata.
/// The descriptor remains owned by the caller; this does not follow its old name.
pub fn file_metadata(file: &File) -> io::Result<Metadata> {
    file.metadata().map(|raw| Metadata::from_std(&raw))
}

/// One synchronous durability attempt on a borrowed descriptor. Darwin requires
/// F_FULLFSYNC; Linux uses fsync (not a platform qualification). Interrupted and
/// unsupported calls return their original error, without retry or weaker fallback.
/// The caller retains publication/cancellation/cleanup authority and file lifetime.
pub fn sync_all(file: &File) -> io::Result<()> {
    syscall::sync_all(file)
}

/// Inspect the named object without following its final symlink. No descriptor,
/// pathname allocation, mutation, or repair authority is acquired.
pub fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    syscall::metadata(path.as_ref(), false)
}

/// Inspect the final target, following symlinks, without opening it for data I/O.
pub fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    syscall::metadata(path.as_ref(), true)
}

/// A fresh read-only descriptor, transferred to the caller. Nonblocking admission
/// prevents waiting for a writer on a substituted FIFO; regular-file I/O retains
/// ordinary semantics. Callers must validate the opened object's type/identity.
pub fn open_read(path: impl AsRef<Path>) -> io::Result<File> {
    syscall::open_read(path.as_ref())
}

/// Open an existing file for in-place reads/writes, without creating/truncating it.
/// As with open_read, the caller owns descriptor validation and synchronization.
pub fn open_read_write(path: impl AsRef<Path>) -> io::Result<File> {
    syscall::open_read_write(path.as_ref())
}

/// macOS work admission counts native helper entries, not kernel-internal work
/// or wall time. It admits the retained 16,150-call deep-prefix case with room for
/// fallback metadata lookups; more expensive shapes return a typed refusal.
pub const MAX_NATIVE_PATH_CALLS: u32 = 65_536;

#[derive(Debug)]
pub enum CanonicalizeError {
    Io(io::Error),
    WorkLimit,
}

impl From<io::Error> for CanonicalizeError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Canonicalize an absolute bounded pathname. macOS traversal has call-local
/// state and a native-call budget; Linux's foreign resolver remains unqualified.
/// Callers own cancellation around this synchronous operation and must validate
/// physical identity independently. No partial name escapes on failure.
pub fn canonicalize(path: impl AsRef<Path>) -> Result<std::path::PathBuf, CanonicalizeError> {
    let path = path.as_ref();
    if !path.is_absolute() {
        return Err(CanonicalizeError::Io(io::ErrorKind::InvalidInput.into()));
    }
    let mut resolved = [0; MAX_PATH_BYTES + 1];

    #[cfg(target_os = "macos")]
    let length = syscall::canonicalize(path, &mut resolved)?;

    #[cfg(target_os = "linux")]
    let length = syscall::canonicalize(path, &mut resolved).map_err(CanonicalizeError::Io)?;
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

/// Both bounded names are validated before the one native mutation. No retry,
/// allocation, synchronization or implied durability acknowledgement occurs.
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

/// Caller-owned fixed workspace; never returned by a directory read or allocated.
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
    /// Opens an independent directory description; does not borrow/duplicate a
    /// caller's cursor or permit mixing native directory-reading mechanisms.
    pub fn open(path: &Path, buffer: &'buffer mut DirectoryBuffer) -> io::Result<Self> {
        Self::open_bounded(path, buffer, MAX_RAW_ENTRIES)
    }

    /// Total raw records include dot and vacant records. Per-call work remains
    /// bounded independently; exhausting either limit fuses the cursor.
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
        // Errors fuse the reader even when the caller retains it. advance returns
        // offsets, not a reference that would overlap this state mutation.
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
        // Every iteration consumes one raw record or terminates. Dot entries and
        // vacant Linux records count too, so skipping cannot conceal unbounded work.
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
