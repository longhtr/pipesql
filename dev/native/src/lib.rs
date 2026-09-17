//! Observe filesystem changes and inject publication or pathname lookup failures.
//!
//! Each driver serializes mode setup before starting observed work; start/stop
//! calls are not a concurrent mode-selection API. An active mode rejects every
//! other start, including its own. Interruption and publication refusal are
//! single-threaded; initialization distinguishes two creators by thread.
//! Interruption records are fixed
//! bytes written directly to a retained descriptor; no formatting, allocation or
//! loader lookup occurs while armed. A failed trace write terminates the fixture.
//! Visible writes survive process interruption; this is not a power-loss model.

mod calls;
mod initialization;
mod io;
mod publication;
mod sync;

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

static ACTIVE: AtomicBool = AtomicBool::new(false);
static TRACE: AtomicI32 = AtomicI32::new(-1);
static TARGET: AtomicU32 = AtomicU32::new(0);
static SEQUENCE: AtomicU32 = AtomicU32::new(0);

#[unsafe(no_mangle)]
pub extern "C" fn interruption_start(fd: i32, cut: u32) {
    if fd < 0
        || cut > 4096
        || ACTIVE.load(Ordering::Relaxed)
        || publication::active()
        || initialization::active()
        || io::active()
        || sync::active()
    {
        stop(90);
    }
    TRACE.store(fd, Ordering::Relaxed);
    TARGET.store(cut, Ordering::Relaxed);
    SEQUENCE.store(0, Ordering::Relaxed);
    ACTIVE.store(true, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub extern "C" fn interruption_stop() {
    ACTIVE.store(false, Ordering::Relaxed);
}

fn stop(code: i32) -> ! {
    // SAFETY: _exit terminates the isolated fixture without destructors.
    unsafe { libc::_exit(code) }
}

fn event(kind: u8, after: bool, role: u8) {
    if !ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
    if sequence > 4096 {
        stop(91);
    }
    // Eight-byte versioned records: version, kind, phase, root role, sequence LE.
    let mut record = [1, kind, u8::from(after), role, 0, 0, 0, 0];
    record[4..].copy_from_slice(&sequence.to_le_bytes());
    // SAFETY: errno is thread-local; real_write bypasses this image's observer.
    // The driver keeps TRACE open throughout the single-threaded interval.
    unsafe {
        let saved = *calls::errno();
        let wrote = calls::real_write(
            TRACE.load(Ordering::Relaxed),
            record.as_ptr().cast(),
            record.len(),
        );
        *calls::errno() = saved;
        if wrote != record.len() as isize {
            stop(93);
        }
    }
    if sequence == TARGET.load(Ordering::Relaxed) {
        stop(86);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pipesql_sync_event(after: u32) {
    event(b's', after != 0, 0);
}

unsafe fn regular(fd: i32) -> bool {
    if !ACTIVE.load(Ordering::Relaxed) {
        return false;
    }
    // SAFETY: fstat initializes the stack value on success; fd validity is an OS
    // precondition and a failure is an observer error, never silently skipped.
    unsafe {
        let saved = *calls::errno();
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        if libc::fstat(fd, metadata.as_mut_ptr()) != 0 {
            stop(95);
        }
        *calls::errno() = saved;
        metadata.assume_init().st_mode & libc::S_IFMT == libc::S_IFREG
    }
}

unsafe fn role(path: *const libc::c_char) -> u8 {
    // SAFETY: rename's original ABI requires a NUL-terminated destination.
    let path = unsafe { std::ffi::CStr::from_ptr(path) }.to_bytes();
    match path.rsplit(|byte| *byte == b'/').next() {
        Some(b"ROOT.A") => b'A',
        Some(b"ROOT.B") => b'B',
        _ => 0,
    }
}

#[cfg_attr(target_os = "linux", unsafe(export_name = "write"))]
unsafe extern "C" fn observed_write(fd: i32, bytes: *const libc::c_void, count: usize) -> isize {
    unsafe {
        let action = io::action(1, fd);
        if matches!(action, io::Action::Fail) {
            return -1;
        }
        let observe = regular(fd);
        if observe {
            event(b'w', false, 0);
        }
        let result = action.observe(calls::real_write(fd, bytes, action.request(count)), count);
        if observe && result >= 0 {
            event(b'w', true, 0);
        }
        result
    }
}

#[cfg_attr(target_os = "linux", unsafe(export_name = "pwrite"))]
unsafe extern "C" fn observed_pwrite(
    fd: i32,
    bytes: *const libc::c_void,
    count: usize,
    offset: libc::off_t,
) -> isize {
    unsafe {
        let action = io::action(3, fd);
        if matches!(action, io::Action::Fail) {
            return -1;
        }
        let observe = regular(fd);
        if observe {
            event(b'p', false, 0);
        }
        let result = action.observe(
            calls::real_pwrite(fd, bytes, action.request(count), offset),
            count,
        );
        if observe && result >= 0 {
            event(b'p', true, 0);
        }
        result
    }
}

#[cfg(target_os = "linux")]
#[unsafe(export_name = "pwrite64")]
unsafe extern "C" fn observed_pwrite64(
    fd: i32,
    bytes: *const libc::c_void,
    count: usize,
    offset: libc::off64_t,
) -> isize {
    const _: () =
        assert!(std::mem::size_of::<libc::off64_t>() == std::mem::size_of::<libc::off_t>());
    unsafe { observed_pwrite(fd, bytes, count, offset) }
}

#[cfg_attr(target_os = "linux", unsafe(export_name = "rename"))]
unsafe extern "C" fn observed_rename(
    source: *const libc::c_char,
    destination: *const libc::c_char,
) -> i32 {
    unsafe {
        let role = if ACTIVE.load(Ordering::Relaxed) {
            role(destination)
        } else {
            0
        };
        event(b'r', false, role);
        if publication::refuse() {
            *calls::errno() = libc::EIO;
            return -1;
        }
        let result = calls::real_rename(source, destination);
        if result == 0 {
            event(b'r', true, role);
        }
        result
    }
}

#[cfg_attr(target_os = "linux", unsafe(export_name = "unlink"))]
unsafe extern "C" fn observed_unlink(path: *const libc::c_char) -> i32 {
    event(b'u', false, 0);
    let result = unsafe { calls::real_unlink(path) };
    if result == 0 {
        event(b'u', true, 0);
    }
    result
}

#[cfg(target_os = "linux")]
#[unsafe(export_name = "fsync")]
unsafe extern "C" fn observed_fsync(fd: i32) -> i32 {
    if sync::pipesql_sync_refuse() != 0 {
        return -1;
    }
    event(b's', false, 0);
    let result = unsafe { calls::real_fsync(fd) };
    if result == 0 {
        event(b's', true, 0);
    }
    result
}

#[cfg(target_os = "macos")]
mod interpose {
    use super::*;
    #[repr(C)]
    struct Pair(*const (), *const ());
    // Loader metadata is immutable and points only to process-lifetime code.
    unsafe impl Sync for Pair {}
    macro_rules! pair {
        ($name:ident, $replacement:ident, $original:ident) => {
            #[used]
            #[unsafe(link_section = "__DATA,__interpose")]
            static $name: Pair = Pair($replacement as *const (), libc::$original as *const ());
        };
    }
    pair!(WRITE, observed_write, write);
    pair!(PWRITE, observed_pwrite, pwrite);
    pair!(RENAME, observed_rename, rename);
    pair!(UNLINK, observed_unlink, unlink);
}
