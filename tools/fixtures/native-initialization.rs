//! Unmodified public rlib under native effect refusal and explicit scheduling.
use pipesql::{Config, Database, Error};
use std::path::{Path, PathBuf};
unsafe extern "C" {
    fn probe_actor(value: i32);
    fn probe_start(mode: i32, error: i32);
    fn probe_release();
    fn probe_stop();
    fn probe_roots(actor: i32) -> u64;
    fn probe_mounts(actor: i32) -> u64;
    fn probe_wait() -> i32;
    fn probe_pending() -> i32;
}
fn create(root: &Path, actor: i32) -> Result<PathBuf, Error> {
    // SAFETY: bounded actor index, initialized before scoped calls on this thread.
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
fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    assert_eq!(args.len(), 5);
    let root = PathBuf::from(&args[1]);
    let expected = PathBuf::from(&args[2]);
    assert!(root.is_absolute() && expected.is_absolute());
    let mode = args[3].to_str().unwrap().parse::<i32>().unwrap();
    let error = args[4].to_str().unwrap().parse::<i32>().unwrap();
    assert!((1..=4).contains(&mode));
    assert!([libc::EINTR, libc::EIO, libc::ENOMEM, libc::EACCES].contains(&error));
    let a = root.join("a");
    let b = root.join("b");
    let c = root.join("c");
    // SAFETY: the disposable child owns the shim; no invocation is outstanding.
    unsafe { probe_start(mode, error) };
    let (first, second) = if mode >= 3 {
        let path = a.clone();
        let first = std::thread::spawn(move || create(&path, 0));
        // SAFETY: bounded wait for the public resolver's native root-stat call.
        let reached = unsafe { probe_wait() };
        if reached != 1 {
            unsafe { probe_release() };
            first.join().unwrap().unwrap();
            panic!("native root initialization boundary not reached");
        }
        let pending_before = unsafe { probe_pending() };
        let second = create(&b, 1);
        let pending_after = unsafe { probe_pending() };
        // Release and join before interpreting outcomes or schedule witnesses.
        unsafe { probe_release() };
        let first = first.join().unwrap();
        assert_eq!(
            (pending_before, pending_after),
            (1, 1),
            "peer did not overlap pending native initialization"
        );
        (first, second)
    } else {
        (create(&a, 0), create(&b, 1))
    };
    // A subsequent call after BOTH first actors finish must retain correct names
    // and admission. Actor 1's mount count includes this continuing attempt.
    let third = create(&c, 1);
    // SAFETY: all invocations finished before disabling effects/counter inspection.
    unsafe { probe_stop() };
    let fails = mode == 2 || mode == 4;
    if fails {
        assert!(
            matches!(first, Err(Error::Io { operation: "canonicalize database parent", ref source }) if source.raw_os_error() == Some(error)),
            "unexpected first outcome: {first:?}"
        );
        assert!(
            !a.exists(),
            "failed pre-namespace operation published a directory"
        );
    } else {
        assert_eq!(first.unwrap(), expected.join("a"));
    }
    assert_eq!(second.unwrap(), expected.join("b"));
    assert_eq!(third.unwrap(), expected.join("c"));
    // Verify healed public reopen and byte-preserving inspection, not a test reader.
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
    // Each public resolution now owns its root observation. Restoring a shared
    // cache must fail even if returned names happen to agree on this filesystem.
    // Mount counts remain observations, not an expected upstream implementation.
    assert_eq!(unsafe { probe_roots(0) }, 1);
    assert_eq!(unsafe { probe_roots(1) }, 2);
    println!(
        "mode={mode} error={error} roots={},{} mounts={},{} outcomes=checked",
        unsafe { probe_roots(0) },
        unsafe { probe_roots(1) },
        unsafe { probe_mounts(0) },
        unsafe { probe_mounts(1) }
    );
}
