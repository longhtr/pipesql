//! Connect the synchronization driver to native refusal and fallback counters.
//!
//! Resolve all symbols before arming so loader activity cannot enter the measured
//! operation. Cached calls choose the refusal interval and expose separate counts
//! for synchronization attempts, injected errors and weaker synchronization calls.
//! Missing instrumentation fails setup. The driver stops observation and judges the
//! database outcome; this adapter only carries the observer's controls and counts.

use std::sync::OnceLock;
struct Calls {
    start: unsafe extern "C" fn(u32, u32, i32),
    stop: unsafe extern "C" fn(),
    calls: unsafe extern "C" fn() -> u32,
    refused: unsafe extern "C" fn() -> u32,
    weak: unsafe extern "C" fn() -> u32,
}
static CALLS: OnceLock<Calls> = OnceLock::new();
pub fn load() {
    macro_rules! symbol {
        ($name:literal, $ty:ty) => {{
            let name = concat!($name, "\0");
            let address = libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr().cast());
            assert!(
                !address.is_null(),
                "native synchronization observer missing"
            );
            std::mem::transmute::<*mut libc::c_void, $ty>(address)
        }};
    }
    // SAFETY: exact C signatures match the observer; loading precedes arming.
    let calls = unsafe {
        Calls {
            start: symbol!("sync_probe_start", unsafe extern "C" fn(u32, u32, i32)),
            stop: symbol!("sync_probe_stop", unsafe extern "C" fn()),
            calls: symbol!("sync_probe_calls", unsafe extern "C" fn() -> u32),
            refused: symbol!("sync_probe_refused", unsafe extern "C" fn() -> u32),
            weak: symbol!("sync_probe_weak", unsafe extern "C" fn() -> u32),
        }
    };
    assert!(CALLS.set(calls).is_ok());
}
fn calls() -> &'static Calls {
    CALLS.get().expect("observer loaded")
}
pub unsafe fn sync_probe_start(at: u32, burst: u32, error: i32) {
    unsafe { (calls().start)(at, burst, error) }
}
pub unsafe fn sync_probe_stop() {
    unsafe { (calls().stop)() }
}
pub unsafe fn sync_probe_calls() -> u32 {
    unsafe { (calls().calls)() }
}
pub unsafe fn sync_probe_refused() -> u32 {
    unsafe { (calls().refused)() }
}
pub unsafe fn sync_probe_weak() -> u32 {
    unsafe { (calls().weak)() }
}
