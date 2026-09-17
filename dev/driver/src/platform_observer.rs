//! Check that native observer modes cannot overlap in one process.
//!
//! Setup resolves every symbol before starting two selected modes in sequence.
//! Starting the second must terminate the process unless the first was stopped;
//! markers let the supervising campaign distinguish those outcomes. A retained
//! null-device descriptor supplies the interruption mode's trace destination.
//! No filesystem operations or workers run while armed: this checks serialized
//! mode setup, not concurrent calls to the observer's start and stop functions.

use std::{ffi::CStr, os::fd::AsRawFd};

enum Start {
    Interruption(unsafe extern "C" fn(i32, u32)),
    Publication(unsafe extern "C" fn(u32)),
    Initialization(unsafe extern "C" fn(i32, i32, i32)),
    Io(unsafe extern "C" fn(u32, u32, u32, i32)),
    Sync(unsafe extern "C" fn(u32, u32, i32)),
}

enum Stop {
    Plain(unsafe extern "C" fn()),
    Publication(unsafe extern "C" fn() -> u32),
}

fn symbol(name: &CStr) -> *mut libc::c_void {
    // SAFETY: the name is terminated; the preloaded image lives until process exit.
    let pointer = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    assert!(!pointer.is_null(), "native observer missing: {name:?}");
    pointer
}

fn load(mode: &str) -> (Start, Stop) {
    macro_rules! function {
        ($name:literal, $kind:ty) => {
            std::mem::transmute::<*mut libc::c_void, $kind>(symbol($name))
        };
    }
    // SAFETY: these signatures match the observer's exported C functions.
    unsafe {
        match mode {
            "interruption" => (
                Start::Interruption(function!(
                    c"interruption_start",
                    unsafe extern "C" fn(i32, u32)
                )),
                Stop::Plain(function!(c"interruption_stop", unsafe extern "C" fn())),
            ),
            "publication" => (
                Start::Publication(function!(c"publication_start", unsafe extern "C" fn(u32))),
                Stop::Publication(function!(
                    c"publication_stop",
                    unsafe extern "C" fn() -> u32
                )),
            ),
            "initialization" => (
                Start::Initialization(function!(
                    c"init_start",
                    unsafe extern "C" fn(i32, i32, i32)
                )),
                Stop::Plain(function!(c"init_stop", unsafe extern "C" fn())),
            ),
            "io" => (
                Start::Io(function!(
                    c"io_probe_start",
                    unsafe extern "C" fn(u32, u32, u32, i32)
                )),
                Stop::Plain(function!(c"io_probe_stop", unsafe extern "C" fn())),
            ),
            "sync" => (
                Start::Sync(function!(
                    c"sync_probe_start",
                    unsafe extern "C" fn(u32, u32, i32)
                )),
                Stop::Plain(function!(c"sync_probe_stop", unsafe extern "C" fn())),
            ),
            _ => panic!("unknown observer mode"),
        }
    }
}

impl Start {
    fn call(&self, fd: i32) {
        // SAFETY: setup is single-threaded and all arguments are valid controls.
        unsafe {
            match self {
                Self::Interruption(start) => start(fd, 0),
                Self::Publication(start) => start(0),
                Self::Initialization(start) => start(1, libc::EIO, 0),
                Self::Io(start) => start(0, 0, 0, libc::EIO),
                Self::Sync(start) => start(0, 0, libc::EIO),
            }
        }
    }
}

impl Stop {
    fn call(&self) {
        // SAFETY: the matching mode is active and no observed work remains.
        unsafe {
            match self {
                Self::Plain(stop) => stop(),
                Self::Publication(stop) => assert_eq!(stop(), 0),
            }
        }
    }
}

pub fn run(first: &str, second: &str, release: bool) {
    let first = load(first);
    let second = load(second);
    let trace = std::fs::File::open("/dev/null").unwrap();
    println!("observer modes entered");
    first.0.call(trace.as_raw_fd());
    if release {
        first.1.call();
    }
    second.0.call(trace.as_raw_fd());
    assert!(release, "overlapping observer modes accepted");
    second.1.call();
    println!("observer modes released");
}
