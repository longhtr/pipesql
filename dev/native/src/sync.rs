//! Inject synchronization errors and detect weaker calls during one operation.
//!
//! A bounded call interval returns the configured errno instead of performing full
//! synchronization. Separate counters record attempts, refusals and weaker calls,
//! so a driver can detect a retry or fallback that concealed the original error.
//! Darwin's fcntl bridge and Linux's fsync wrapper share the refusal decision.
//! Invalid setup, overlapping modes and excess calls terminate the process.
//! These observations check syscall behavior, not whether the underlying storage
//! stack would preserve data through power loss.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering::Relaxed};
static ACTIVE: AtomicBool = AtomicBool::new(false);
static POSITION: AtomicU32 = AtomicU32::new(0);
static BURST: AtomicU32 = AtomicU32::new(0);
static ERROR: AtomicI32 = AtomicI32::new(0);
static CALLS: AtomicU32 = AtomicU32::new(0);
static REFUSED: AtomicU32 = AtomicU32::new(0);
static WEAK: AtomicU32 = AtomicU32::new(0);

pub fn active() -> bool {
    ACTIVE.load(Relaxed)
}
#[unsafe(no_mangle)]
pub extern "C" fn sync_probe_start(at: u32, burst: u32, error: i32) {
    if active()
        || super::ACTIVE.load(Relaxed)
        || super::publication::active()
        || super::initialization::active()
        || super::io::active()
        || at > 4096
        || burst > 16
        || ![libc::EINTR, libc::EIO, libc::ENOTSUP].contains(&error)
    {
        super::stop(90);
    }
    POSITION.store(at, Relaxed);
    BURST.store(burst, Relaxed);
    ERROR.store(error, Relaxed);
    CALLS.store(0, Relaxed);
    REFUSED.store(0, Relaxed);
    WEAK.store(0, Relaxed);
    ACTIVE.store(true, Relaxed);
}
#[unsafe(no_mangle)]
pub extern "C" fn sync_probe_stop() {
    ACTIVE.store(false, Relaxed);
}
#[unsafe(no_mangle)]
pub extern "C" fn sync_probe_calls() -> u32 {
    CALLS.load(Relaxed)
}
#[unsafe(no_mangle)]
pub extern "C" fn sync_probe_refused() -> u32 {
    REFUSED.load(Relaxed)
}
#[unsafe(no_mangle)]
pub extern "C" fn sync_probe_weak() -> u32 {
    WEAK.load(Relaxed)
}

// The Darwin variadic bridge delegates the fault decision here before calling
// F_FULLFSYNC. Linux's fsync wrapper uses the same bounded counter.
#[unsafe(no_mangle)]
pub extern "C" fn pipesql_sync_refuse() -> i32 {
    if !active() {
        return 0;
    }
    let call = CALLS.fetch_add(1, Relaxed) + 1;
    if call > 4096 {
        super::stop(91);
    }
    let at = POSITION.load(Relaxed);
    if at != 0 && call >= at && call - at < BURST.load(Relaxed) {
        REFUSED.fetch_add(1, Relaxed);
        // SAFETY: errno is thread-local and the wrapper immediately returns -1.
        unsafe {
            *super::calls::errno() = ERROR.load(Relaxed);
        }
        return 1;
    }
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn pipesql_sync_weak() {
    if active() {
        WEAK.fetch_add(1, Relaxed);
    }
}

#[cfg(target_os = "linux")]
#[unsafe(export_name = "fdatasync")]
unsafe extern "C" fn observed_fdatasync(fd: i32) -> i32 {
    pipesql_sync_weak();
    unsafe { super::calls::real_fdatasync(fd) }
}
#[cfg(target_os = "macos")]
unsafe extern "C" fn observed_fsync(fd: i32) -> i32 {
    pipesql_sync_weak();
    unsafe { libc::fsync(fd) }
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
    static FSYNC: Pair = Pair(observed_fsync as *const (), libc::fsync as *const ());
}
