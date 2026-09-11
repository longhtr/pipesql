//! Native filesystem calls. libc supplies target ABI types/constants; no layouts,
//! allocator pointers or mutable native buffers escape. Fresh descriptors are
//! immediately transferred into safe File owners.
use crate::{BUFFER_BYTES, DirectoryBuffer, MAX_PATH_BYTES};
use std::ffi::CStr;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

#[cfg(all(feature = "test-stack-observation", target_os = "macos"))]
pub(super) fn test_current_thread_stack_bytes() -> usize {
    // SAFETY: pthread_self supplies the live current thread; the synchronous
    // observer retains no pointer or handle. libc owns the target ABI signatures.
    unsafe { libc::pthread_get_stacksize_np(libc::pthread_self()) }
}

#[cfg(all(feature = "test-stack-observation", target_os = "linux"))]
pub(super) fn test_current_thread_stack_bytes() -> usize {
    let mut attributes = std::mem::MaybeUninit::<libc::pthread_attr_t>::uninit();
    // SAFETY: pthread_self identifies this live thread. On success, getattr
    // initializes the target ABI's attribute object in the supplied storage.
    let result = unsafe { libc::pthread_getattr_np(libc::pthread_self(), attributes.as_mut_ptr()) };
    assert_eq!(result, 0, "cannot observe current thread attributes");
    let mut bytes = 0;
    // SAFETY: getattr succeeded; both pointers refer to live, correctly aligned
    // storage. Query and destroy are synchronous and retain neither pointer.
    // Always destroy the initialized attributes before checking the query result.
    let (query, destroy) = unsafe {
        let query = libc::pthread_attr_getstacksize(attributes.as_ptr(), &mut bytes);
        let destroy = libc::pthread_attr_destroy(attributes.as_mut_ptr());
        (query, destroy)
    };
    assert_eq!(query, 0, "cannot observe current thread stack size");
    assert_eq!(destroy, 0, "cannot release current thread attributes");
    assert_ne!(bytes, 0, "native stack observation must be nonzero");
    bytes
}

pub(super) fn sync_all(file: &File) -> io::Result<()> {
    // SAFETY: the borrow keeps this owned descriptor live for the synchronous
    // call. These commands take no variadic argument or retained pointer. Do not
    // use std sync_all here: its EINTR retry loop has no finite attempt bound.
    #[cfg(target_os = "macos")]
    let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) };

    #[cfg(target_os = "linux")]
    let result = unsafe { libc::fsync(file.as_raw_fd()) };
    mutation_result(result)
}

fn path_name<'a>(
    path: &Path,
    terminated: &'a mut [u8; MAX_PATH_BYTES + 1],
) -> io::Result<&'a CStr> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() > MAX_PATH_BYTES {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    terminated[..bytes.len()].copy_from_slice(bytes);
    terminated[bytes.len()] = 0;
    CStr::from_bytes_with_nul(&terminated[..bytes.len() + 1])
        .map_err(|_| io::ErrorKind::InvalidInput.into())
}

pub(super) fn metadata(path: &Path, follow: bool) -> io::Result<crate::Metadata> {
    let mut terminated = [0; MAX_PATH_BYTES + 1];
    let name = path_name(path, &mut terminated)?;
    let mut raw = std::mem::MaybeUninit::<libc::stat>::zeroed();
    // SAFETY: validated live C string and writable storage for the target ABI's
    // stat. lstat is synchronous and retains neither pointer. Zero initialization
    // also covers reserved scalar fields the OS might leave untouched.
    let result = unsafe {
        if follow {
            libc::stat(name.as_ptr(), raw.as_mut_ptr())
        } else {
            libc::lstat(name.as_ptr(), raw.as_mut_ptr())
        }
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful lstat initializes the scalar stat record; remaining
    // reserved integer fields were zeroed. No uninitialized value is read.
    crate::Metadata::from_native(&unsafe { raw.assume_init() })
}

pub(super) fn open_directory(path: &Path) -> io::Result<File> {
    open_file(path, libc::O_RDONLY | libc::O_DIRECTORY)
}

pub(super) fn open_read(path: &Path) -> io::Result<File> {
    open_file(path, libc::O_RDONLY)
}

pub(super) fn open_read_write(path: &Path) -> io::Result<File> {
    open_file(path, libc::O_RDWR)
}

pub(super) fn create_new_read_write(path: &Path) -> io::Result<File> {
    open_file(path, libc::O_RDWR | libc::O_CREAT | libc::O_EXCL)
}

fn open_file(path: &Path, access: libc::c_int) -> io::Result<File> {
    let mut terminated = [0; MAX_PATH_BYTES + 1];
    let name = path_name(path, &mut terminated)?;
    let flags = access | libc::O_NONBLOCK | libc::O_CLOEXEC;
    // SAFETY: live NUL-terminated path, no retained pointer. Flags come only from
    // the fixed operations above. The promoted int mode is supplied for O_CREAT,
    // and ignored otherwise. Nonblocking admission prevents waiting on a FIFO.
    // A successful open returns a fresh descriptor, never an alias supplied by a caller.
    let fd = unsafe { libc::open(name.as_ptr(), flags, 0o666_i32) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is newly owned from successful open. No alias has escaped and no
    // fallible operation occurs between acquisition and this sole closing owner.
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(target_os = "macos")]
mod path;

#[cfg(target_os = "macos")]
pub(super) fn canonicalize(
    path: &Path,
    resolved: &mut [u8; MAX_PATH_BYTES + 1],
) -> Result<usize, crate::CanonicalizeError> {
    path::canonicalize(path.as_os_str().as_bytes(), resolved)
}

#[cfg(target_os = "linux")]
pub(super) fn canonicalize(
    path: &Path,
    resolved: &mut [u8; MAX_PATH_BYTES + 1],
) -> io::Result<usize> {
    assert!(usize::try_from(libc::PATH_MAX).expect("positive ABI PATH_MAX") <= resolved.len());
    let mut terminated = [0; MAX_PATH_BYTES + 1];
    let name = path_name(path, &mut terminated)?;
    // SAFETY: live terminated input and a distinct writable result buffer at
    // least PATH_MAX bytes, as required by realpath's non-null-buffer contract.
    // Neither pointer is retained. Core callers supply bounded absolute paths,
    // avoiding cwd traversal. libc internal work remains a foreign-runtime owner.
    let pointer = unsafe { libc::realpath(name.as_ptr(), resolved.as_mut_ptr().cast()) };
    if pointer.is_null() {
        return Err(io::Error::last_os_error());
    }
    if pointer != resolved.as_mut_ptr().cast() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let length = resolved
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(io::ErrorKind::InvalidData)?;
    if length == 0 || length > MAX_PATH_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(length)
}

pub(super) fn create_dir(path: &Path) -> io::Result<()> {
    let mut terminated = [0; MAX_PATH_BYTES + 1];
    let name = path_name(path, &mut terminated)?;
    // SAFETY: live terminated input, valid scalar mode, synchronous non-retaining call.
    let result = unsafe { libc::mkdir(name.as_ptr(), 0o777) };
    mutation_result(result)
}

pub(super) fn remove_file(path: &Path) -> io::Result<()> {
    let mut terminated = [0; MAX_PATH_BYTES + 1];
    let name = path_name(path, &mut terminated)?;
    // SAFETY: live terminated input; unlink retains no pointer.
    let result = unsafe { libc::unlink(name.as_ptr()) };
    mutation_result(result)
}

pub(super) fn remove_dir(path: &Path) -> io::Result<()> {
    let mut terminated = [0; MAX_PATH_BYTES + 1];
    let name = path_name(path, &mut terminated)?;
    // SAFETY: live terminated input; rmdir retains no pointer.
    let result = unsafe { libc::rmdir(name.as_ptr()) };
    mutation_result(result)
}

pub(super) fn rename(from: &Path, to: &Path) -> io::Result<()> {
    let mut from_buffer = [0; MAX_PATH_BYTES + 1];
    let mut to_buffer = [0; MAX_PATH_BYTES + 1];
    let from = path_name(from, &mut from_buffer)?;
    let to = path_name(to, &mut to_buffer)?;
    // SAFETY: two independently validated live C strings, no retained pointers.
    // No fallible allocation or partial user-space mutation precedes this call.
    let result = unsafe { libc::rename(from.as_ptr(), to.as_ptr()) };
    mutation_result(result)
}

pub(super) fn hard_link(from: &Path, to: &Path) -> io::Result<()> {
    let mut from_buffer = [0; MAX_PATH_BYTES + 1];
    let mut to_buffer = [0; MAX_PATH_BYTES + 1];
    let from = path_name(from, &mut from_buffer)?;
    let to = path_name(to, &mut to_buffer)?;
    // SAFETY: two independently validated live C strings, no retained pointers.
    // link creates a name, never replaces an existing destination or acknowledges durability.
    let result = unsafe { libc::link(from.as_ptr(), to.as_ptr()) };
    mutation_result(result)
}

fn mutation_result(result: libc::c_int) -> io::Result<()> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub(super) fn read_directory(file: &File, buffer: &mut DirectoryBuffer) -> io::Result<usize> {
    let bytes = &mut buffer.bytes;
    let mut attrs = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_RETURNED_ATTRS | crate::ATTR_CMN_ERROR | libc::ATTR_CMN_NAME,
        volattr: 0,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    // SAFETY: uniquely owned directory description, never shared or mixed with
    // readdir; initialized ABI-correct attrs; exclusive initialized buffer whose
    // exact size is supplied. DirectoryBuffer guarantees 8-byte alignment. Both
    // pointers remain live throughout the synchronous call and are not retained.
    let count = unsafe {
        libc::getattrlistbulk(
            file.as_raw_fd(),
            std::ptr::from_mut(&mut attrs).cast(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
            u64::from(libc::FSOPT_PACK_INVAL_ATTRS),
        )
    };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    let count = usize::try_from(count).map_err(|_| io::ErrorKind::InvalidData)?;
    if count > BUFFER_BYTES / 40 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut end = 0_usize;
    for _ in 0..count {
        let length = crate::record_length(bytes.get(end..).ok_or(io::ErrorKind::InvalidData)?)?;
        end = end.checked_add(length).ok_or(io::ErrorKind::InvalidData)?;
    }
    Ok(end)
}

#[cfg(target_os = "linux")]
pub(super) fn read_directory(file: &File, buffer: &mut DirectoryBuffer) -> io::Result<usize> {
    let bytes = &mut buffer.bytes;
    // SAFETY: target libc supplies the syscall number; getdents64 takes this live
    // directory fd, exclusive writable buffer and its exact byte extent. No
    // pointer is retained. The description is not exposed or shared with callers.
    let count = unsafe {
        libc::syscall(
            libc::SYS_getdents64,
            file.as_raw_fd(),
            bytes.as_mut_ptr(),
            bytes.len(),
        )
    };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    let count = usize::try_from(count).map_err(|_| io::ErrorKind::InvalidData)?;
    if count > BUFFER_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(count)
}
