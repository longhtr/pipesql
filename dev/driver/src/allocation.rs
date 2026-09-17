//! Run database workloads with an allocator that measures and can refuse requests.
//!
//! Cargo builds this executable against the stock library. The development runner
//! chooses one mode per process. The allocator forwards successful requests
//! to `System`, records their requested and native usable sizes, and can return
//! null after a chosen number of requests. Refusal remains active during error
//! handling, so a diagnostic cannot depend on a later allocation succeeding.
//!
//! Workload modules own expected answers, reservation comparisons and cleanup
//! checks. This file also exercises creation, opening and error formatting. The
//! capacity probe includes the private production helper separately; other modes
//! use the public library. `transient_ownership` can sample reservations inside a
//! call, when a temporary buffer would be invisible to return-time measurements.
//!
//! Inputs and output buffers are prepared before observing each operation. Counts
//! include Rust allocations through this allocator, not foreign allocations,
//! resident stack pages or total process memory.

// The capacity check includes the production resource owner. Cargo's all-target
// test build also compiles that owner's private tests and their process support.
#[cfg(test)]
#[path = "../../../test/support/child.rs"]
mod test_child;
#[cfg(test)]
#[path = "../../../test/support/subprocess.rs"]
mod test_subprocess;
#[cfg(test)]
#[allow(dead_code)]
#[path = "../../../test/support/mod.rs"]
mod test_support;

use pipesql::{CancellationToken, Config, Database, Error, TransactionId};
use std::alloc::{GlobalAlloc, Layout, System};
use std::fmt::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

// Refusal and event counting are independent: formatting probes deny all requests
// without a census. Live-byte counters run continuously, even when TRACK is false.
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

// The pointer must still name a live System allocation. On these targets System
// uses malloc/posix_memalign storage accepted by the corresponding size query.
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

// Call before arming an observer. Make the allocation observable so optimization
// cannot erase the cache history. The allocator may reuse this larger block for
// a later smaller request; tests must still pass when it chooses fresh storage.
fn cache_allocation(bytes: usize) {
    let layout = Layout::from_size_align(bytes, 1).unwrap();
    // SAFETY: retain a live allocation with this layout, inspect it, then free it
    // with the same layout. No pointer is read after release.
    unsafe {
        let pointer = std::hint::black_box(std::alloc::alloc(layout));
        assert!(!pointer.is_null());
        std::hint::black_box(usable_size(pointer));
        std::alloc::dealloc(pointer, layout);
    }
}

fn change_live(counter: &AtomicUsize, size: usize, add: bool) -> usize {
    // Allocation callbacks cannot unwind. Abort if accounting overflows or if
    // contention exhausts this bound, rather than report unreliable measurements.
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
                transient_ownership::sample(true);
            }
            pointer
        }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller supplies the live System pointer and original layout.
        // Query its usable extent before freeing it.
        let usable = unsafe { usable_size(pointer) };
        transient_ownership::sample(false);
        unsafe { System.dealloc(pointer, layout) };
        // Keep the bytes counted through the free. Subtracting earlier could hide
        // a reservation released while its buffer was still live.
        change_live(&LIVE_REQUESTED, layout.size(), false);
        change_live(&LIVE_USABLE, usable, false);
    }
}

#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

#[path = "allocation/capacity.rs"]
mod allocation_capacity;
#[path = "allocation/catalog.rs"]
mod catalog;
#[path = "allocation/concurrent.rs"]
mod concurrent;
#[path = "allocation/csv.rs"]
mod csv;
#[path = "allocation/import.rs"]
mod csv_import;
#[path = "allocation/grouping.rs"]
mod grouping_ownership;
#[path = "allocation/join.rs"]
mod join;
#[path = "allocation/measurement.rs"]
mod measurement;
#[path = "allocation/parquet.rs"]
mod parquet_ownership;
#[path = "allocation/results.rs"]
mod partial_results;
#[path = "allocation/report.rs"]
mod report;
#[path = "allocation/shapes.rs"]
mod shapes;
#[path = "allocation/transient.rs"]
mod transient_ownership;
#[path = "allocation/workload.rs"]
mod workload;

// Formatting into caller storage must not introduce an allocation of its own.
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
    let canonical = root.join("database");
    let expanded = operation.ends_with("-expanded");
    let creating = operation.starts_with("create");
    let path = if expanded {
        if !cfg!(target_os = "linux") {
            return Err("expanded pathname checks require Linux".into());
        }
        // The final path is short, but resolving forty links retains long pending
        // suffixes. Allocation refusal must cover that scratch space as well.
        for index in 0..40 {
            let next = if index == 39 {
                ".".to_owned()
            } else {
                format!("s{}", index + 1)
            };
            std::os::unix::fs::symlink(next + &"/.".repeat(1_900), root.join(format!("s{index}")))?;
        }
        root.join("s0/database")
    } else {
        canonical.clone()
    };
    let config = Config::new(2_000_000, 1_000_000)?;
    if !creating {
        Database::create(&canonical, config)?.close()?;
        for name in ["CONTROL", "ROOT.A", "ROOT.B", "WAL"] {
            std::fs::copy(canonical.join(name), root.join(name))?;
        }
    }
    println!("entered {operation} lifecycle probe");
    let baseline_requested = LIVE_REQUESTED.load(Ordering::Relaxed);
    let baseline_usable = LIVE_USABLE.load(Ordering::Relaxed);
    ALLOW.store(after.unwrap_or(0), Ordering::Relaxed);
    CALLS.store(0, Ordering::Relaxed);
    TRACK.store(true, Ordering::Relaxed);
    DENY.store(after.is_some(), Ordering::Relaxed);
    let result = if creating {
        Database::create(&path, config)
    } else {
        Database::open(&path, config)
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
        Err(Error::Resource {
            owner: "pathname scratch allocation",
            ..
        }) if after.is_some() && expanded => {
            if creating {
                assert!(!canonical.exists(), "scratch refusal created a namespace");
            }
            println!("returned pathname scratch allocation refusal");
            let healed = if creating {
                Database::create(&path, config)?
            } else {
                Database::open(&path, config)?
            };
            assert_eq!(healed.path(), canonical);
            healed.close()?;
            println!("pathname scratch healed");
        }
        Err(Error::Io { source, .. })
            if after.is_some() && source.kind() == std::io::ErrorKind::OutOfMemory =>
        {
            if creating {
                assert!(
                    !canonical.exists(),
                    "definite create failure left a namespace"
                );
            }
            println!("returned typed lifecycle allocation refusal");
        }
        Err(Error::RecoveryRequired {
            generation: 0,
            source,
        }) if after.is_some()
            && !creating
            && matches!(source.kind(), pipesql::CauseKind::Io { source, .. }
                    if source.kind() == std::io::ErrorKind::OutOfMemory) =>
        {
            println!("returned explicit open recovery debt");
        }
        Err(Error::CleanupRequired { .. }) if after.is_some() && creating => {
            // Cleanup may itself need an allocation and fail. The parent owns
            // this test directory and removes it after checking the error outcome.
            println!("returned explicit create cleanup debt");
        }
        Err(unexpected) => panic!("unexpected lifecycle outcome: {unexpected:?}"),
    }
    assert_eq!(LIVE_REQUESTED.load(Ordering::Relaxed), baseline_requested);
    assert_eq!(LIVE_USABLE.load(Ordering::Relaxed), baseline_usable);
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let root = std::path::PathBuf::from(args.next().expect("owned probe directory"));
    let mode = args.next().expect("probe mode");
    assert!(args.next().is_none());
    if mode == "parquet-ownership" {
        parquet_ownership::run(&root);
        return Ok(());
    }
    if mode == "csv-import-receipt" {
        csv_import::run(&root, None, true);
        return Ok(());
    }
    if mode == "csv-import"
        || mode
            .to_str()
            .is_some_and(|mode| mode.starts_with("csv-import-"))
    {
        let after = mode
            .to_str()
            .unwrap()
            .strip_prefix("csv-import-")
            .map(|prefix| prefix.parse::<usize>().unwrap());
        csv_import::run(&root, after, false);
        return Ok(());
    }
    if mode == "csv" {
        csv::run();
        return Ok(());
    }
    if mode == "allocation-capacity" {
        allocation_capacity::run();
        return Ok(());
    }
    if mode == "partial-result-shapes"
        || mode == "partial-result-prefix-negative"
        || mode == "partial-result-terminal-negative"
    {
        use partial_results::{Control, run};
        let control = if mode == "partial-result-prefix-negative" {
            Control::WrongPrefix
        } else if mode == "partial-result-terminal-negative" {
            Control::MissingTerminal
        } else {
            Control::Healthy
        };
        return run(&root, control);
    }
    if mode
        .to_str()
        .is_some_and(|mode| mode.starts_with("event-report-"))
    {
        use report::EventReportControl;
        let control = match mode.to_str().expect("ASCII report mode") {
            "event-report-history" => EventReportControl::Healthy,
            "event-report-attribution-negative" => EventReportControl::WrongAttribution,
            "event-report-preparation-negative" => EventReportControl::MissingPreparation,
            "event-report-construction-negative" => EventReportControl::MissingConstruction,
            "event-report-terminal-negative" => EventReportControl::MissingTerminal,
            _ => panic!("unknown event report control"),
        };
        return report::event_report_history(&root, control);
    }
    if mode == "analytic-shapes" || mode == "analytic-attribution-negative" {
        return shapes::analytic_shapes(&root, mode == "analytic-attribution-negative");
    }
    if mode == "wide-set-shapes" || mode == "wide-set-attribution-negative" {
        return shapes::wide_set_shapes(&root, mode == "wide-set-attribution-negative");
    }
    if mode == "wide-left-join-shape"
        || mode == "wide-left-join-attribution-negative"
        || mode == "wide-left-join-observer-negative"
        || mode == "wide-left-join-lifecycle-negative"
        || mode == "wide-left-join-failure-negative"
        || mode == "wide-left-join-execution-negative"
        || mode == "wide-left-join-construction-negative"
        || mode == "wide-left-join-construction"
        || mode == "wide-left-join-construction-sequence"
    {
        let control = if mode == "wide-left-join-attribution-negative" {
            join::WideJoinControl::WrongAttribution
        } else if mode == "wide-left-join-observer-negative" {
            join::WideJoinControl::DisabledObserver
        } else if mode == "wide-left-join-lifecycle-negative" {
            join::WideJoinControl::MissingPreparation
        } else if mode == "wide-left-join-construction" {
            join::WideJoinControl::Construction
        } else if mode == "wide-left-join-construction-sequence" {
            join::WideJoinControl::ConstructionSequence
        } else if mode == "wide-left-join-construction-negative" {
            join::WideJoinControl::MissingConstruction
        } else if mode == "wide-left-join-execution-negative" {
            join::WideJoinControl::MissingFailedExecution
        } else if mode == "wide-left-join-failure-negative" {
            join::WideJoinControl::MissingFailedPreparation
        } else {
            join::WideJoinControl::Healthy
        };
        return join::wide_left_join_shape(&root, control);
    }
    if mode == "joined-shapes" || mode == "joined-attribution-negative" {
        return join::joined_shapes(&root, mode == "joined-attribution-negative");
    }
    if mode == "mixed-grouping-shapes" {
        return grouping_ownership::run(&root);
    }
    if mode == "prepared-aggregate-shapes" || mode == "prepared-aggregate-attribution-negative" {
        return shapes::prepared_aggregate_shapes(
            &root,
            mode == "prepared-aggregate-attribution-negative",
        );
    }
    if mode == "grouped-allocation-shapes" {
        shapes::grouped_allocation_shapes();
        return Ok(());
    }
    if mode == "legacy-constant-shapes" || mode == "legacy-constant-attribution-negative" {
        return shapes::legacy_constant_shapes(
            &root,
            mode == "legacy-constant-attribution-negative",
        );
    }
    if mode == "reader-allocation-shapes" {
        return shapes::reader_shapes(&root);
    }
    if mode == "append-allocation-shapes" || mode == "append-allocation-shapes-negative" {
        shapes::allocation_shapes(mode == "append-allocation-shapes-negative");
        return Ok(());
    }
    if mode == "append-cached-buffers" {
        return shapes::append_shapes(&root);
    }
    let ownership_control = match mode.to_str() {
        Some("ownership") => Some(concurrent::OwnershipControl::Healthy),
        Some("ownership-negative") => Some(concurrent::OwnershipControl::WrongRows),
        Some("ownership-attribution-negative") => {
            Some(concurrent::OwnershipControl::WrongAllowance)
        }
        Some("ownership-worker-panic") => Some(concurrent::OwnershipControl::WorkerPanic),
        Some("ownership-coordinator-panic") => Some(concurrent::OwnershipControl::CoordinatorPanic),
        _ => None,
    };
    if let Some(control) = ownership_control {
        return concurrent::run(&root, control);
    }
    if mode == "mutex" {
        catalog::mutex_contention();
        return Ok(());
    }
    if mode == "declaration-refusal" {
        return catalog::declaration_refusal(&root);
    }
    let lifecycle = mode.to_str().and_then(|mode| {
        [
            "catalog",
            "catalog-recover-empty",
            "catalog-recover-data",
            "catalog-recover-corrupt",
            "catalog-recover-permission",
            "create",
            "create-expanded",
            "open",
            "open-expanded",
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
        // Keep pre-call bytes outside the database so the parent can compare
        // them even if the child aborts before reaching its assertions.
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
        // Error spans must remain usable after the caller releases the SQL text.
        drop(sql);
        let division = db
            .prepare("FROM lineitem |> WHERE l_quantity > 1/0")
            .err()
            .expect("constant division by zero");
        let Error::DivisionByZero {
            span: division_span,
        } = division
        else {
            panic!("division error");
        };
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
        let division_cause = pipesql::CauseKind::DivisionByZero {
            span: division_span,
        };
        let domain = Error::ArithmeticDomain {
            operation: "square root",
            span: division_span,
        };
        let domain_cause = pipesql::CauseKind::ArithmeticDomain {
            operation: "square root",
            span: division_span,
        };
        let logarithm = Error::ArithmeticDomain {
            operation: "natural logarithm",
            span: division_span,
        };
        let logarithm_cause = pipesql::CauseKind::ArithmeticDomain {
            operation: "natural logarithm",
            span: division_span,
        };
        let decimal_logarithm = Error::ArithmeticDomain {
            operation: "base-ten logarithm",
            span: division_span,
        };
        let decimal_logarithm_cause = pipesql::CauseKind::ArithmeticDomain {
            operation: "base-ten logarithm",
            span: division_span,
        };
        let exponential = Error::ArithmeticOverflow {
            operation: "exponentiation",
            span: division_span,
        };
        let exponential_cause = pipesql::CauseKind::ArithmeticOverflow {
            operation: "exponentiation",
            span: division_span,
        };
        let formatted = write!(
            &mut text,
            "{error}; {arithmetic}; {cause}; {division}; {division_cause}; {domain}; {domain_cause}; {logarithm}; {logarithm_cause}; {decimal_logarithm}; {decimal_logarithm_cause}; {exponential}; {exponential_cause}"
        );
        DENY.store(false, Ordering::Relaxed);
        formatted.unwrap();
        let text = std::str::from_utf8(&text.bytes[..text.length]).unwrap();
        assert!(text.contains("open fence for recovery") && text.contains("os error 13"));
        assert_eq!(
            text.matches("arithmetic overflow during addition at bytes 38..61")
                .count(),
            2
        );
        assert_eq!(text.matches("division by zero at bytes 36..39").count(), 2);
        assert_eq!(
            text.matches("arithmetic domain error during square root at bytes 36..39")
                .count(),
            2
        );
        assert_eq!(
            text.matches("arithmetic domain error during natural logarithm at bytes 36..39")
                .count(),
            2
        );
        assert_eq!(
            text.matches("arithmetic domain error during base-ten logarithm at bytes 36..39")
                .count(),
            2
        );
        assert_eq!(
            text.matches("arithmetic overflow during exponentiation at bytes 36..39")
                .count(),
            2
        );
        println!("returned rendered diagnostic: {text}");
        let mut power_text = FixedText {
            bytes: [0; 1_024],
            length: 0,
        };
        DENY.store(deny, Ordering::Relaxed);
        let domain = Error::ArithmeticDomain {
            operation: "power",
            span: division_span,
        };
        let domain_cause = pipesql::CauseKind::ArithmeticDomain {
            operation: "power",
            span: division_span,
        };
        let overflow = Error::ArithmeticOverflow {
            operation: "power",
            span: division_span,
        };
        let overflow_cause = pipesql::CauseKind::ArithmeticOverflow {
            operation: "power",
            span: division_span,
        };
        let formatted = write!(
            &mut power_text,
            "{domain}; {domain_cause}; {overflow}; {overflow_cause}"
        );
        DENY.store(false, Ordering::Relaxed);
        formatted.unwrap();
        let text = std::str::from_utf8(&power_text.bytes[..power_text.length]).unwrap();
        assert_eq!(
            text.matches("arithmetic domain error during power at bytes 36..39")
                .count(),
            2
        );
        assert_eq!(
            text.matches("arithmetic overflow during power at bytes 36..39")
                .count(),
            2
        );
        return Ok(());
    }
    // An unrecognized private file makes loading refuse cleanup authority and
    // leaves the handle unavailable. Resolving a token must report that state
    // even when allocation is subsequently refused.
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
