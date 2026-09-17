//! Resolve an absolute Linux path by reading its components and symlink targets.
//!
//! Keep two parts: `ResolvedName` is the path processed so far, and `Pending` is
//! the suffix still to read. On a symlink, put its target before the unread suffix.
//! A relative target continues from the link's parent; an absolute target starts
//! again at root. Keeping the suffix avoids rereading a symlink that may change.
//!
//! Symlink expansion can temporarily exceed the final path-length limit. Pending
//! bytes start in an inline buffer and move to caller-accounted scratch only when
//! needed. A refused growth leaves the old bytes intact and returns the caller's
//! error. Separate limits bound symlink expansions and native calls.
//!
//! Dot components and trailing slashes still require the preceding object to be
//! a directory. Check that before simplifying the spelling: `/file/..` must fail
//! if `file` is a regular file. The resolved name is an observation, not a lock;
//! database code must still check the identities of files it opens afterward.

use crate::{CanonicalizeError, MAX_NATIVE_PATH_CALLS, MAX_PATH_BYTES, PathScratch};
use std::ffi::CStr;
use std::io;
use std::ops::Range;

const MAX_SYMLINKS: usize = crate::LINUX_SYMLINKS;

type Result<T, E> = std::result::Result<T, CanonicalizeError<E>>;

fn native_error<E>(code: i32) -> CanonicalizeError<E> {
    CanonicalizeError::Io(io::Error::from_raw_os_error(code))
}

struct NativeWork {
    remaining: u32,
}

impl NativeWork {
    fn before_call<E>(&mut self) -> Result<(), E> {
        if self.remaining == 0 {
            return Err(CanonicalizeError::WorkLimit);
        }
        self.remaining -= 1;
        Ok(())
    }

    fn file_type<E>(&mut self, path: &CStr) -> Result<libc::mode_t, E> {
        self.before_call()?;
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::zeroed();
        // SAFETY: live terminated input and correctly aligned output for the
        // target ABI. The synchronous call retains neither pointer.
        let status = unsafe { libc::lstat(path.as_ptr(), metadata.as_mut_ptr()) };
        if status != 0 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: lstat succeeded; reserved scalar fields were zeroed.
        Ok(unsafe { metadata.assume_init() }.st_mode & libc::S_IFMT)
    }

    fn link_target<E>(
        &mut self,
        path: &CStr,
        output: &mut [u8; MAX_PATH_BYTES + 1],
    ) -> Result<usize, E> {
        self.before_call()?;
        // SAFETY: independent live input/output storage and its exact writable
        // length. readlink neither appends a NUL nor retains either pointer.
        let length =
            unsafe { libc::readlink(path.as_ptr(), output.as_mut_ptr().cast(), output.len()) };
        if length < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let length = usize::try_from(length).expect("nonnegative readlink length");
        if length > MAX_PATH_BYTES {
            return Err(native_error(libc::ENAMETOOLONG));
        }
        if length == 0 || output[..length].contains(&0) {
            return Err(native_error(libc::EIO));
        }
        Ok(length)
    }
}

struct ResolvedName<'buffer> {
    bytes: &'buffer mut [u8; MAX_PATH_BYTES + 1],
    len: usize,
}

impl ResolvedName<'_> {
    fn cstr(&self) -> &CStr {
        CStr::from_bytes_with_nul(&self.bytes[..=self.len]).expect("one terminal NUL")
    }

    fn truncate(&mut self, len: usize) {
        assert!(len >= 1 && len <= self.len);
        self.len = len;
        self.bytes[len] = 0;
    }

    fn parent(&mut self) {
        if self.len > 1 {
            let slash = self.bytes[..self.len]
                .iter()
                .rposition(|byte| *byte == b'/')
                .expect("absolute resolved name");
            self.truncate(slash.max(1));
        }
    }

    fn require_directory<E>(&mut self, work: &mut NativeWork) -> Result<(), E> {
        if self.len == 1 {
            return Ok(()); // Root was inspected before traversal.
        }
        // Append the slash before lstat so the kernel checks directory traversal.
        // The length check also preserves ENAMETOOLONG before ENOTDIR when the
        // prefix is already at the path limit.
        let length = self.len;
        self.append(b"")?;
        let result = work.file_type(self.cstr());
        self.truncate(length);
        if result? != libc::S_IFDIR {
            return Err(native_error(libc::ENOTDIR));
        }
        Ok(())
    }

    fn append<E>(&mut self, component: &[u8]) -> Result<(), E> {
        let separator = usize::from(self.len > 1);
        let required = self
            .len
            .checked_add(separator)
            .and_then(|length| length.checked_add(component.len()))
            .filter(|length| *length <= MAX_PATH_BYTES)
            .ok_or_else(|| native_error(libc::ENAMETOOLONG))?;
        if separator != 0 {
            self.bytes[self.len] = b'/';
        }
        self.bytes[self.len + separator..required].copy_from_slice(component);
        self.len = required;
        self.bytes[self.len] = 0;
        Ok(())
    }
}

struct Pending<'scratch, S> {
    inline: [u8; MAX_PATH_BYTES],
    scratch: &'scratch mut S,
    overflow: bool,
    cursor: usize,
    len: usize,
}

impl<'scratch, S: PathScratch> Pending<'scratch, S> {
    fn new(path: &[u8], scratch: &'scratch mut S) -> Result<Self, S::Error> {
        if path.len() > MAX_PATH_BYTES || path.contains(&0) {
            return Err(CanonicalizeError::Io(io::ErrorKind::InvalidInput.into()));
        }
        let mut pending = Self {
            inline: [0; MAX_PATH_BYTES],
            scratch,
            overflow: false,
            cursor: 0,
            len: path.len(),
        };
        pending.inline[..path.len()].copy_from_slice(path);
        Ok(pending)
    }

    fn bytes(&mut self) -> &mut [u8] {
        if self.overflow {
            self.scratch.bytes()
        } else {
            &mut self.inline
        }
    }

    fn next_component(&mut self) -> Option<Range<usize>> {
        let mut cursor = self.cursor;
        let len = self.len;
        let bytes = self.bytes();
        while cursor < len && bytes[cursor] == b'/' {
            cursor += 1;
        }
        let start = cursor;
        while cursor < len && bytes[cursor] != b'/' {
            cursor += 1;
        }
        self.cursor = cursor;
        (start != cursor).then_some(start..cursor)
    }

    fn prepend(&mut self, target: &[u8]) -> Result<(), S::Error> {
        let remaining = self.len - self.cursor;
        let required = target
            .len()
            .checked_add(remaining)
            .expect("bounded suffix lengths");
        // One input plus at most forty symlink targets bounds pending storage.
        // Keep each observed target instead of rereading it after later expansion.
        assert!(required <= crate::MAX_CANONICALIZE_SCRATCH_BYTES);
        if required > self.bytes().len() {
            if required > self.scratch.bytes().len() {
                self.scratch
                    .grow(required)
                    .map_err(CanonicalizeError::Scratch)?;
            }
            assert!(self.scratch.bytes().len() >= required);
            if !self.overflow {
                self.scratch.bytes()[..self.len].copy_from_slice(&self.inline[..self.len]);
                self.overflow = true;
            }
        }
        // Preserve the unread suffix, including its separator when present. Move
        // it before copying the target because the ranges can overlap.
        let suffix = self.cursor..self.len;
        let bytes = self.bytes();
        bytes.copy_within(suffix, target.len());
        bytes[..target.len()].copy_from_slice(target);
        self.cursor = 0;
        self.len = required;
        Ok(())
    }
}

pub(super) fn canonicalize<S: PathScratch>(
    path: &[u8],
    output: &mut [u8; MAX_PATH_BYTES + 1],
    scratch: &mut S,
) -> Result<usize, S::Error> {
    resolve(path, output, scratch, MAX_NATIVE_PATH_CALLS)
}

fn resolve<S: PathScratch>(
    path: &[u8],
    output: &mut [u8; MAX_PATH_BYTES + 1],
    scratch: &mut S,
    native_calls: u32,
) -> Result<usize, S::Error> {
    assert!(path.starts_with(b"/"), "public entry admits absolute paths");
    let mut pending = Pending::new(path, scratch)?;
    output[..2].copy_from_slice(b"/\0");
    let mut resolved = ResolvedName {
        bytes: output,
        len: 1,
    };
    let mut work = NativeWork {
        remaining: native_calls,
    };
    if work.file_type(resolved.cstr())? != libc::S_IFDIR {
        return Err(native_error(libc::ENOTDIR));
    }
    let mut link = [0; MAX_PATH_BYTES + 1];
    let mut symlinks = 0;
    loop {
        let remaining = pending.cursor < pending.len;
        let Some(component) = pending.next_component() else {
            if remaining {
                resolved.require_directory(&mut work)?;
            }
            break;
        };
        let last = pending.cursor == pending.len;
        let component = &pending.bytes()[component];
        match component {
            b"." => {
                if last {
                    resolved.require_directory(&mut work)?;
                }
                continue;
            }
            b".." => {
                resolved.require_directory(&mut work)?;
                resolved.parent();
                continue;
            }
            _ => {}
        }
        let parent_len = resolved.len;
        resolved.append(component)?;
        let kind = work.file_type(resolved.cstr())?;
        if kind == libc::S_IFLNK {
            if symlinks == MAX_SYMLINKS {
                return Err(native_error(libc::ELOOP));
            }
            symlinks += 1;
            let length = work.link_target(resolved.cstr(), &mut link)?;
            resolved.truncate(if link[0] == b'/' { 1 } else { parent_len });
            pending.prepend(&link[..length])?;
        }
        // A following child name will make lstat check its parent. Dot-dot and
        // trailing separators have no child name, so require_directory performs
        // that check explicitly before the path can be shortened or returned.
    }
    Ok(resolved.len)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RefusingScratch;

    impl PathScratch for RefusingScratch {
        type Error = &'static str;

        fn bytes(&mut self) -> &mut [u8] {
            &mut []
        }

        fn grow(&mut self, _: usize) -> std::result::Result<(), Self::Error> {
            Err("test admission refusal")
        }
    }

    #[test]
    fn native_admission_precedes_entry_and_preserves_native_errors() {
        let mut output = [0; MAX_PATH_BYTES + 1];
        let mut scratch = RefusingScratch;
        assert!(matches!(
            resolve(b"/", &mut output, &mut scratch, 0),
            Err(CanonicalizeError::WorkLimit)
        ));
        assert_eq!(resolve(b"/", &mut output, &mut scratch, 1).unwrap(), 1);
        // With only one call, stop after root inspection. ENOTDIR must appear
        // only when enough calls are permitted to reach the offending component.
        assert!(matches!(
            resolve(b"/dev/null/child", &mut output, &mut scratch, 1),
            Err(CanonicalizeError::WorkLimit)
        ));
        assert!(matches!(
            resolve(b"/dev/null/child", &mut output, &mut scratch, 4),
            Err(CanonicalizeError::Io(error)) if error.raw_os_error() == Some(libc::ENOTDIR)
        ));
    }

    #[test]
    fn suffix_refusal_preserves_pending_bytes_and_typed_cause() {
        let mut scratch = RefusingScratch;
        let mut pending = Pending::new(b"/", &mut scratch).unwrap();
        assert!(pending.next_component().is_none());
        let target = [b'a'; MAX_PATH_BYTES];
        pending.prepend(&target).unwrap();
        assert!(matches!(
            pending.prepend(b"more/"),
            Err(CanonicalizeError::Scratch("test admission refusal"))
        ));
        assert_eq!(pending.len, MAX_PATH_BYTES);
        assert_eq!(pending.cursor, 0);
        assert_eq!(pending.bytes(), target);
    }
}
