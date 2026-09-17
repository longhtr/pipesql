//! Check report answers and allocation release through repeated query histories.
//!
//! Input setup is shared with the example; literal expected groups remain here.
//! Preparation, execution, cancellation and refusal each retain their own
//! observation window so omitted work or cleanup cannot pass as lower memory use.
//!
//! Each history abandons a partial result, cancels after spill and then completes
//! the report on the same handle. Literal groups and restored owner baselines
//! jointly check that both the answer and cleanup survived the preceding failures.

use super::{LIVE_REQUESTED, LIVE_USABLE, measurement::Live};
use pipesql::{
    AppendLimits, CancellationToken, Config, DataType, Database, Error, QueryStep, Value,
};
use std::{path::Path, sync::atomic::Ordering};

#[path = "../../../../examples/support/event_data.rs"]
mod event_data;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EventReportControl {
    Healthy,
    WrongAttribution,
    MissingPreparation,
    MissingConstruction,
    MissingTerminal,
}

/// Check report answers and memory release across repeated uses of one database.
///
/// Each history prepares the report, abandons one execution after rows arrive,
/// cancels another after temporary space is reserved, then consumes a complete
/// result. The middle history also refuses each allocation in preparation and
/// execution construction. Every path must return to its prior memory and
/// temporary-space usage while leaving the database usable for the next run.
///
/// Compare rows directly with the literal groups below: collecting result rows
/// in a new vector would add test allocations to the engine observations. The
/// controls deliberately misattribute memory or omit observations, so the same
/// checks must reject an incomplete account even when query answers are correct.
pub(super) fn event_report_history(
    root: &Path,
    control: EventReportControl,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(root)?;
    let path = root.join("database");
    let cancel = CancellationToken::new();
    let config = Config::new(32_000_000, 32_000_000)?;
    let db = Database::create_empty(&path, config)?;
    event_data::declare(&db, &cancel)?;
    let mut append = db.begin_append(
        "events",
        AppendLimits {
            batches: 1,
            encoded_bytes: 4096,
        },
        &cancel,
    )?;
    event_data::write_events(&mut append, &event_data::EVENTS, &cancel)?;
    append.commit(&cancel)?;
    db.close()?;
    let db = Database::open(&path, config)?;
    println!("entered event report allocation history");
    super::transient_ownership::check_calibration(&db, false);
    let before = Live::now();
    let resident = db.reserved_memory_bytes();
    let expected = [
        (None, None, 1, 1, Some(11)),
        (None, Some(""), 1, 1, Some(5)),
        (None, Some("north"), 1, 1, Some(7)),
        (None, Some("south"), 1, 0, None),
        (None, Some("南"), 1, 0, None),
        (Some(1999), Some("north"), 2, 2, Some(8)),
        (Some(2000), None, 3, 2, Some(33)),
        (Some(2000), Some(""), 2, 2, Some(9)),
        (Some(2000), Some("south"), 2, 2, Some(15)),
        (Some(2000), Some("南"), 2, 2, Some(15)),
        (Some(2001), Some("north"), 2, 1, Some(13)),
        (Some(2001), Some("south"), 1, 1, Some(40)),
        (Some(2001), Some("南"), 1, 1, Some(40)),
    ];
    for history in 0..3 {
        if history == 1 {
            refuse_report_preparation(&db, control == EventReportControl::MissingPreparation);
            refuse_report_construction(&db, control == EventReportControl::MissingConstruction);
        }
        // Subtracting a fictitious resident reservation leaves too little charge
        // for the query's allocations. The observer must detect this deficit.
        let attributed_resident = resident
            + if control == EventReportControl::WrongAttribution {
                32_000_000
            } else {
                0
            };
        let prepare = super::transient_ownership::Observer::new(
            &db,
            attributed_resident,
            before.requested,
            before.usable,
        );
        let query =
            prepare.during(|| db.prepare(include_str!("../../../../examples/event_report.sql")))?;
        check_report_phase("prepare", prepare.samples(), true);
        assert_eq!(query.result_column_count(), 5);
        for (index, (name, data_type, nullable)) in [
            ("calendar_year", DataType::Int64, true),
            ("label", DataType::String, true),
            ("entries", DataType::Int64, false),
            ("present", DataType::Int64, false),
            ("total", DataType::Int64, true),
        ]
        .into_iter()
        .enumerate()
        {
            let actual = query.result_column(index).unwrap();
            assert_eq!(
                (actual.name, actual.data_type, actual.nullable),
                (Some(name), data_type, nullable)
            );
        }
        // Dropping after rows and cancelling during spill end execution at
        // different points. Both must free its buffers while keeping the plan.
        for cancelled in [false, true] {
            let observer = super::transient_ownership::Observer::new(
                &db,
                resident,
                before.requested,
                before.usable,
            );
            let token = CancellationToken::new();
            let mut partial = observer.during(|| db.execute(&query, &token))?;
            let mut reached = false;
            let mut prefix = 0;
            for _ in 0..200_000 {
                match observer.during(|| partial.step()) {
                    QueryStep::Progress => (),
                    QueryStep::Rows(batch) if !cancelled => {
                        check_event_rows(&batch, &expected, &mut prefix);
                        reached = true;
                        break;
                    }
                    _ => panic!("report missed its interruption point"),
                }
                if cancelled && db.reserved_temp_bytes() > 0 {
                    reached = true;
                    break;
                }
            }
            assert!(reached);
            if cancelled {
                token.cancel();
                let terminal = if control == EventReportControl::MissingTerminal {
                    partial.step()
                } else {
                    observer.during(|| partial.step())
                };
                assert!(matches!(terminal, QueryStep::Failed(Error::Cancelled)));
                let error = observer
                    .during(|| partial.into_error())
                    .expect("owned cancellation error");
                assert!(matches!(error, Error::Cancelled));
                assert_eq!(
                    db.reserved_memory_bytes(),
                    resident + query.accounted_memory_bytes()
                );
                assert_eq!(db.reserved_temp_bytes(), 0);
                observer.during(|| drop(error));
            } else {
                assert!(prefix > 0);
                observer.during(|| drop(partial));
            }
            assert_eq!(
                db.reserved_memory_bytes(),
                resident + query.accounted_memory_bytes()
            );
            assert_eq!(db.reserved_temp_bytes(), 0);
            assert_eq!(
                observer.samples().allocations,
                observer.samples().frees,
                "missing report terminal allocation events"
            );
            check_report_phase(
                if cancelled {
                    "cancelled"
                } else {
                    "partial drop"
                },
                observer.samples(),
                true,
            );
        }
        let execution = super::transient_ownership::Observer::new(
            &db,
            resident,
            before.requested,
            before.usable,
        );
        let mut result = execution.during(|| db.execute(&query, &cancel))?;
        let (mut seen, mut finished) = (0, false);
        for _ in 0..200_000 {
            assert_eq!(
                db.reserved_memory_bytes(),
                resident + query.accounted_memory_bytes() + result.accounted_memory_bytes()
            );
            match execution.during(|| result.step()) {
                QueryStep::Progress => (),
                QueryStep::Rows(batch) => check_event_rows(&batch, &expected, &mut seen),
                QueryStep::Finished => {
                    finished = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("event report: {error}"),
            }
        }
        assert!(finished);
        assert_eq!(seen, expected.len());
        execution.during(|| drop(result));
        assert_eq!(execution.samples().allocations, execution.samples().frees);
        check_report_phase("execute/finish/drop", execution.samples(), true);
        let release = super::transient_ownership::Observer::new(
            &db,
            resident,
            before.requested,
            before.usable,
        );
        release.during(|| drop(query));
        check_report_phase("prepared drop", release.samples(), false);
        assert_eq!(Live::now(), before);
        assert_eq!(db.reserved_memory_bytes(), resident);
        assert_eq!(db.reserved_temp_bytes(), 0);
        println!("event report history={history} rows={seen} release=complete");
    }
    db.close()?;
    println!("event report histories passed: 3 complete runs");
    Ok(())
}

fn check_report_phase(phase: &str, samples: super::transient_ownership::Samples, allocates: bool) {
    println!("event report {phase}: {samples:?}");
    assert!(
        samples.frees > 0 && (!allocates || samples.allocations > 0),
        "missing transient report events: {phase}"
    );
    assert!(
        samples.requested_headroom >= 0,
        "event report requested ownership: {phase}: {samples:?}"
    );
    assert!(
        samples.usable_headroom >= 0,
        "event report usable ownership: {phase}: {samples:?}"
    );
}

type EventGroup = (Option<i64>, Option<&'static str>, i64, i64, Option<i64>);

fn check_event_rows(batch: &pipesql::ResultBatch<'_>, expected: &[EventGroup], seen: &mut usize) {
    for row in 0..batch.len() {
        let &(year, label, entries, present, total) =
            expected.get(*seen).expect("extra event report group");
        assert_eq!(
            batch.value(row, 0),
            Some(year.map_or(Value::Null, Value::Int64))
        );
        match (batch.value(row, 1), label) {
            (Some(Value::Null), None) => (),
            (Some(Value::String(value)), Some(label)) => assert_eq!(value.as_str(), label),
            _ => panic!("event report label"),
        }
        assert_eq!(batch.value(row, 2), Some(Value::Int64(entries)));
        assert_eq!(batch.value(row, 3), Some(Value::Int64(present)));
        assert_eq!(
            batch.value(row, 4),
            Some(total.map_or(Value::Null, Value::Int64))
        );
        *seen += 1;
    }
}

fn refuse_report_construction(db: &Database, missing_observation: bool) {
    // Keep preparation outside the refusal window. Failed execution construction
    // must release its own memory without releasing or damaging the prepared plan.
    use super::{CALLS, REFUSED, workload};
    const LIMIT: usize = 512;
    let resident = db.reserved_memory_bytes();
    let outside = Live::now();
    let query = db
        .prepare(include_str!("../../../../examples/event_report.sql"))
        .unwrap();
    let prepared = Live::now();
    let memory = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let census_observer =
        super::transient_ownership::Observer::new(db, memory, prepared.requested, prepared.usable);
    let baseline = workload::arm(None, LIMIT);
    let result = census_observer.during(|| db.execute(&query, &cancel));
    workload::suspend_faults();
    let census = CALLS.load(Ordering::Relaxed);
    assert!(census > 0 && census <= LIMIT);
    assert_eq!(REFUSED.load(Ordering::Relaxed), 0);
    census_observer.during(|| drop(result.unwrap()));
    workload::finish("report construction census", baseline);
    check_report_phase("construction census", census_observer.samples(), true);
    for prefix in 0..=census {
        let observer = super::transient_ownership::Observer::new(
            db,
            memory,
            prepared.requested,
            prepared.usable,
        );
        let baseline = workload::arm(Some(prefix), LIMIT);
        let result = if missing_observation && prefix == 1 {
            db.execute(&query, &cancel)
        } else {
            observer.during(|| db.execute(&query, &cancel))
        };
        if let Err(error) = &result {
            assert_eq!(LIVE_REQUESTED.load(Ordering::Relaxed), prepared.requested);
            assert_eq!(LIVE_USABLE.load(Ordering::Relaxed), prepared.usable);
            assert_eq!(db.reserved_memory_bytes(), memory);
            assert_eq!(db.reserved_temp_bytes(), 0);
            assert!(workload::format_error(Some(error)));
        }
        workload::suspend_faults();
        let calls = CALLS.load(Ordering::Relaxed);
        let refusals = REFUSED.load(Ordering::Relaxed);
        if prefix < census {
            let error = match result {
                Err(error) => error,
                Ok(_) => panic!("report accepted refused construction"),
            };
            assert!(
                matches!(&error, Error::Resource { .. })
                    || matches!(&error, Error::Io { source, .. } if source.kind() == std::io::ErrorKind::OutOfMemory)
            );
            assert!(calls > prefix && refusals > 0);
            assert_eq!(Live::now(), prepared);
            println!("event report refused prefix={prefix}: {error}");
            workload::finish("report construction refusal", baseline);
            observer.during(|| drop(error));
        } else {
            assert_eq!(calls, census);
            assert_eq!(refusals, 0);
            observer.during(|| drop(result.unwrap()));
            workload::finish("report construction control", baseline);
        }
        assert_eq!(Live::now(), prepared);
        assert_eq!(db.reserved_memory_bytes(), memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
        let samples = observer.samples();
        assert_eq!(
            samples.allocations, prefix,
            "missing report construction allocation events"
        );
        assert_eq!(samples.frees, prefix);
        assert!(
            samples.requested_headroom >= 0,
            "report refused requested ownership: {prefix}: {samples:?}"
        );
        assert!(
            samples.usable_headroom >= 0,
            "report refused usable ownership: {prefix}: {samples:?}"
        );
        println!(
            "event report construction prefix={prefix} calls={calls} refusals={refusals} samples={samples:?}"
        );
    }
    drop(query);
    assert_eq!(Live::now(), outside);
    assert_eq!(db.reserved_memory_bytes(), resident);
    println!("event report construction passed: prefixes=0..={census}; live errors and release");
}

fn refuse_report_preparation(db: &Database, missing_observation: bool) {
    // Each prefix starts from the same live database. Inspect and format the
    // error before suspending faults, so reporting cannot hide an allocation.
    use super::{CALLS, REFUSED, workload};
    const LIMIT: usize = 32;
    let before = Live::now();
    let resident = db.reserved_memory_bytes();
    let source = include_str!("../../../../examples/event_report.sql");
    let observer =
        super::transient_ownership::Observer::new(db, resident, before.requested, before.usable);
    let baseline = workload::arm(None, LIMIT);
    let prepared = observer.during(|| db.prepare(source));
    workload::suspend_faults();
    let census = CALLS.load(Ordering::Relaxed);
    assert!(census > 0 && census <= LIMIT);
    assert_eq!(REFUSED.load(Ordering::Relaxed), 0);
    observer.during(|| drop(prepared.unwrap()));
    workload::finish("report preparation census", baseline);
    check_report_phase("preparation census", observer.samples(), true);
    for prefix in 0..=census {
        let observer = super::transient_ownership::Observer::new(
            db,
            resident,
            before.requested,
            before.usable,
        );
        let baseline = workload::arm(Some(prefix), LIMIT);
        let prepared = if missing_observation && prefix == 1 {
            db.prepare(source)
        } else {
            observer.during(|| db.prepare(source))
        };
        if let Err(error) = &prepared {
            assert_eq!(LIVE_REQUESTED.load(Ordering::Relaxed), before.requested);
            assert_eq!(LIVE_USABLE.load(Ordering::Relaxed), before.usable);
            assert_eq!(db.reserved_memory_bytes(), resident);
            assert_eq!(db.reserved_temp_bytes(), 0);
            assert!(workload::format_error(Some(error)));
        }
        workload::suspend_faults();
        let calls = CALLS.load(Ordering::Relaxed);
        let refusals = REFUSED.load(Ordering::Relaxed);
        if prefix < census {
            let error = match prepared {
                Err(error) => error,
                Ok(_) => panic!("report accepted refused preparation"),
            };
            assert!(
                matches!(&error, Error::Resource { .. })
                    || matches!(&error, Error::Io { source, .. } if source.kind() == std::io::ErrorKind::OutOfMemory)
            );
            assert!(calls > prefix && refusals > 0);
            assert_eq!(Live::now(), before);
            workload::finish("report preparation refusal", baseline);
            observer.during(|| drop(error));
        } else {
            assert_eq!(calls, census);
            assert_eq!(refusals, 0);
            observer.during(|| drop(prepared.unwrap()));
            workload::finish("report preparation control", baseline);
        }
        assert_eq!(Live::now(), before);
        assert_eq!(db.reserved_memory_bytes(), resident);
        assert_eq!(db.reserved_temp_bytes(), 0);
        let samples = observer.samples();
        assert_eq!(
            samples.allocations, prefix,
            "missing report preparation allocation events"
        );
        assert_eq!(samples.frees, prefix);
        assert!(
            samples.requested_headroom >= 0 && samples.usable_headroom >= 0,
            "report preparation transient ownership: {prefix}: {samples:?}"
        );
        println!(
            "event report preparation prefix={prefix} calls={calls} refusals={refusals} samples={samples:?}"
        );
    }
    println!("event report preparation passed: prefixes=0..={census}; live errors and release");
}
