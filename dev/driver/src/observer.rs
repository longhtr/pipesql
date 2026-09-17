//! Connect recovery drivers to the preloaded filesystem-interruption observer.
//!
//! Loading resolves both functions once, before the measured operation starts.
//! Arming supplies a retained trace descriptor and the event at which to stop the
//! process; disarming ends observation when the operation survives. Missing symbols
//! or a second load fail immediately, so a run without instrumentation cannot pass
//! as an uninterrupted recovery case. The caller owns the descriptor's lifetime.

use std::sync::OnceLock;

type Start = unsafe extern "C" fn(i32, u32);
type Stop = unsafe extern "C" fn();
static CALLS: OnceLock<(Start, Stop)> = OnceLock::new();

pub fn load() {
    // SAFETY: the observer exports these exact C signatures. Both lookups happen
    // before arming; no loader work is hidden inside the measured operation.
    let calls = unsafe {
        let start = libc::dlsym(libc::RTLD_DEFAULT, c"interruption_start".as_ptr());
        let stop = libc::dlsym(libc::RTLD_DEFAULT, c"interruption_stop".as_ptr());
        assert!(
            !start.is_null() && !stop.is_null(),
            "native observer missing"
        );
        (
            std::mem::transmute::<*mut libc::c_void, Start>(start),
            std::mem::transmute::<*mut libc::c_void, Stop>(stop),
        )
    };
    assert!(CALLS.set(calls).is_ok(), "observer already loaded");
}

pub unsafe fn interruption_start(fd: i32, cut: u32) {
    unsafe {
        CALLS.get().expect("observer loaded").0(fd, cut);
    }
}

pub unsafe fn interruption_stop() {
    unsafe {
        CALLS.get().expect("observer loaded").1();
    }
}
