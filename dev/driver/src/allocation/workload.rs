//! Refuse allocations during legacy loading and queries, then check recovery.
//!
//! `allocation.rs` calls `run` with an operation and the number of
//! allocations to allow before refusing the rest. A single input row gives
//! literal answers for Q1 and Q6. Load cases also make the construction directory
//! read-only; query cases also damage a payload that the query must read. Combining
//! failures checks whether reporting or cleaning up one failure can lose another.
//!
//! A definite load failure must leave generation zero. An ambiguous commit may
//! have published the row, so reopening must resolve its original transaction
//! before retrying. Queries must release their buffers and leave persisted bytes
//! unchanged. After repairing deliberate damage, the same prepared query must work.
//!
//! The allocator controls below are also used by catalog probes. Their counters
//! cover Rust allocations through `System`; they do not measure all process memory.

use super::{
    ALLOW, CALLS, DENY, LIVE_REQUESTED, LIVE_USABLE, PEAK_REQUESTED, PEAK_USABLE, REFUSED, TRACK,
};
use pipesql::{
    CancellationToken, CauseKind, CommitResolution, Config, Database, Error, QueryResult,
    QueryStep, ResultBatch, TransactionId, Value,
};
use std::os::unix::fs::{FileExt, PermissionsExt};
use std::path::Path;
use std::sync::atomic::Ordering;

const ROW: &[u8] = b"1|2|3|4|1|100|0.08|0.25|R|A|1994-01-01|x|x|x|x|x|\n";
const Q1: &str = include_str!("../../../../test/data/upstream/q1-upstream.pipe.sql");
const Q6: &str = include_str!("../../../../test/data/q6.pipe.sql");
const NAMES: [&str; 5] = [
    "CONTROL",
    "ROOT.A",
    "ROOT.B",
    "WAL",
    "units/0000000000000001.unit",
];
const TEMPORARY_PEAK: u64 = 28_672 + 76;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Live {
    pub(super) requested: usize,
    pub(super) usable: usize,
}

pub(super) fn live() -> Live {
    Live {
        requested: LIVE_REQUESTED.load(Ordering::Relaxed),
        usable: LIVE_USABLE.load(Ordering::Relaxed),
    }
}

// None records a healthy run. Some(n) allows n allocations, then refuses every
// later allocation, including allocations attempted during error handling.
pub(super) fn arm(after: Option<usize>, limit: usize) -> Live {
    assert!(after.is_none_or(|prefix| prefix <= limit));
    ALLOW.store(after.unwrap_or(0), Ordering::Relaxed);
    CALLS.store(0, Ordering::Relaxed);
    REFUSED.store(0, Ordering::Relaxed);
    let baseline = live();
    PEAK_REQUESTED.store(baseline.requested, Ordering::Relaxed);
    PEAK_USABLE.store(baseline.usable, Ordering::Relaxed);
    TRACK.store(true, Ordering::Relaxed);
    DENY.store(after.is_some(), Ordering::Relaxed);
    baseline
}

// Callers check release before suspending faults for a retry. Suspension stops
// refusal and event counting, but live-byte accounting continues.
pub(super) fn suspend_faults() -> (bool, bool) {
    let denied = DENY.swap(false, Ordering::Relaxed);
    let tracked = TRACK.swap(false, Ordering::Relaxed);
    (denied, tracked)
}
pub(super) fn resume_faults((denied, tracked): (bool, bool)) {
    TRACK.store(tracked, Ordering::Relaxed);
    DENY.store(denied, Ordering::Relaxed);
}

pub(super) fn finish(operation: &str, baseline: Live) {
    DENY.store(false, Ordering::Relaxed);
    TRACK.store(false, Ordering::Relaxed);
    // Check release while the caller still holds the outcome. Discarding an
    // error first could hide heap storage retained solely for its diagnostic.
    assert_eq!(
        live(),
        baseline,
        "workload retained a heap owner after return"
    );
    println!(
        "{operation} peak-requested-delta={}",
        PEAK_REQUESTED
            .load(Ordering::Relaxed)
            .checked_sub(baseline.requested)
            .unwrap()
    );
    println!(
        "{operation} peak-usable-delta={}",
        PEAK_USABLE
            .load(Ordering::Relaxed)
            .checked_sub(baseline.usable)
            .unwrap()
    );
    println!("{operation} allocations={}", CALLS.load(Ordering::Relaxed));
    println!("{operation} refusals={}", REFUSED.load(Ordering::Relaxed));
}

pub(super) fn format_error(error: Option<&Error>) -> bool {
    use std::fmt::Write;
    let Some(error) = error else {
        return true;
    };
    let mut text = super::FixedText {
        bytes: [0; 1_024],
        length: 0,
    };
    write!(&mut text, "{error}").is_ok() && text.length > 0
}

pub(super) fn allocation_cause(cause: &CauseKind) -> bool {
    matches!(cause, CauseKind::Resource { .. })
        || matches!(cause, CauseKind::Io { source, .. } if source.kind() == std::io::ErrorKind::OutOfMemory)
}

fn row_matches(batch: &ResultBatch<'_>, row: usize, operation: &str) {
    if operation == "q6" {
        assert_eq!(batch.column_count(), 1);
        assert!(
            matches!(batch.value(row, 0), Some(Value::Double(value)) if value.to_bits() == 8_f64.to_bits())
        );
    } else {
        assert_eq!(batch.column_count(), 10);
        for (index, text) in [(0, "R"), (1, "A")] {
            assert!(
                matches!(batch.value(row, index), Some(Value::String(value)) if value.as_str() == text)
            );
        }
        for (index, expected) in [
            (2, 1_f64),
            (3, 100.),
            (4, 92.),
            (5, 115.),
            (6, 1.),
            (7, 100.),
            (8, 0.08),
        ] {
            assert!(
                matches!(batch.value(row, index), Some(Value::Double(value)) if value.to_bits() == expected.to_bits())
            );
        }
        assert!(matches!(batch.value(row, 9), Some(Value::Int64(1))));
    }
}

fn consume(mut rows: QueryResult<'_, '_>, operation: &str) -> Result<(), Error> {
    let mut count = 0;
    for _ in 0..10_000 {
        match rows.step() {
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    row_matches(&batch, row, operation);
                }
                count += batch.len();
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                assert_eq!(count, 1, "exactly one fixture aggregate row");
                return Ok(());
            }
            QueryStep::Failed(_) => {
                assert_eq!(count, 0, "fixture aggregate failure must not publish rows");
                return Err(rows.into_error().expect("terminal query error"));
            }
        }
    }
    panic!("query exceeded finite fixture step allowance");
}

pub(super) fn run(
    root: &Path,
    operation: &str,
    after: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(root)?;
    let path = root.join("database");
    let input = root.join("lineitem.tbl");
    std::fs::write(&input, ROW)?;
    let config = Config::new(2_000_000, 1_000_000)?;
    let mut db = Database::create(&path, config)?;
    let resident = db.reserved_memory_bytes();
    let cancellation = CancellationToken::new();
    let readonly = operation == "load-readonly";
    if operation == "load" || readonly {
        let private = path.join("private");
        if readonly {
            std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o500))?;
        }
        let mut bytes = [0; 24];
        bytes[..16].copy_from_slice(db.database_identity().as_bytes());
        bytes[16..].copy_from_slice(&1_u64.to_le_bytes());
        let anticipated = TransactionId::from_bytes(bytes)?;
        println!("entered load/publication allocation probe");
        let baseline = arm(after, 128);
        let result = db.load_lineitem(&input, &cancellation);
        let formatted = format_error(result.as_ref().err());
        finish(operation, baseline);
        assert!(formatted, "load diagnostic exceeded fixed caller output");
        if readonly {
            std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700))?;
        }
        let (expected_generation, ambiguous, cleanup) = match result {
            Ok(commit) => {
                assert!(!readonly, "read-only construction unexpectedly succeeded");
                assert_eq!(commit.transaction(), anticipated);
                assert_eq!(db.generation(), 1);
                println!("returned healthy load");
                (Some(1), false, false)
            }
            Err(Error::Io { source, .. })
                if after.is_some() && source.kind() == std::io::ErrorKind::OutOfMemory =>
            {
                println!("returned definite load allocation refusal");
                (Some(0), false, false)
            }
            Err(Error::Io { source, .. })
                if readonly
                    && source.kind() == std::io::ErrorKind::PermissionDenied
                    && source.raw_os_error() == Some(13) =>
            {
                println!("returned expected load-readonly");
                (Some(0), false, false)
            }
            Err(Error::Resource { .. }) if after.is_some() => {
                println!("returned load resource refusal");
                (Some(0), false, false)
            }
            Err(Error::RecoveryRequired {
                generation: 0,
                source,
            }) if after.is_some() && allocation_cause(source.kind()) => {
                println!("returned load admission/issuance recovery debt");
                (Some(0), false, false)
            }
            Err(Error::CleanupRequired { primary, cleanup })
                if after.is_some()
                    && (allocation_cause(primary.kind())
                        || (readonly
                            && matches!(primary.kind(), CauseKind::Io { source, .. }
                        if source.kind() == std::io::ErrorKind::PermissionDenied && source.raw_os_error() == Some(13))))
                    && allocation_cause(cleanup.kind()) =>
            {
                if readonly
                    && matches!(primary.kind(), CauseKind::Io { source, .. }
                    if source.kind() == std::io::ErrorKind::PermissionDenied)
                {
                    println!("returned crossed permission/allocation cleanup debt");
                } else {
                    println!("returned load cleanup debt");
                }
                (Some(0), false, true)
            }
            Err(Error::CommitAmbiguous {
                transaction,
                source,
            }) if after.is_some() && allocation_cause(source.kind()) => {
                assert_eq!(transaction, anticipated);
                assert_eq!(db.generation(), 0);
                assert!(
                    !readonly,
                    "construction refusal cannot enter data publication"
                );
                println!("returned ambiguous load with original token");
                (None, true, false)
            }
            Err(error) => panic!("unexpected load outcome: {error:?}"),
        };
        assert_eq!(db.reserved_memory_bytes(), resident);
        assert_eq!(
            db.reserved_temp_bytes(),
            if ambiguous || cleanup {
                TEMPORARY_PEAK
            } else {
                0
            }
        );
        if ambiguous || cleanup {
            assert!(matches!(
                db.resolve_commit(anticipated),
                Err(Error::RecoveryRequired { .. })
            ));
        }
        db.close()?;
        let mut reopened = Database::open(&path, config)?;
        if let Some(expected) = expected_generation {
            assert_eq!(
                reopened.generation(),
                expected,
                "healed generation contradicts definite load outcome"
            );
        }
        let old_aborted = match reopened.resolve_commit(anticipated) {
            Ok(CommitResolution::Durable(commit)) => {
                assert_eq!(reopened.generation(), 1);
                assert_eq!(commit.transaction(), anticipated);
                false
            }
            Ok(CommitResolution::Aborted) => {
                assert_eq!(reopened.generation(), 0);
                true
            }
            Err(Error::NotFound) if !ambiguous => {
                assert_eq!(reopened.generation(), 0);
                false
            }
            other => panic!("unresolved load after healed reopen: {other:?}"),
        };
        assert_eq!(reopened.reserved_memory_bytes(), resident);
        assert_eq!(reopened.reserved_temp_bytes(), 0);
        if reopened.generation() == 0 {
            let retry = reopened.load_lineitem(&input, &cancellation)?;
            if old_aborted {
                assert_ne!(retry.transaction(), anticipated);
                assert_eq!(
                    reopened.resolve_commit(anticipated)?,
                    CommitResolution::Aborted
                );
            }
            assert_eq!(
                reopened.resolve_commit(retry.transaction())?,
                CommitResolution::Durable(retry)
            );
        }
        // Check recovered data through a query: the generation and transaction
        // receipt alone cannot show whether the row was lost or duplicated.
        let prepared = reopened.prepare(Q1)?;
        consume(reopened.execute(&prepared, &cancellation)?, "q1")?;
        drop(prepared);
        reopened.close()?;
    } else {
        let corrupt = operation.ends_with("-corrupt");
        let query = operation.strip_suffix("-corrupt").unwrap_or(operation);
        assert!(matches!(query, "q1" | "q6"));
        db.load_lineitem(&input, &cancellation)?;
        for (index, name) in NAMES.iter().enumerate() {
            std::fs::copy(path.join(name), root.join(format!("before-{index}")))?;
        }
        let unit = path.join(NAMES[4]);
        let mut original = [0; 1];
        if corrupt {
            // Flip a payload byte without changing its checksum. Keep copies of
            // the damaged files too, so a failed query cannot silently repair them.
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&unit)?;
            file.read_exact_at(&mut original, 28_672)?;
            file.write_all_at(&[original[0] ^ 1], 28_672)?;
            for (index, name) in NAMES.iter().enumerate() {
                std::fs::copy(path.join(name), root.join(format!("fault-{index}")))?;
            }
            std::fs::write(root.join("fault-active"), [])?;
        }
        let prepared = db.prepare(if query == "q1" { Q1 } else { Q6 })?;
        let baseline_memory = db.reserved_memory_bytes();
        println!("entered loaded {operation} execution/iteration/drop allocation probe");
        let baseline = arm(after, 128);
        let mut retained = None;
        let result = db.execute(&prepared, &cancellation).and_then(|rows| {
            let retained_live = live();
            retained = Some((
                db.reserved_memory_bytes(),
                rows.accounted_memory_bytes(),
                retained_live
                    .requested
                    .checked_sub(baseline.requested)
                    .unwrap(),
                retained_live.usable.checked_sub(baseline.usable).unwrap(),
            ));
            consume(rows, query)
        });
        let formatted = format_error(result.as_ref().err());
        finish(operation, baseline);
        assert!(formatted, "query diagnostic exceeded fixed caller output");
        if let Some((charged, running, requested, usable)) = retained {
            assert_eq!(charged, baseline_memory + running);
            assert!(charged <= db.config().memory_limit_bytes());
            assert!(requested <= usable && u64::try_from(usable).unwrap() <= running);
            println!(
                "{operation} retained-requested={requested} retained-usable={usable} retained-charged={charged}"
            );
        }
        match result {
            Ok(()) => {
                assert!(!corrupt, "demanded corrupt payload produced a row");
                println!("returned healthy {operation}");
            }
            Err(Error::Io { source, .. })
                if after.is_some() && source.kind() == std::io::ErrorKind::OutOfMemory =>
            {
                println!("returned typed query allocation refusal");
            }
            Err(Error::Resource { .. }) if after.is_some() => {
                println!("returned query resource refusal")
            }
            Err(Error::Corrupt("demanded unit payload checksum failed")) if corrupt => {
                println!("returned expected {operation}");
            }
            other => panic!("unexpected query outcome: {other:?}"),
        }
        assert_eq!(db.reserved_memory_bytes(), baseline_memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(db.generation(), 1);
        if corrupt {
            for (index, name) in NAMES.iter().enumerate() {
                assert_eq!(
                    std::fs::read(path.join(name))?,
                    std::fs::read(root.join(format!("fault-{index}")))?
                );
            }
            let file = std::fs::OpenOptions::new().write(true).open(&unit)?;
            file.write_all_at(&original, 28_672)?;
            std::fs::remove_file(root.join("fault-active"))?;
        }
        // Reuse the prepared query after refusal or repaired corruption. Cleanup
        // must leave both its buffers and the database ready for another execution.
        consume(db.execute(&prepared, &cancellation)?, query)?;
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), resident);
        for (index, name) in NAMES.iter().enumerate() {
            assert_eq!(
                std::fs::read(path.join(name))?,
                std::fs::read(root.join(format!("before-{index}")))?
            );
        }
        db.close()?;
        assert_eq!(Database::open(&path, config)?.generation(), 1);
    }
    Ok(())
}
