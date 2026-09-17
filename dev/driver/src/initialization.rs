//! Check that one creator's pathname resolution does not corrupt another's state.
//!
//! The observer can pause creator A inside a native resolver call while B creates
//! a different database. Handshakes require A to remain paused throughout B's call.
//! After releasing and joining A, creator C checks that later calls still work.
//! Other modes run without overlap or return an exact OS error for A.
//!
//! A resolution failure must leave no database directory. Successful creations
//! must return the expected path and reopen without changing authority-file bytes.
//! The Rust development runner supplies the path spelling and failure mode,
//! starts a fresh process for each case, and removes its directories.

use pipesql::{Config, Database, Error};
use std::path::{Path, PathBuf};
mod initialization_observer;
use initialization_observer::*;
fn create(root: &Path, actor: i32) -> Result<PathBuf, Error> {
    // SAFETY: callers pass actor 0 or 1; registration affects only this thread.
    unsafe { probe_actor(actor) };
    let database = Database::create(root, Config::new(2_000_000, 1_000_000)?)?;
    let path = database.path().to_owned();
    database.close()?;
    Ok(path)
}
fn authority(root: &Path) -> Vec<Vec<u8>> {
    ["CONTROL", "ROOT.A", "ROOT.B", "WAL", "LOCK"]
        .iter()
        .map(|name| std::fs::read(root.join(name)).unwrap())
        .collect()
}
struct ReleaseOnDrop;
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        // SAFETY: the observer is loaded before this guard is constructed.
        unsafe { probe_release() };
    }
}
fn main() {
    initialization_observer::load();
    let inject_panic = std::env::var_os("PIPESQL_INIT_PANIC").is_some();
    let result = std::panic::catch_unwind(|| run(inject_panic));
    if let Err(payload) = result {
        if inject_panic {
            assert_eq!(unsafe { probe_pending() }, 0);
            eprintln!("initialization panic: creator released and joined");
        }
        std::panic::resume_unwind(payload);
    }
}

fn run(inject_panic: bool) {
    let args: Vec<_> = std::env::args_os().collect();
    assert_eq!(args.len(), 6);
    let root = PathBuf::from(&args[1]);
    let expected = PathBuf::from(&args[2]);
    assert!(root.is_absolute() && expected.is_absolute());
    let mode = args[3].to_str().unwrap().parse::<i32>().unwrap();
    let error = args[4].to_str().unwrap().parse::<i32>().unwrap();
    assert!((1..=4).contains(&mode));
    assert!([libc::EINTR, libc::EIO, libc::ENOMEM, libc::EACCES].contains(&error));
    let site = match args[5].to_str().unwrap() {
        "root" => 0,
        "component" => 1,
        "symlink" => 2,
        _ => panic!("unknown native observation site"),
    };
    let a = root.join("a");
    let b = root.join("b");
    let c = root.join("c");
    // SAFETY: setup precedes both creators, so no thread is using the observer yet.
    unsafe { probe_start(mode, error, site) };
    let (first, second) = if mode >= 3 {
        std::thread::scope(|scope| {
            let path = a.clone();
            let first = scope.spawn(move || create(&path, 0));
            // Release before scope joins on unwind, including a panic during B.
            let release = ReleaseOnDrop;
            // SAFETY: this reads atomic handshake state and waits for a bounded interval.
            let reached = unsafe { probe_wait() };
            if reached != 1 {
                unsafe { probe_release() };
                first.join().unwrap().unwrap();
                panic!("native resolution boundary not reached");
            }
            let pending_before = unsafe { probe_pending() };
            assert!(!inject_panic, "deliberate initialization coordinator panic");
            let second = create(&b, 1);
            let pending_after = unsafe { probe_pending() };
            // Release and join before assertions, so a failed check cannot strand A.
            drop(release);
            let first = first.join().unwrap();
            assert_eq!(
                (pending_before, pending_after),
                (1, 1),
                "peer did not overlap pending native initialization"
            );
            (first, second)
        })
    } else {
        (create(&a, 0), create(&b, 1))
    };
    // Reuse actor 1 after both earlier calls finish. Its counters include B and C,
    // so a stale per-thread resolver result cannot pass by serving B alone.
    let third = create(&c, 1);
    // SAFETY: both threads finished their calls before observation is disabled.
    unsafe { probe_stop() };
    let fails = mode == 2 || mode == 4;
    if fails {
        assert!(
            matches!(first, Err(Error::Io { operation: "canonicalize database parent", ref source }) if source.raw_os_error() == Some(error)),
            "unexpected first outcome: {first:?}"
        );
        assert!(
            !expected.join("a").exists(),
            "failed pre-namespace operation published a directory"
        );
    } else {
        assert_eq!(first.unwrap(), expected.join("a"));
    }
    assert_eq!(second.unwrap(), expected.join("b"));
    assert_eq!(third.unwrap(), expected.join("c"));
    // Reopen through the public API and compare the authority files before and after.
    for name in if fails {
        &['b', 'c'][..]
    } else {
        &['a', 'b', 'c'][..]
    } {
        let path = expected.join(name.to_string());
        let before = authority(&path);
        let database = Database::open(&path, Config::new(2_000_000, 1_000_000).unwrap()).unwrap();
        assert_eq!(database.path(), path);
        database.close().unwrap();
        assert_eq!(authority(&path), before);
    }
    // A runs once; actor 1 runs B and C. Require a root inspection for each,
    // even if a cached answer would happen to return the expected path.
    // Mount calls are printed for observation without requiring a fixed count.
    assert_eq!(unsafe { probe_resolutions(0) }, 1);
    assert_eq!(unsafe { probe_resolutions(1) }, 2);
    #[cfg(target_os = "linux")]
    {
        // Counts include metadata checks after path resolution as well as
        // traversal. No public creation may delegate to libc realpath.
        for actor in [0, 1] {
            assert!(unsafe { probe_lstats(actor) } > 0);
            assert_eq!(
                unsafe { probe_realpaths(actor) },
                0,
                "foreign resolver remained in stock creation"
            );
        }
        if root != expected {
            assert!(unsafe { probe_readlinks(1) } > 0);
        }
        println!(
            "site={site} lstats={},{} readlinks={},{} foreign-realpaths=0",
            unsafe { probe_lstats(0) },
            unsafe { probe_lstats(1) },
            unsafe { probe_readlinks(0) },
            unsafe { probe_readlinks(1) }
        );
    }
    println!(
        "mode={mode} error={error} resolutions={},{} mounts={},{} outcomes=checked",
        unsafe { probe_resolutions(0) },
        unsafe { probe_resolutions(1) },
        unsafe { probe_mounts(0) },
        unsafe { probe_mounts(1) }
    );
}
