//! Inject failed or short regular-file transfers during one isolated operation.
//!
//! Only calls of the selected kind on regular files enter the position counter.
//! A configured position and burst select failures, short transfers or an error
//! after partial progress; other calls pass through to the OS. Separate counters
//! let the driver verify both progress and refusal. Observation preserves errno
//! when inspecting a descriptor. Invalid setup, failed inspection or excess calls
//! terminate the process so a broken observer cannot masquerade as engine behavior.
//! The observed interval runs in one thread with all other modes disarmed.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering::Relaxed};
static ACTIVE: AtomicBool = AtomicBool::new(false);
static KIND: AtomicU32 = AtomicU32::new(0);
static POSITION: AtomicU32 = AtomicU32::new(0);
static BURST: AtomicU32 = AtomicU32::new(0);
static ERROR: AtomicI32 = AtomicI32::new(0);
static CALLS: AtomicU32 = AtomicU32::new(0);
static REFUSED: AtomicU32 = AtomicU32::new(0);
static PARTIAL: AtomicU32 = AtomicU32::new(0);

pub fn active() -> bool {
    ACTIVE.load(Relaxed)
}
#[unsafe(no_mangle)]
pub extern "C" fn io_probe_start(kind: u32, at: u32, burst: u32, error: i32) {
    if active()
        || super::ACTIVE.load(Relaxed)
        || super::publication::active()
        || super::initialization::active()
        || super::sync::active()
        || kind > 3
        || at > 4096
        || burst > 16
        || ![0, libc::EINTR, libc::EIO, -libc::EINTR, -libc::EIO].contains(&error)
    {
        super::stop(90);
    }
    KIND.store(kind, Relaxed);
    POSITION.store(at, Relaxed);
    BURST.store(burst, Relaxed);
    ERROR.store(error, Relaxed);
    CALLS.store(0, Relaxed);
    REFUSED.store(0, Relaxed);
    PARTIAL.store(0, Relaxed);
    ACTIVE.store(true, Relaxed);
}
#[unsafe(no_mangle)]
pub extern "C" fn io_probe_stop() {
    ACTIVE.store(false, Relaxed);
}
#[unsafe(no_mangle)]
pub extern "C" fn io_probe_calls() -> u32 {
    CALLS.load(Relaxed)
}
#[unsafe(no_mangle)]
pub extern "C" fn io_probe_refused() -> u32 {
    REFUSED.load(Relaxed)
}
#[unsafe(no_mangle)]
pub extern "C" fn io_probe_partial() -> u32 {
    PARTIAL.load(Relaxed)
}

pub enum Action {
    Pass,
    Fail,
    Short,
}
pub fn action(kind: u32, fd: i32) -> Action {
    if !active() || kind != KIND.load(Relaxed) {
        return Action::Pass;
    }
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: fstat writes this stack value on success. Save errno so observation
    // cannot change the result of an otherwise ordinary call.
    let regular = unsafe {
        let saved = *super::calls::errno();
        if libc::fstat(fd, metadata.as_mut_ptr()) != 0 {
            super::stop(91);
        }
        *super::calls::errno() = saved;
        metadata.assume_init().st_mode & libc::S_IFMT == libc::S_IFREG
    };
    if !regular {
        return Action::Pass;
    }
    let call = CALLS.fetch_add(1, Relaxed) + 1;
    if call > 4096 {
        super::stop(92);
    }
    let position = POSITION.load(Relaxed);
    if position == 0 || call < position {
        return Action::Pass;
    }
    let distance = call - position;
    let error = ERROR.load(Relaxed);
    let crossed = error < 0;
    if crossed && distance == 0 {
        return Action::Short;
    }
    if (crossed && distance <= BURST.load(Relaxed)) || (!crossed && distance < BURST.load(Relaxed))
    {
        if error == 0 {
            return Action::Short;
        }
        REFUSED.fetch_add(1, Relaxed);
        // SAFETY: errno is thread-local; the wrapper immediately returns -1.
        unsafe {
            *super::calls::errno() = error.abs();
        }
        return Action::Fail;
    }
    Action::Pass
}
impl Action {
    pub fn request(&self, count: usize) -> usize {
        if matches!(self, Self::Short) {
            count.min(1)
        } else {
            count
        }
    }
    pub fn observe(&self, result: isize, count: usize) -> isize {
        // EOF and one-byte requests do not prove a shortened transfer made progress.
        if matches!(self, Self::Short) && result > 0 && (result as usize) < count {
            PARTIAL.fetch_add(1, Relaxed);
        }
        result
    }
}

#[cfg_attr(target_os = "linux", unsafe(export_name = "read"))]
unsafe extern "C" fn observed_read(fd: i32, bytes: *mut libc::c_void, count: usize) -> isize {
    let action = action(0, fd);
    if matches!(action, Action::Fail) {
        return -1;
    }
    action.observe(
        unsafe { super::calls::real_read(fd, bytes, action.request(count)) },
        count,
    )
}
#[cfg_attr(target_os = "linux", unsafe(export_name = "pread"))]
unsafe extern "C" fn observed_pread(
    fd: i32,
    bytes: *mut libc::c_void,
    count: usize,
    offset: libc::off_t,
) -> isize {
    let action = action(2, fd);
    if matches!(action, Action::Fail) {
        return -1;
    }
    action.observe(
        unsafe { super::calls::real_pread(fd, bytes, action.request(count), offset) },
        count,
    )
}
#[cfg(target_os = "linux")]
#[unsafe(export_name = "pread64")]
unsafe extern "C" fn observed_pread64(
    fd: i32,
    bytes: *mut libc::c_void,
    count: usize,
    offset: libc::off64_t,
) -> isize {
    const _: () =
        assert!(std::mem::size_of::<libc::off64_t>() == std::mem::size_of::<libc::off_t>());
    unsafe { observed_pread(fd, bytes, count, offset) }
}
#[cfg(target_os = "macos")]
mod interpose {
    use super::*;
    #[repr(C)]
    struct Pair(*const (), *const ());
    // Immutable loader metadata points only to process-lifetime code.
    unsafe impl Sync for Pair {}
    #[used]
    #[unsafe(link_section = "__DATA,__interpose")]
    static READ: Pair = Pair(observed_read as *const (), libc::read as *const ());
    #[used]
    #[unsafe(link_section = "__DATA,__interpose")]
    static PREAD: Pair = Pair(observed_pread as *const (), libc::pread as *const ());
}
