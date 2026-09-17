//! Connect the initialization driver to controls in the preloaded native observer.
//!
//! Load every function before starting either creator. The cached calls select each
//! thread's actor, pause and release the chosen lookup, and read per-actor counts.
//! This keeps dynamic-loader work outside the schedule being tested. Missing symbols
//! or repeated loading fail setup; these wrappers do not decide whether creation
//! succeeded or whether the resulting directory is valid.

use std::sync::OnceLock;

struct Calls {
    actor: unsafe extern "C" fn(i32),
    start: unsafe extern "C" fn(i32, i32, i32),
    release: unsafe extern "C" fn(),
    stop: unsafe extern "C" fn(),
    wait: unsafe extern "C" fn() -> i32,
    pending: unsafe extern "C" fn() -> i32,
    count: unsafe extern "C" fn(i32, i32) -> u64,
}
static CALLS: OnceLock<Calls> = OnceLock::new();

pub fn load() {
    macro_rules! symbol {
        ($name:literal, $ty:ty) => {{
            let name = concat!($name, "\0");
            let address = libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr().cast());
            assert!(!address.is_null(), "native initialization observer missing");
            std::mem::transmute::<*mut libc::c_void, $ty>(address)
        }};
    }
    // SAFETY: these signatures exactly match the separately built observer.
    // Every lookup precedes arming or spawning a creator.
    let calls = unsafe {
        Calls {
            actor: symbol!("init_actor", unsafe extern "C" fn(i32)),
            start: symbol!("init_start", unsafe extern "C" fn(i32, i32, i32)),
            release: symbol!("init_release", unsafe extern "C" fn()),
            stop: symbol!("init_stop", unsafe extern "C" fn()),
            wait: symbol!("init_wait", unsafe extern "C" fn() -> i32),
            pending: symbol!("init_pending", unsafe extern "C" fn() -> i32),
            count: symbol!("init_count", unsafe extern "C" fn(i32, i32) -> u64),
        }
    };
    assert!(CALLS.set(calls).is_ok());
}
fn calls() -> &'static Calls {
    CALLS.get().expect("observer loaded")
}
pub unsafe fn probe_actor(actor: i32) {
    unsafe { (calls().actor)(actor) }
}
pub unsafe fn probe_start(mode: i32, error: i32, site: i32) {
    unsafe { (calls().start)(mode, error, site) }
}
pub unsafe fn probe_release() {
    unsafe { (calls().release)() }
}
pub unsafe fn probe_stop() {
    unsafe { (calls().stop)() }
}
pub unsafe fn probe_wait() -> i32 {
    unsafe { (calls().wait)() }
}
pub unsafe fn probe_pending() -> i32 {
    unsafe { (calls().pending)() }
}
pub unsafe fn probe_resolutions(actor: i32) -> u64 {
    unsafe { (calls().count)(0, actor) }
}
pub unsafe fn probe_mounts(actor: i32) -> u64 {
    unsafe { (calls().count)(1, actor) }
}
#[cfg(target_os = "linux")]
pub unsafe fn probe_lstats(actor: i32) -> u64 {
    unsafe { (calls().count)(2, actor) }
}
#[cfg(target_os = "linux")]
pub unsafe fn probe_readlinks(actor: i32) -> u64 {
    unsafe { (calls().count)(3, actor) }
}
#[cfg(target_os = "linux")]
pub unsafe fn probe_realpaths(actor: i32) -> u64 {
    unsafe { (calls().count)(4, actor) }
}
