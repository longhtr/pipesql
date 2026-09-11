//! Bounded pathname construction and lexical admission.
//! Native canonicalization follows admission; it does not replace these checks.

use crate::Error;
use crate::error::io_error;
use pipesql_filesystem as filesystem;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub(crate) const MAX_PATH_BYTES: usize = filesystem::MAX_PATH_BYTES;

// Unix Path::join semantics, with one bounded fallible allocation before either
// push. PathBuf::join first clones the parent and may then grow infallibly.
pub(crate) fn try_join_path(parent: &Path, child: impl AsRef<Path>) -> io::Result<PathBuf> {
    let child = child.as_ref();
    let parent = if child.is_absolute() {
        Path::new("")
    } else {
        parent
    };
    let bytes = parent.as_os_str().as_bytes();
    let separator = usize::from(!bytes.is_empty() && !bytes.ends_with(b"/"));
    let required = bytes
        .len()
        .checked_add(separator)
        .and_then(|length| length.checked_add(child.as_os_str().len()))
        .filter(|length| *length <= MAX_PATH_BYTES)
        .ok_or(io::ErrorKind::InvalidInput)?;
    let mut path = PathBuf::new();
    path.try_reserve_exact(required)
        .map_err(|_| io::ErrorKind::OutOfMemory)?;
    if path.capacity() > MAX_PATH_BYTES {
        return Err(io::ErrorKind::OutOfMemory.into());
    }
    path.push(parent);
    path.push(child);
    assert_eq!(
        path.as_os_str().len(),
        required,
        "reserved Unix join geometry"
    );
    Ok(path)
}

pub(crate) fn joined_path(parent: &Path, child: impl AsRef<Path>) -> Result<PathBuf, Error> {
    try_join_path(parent, child).map_err(|source| io_error("construct filesystem path", source))
}

pub(crate) fn native_path_work_limit() -> Error {
    let limit = u64::from(filesystem::MAX_NATIVE_PATH_CALLS);
    Error::Resource {
        owner: "pathname native calls",
        required: limit + 1, // u32 capacity plus one is representable in u64.
        limit,
    }
}

pub(crate) fn validate_requested_path(path: &Path) -> Result<(), Error> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_PATH_BYTES {
        return Err(Error::InvalidPath("path length is outside 1..=4096 bytes"));
    }
    if bytes.contains(&0) {
        return Err(Error::InvalidPath("path contains a NUL byte"));
    }
    if !path.is_absolute() {
        return Err(Error::InvalidPath("path must be absolute"));
    }
    // Path::components normalizes interior `.` components away. Admission is
    // lexical, before canonicalization, so inspect the bounded Unix path bytes.
    if bytes
        .split(|byte| *byte == b'/')
        .any(|part| part == b"." || part == b"..")
    {
        return Err(Error::InvalidPath("path may not contain dot components"));
    }
    match path.file_name() {
        Some(name) if !name.as_bytes().is_empty() => Ok(()),
        _ => Err(Error::InvalidPath("path has no final component")),
    }
}
