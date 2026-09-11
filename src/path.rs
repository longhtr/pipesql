//! Bounded pathname construction and lexical admission.
//! Native canonicalization follows admission; it does not replace these checks.

use crate::Error;
use crate::error::io_error;
use crate::resources::{MemoryAuthority, Reservation};
use pipesql_filesystem as filesystem;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub(crate) const MAX_PATH_BYTES: usize = filesystem::MAX_PATH_BYTES;

/// Overflow traversal bytes drop before their reservation. Ordinary short paths
/// use the native walker's inline storage and never grow this owner.
pub(crate) struct CanonicalizeScratch<'database> {
    bytes: Vec<u8>,
    charge: Option<Reservation<'database>>,
    memory: &'database MemoryAuthority,
}

impl<'database> CanonicalizeScratch<'database> {
    pub(crate) fn new(memory: &'database MemoryAuthority) -> Self {
        Self {
            bytes: Vec::new(),
            charge: None,
            memory,
        }
    }
}

impl filesystem::PathScratch for CanonicalizeScratch<'_> {
    type Error = Error;

    fn bytes(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    fn grow(&mut self, required: usize) -> Result<(), Error> {
        assert!(required > self.bytes.len());
        assert!(required <= filesystem::MAX_CANONICALIZE_SCRATCH_BYTES);
        let capacity = required
            .next_power_of_two()
            .min(filesystem::MAX_CANONICALIZE_SCRATCH_BYTES);
        let charge = self
            .memory
            .reserve(capacity as u64 + 4_096, "pathname scratch")?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| Error::Resource {
                owner: "pathname scratch allocation",
                required: charge.bytes(),
                limit: self.memory.limit(),
            })?;
        if bytes.capacity() > capacity {
            return Err(Error::Resource {
                owner: "pathname scratch capacity",
                required: bytes.capacity() as u64,
                limit: capacity as u64,
            });
        }
        bytes.resize(capacity, 0);
        bytes[..self.bytes.len()].copy_from_slice(&self.bytes);
        // Both allocations are charged during copying. Release the old charge
        // only after dropping its physical storage.
        drop(std::mem::replace(&mut self.bytes, bytes));
        self.charge = Some(charge);
        Ok(())
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use filesystem::PathScratch;

    #[test]
    fn scratch_growth_admits_copy_overlap_and_preserves_state_on_refusal() {
        // 8-KiB old storage and 16-KiB replacement each carry a 4-KiB allowance.
        // One byte below their overlap must refuse, even though the final owner
        // would fit. Refusal cannot damage bytes still needed by the traversal.
        for limit in [32_767, 32_768] {
            let memory = MemoryAuthority::new(limit);
            let mut scratch = CanonicalizeScratch::new(&memory);
            scratch.grow(4_097).unwrap();
            scratch.bytes()[0] = 0xa5;
            let result = scratch.grow(8_193);
            if limit == 32_767 {
                assert!(matches!(
                    result,
                    Err(Error::Resource {
                        owner: "pathname scratch",
                        required: 32_768,
                        limit: 32_767,
                    })
                ));
                assert_eq!(scratch.bytes().len(), 8_192);
                assert_eq!(memory.reserved(), 12_288);
            } else {
                result.unwrap();
                assert_eq!(scratch.bytes().len(), 16_384);
                assert_eq!(memory.reserved(), 20_480);
            }
            assert_eq!(scratch.bytes()[0], 0xa5);
            drop(scratch);
            assert_eq!(memory.reserved(), 0);
        }
    }
}
