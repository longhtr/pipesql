//! Control pathname lookup while two database creators overlap.
//!
//! Each creator registers a thread-local actor. The selected lookup can pause or
//! fail while counters distinguish which actor reached each native operation.
//! Entered, pending and release signals let the driver establish the intended order;
//! waits are bounded so a missing peer cannot hold the observer indefinitely.
//! Setup rejects invalid controls and overlapping observer modes. The driver owns
//! the creators and checks the directory and database outcomes after the schedule.

use std::{
    cell::Cell,
    sync::atomic::{AtomicI32, AtomicU64, Ordering::SeqCst},
};

static MODE: AtomicI32 = AtomicI32::new(0);
static SITE: AtomicI32 = AtomicI32::new(0);
static ERROR: AtomicI32 = AtomicI32::new(0);
static ENTERED: AtomicI32 = AtomicI32::new(0);
static RELEASED: AtomicI32 = AtomicI32::new(0);
static PENDING: AtomicI32 = AtomicI32::new(0);
static ROOTS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static MOUNTS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static LSTATS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static LINKS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static REALPATHS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
thread_local! {
    static ACTOR: Cell<i32> = const { Cell::new(-1) };
    static SELECTED: Cell<bool> = const { Cell::new(false) };
}

#[unsafe(no_mangle)]
pub extern "C" fn init_actor(actor: i32) {
    if !(0..=1).contains(&actor) {
        super::stop(90);
    }
    ACTOR.set(actor);
    SELECTED.set(false);
}

#[unsafe(no_mangle)]
pub extern "C" fn init_start(mode: i32, error: i32, site: i32) {
    if !(1..=4).contains(&mode)
        || !(0..=2).contains(&site)
        || active()
        || super::ACTIVE.load(SeqCst)
        || super::publication::active()
        || super::io::active()
        || super::sync::active()
    {
        super::stop(90);
    }
    SITE.store(site, SeqCst);
    ERROR.store(error, SeqCst);
    MODE.store(mode, SeqCst);
}

pub fn active() -> bool {
    MODE.load(SeqCst) != 0
}
#[unsafe(no_mangle)]
pub extern "C" fn init_release() {
    RELEASED.store(1, SeqCst);
}
#[unsafe(no_mangle)]
pub extern "C" fn init_stop() {
    MODE.store(0, SeqCst);
}
#[unsafe(no_mangle)]
pub extern "C" fn init_pending() -> i32 {
    PENDING.load(SeqCst)
}

fn wait(flag: &AtomicI32) -> bool {
    for _ in 0..10000 {
        if flag.load(SeqCst) != 0 {
            return true;
        }
        let delay = libc::timespec {
            tv_sec: 0,
            tv_nsec: 1_000_000,
        };
        // SAFETY: delay is initialized; no remaining-time output is requested.
        if unsafe { libc::nanosleep(&delay, std::ptr::null_mut()) } < 0
            && unsafe { *super::calls::errno() } != libc::EINTR
        {
            return false;
        }
    }
    false
}
#[unsafe(no_mangle)]
pub extern "C" fn init_wait() -> i32 {
    i32::from(wait(&ENTERED))
}

#[unsafe(no_mangle)]
pub extern "C" fn init_count(kind: i32, actor: i32) -> u64 {
    if !(0..=1).contains(&actor) {
        super::stop(90);
    }
    let counts = match kind {
        0 => &ROOTS,
        1 => &MOUNTS,
        2 => &LSTATS,
        3 => &LINKS,
        4 => &REALPATHS,
        _ => super::stop(90),
    };
    counts[actor as usize].load(SeqCst)
}

fn actor() -> Option<usize> {
    if !active() {
        return None;
    }
    match ACTOR.get() {
        0 => Some(0),
        1 => Some(1),
        _ => None,
    }
}

fn step(site: i32) -> bool {
    let Some(actor) = actor() else {
        return false;
    };
    if site == 0 {
        ROOTS[actor].fetch_add(1, SeqCst);
    }
    if site != SITE.load(SeqCst) || SELECTED.replace(true) {
        return false;
    }
    let mode = MODE.load(SeqCst);
    if actor != 0 {
        return false;
    }
    let mut error = 0;
    if mode >= 3 {
        PENDING.store(1, SeqCst);
        ENTERED.store(1, SeqCst);
        if !wait(&RELEASED) {
            error = libc::ETIMEDOUT;
        }
        PENDING.store(0, SeqCst);
    }
    if error == 0 && (mode == 2 || mode == 4) {
        error = ERROR.load(SeqCst);
    }
    if error != 0 {
        // SAFETY: errno belongs to this thread; caller returns failure immediately.
        unsafe {
            *super::calls::errno() = error;
        }
    }
    error != 0
}

unsafe fn root(path: *const libc::c_char) -> bool {
    // SAFETY: the native ABI requires a NUL-terminated pathname.
    unsafe { *path == b'/' as libc::c_char && *path.add(1) == 0 }
}

#[cfg(target_os = "linux")]
#[unsafe(export_name = "lstat")]
unsafe extern "C" fn observed_lstat(path: *const libc::c_char, output: *mut libc::stat) -> i32 {
    if let Some(actor) = actor() {
        LSTATS[actor].fetch_add(1, SeqCst);
    }
    if step(if unsafe { root(path) } { 0 } else { 1 }) {
        return -1;
    }
    unsafe { super::calls::real_lstat(path, output) }
}
#[cfg(target_os = "linux")]
#[unsafe(export_name = "readlink")]
unsafe extern "C" fn observed_readlink(
    path: *const libc::c_char,
    output: *mut libc::c_char,
    capacity: usize,
) -> isize {
    if let Some(actor) = actor() {
        LINKS[actor].fetch_add(1, SeqCst);
    }
    if step(2) {
        return -1;
    }
    unsafe { super::calls::real_readlink(path, output, capacity) }
}
#[cfg(target_os = "linux")]
#[unsafe(export_name = "realpath")]
unsafe extern "C" fn observed_realpath(
    path: *const libc::c_char,
    output: *mut libc::c_char,
) -> *mut libc::c_char {
    if let Some(actor) = actor() {
        REALPATHS[actor].fetch_add(1, SeqCst);
    }
    unsafe { super::calls::real_realpath(path, output) }
}
#[cfg(target_os = "macos")]
unsafe extern "C" fn observed_stat(path: *const libc::c_char, output: *mut libc::stat) -> i32 {
    if unsafe { root(path) } && step(0) {
        return -1;
    }
    unsafe { libc::stat(path, output) }
}
#[cfg(target_os = "macos")]
unsafe extern "C" fn observed_statfs(path: *const libc::c_char, output: *mut libc::statfs) -> i32 {
    if let Some(actor) = actor() {
        MOUNTS[actor].fetch_add(1, SeqCst);
    }
    unsafe { libc::statfs(path, output) }
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
    static STAT: Pair = Pair(observed_stat as *const (), libc::stat as *const ());
    #[used]
    #[unsafe(link_section = "__DATA,__interpose")]
    static STATFS: Pair = Pair(observed_statfs as *const (), libc::statfs as *const ());
}
