//! Run the real CLI while refusing Rust allocations or closing an output stream.
//!
//! The development runner selects the mode through `PIPESQL_CLI_PROBE`. Parse
//! modes allocate the argument list first, then optionally refuse allocations
//! during parsing and diagnostic construction. Entry modes call the CLI's main
//! function with real process arguments and can refuse all allocations after a
//! chosen prefix. Both paths use the production CLI source included below.
//!
//! After the call, live requested bytes and, for entry modes, open descriptors
//! must return to their baselines. Closed-output modes check CLI failure handling;
//! the driver returns the CLI's exit code unchanged. The Rust runner owns process
//! timeouts and requires the expected output, status and refusal counts. These
//! counters observe Rust allocations, not total process memory.

// Parser-only calls need a parent module that can access the private parser and
// diagnostic types. Compile those same sources here as well as under the full
// CLI; this keeps the production visibility and entry path unchanged.
#![allow(clippy::duplicate_mod)]

#[cfg(test)]
#[allow(dead_code)]
#[path = "../../../test/support/mod.rs"]
mod test_support;

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
    assert!(prefix <= 256);
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

// Compile production modules by path so their module documentation is valid.
// Parser controls call the same source directly; entry controls use the full CLI.
#[allow(dead_code)]
#[path = "../../../src/cli/mod.rs"]
mod cli;
#[allow(dead_code)]
#[path = "../../../src/cli/command.rs"]
mod command;
#[allow(dead_code)]
#[path = "../../../src/cli/diagnostic.rs"]
mod diagnostic;
#[allow(dead_code)]
#[path = "../../../src/cli/stdio.rs"]
mod stdio;

fn parsing(arguments: Vec<std::ffi::OsString>, deny: bool, baseline: usize) {
    arm(deny, 0);
    let result = command::parse(arguments.into_iter());
    let rendered = result
        .as_ref()
        .err()
        .map(|error| diagnostic::Diagnostic::new("usage error", error));
    let valid = result.is_ok();
    drop(result);
    // The baseline predates argument allocation. Retain the diagnostic while
    // checking that dropping the parse result releases every input owner.
    assert_eq!(LIVE.load(Ordering::Relaxed), baseline);
    finish();
    if let Some(text) = rendered {
        print!("{}", std::str::from_utf8(text.as_bytes()).unwrap());
    }
    println!("parsed={valid}");
}

fn descriptor_count() -> usize {
    // The runner caps descriptors at 128, making this scan exhaustive for these
    // processes. Enumerating /dev/fd would itself open a directory descriptor.
    (0..128)
        .filter(|fd| {
            // SAFETY: F_GETFD only inspects an integer descriptor; no pointers or
            // ownership transfer. EBADF means the slot is not live.
            unsafe { libc::fcntl(*fd, libc::F_GETFD) != -1 }
        })
        .count()
}

fn main() -> std::process::ExitCode {
    // Bound the descriptor scan and disable core files for deliberate failures.
    // SAFETY: these limits affect only this isolated driver process.
    unsafe {
        let descriptors = libc::rlimit {
            rlim_cur: 128,
            rlim_max: 128,
        };
        let core = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(libc::setrlimit(libc::RLIMIT_NOFILE, &descriptors), 0);
        assert_eq!(libc::setrlimit(libc::RLIMIT_CORE, &core), 0);
    }

    // Keep the fixture's mode allocation outside observation. The CLI receives
    // unchanged argv, so parsing the mode cannot consume a CLI argument.
    let mode = std::env::var("PIPESQL_CLI_PROBE").expect("probe mode");
    if let Some(position) = mode.strip_prefix("publication-") {
        let position: u32 = position.parse().unwrap();
        assert!(position <= 4);
        // Resolve before arming. The native library exports these exact scalar
        // C interfaces and retains no pointers into Rust-owned storage.
        let (start, stop) = unsafe {
            let start = libc::dlsym(libc::RTLD_DEFAULT, c"publication_start".as_ptr());
            let stop = libc::dlsym(libc::RTLD_DEFAULT, c"publication_stop".as_ptr());
            assert!(
                !start.is_null() && !stop.is_null(),
                "native publication observer missing"
            );
            (
                std::mem::transmute::<*mut libc::c_void, unsafe extern "C" fn(u32)>(start),
                std::mem::transmute::<*mut libc::c_void, unsafe extern "C" fn() -> u32>(stop),
            )
        };
        unsafe { start(position) };
        let outcome = cli::main();
        let calls = unsafe { stop() };
        eprintln!("observed_renames={calls}");
        return outcome;
    }
    if mode.starts_with("parse-") {
        let baseline = LIVE.load(Ordering::Relaxed);
        let args = std::env::args_os().collect();
        parsing(args, mode == "parse-deny", baseline);
        return std::process::ExitCode::SUCCESS;
    }
    let after = mode
        .strip_prefix("entry-after-")
        .map(|n| n.parse::<usize>().unwrap());
    let closed = match mode.as_str() {
        "entry-closed-stdin" => Some(0),
        "entry-closed-stdout" => Some(1),
        "entry-closed-stderr" => Some(2),
        _ => None,
    };
    assert!(after.is_some() || mode == "entry-control" || closed.is_some());
    if let Some(descriptor) = closed {
        // SAFETY: the runner supplies these inherited descriptors, and no Rust
        // File owns them here. Close only the selected stream before CLI entry.
        assert_eq!(unsafe { libc::close(descriptor) }, 0);
    }
    let live = LIVE.load(Ordering::Relaxed);
    let descriptors = descriptor_count();
    arm(after.is_some(), after.unwrap_or(0));
    let code = cli::main();
    assert_eq!(LIVE.load(Ordering::Relaxed), live, "CLI leaked Rust owners");
    assert_eq!(descriptor_count(), descriptors, "CLI leaked descriptors");
    if closed == Some(1) {
        // Do not let the fixture's own println panic on the closed stdout and
        // replace the CLI exit code that the parent needs to inspect.
        ACTIVE.store(false, Ordering::Relaxed);
    } else {
        finish();
    }
    code
}
