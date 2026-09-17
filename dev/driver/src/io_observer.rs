//! Connect the I/O driver to native transfer faults and observation counters.
//!
//! Load all symbols before setup or observation, then arm a call kind, position and
//! fault burst through the cached functions. Separate counters report calls,
//! refusals and partial transfers so the driver can prove the fault was reached.
//! A missing symbol fails setup. The driver owns the observed interval and must
//! stop it before unrelated file operations can affect the counts.

use std::sync::OnceLock;
struct Calls {
    start: unsafe extern "C" fn(u32, u32, u32, i32),
    stop: unsafe extern "C" fn(),
    calls: unsafe extern "C" fn() -> u32,
    refused: unsafe extern "C" fn() -> u32,
    partial: unsafe extern "C" fn() -> u32,
}
static CALLS: OnceLock<Calls> = OnceLock::new();
pub fn load() {
    macro_rules! symbol {
        ($name:literal, $ty:ty) => {{
            let name = concat!($name, "\0");
            let address = libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr().cast());
            assert!(!address.is_null(), "native I/O observer missing");
            std::mem::transmute::<*mut libc::c_void, $ty>(address)
        }};
    }
    // SAFETY: exact C signatures match the observer; loading precedes arming.
    let calls = unsafe {
        Calls {
            start: symbol!("io_probe_start", unsafe extern "C" fn(u32, u32, u32, i32)),
            stop: symbol!("io_probe_stop", unsafe extern "C" fn()),
            calls: symbol!("io_probe_calls", unsafe extern "C" fn() -> u32),
            refused: symbol!("io_probe_refused", unsafe extern "C" fn() -> u32),
            partial: symbol!("io_probe_partial", unsafe extern "C" fn() -> u32),
        }
    };
    assert!(CALLS.set(calls).is_ok());
}
fn calls() -> &'static Calls {
    CALLS.get().expect("observer loaded")
}
pub unsafe fn io_probe_start(kind: u32, at: u32, burst: u32, error: i32) {
    unsafe { (calls().start)(kind, at, burst, error) }
}
pub unsafe fn io_probe_stop() {
    unsafe { (calls().stop)() }
}
pub unsafe fn io_probe_calls() -> u32 {
    unsafe { (calls().calls)() }
}
pub unsafe fn io_probe_refused() -> u32 {
    unsafe { (calls().refused)() }
}
pub unsafe fn io_probe_partial() -> u32 {
    unsafe { (calls().partial)() }
}
