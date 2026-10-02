//! Refuse native allocations while JSON Lines owns a buffered result prefix.
//!
//! The supervisor independently checks schema, every value and completion in the
//! saved initial/retry streams. A wide repeated projection makes the schema cross
//! the encoder's buffer before query stepping can fail. Caller storage is fixed
//! before the census; diagnostics and owner release remain under allocation denial.

use super::{
    export_support::{Output, database},
    workload::{allocation_cause, arm, finish, format_error, live},
};
use pipesql::{CancellationToken, Database, Error, ExportLimits, PreparedQuery};
use std::{path::Path, sync::atomic::Ordering};

const LIMITS: ExportLimits = ExportLimits {
    rows: 8,
    bytes: 1_000_000,
};
const ALLOCATIONS: usize = 1024;

pub(super) fn run(root: &Path, after: Option<usize>, fail_flush: bool) {
    let (db, config) = database(root);
    let path = root.join("database");
    let cancel = CancellationToken::new();
    let mut sql = String::from("FROM facts |> ORDER BY id |> SELECT ");
    for column in 0..64 {
        if column != 0 {
            sql.push_str(", ");
        }
        sql.push_str(["id", "amount", "number", "day", "note"][column % 5]);
    }
    let query = db.prepare(&sql).unwrap();
    assert_eq!(query.result_column_count(), 64);
    let memory = db.reserved_memory_bytes();
    let generation = db.generation();
    let mut bytes = vec![0; LIMITS.bytes as usize];
    let mut output = Output {
        bytes: &mut bytes,
        length: 0,
        flushes: 0,
        fail_flush,
    };
    let baseline = arm(after, ALLOCATIONS);
    let (result, samples) = {
        let observer = super::transient_ownership::Observer::new(
            &db,
            memory,
            baseline.requested,
            baseline.usable,
        );
        let result = observer.during(|| db.export_jsonl(&query, &mut output, LIMITS, &cancel));
        (result, observer.samples())
    };
    assert!(format_error(result.as_ref().err()));
    assert_eq!(live(), baseline);
    assert_eq!(db.reserved_memory_bytes(), memory);
    assert_eq!(db.reserved_temp_bytes(), 0);
    if fail_flush {
        let mut canary = Vec::<u8>::new();
        assert!(canary.try_reserve_exact(1).is_err());
    }
    finish("jsonl-export", baseline);
    assert!(
        samples.requested_headroom >= 0 && samples.usable_headroom >= 0,
        "JSONL export buffers exceeded reservations: {samples:?}"
    );
    if after.is_none() {
        assert!(samples.allocations > 0 && samples.frees > 0);
    }
    println!("JSONL export allocation/free ownership: {samples:?}");
    let calls = super::CALLS.load(Ordering::Relaxed);
    assert!(calls > 0 && calls <= ALLOCATIONS);
    match &result {
        Ok(rows) => {
            assert!(!fail_flush);
            assert_eq!(*rows, 8);
            assert_eq!(output.flushes, 1);
            println!("JSONL export outcome=exported");
        }
        Err(Error::Io {
            operation: "write result export",
            source,
        }) if fail_flush => {
            assert_eq!(source.kind(), std::io::ErrorKind::BrokenPipe);
            assert_eq!(output.flushes, 1);
            assert!(super::REFUSED.load(Ordering::Relaxed) > 0);
            println!("JSONL export outcome=flush-failed");
        }
        Err(error) => {
            assert!(
                matches!(error, Error::Resource { .. })
                    || matches!(error, Error::Io { source, .. } if source.kind() == std::io::ErrorKind::OutOfMemory)
                    || matches!(error, Error::RecoveryRequired { generation: observed, source }
                    if *observed == generation && allocation_cause(source.kind())),
                "unexpected JSONL allocation failure: {error:?}"
            );
            assert!(!fail_flush);
            assert_eq!(output.flushes, 0);
            println!(
                "JSONL export outcome={}",
                if matches!(error, Error::RecoveryRequired { .. }) {
                    "recovery"
                } else {
                    "refused"
                }
            );
        }
    }
    println!("JSONL export bytes={}", output.length);
    // Faults and observation have ended. Saving caller-owned bytes cannot turn
    // an allocation failure into successful export or flush the encoder's tail.
    std::fs::write(root.join("initial.jsonl"), &output.bytes[..output.length]).unwrap();
    if matches!(result, Err(Error::RecoveryRequired { .. })) {
        let retry_live = live();
        output.length = 0;
        output.flushes = 0;
        let blocked = db.export_jsonl(&query, &mut output, LIMITS, &cancel);
        assert!(
            matches!(blocked, Err(Error::RecoveryRequired { generation: observed, .. }) if observed == generation)
        );
        assert_eq!(output.flushes, 0);
        assert_eq!(live(), retry_live);
        assert_eq!(db.reserved_memory_bytes(), memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
        // Recovery can fail at the first query step, after schema bytes were
        // flushed. The supervisor checks this blocked attempt as a prefix too.
        std::fs::write(root.join("blocked.jsonl"), &output.bytes[..output.length]).unwrap();
        drop(query);
        db.close().unwrap();
        let db = Database::open(&path, config).unwrap();
        assert_eq!(db.generation(), generation);
        let query = db.prepare(&sql).unwrap();
        retry(&db, &query, &mut output, &cancel);
        drop(query);
        db.close().unwrap();
    } else {
        retry(&db, &query, &mut output, &cancel);
        assert_eq!(db.generation(), generation);
        drop(query);
        db.close().unwrap();
    }
    std::fs::write(root.join("retry.jsonl"), &output.bytes[..output.length]).unwrap();
    println!("JSONL export release and complete retry passed");
}

fn retry(
    db: &Database,
    query: &PreparedQuery<'_>,
    output: &mut Output<'_>,
    cancel: &CancellationToken,
) {
    let before = live();
    let memory = db.reserved_memory_bytes();
    output.length = 0;
    output.flushes = 0;
    output.fail_flush = false;
    assert_eq!(db.export_jsonl(query, output, LIMITS, cancel).unwrap(), 8);
    assert_eq!(output.flushes, 1);
    assert_eq!(live(), before);
    assert_eq!(db.reserved_memory_bytes(), memory);
    assert_eq!(db.reserved_temp_bytes(), 0);
}
