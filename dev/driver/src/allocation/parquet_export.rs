//! Refuse native allocations through Parquet export and require complete retry.
//!
//! Caller-owned fixed storage excludes writer growth from the allocation census.
//! The external input and independently checked stored output cover every scalar
//! type, NULLs and a maximum-length string. Denial stays armed through diagnostic
//! formatting and owner release; a final flush can fail after complete file bytes.

use super::export_support::Output;
use super::workload::{allocation_cause, arm, finish, format_error, live};
use pipesql::{CancellationToken, Database, Error, ParquetExportLimits, PreparedQuery};
use std::{path::Path, sync::atomic::Ordering};

const EXPECTED: &[u8] = include_bytes!("../../../../test/data/parquet/pipesql-v1.parquet");
const ALLOCATIONS: usize = 512;

pub(super) fn run(root: &Path, after: Option<usize>, fail_flush: bool) {
    let (db, config) = super::export_support::database(root);
    let path = root.join("database");
    let cancel = CancellationToken::new();
    let query = db.prepare("FROM facts |> ORDER BY id").unwrap();
    let limits = ParquetExportLimits {
        rows: 8,
        bytes: 100_000,
        row_group_rows: 3,
        row_group_text_bytes: 65_536,
        row_groups: 8,
        metadata_bytes: 16_384,
    };
    let memory = db.reserved_memory_bytes();
    let generation = db.generation();
    let mut bytes = [0_u8; 100_000];
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
        let result = observer.during(|| db.export_parquet(&query, &mut output, limits, &cancel));
        (result, observer.samples())
    };
    assert!(format_error(result.as_ref().err()));
    assert_eq!(live(), baseline);
    assert_eq!(db.reserved_memory_bytes(), memory);
    assert_eq!(db.reserved_temp_bytes(), 0);
    if fail_flush {
        // An actual refused request proves the final-writer fault armed denial.
        let mut canary = Vec::<u8>::new();
        assert!(canary.try_reserve_exact(1).is_err());
    }
    finish("parquet-export", baseline);
    assert!(
        samples.requested_headroom >= 0 && samples.usable_headroom >= 0,
        "Parquet export buffers exceeded reservations: {samples:?}"
    );
    if after.is_none() {
        assert!(samples.allocations > 0 && samples.frees > 0);
    }
    println!("Parquet export allocation/free ownership: {samples:?}");
    let calls = super::CALLS.load(Ordering::Relaxed);
    assert!(calls > 0 && calls <= ALLOCATIONS);
    match &result {
        Ok(rows) => {
            assert!(!fail_flush);
            assert_eq!(*rows, 8);
            assert_eq!(output.flushes, 1);
            assert_eq!(&output.bytes[..output.length], EXPECTED);
            println!("Parquet export outcome=exported");
        }
        Err(Error::Io {
            operation: "write result export",
            source,
        }) if fail_flush => {
            assert_eq!(source.kind(), std::io::ErrorKind::BrokenPipe);
            assert_eq!(output.flushes, 1);
            assert_eq!(&output.bytes[..output.length], EXPECTED);
            assert!(super::REFUSED.load(Ordering::Relaxed) > 0);
            println!("Parquet export outcome=flush-failed");
        }
        Err(error) => {
            assert!(
                matches!(error, Error::Resource { .. })
                    || matches!(error, Error::Io { source, .. } if source.kind() == std::io::ErrorKind::OutOfMemory)
                    || matches!(error, Error::RecoveryRequired { generation: observed, source }
                        if *observed == generation && allocation_cause(source.kind())),
                "unexpected Parquet allocation failure: {error:?}"
            );
            assert!(!fail_flush);
            assert_eq!(output.flushes, 0);
            assert!(output.length < EXPECTED.len());
            assert_eq!(&output.bytes[..output.length], &EXPECTED[..output.length]);
            println!(
                "Parquet export outcome={}",
                if matches!(error, Error::RecoveryRequired { .. }) {
                    "recovery"
                } else {
                    "refused"
                }
            );
        }
    }
    println!("Parquet export bytes={}", output.length);
    if matches!(result, Err(Error::RecoveryRequired { .. })) {
        // Scratch bootstrap may have changed its namespace before refusal.
        // The same query must remain blocked until exclusive reopen repairs it.
        let retry_live = live();
        output.length = 0;
        output.flushes = 0;
        let blocked = db.export_parquet(&query, &mut output, limits, &cancel);
        assert!(
            matches!(blocked, Err(Error::RecoveryRequired { generation: observed, .. }) if observed == generation)
        );
        assert_eq!(output.flushes, 0);
        assert!(output.length < EXPECTED.len());
        assert_eq!(live(), retry_live);
        assert_eq!(db.reserved_memory_bytes(), memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
        drop(query);
        db.close().unwrap();
        let db = Database::open(&path, config).unwrap();
        assert_eq!(db.generation(), generation);
        let query = db.prepare("FROM facts |> ORDER BY id").unwrap();
        retry(&db, &query, &mut output, limits, &cancel);
        drop(query);
        db.close().unwrap();
    } else {
        retry(&db, &query, &mut output, limits, &cancel);
        assert_eq!(db.generation(), generation);
        drop(query);
        db.close().unwrap();
    }
    println!("Parquet export release and complete retry passed");
}

fn retry(
    db: &Database,
    query: &PreparedQuery<'_>,
    output: &mut Output<'_>,
    limits: ParquetExportLimits,
    cancel: &CancellationToken,
) {
    // Complete byte identity checks schema, every typed value, group boundaries
    // and checksums against a retained independently read fixture.
    let retry_live = live();
    let memory = db.reserved_memory_bytes();
    output.length = 0;
    output.flushes = 0;
    output.fail_flush = false;
    assert_eq!(db.export_parquet(query, output, limits, cancel).unwrap(), 8);
    assert_eq!(output.flushes, 1);
    assert_eq!(&output.bytes[..output.length], EXPECTED);
    assert_eq!(live(), retry_live);
    assert_eq!(db.reserved_memory_bytes(), memory);
    assert_eq!(db.reserved_temp_bytes(), 0);
}
