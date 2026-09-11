//! Disposable allocator-fault probe around the unmodified public rlib.
use pipesql::{CancellationToken, Config, Database, Error, TransactionId};
use std::alloc::{GlobalAlloc, Layout, System};
use std::fmt::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static DENY: AtomicBool = AtomicBool::new(false);
static TRACK: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicUsize = AtomicUsize::new(0);
static REFUSED: AtomicUsize = AtomicUsize::new(0);
static ALLOW: AtomicUsize = AtomicUsize::new(0);
static LIVE_REQUESTED: AtomicUsize = AtomicUsize::new(0);
static LIVE_USABLE: AtomicUsize = AtomicUsize::new(0);
static PEAK_REQUESTED: AtomicUsize = AtomicUsize::new(0);
static PEAK_USABLE: AtomicUsize = AtomicUsize::new(0);
struct Allocator;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn malloc_size(pointer: *const std::ffi::c_void) -> usize;
}
#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn malloc_usable_size(pointer: *mut std::ffi::c_void) -> usize;
}

// Caller scaffolding only: a System allocation remains live while its usable
// extent is queried. Both native functions accept malloc/posix_memalign owners.
unsafe fn usable_size(pointer: *mut u8) -> usize {
    #[cfg(target_os = "macos")]
    {
        unsafe { malloc_size(pointer.cast()) }
    }
    #[cfg(target_os = "linux")]
    {
        unsafe { malloc_usable_size(pointer.cast()) }
    }
}

fn change_live(counter: &AtomicUsize, size: usize, add: bool) -> usize {
    // Bound observer contention even in synchronized multi-threaded probes;
    // neither arithmetic failure nor contention may unwind through GlobalAlloc.
    const OBSERVER_RETRIES: usize = 64;
    let mut old = counter.load(Ordering::Relaxed);
    for _ in 0..OBSERVER_RETRIES {
        let next = if add {
            old.checked_add(size)
        } else {
            old.checked_sub(size)
        }
        .unwrap_or_else(|| std::process::abort());
        match counter.compare_exchange(old, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(current) => old = current,
        }
    }
    std::process::abort();
}

// SAFETY: this test allocator forwards successful allocations and every
// deallocation unchanged to System. Its sole injected effect is a permitted null
// allocation result; it never changes layouts, ownership, pointers or contents.
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let tracking = TRACK.load(Ordering::Relaxed);
        let index = if tracking {
            CALLS.fetch_add(1, Ordering::Relaxed)
        } else {
            0
        };
        if DENY.load(Ordering::Relaxed) && (!tracking || index >= ALLOW.load(Ordering::Relaxed)) {
            if tracking {
                REFUSED.fetch_add(1, Ordering::Relaxed);
            }
            std::ptr::null_mut()
        } else {
            // SAFETY: caller supplied GlobalAlloc's valid layout unchanged.
            let pointer = unsafe { System.alloc(layout) };
            if !pointer.is_null() {
                // SAFETY: pointer is a live System allocation, not yet published.
                let usable = unsafe { usable_size(pointer) };
                if usable < layout.size() {
                    std::process::abort();
                }
                let requested = change_live(&LIVE_REQUESTED, layout.size(), true);
                let physical = change_live(&LIVE_USABLE, usable, true);
                if tracking {
                    PEAK_REQUESTED.fetch_max(requested, Ordering::Relaxed);
                    PEAK_USABLE.fetch_max(physical, Ordering::Relaxed);
                }
            }
            pointer
        }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: every non-null allocation came from System with this layout;
        // the GlobalAlloc caller retains the matching lifetime obligations.
        // SAFETY: obtain the extent while the System owner is still live.
        let usable = unsafe { usable_size(pointer) };
        unsafe { System.dealloc(pointer, layout) };
        // Release observations only after the physical allocation has been freed.
        change_live(&LIVE_REQUESTED, layout.size(), false);
        change_live(&LIVE_USABLE, usable, false);
    }
}

#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

#[path = "catalog-allocation.rs"]
mod catalog;
#[path = "composed-ownership.rs"]
mod ownership;
#[path = "workload-allocation.rs"]
mod workload;

struct FixedText {
    bytes: [u8; 1_024],
    length: usize,
}

impl fmt::Write for FixedText {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.length.checked_add(text.len()).ok_or(fmt::Error)?;
        if end > self.bytes.len() {
            return Err(fmt::Error);
        }
        self.bytes[self.length..end].copy_from_slice(text.as_bytes());
        self.length = end;
        Ok(())
    }
}

fn lifecycle_probe(
    root: &std::path::Path,
    operation: &str,
    after: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    assert!(after.is_none_or(|prefix| prefix <= 128));
    std::fs::create_dir(root)?;
    let path = root.join("database");
    let config = Config::new(2_000_000, 1_000_000)?;
    if operation == "open" {
        Database::create(&path, config)?.close()?;
        for name in ["CONTROL", "ROOT.A", "ROOT.B", "WAL"] {
            std::fs::copy(path.join(name), root.join(name))?;
        }
    }
    println!("entered {operation} lifecycle probe");
    let baseline_requested = LIVE_REQUESTED.load(Ordering::Relaxed);
    let baseline_usable = LIVE_USABLE.load(Ordering::Relaxed);
    ALLOW.store(after.unwrap_or(0), Ordering::Relaxed);
    CALLS.store(0, Ordering::Relaxed);
    TRACK.store(true, Ordering::Relaxed);
    DENY.store(after.is_some(), Ordering::Relaxed);
    let result = match operation {
        "create" => Database::create(&path, config),
        "open" => Database::open(&path, config),
        _ => unreachable!("validated lifecycle operation"),
    };
    DENY.store(false, Ordering::Relaxed);
    TRACK.store(false, Ordering::Relaxed);
    println!("{operation} allocations={}", CALLS.load(Ordering::Relaxed));
    println!("{operation} refusals={}", REFUSED.load(Ordering::Relaxed));
    match result {
        Ok(db) => {
            assert_eq!(db.generation(), 0);
            let resident = db.reserved_memory_bytes();
            assert!(resident >= db.path().as_os_str().len() as u64 && resident <= 4096);
            assert_eq!(
                LIVE_REQUESTED.load(Ordering::Relaxed) - baseline_requested,
                resident as usize
            );
            assert_eq!(db.reserved_temp_bytes(), 0);
            println!("{operation} retained-path-bytes={resident}");
            db.close()?;
            assert_eq!(LIVE_REQUESTED.load(Ordering::Relaxed), baseline_requested);
            assert_eq!(LIVE_USABLE.load(Ordering::Relaxed), baseline_usable);
            Database::open(&path, config)?.close()?;
            println!("returned healthy {operation}");
        }
        Err(Error::Io { source, .. })
            if after.is_some() && source.kind() == std::io::ErrorKind::OutOfMemory =>
        {
            if operation == "create" {
                assert!(!path.exists(), "definite create failure left a namespace");
            }
            println!("returned typed lifecycle allocation refusal");
        }
        Err(Error::RecoveryRequired {
            generation: 0,
            source,
        }) if after.is_some()
            && operation == "open"
            && matches!(source.kind(), pipesql::CauseKind::Io { source, .. }
                    if source.kind() == std::io::ErrorKind::OutOfMemory) =>
        {
            println!("returned explicit open recovery debt");
        }
        Err(Error::CleanupRequired { .. }) if after.is_some() && operation == "create" => {
            // Cleanup failure must be explicit. Root is exclusively probe-owned;
            // the parent removes it after examining the bounded process outcome.
            println!("returned explicit create cleanup debt");
        }
        Err(unexpected) => panic!("unexpected lifecycle outcome: {unexpected:?}"),
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let root = std::path::PathBuf::from(args.next().expect("owned probe directory"));
    let mode = args.next().expect("probe mode");
    assert!(args.next().is_none());
    if mode == "ownership" || mode == "ownership-negative" {
        return ownership::run(&root, mode == "ownership-negative");
    }
    if mode == "mutex" {
        catalog::mutex_contention();
        return Ok(());
    }
    let lifecycle = mode.to_str().and_then(|mode| {
        [
            "catalog",
            "catalog-recover-empty",
            "catalog-recover-data",
            "catalog-recover-corrupt",
            "catalog-recover-permission",
            "create",
            "open",
            "load",
            "load-readonly",
            "q1",
            "q6",
            "q1-corrupt",
            "q6-corrupt",
        ]
        .into_iter()
        .find_map(|operation| {
            if mode == format!("{operation}-control") {
                Some((operation, None))
            } else {
                mode.strip_prefix(&format!("{operation}-after-"))
                    .map(|number| {
                        (
                            operation,
                            Some(number.parse::<usize>().expect("allocation index")),
                        )
                    })
            }
        })
    });
    if let Some((operation, after)) = lifecycle {
        return if operation == "catalog" {
            catalog::run(&root, after)
        } else if operation.starts_with("catalog-recover-") {
            catalog::recovery::run(&root, operation, after)
        } else if matches!(
            operation,
            "load" | "load-readonly" | "q1" | "q6" | "q1-corrupt" | "q6-corrupt"
        ) {
            workload::run(&root, operation, after)
        } else {
            lifecycle_probe(&root, operation, after)
        };
    }
    let after = mode
        .to_str()
        .and_then(|text| text.strip_prefix("filesystem-after-"))
        .map(|text| text.parse::<usize>().expect("allocation index"));
    assert!(after.is_none_or(|index| index <= 128));
    assert!(
        after.is_some()
            || matches!(
                mode.to_str(),
                Some(
                    "control"
                        | "deny"
                        | "format-control"
                        | "format-deny"
                        | "filesystem-control"
                        | "filesystem-deny"
                )
            )
    );
    let deny =
        after.is_some() || mode == "deny" || mode == "format-deny" || mode == "filesystem-deny";
    ALLOW.store(after.unwrap_or(0), Ordering::Relaxed);
    std::fs::create_dir(&root)?;
    let database = root.join("database");
    let input = root.join("lineitem.tbl");
    std::fs::write(&input, b"1|2|3|4|1|2|0.5|0.25|R|A|1970-01-01|x|x|x|x|x|\n")?;
    let mut db = Database::create(&database, Config::new(2_000_000, 1_000_000)?)?;
    let resident = db.reserved_memory_bytes();
    if after.is_some() || mode == "filesystem-control" || mode == "filesystem-deny" {
        let mut bytes = [0; 24];
        bytes[..16].copy_from_slice(db.database_identity().as_bytes());
        bytes[16..].copy_from_slice(&1_u64.to_le_bytes());
        let token = TransactionId::from_bytes(bytes)?;
        let names = ["CONTROL", "ROOT.A", "ROOT.B", "WAL"];
        let before = names.map(|name| std::fs::read(database.join(name)).unwrap());
        // Parent-owned copies let the supervisor check bytes even after an abort.
        for (name, bytes) in names.iter().zip(&before) {
            std::fs::write(root.join(name), bytes)?;
        }
        println!("entered healthy-handle filesystem probe");
        CALLS.store(0, Ordering::Relaxed);
        TRACK.store(true, Ordering::Relaxed);
        DENY.store(deny, Ordering::Relaxed);
        let result = db.resolve_commit(token);
        DENY.store(false, Ordering::Relaxed);
        TRACK.store(false, Ordering::Relaxed);
        println!("filesystem allocations={}", CALLS.load(Ordering::Relaxed));
        println!("filesystem refusals={}", REFUSED.load(Ordering::Relaxed));
        match result {
            Err(Error::NotFound) => println!("returned not-found for unissued attempt"),
            Err(Error::Resource { .. }) if deny => println!("returned typed resource refusal"),
            Err(Error::Io { source, .. })
                if deny && source.kind() == std::io::ErrorKind::OutOfMemory =>
            {
                println!("returned typed OS allocation refusal");
            }
            unexpected => panic!("unexpected filesystem outcome: {unexpected:?}"),
        }
        assert_eq!(db.reserved_memory_bytes(), resident);
        assert_eq!(db.reserved_temp_bytes(), 0);
        for (name, expected) in names.into_iter().zip(before) {
            assert_eq!(std::fs::read(database.join(name))?, expected);
        }
        assert!(matches!(db.resolve_commit(token), Err(Error::NotFound)));
        db.close()?;
        let db = Database::open(&database, Config::new(2_000_000, 1_000_000)?)?;
        assert!(matches!(db.resolve_commit(token), Err(Error::NotFound)));
        db.close()?;
        return Ok(());
    }
    if mode == "format-control" || mode == "format-deny" {
        let sql = String::from("# 雪\nFROM lineitem |> LIMIT 0 OFFSET 9223372036854775807 + 1");
        let arithmetic = db.prepare(&sql).err().expect("constant overflow");
        let Error::ArithmeticOverflow { operation, span } = arithmetic else {
            panic!("arithmetic error");
        };
        assert_eq!(&sql[span.start()..span.end()], "9223372036854775807 + 1");
        drop(sql);
        let wal = database.join("WAL");
        std::fs::set_permissions(&wal, std::fs::Permissions::from_mode(0o400))?;
        let error = db
            .load_lineitem(&input, &CancellationToken::new())
            .unwrap_err();
        assert!(matches!(&error, Error::RecoveryRequired { source, .. }
            if matches!(source.kind(), pipesql::CauseKind::Io { source, .. }
                if source.kind() == std::io::ErrorKind::PermissionDenied && source.raw_os_error() == Some(13))));
        std::fs::set_permissions(&wal, std::fs::Permissions::from_mode(0o600))?;
        db.close()?;
        let mut text = FixedText {
            bytes: [0; 1_024],
            length: 0,
        };
        println!("entered fixed-buffer I/O diagnostic probe");
        DENY.store(deny, Ordering::Relaxed);
        let cause = pipesql::CauseKind::ArithmeticOverflow { operation, span };
        let formatted = write!(&mut text, "{error}; {arithmetic}; {cause}");
        DENY.store(false, Ordering::Relaxed);
        formatted.unwrap();
        let text = std::str::from_utf8(&text.bytes[..text.length]).unwrap();
        assert!(text.contains("open fence for recovery") && text.contains("os error 13"));
        assert_eq!(
            text.matches("arithmetic overflow during addition at bytes 38..61")
                .count(),
            2
        );
        println!("returned rendered diagnostic: {text}");
        return Ok(());
    }
    std::fs::write(database.join("private/caller-canary"), b"do not remove")?;
    assert!(matches!(
        db.load_lineitem(&input, &CancellationToken::new()),
        Err(Error::Corrupt(_))
    ));
    assert_eq!(db.reserved_memory_bytes(), resident);
    assert_eq!(db.reserved_temp_bytes(), 0);
    let mut bytes = [0; 24];
    bytes[..16].copy_from_slice(db.database_identity().as_bytes());
    bytes[16..].copy_from_slice(&1_u64.to_le_bytes());
    let token = TransactionId::from_bytes(bytes)?;
    println!("entered unavailable-handle diagnostic probe");
    DENY.store(deny, Ordering::Relaxed);
    let result = db.resolve_commit(token);
    DENY.store(false, Ordering::Relaxed);
    assert!(matches!(result, Err(Error::RecoveryRequired { .. })));
    println!("returned typed recovery-required outcome");
    db.close()?;
    Ok(())
}
