//! Caller scaffolding around the unchanged CLI source, not a CLI reimplementation.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static ACTIVE: AtomicBool = AtomicBool::new(false);
static DENY: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicUsize = AtomicUsize::new(0);
static REFUSED: AtomicUsize = AtomicUsize::new(0);
static ALLOW: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
struct Allocator;

// SAFETY: successful allocations and all deallocations forward the caller's
// unchanged layout/pointer to System. Refusal is a permitted null result.
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let active = ACTIVE.load(Ordering::Relaxed);
        let index = if active {
            CALLS.fetch_add(1, Ordering::Relaxed)
        } else {
            0
        };
        if active && DENY.load(Ordering::Relaxed) && index >= ALLOW.load(Ordering::Relaxed) {
            REFUSED.fetch_add(1, Ordering::Relaxed);
            std::ptr::null_mut()
        } else {
            // SAFETY: GlobalAlloc caller supplied this valid layout.
            let pointer = unsafe { System.alloc(layout) };
            if !pointer.is_null()
                && LIVE
                    .fetch_add(layout.size(), Ordering::Relaxed)
                    .checked_add(layout.size())
                    .is_none()
            {
                std::process::abort();
            }
            pointer
        }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: every published pointer came from System with this layout.
        unsafe { System.dealloc(pointer, layout) }
        if LIVE.fetch_sub(layout.size(), Ordering::Relaxed) < layout.size() {
            std::process::abort();
        }
    }
}

#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

fn arm(deny: bool, prefix: usize) {
    assert!(prefix <= 128);
    ALLOW.store(prefix, Ordering::Relaxed);
    DENY.store(deny, Ordering::Relaxed);
    CALLS.store(0, Ordering::Relaxed);
    REFUSED.store(0, Ordering::Relaxed);
    ACTIVE.store(true, Ordering::Relaxed);
}

fn finish() {
    ACTIVE.store(false, Ordering::Relaxed);
    println!(
        "cli allocations={} refusals={}",
        CALLS.load(Ordering::Relaxed),
        REFUSED.load(Ordering::Relaxed)
    );
}

// A narrow test-only visibility boundary. All production parsing, execution,
// output and diagnostics are included verbatim; no source substitutions.
#[allow(dead_code)]
mod cli {
    include!("../../src/cli/mod.rs");
    pub(super) fn entry() -> ExitCode {
        main()
    }
    pub(super) fn parsing(arguments: Vec<std::ffi::OsString>, deny: bool, baseline: usize) {
        super::arm(deny, 0);
        let result = command::parse(arguments.into_iter());
        let rendered = result
            .as_ref()
            .err()
            .map(|error| diagnostic::Diagnostic::new("usage error", error));
        let valid = result.is_ok();
        drop(result);
        assert_eq!(super::LIVE.load(super::Ordering::Relaxed), baseline);
        super::finish();
        if let Some(text) = rendered {
            let text = std::str::from_utf8(text.as_bytes()).unwrap();
            print!("{text}");
        }
        println!("parsed={valid}");
    }
}

fn descriptor_count() -> usize {
    // The supervisor sets RLIMIT_NOFILE=128 for every cell, including controls.
    (0..128)
        .filter(|fd| {
            // SAFETY: F_GETFD only inspects an integer descriptor; no pointers or
            // ownership transfer. EBADF means the slot is not live.
            unsafe { libc::fcntl(*fd, libc::F_GETFD) != -1 }
        })
        .count()
}

fn main() -> std::process::ExitCode {
    // Read the probe selector before arming; actual CLI entry then consumes the
    // real process argv unchanged. The selector is not a production argument.
    let mode = std::env::var("PIPESQL_CLI_PROBE").expect("probe mode");
    if mode.starts_with("parse-") {
        let baseline = LIVE.load(Ordering::Relaxed);
        let args = std::env::args_os().collect();
        cli::parsing(args, mode == "parse-deny", baseline);
        return std::process::ExitCode::SUCCESS;
    }
    let after = mode
        .strip_prefix("entry-after-")
        .map(|n| n.parse::<usize>().unwrap());
    let closed = match mode.as_str() {
        "entry-closed-stdout" => Some(1),
        "entry-closed-stderr" => Some(2),
        _ => None,
    };
    assert!(after.is_some() || mode == "entry-control" || closed.is_some());
    if let Some(descriptor) = closed {
        // SAFETY: explicit test effect after Rust bootstrap and before CLI entry;
        // the supervisor owns these inherited descriptors. No Rust File owns it.
        assert_eq!(unsafe { libc::close(descriptor) }, 0);
    }
    let live = LIVE.load(Ordering::Relaxed);
    let descriptors = descriptor_count();
    arm(after.is_some(), after.unwrap_or(0));
    let code = cli::entry();
    assert_eq!(LIVE.load(Ordering::Relaxed), live, "CLI leaked Rust owners");
    assert_eq!(descriptor_count(), descriptors, "CLI leaked descriptors");
    if closed == Some(1) {
        // This cell deliberately has no stdout, including for observer reporting.
        ACTIVE.store(false, Ordering::Relaxed);
    } else {
        finish();
    }
    code
}
