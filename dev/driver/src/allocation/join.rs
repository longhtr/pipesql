//! Check join answers and memory through spill, failure and early release.
//!
//! Literal row pairs distinguish duplicate matches and unmatched left rows.
//! Refusal checks require complete observations during preparation, construction
//! and execution, then reuse the same database to expose retained state.
//!
//! Wide text and composed joins force sorted inputs and exercise overlapping buffer
//! owners. Deliberate controls remove observations or corrupt attribution, so the
//! supervisor can distinguish a real memory bound from a missing measurement.

use super::{
    LIVE_REQUESTED, LIVE_USABLE,
    measurement::{Heap, Live},
};
use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, Error, QueryStep, Value,
};
use std::{path::Path, sync::atomic::Ordering};

const ROWS: usize = 4096;
const TEMP: u64 = 8_000_000;

pub(super) enum WideJoinControl {
    Healthy,
    WrongAttribution,
    DisabledObserver,
    MissingPreparation,
    MissingFailedPreparation,
    MissingFailedExecution,
    MissingConstruction,
    Construction,
    ConstructionSequence,
}

/// Check a 64-column left join through completion, failure and early drop.
///
/// Eleven literal row pairs distinguish duplicate matches from unmatched left
/// rows. Large text fields force temporary storage. Allocation observers cover
/// calls as well as the gaps between them, including the call that reports an
/// error and must release execution buffers. Controls omit observations or
/// falsify memory attribution to show that these checks detect missing evidence.
pub(super) fn wide_left_join_shape(
    root: &Path,
    control: WideJoinControl,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(root)?;
    let path = root.join("database");
    let cancel = CancellationToken::new();
    let db = Database::create_empty(&path, Config::new(192_000_000, 256_000_000)?)?;
    let long = "雪".repeat(21_845) + "x";
    let short = "a\0雪";
    let texts = [None, Some(short), Some(long.as_str()), Some("")];
    let left_text = [
        Some(long.as_str()),
        Some(short),
        None,
        Some(long.as_str()),
        Some(""),
        Some(long.as_str()),
    ];
    let left_keys = [None, Some(1), Some(1), Some(2), Some(3), Some(4)];
    let right_keys = [None, Some(1), Some(1), Some(1), Some(3), Some(3)];
    // Three left fields and 61 right fields fill the width limit. Even the
    // required right-side id must become NULL when a left row has no match.
    for (table, width, keys) in [("left_rows", 3, left_keys), ("right_rows", 61, right_keys)] {
        let names: Vec<_> = (0..width)
            .map(|column| match column {
                0 => "key".to_owned(),
                1 => "id".to_owned(),
                _ => format!("text{column}"),
            })
            .collect();
        let declarations: Vec<_> = names
            .iter()
            .enumerate()
            .map(|(column, name)| ColumnDeclaration {
                name,
                data_type: if column < 2 {
                    DataType::Int64
                } else {
                    DataType::String
                },
                nullable: column != 1,
            })
            .collect();
        db.declare_table(table, &declarations, &cancel)?;
        let mut append = db.begin_append(
            table,
            AppendLimits {
                batches: 6,
                encoded_bytes: 32_000_000,
            },
            &cancel,
        )?;
        for (id, key) in keys.into_iter().enumerate() {
            let key_value = [key.unwrap_or(999)];
            let key_valid = [u8::from(key.is_some())];
            let id_value = [id as i64];
            let text_values: Vec<_> = (2..width)
                .map(|column| {
                    let value = if width == 3 {
                        left_text[id]
                    } else {
                        texts[(id + column - 2) % 4]
                    };
                    ([value.unwrap_or("hidden")], [u8::from(value.is_some())])
                })
                .collect();
            let mut inputs = vec![
                ColumnInput {
                    values: ColumnValues::Int64(&key_value),
                    validity: &key_valid,
                },
                ColumnInput {
                    values: ColumnValues::Int64(&id_value),
                    validity: &[1],
                },
            ];
            inputs.extend(text_values.iter().map(|(values, valid)| ColumnInput {
                values: ColumnValues::String(values),
                validity: valid,
            }));
            append.write(&inputs, &cancel)?;
        }
        append.commit(&cancel)?;
    }
    // Two left rows with key 1 each match three right rows. Key 3 matches twice;
    // NULL, 2 and 4 each produce one unmatched row. Check pairs without assuming
    // an order among equal keys: 2*3 + 2 + 3 = 11.
    let expected = [
        (0, None),
        (1, Some(1)),
        (1, Some(2)),
        (1, Some(3)),
        (2, Some(1)),
        (2, Some(2)),
        (2, Some(3)),
        (3, None),
        (4, Some(4)),
        (4, Some(5)),
        (5, None),
    ];
    let check_text = |actual: Option<Value<'_>>, expected: Option<&str>| match (actual, expected) {
        (Some(Value::Null), None) => (),
        (Some(Value::String(actual)), Some(expected)) => assert_eq!(actual.as_str(), expected),
        _ => panic!("wide left join STRING field"),
    };
    println!("wide left join: 64 columns, unequal duplicate groups and nullable maximum STRING");
    let before = Live::now();
    let memory = db.reserved_memory_bytes();
    const SOURCE: &str = "FROM left_rows AS l |> LEFT JOIN right_rows AS r ON l.key=r.key";
    check_failed_join_preparation(
        &db,
        SOURCE,
        before,
        memory,
        matches!(control, WideJoinControl::MissingFailedPreparation),
    );
    if matches!(
        control,
        WideJoinControl::Construction
            | WideJoinControl::ConstructionSequence
            | WideJoinControl::MissingConstruction
    ) {
        check_failed_join_construction(
            &db,
            SOURCE,
            before,
            memory,
            matches!(control, WideJoinControl::MissingConstruction),
        );
    }
    // The normal campaign runs construction refusal and expression failure in
    // separate processes. ConstructionSequence combines them for diagnosis;
    // results from the separate runs do not qualify that allocator history.
    if !matches!(
        control,
        WideJoinControl::Construction | WideJoinControl::MissingConstruction
    ) {
        check_failed_join_execution(
            &db,
            SOURCE,
            before,
            memory,
            matches!(control, WideJoinControl::MissingFailedExecution),
        );
    }
    let preparation =
        super::transient_ownership::Observer::new(&db, memory, before.requested, before.usable);
    let query = if matches!(control, WideJoinControl::MissingPreparation) {
        db.prepare(SOURCE)?
    } else {
        preparation.during(|| db.prepare(SOURCE))?
    };
    assert_eq!(query.result_column_count(), 64);
    let observer =
        super::transient_ownership::Observer::new(&db, memory, before.requested, before.usable);
    let mut result = observer.during(|| db.execute(&query, &cancel))?;
    let mut seen = [false; 11];
    let mut finished = false;
    let mut steps = 0;
    let mut peak_temp = 0;
    let mut minimum_headroom = i128::MAX;
    loop {
        let charge = query.accounted_memory_bytes() + result.accounted_memory_bytes();
        assert_eq!(db.reserved_memory_bytes(), memory + charge);
        let heap = Heap::now().increase_from(Heap {
            requested: before.requested,
            usable: before.usable,
        });
        assert!(
            heap.requested as u64 <= charge,
            "wide left join requested admission"
        );
        let attributed = heap.usable as u128
            + u128::from(matches!(control, WideJoinControl::WrongAttribution)) * u128::from(charge);
        minimum_headroom = minimum_headroom.min(i128::from(charge) - attributed as i128);
        peak_temp = peak_temp.max(db.reserved_temp_bytes());
        if finished {
            break;
        }
        steps += 1;
        assert!(steps < 200_000, "wide left join did not finish");
        match observer.during(|| result.step()) {
            QueryStep::Progress => (),
            QueryStep::Finished => finished = true,
            QueryStep::Failed(error) => panic!("wide left join: {error}"),
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 64);
                for row in 0..batch.len() {
                    let Some(Value::Int64(left)) = batch.value(row, 1) else {
                        panic!("left row id");
                    };
                    let right = match batch.value(row, 4) {
                        Some(Value::Int64(id)) => Some(id),
                        Some(Value::Null) => None,
                        _ => panic!("right row id"),
                    };
                    let position = expected
                        .iter()
                        .position(|pair| *pair == (left, right))
                        .expect("unexpected joined pair");
                    assert!(!seen[position], "duplicate joined pair");
                    seen[position] = true;
                    assert_eq!(
                        batch.value(row, 0),
                        Some(left_keys[left as usize].map_or(Value::Null, Value::Int64))
                    );
                    check_text(batch.value(row, 2), left_text[left as usize]);
                    assert_eq!(
                        batch.value(row, 3),
                        Some(
                            right
                                .and_then(|id| right_keys[id as usize])
                                .map_or(Value::Null, Value::Int64)
                        )
                    );
                    for column in 5..64 {
                        let text = right.and_then(|id| texts[(id as usize + column - 5) % 4]);
                        check_text(batch.value(row, column), text);
                    }
                }
            }
        }
    }
    assert!(seen.into_iter().all(|seen| seen), "missing joined pair");
    assert!(peak_temp > 0, "wide left join must use external storage");
    let finished_release =
        super::transient_ownership::Observer::new(&db, memory, before.requested, before.usable);
    finished_release.during(|| drop(result));
    // Finished freed execution buffers inside step(). Dropping the remaining
    // handle releases its inline reservation and need not produce a heap event.
    assert_eq!(
        db.reserved_memory_bytes(),
        memory + query.accounted_memory_bytes()
    );
    let prepared_release =
        super::transient_ownership::Observer::new(&db, memory, before.requested, before.usable);
    prepared_release.during(|| drop(query));
    assert_eq!(Live::now(), before);
    assert_eq!(db.reserved_memory_bytes(), memory);
    assert_eq!(db.reserved_temp_bytes(), 0);
    let samples = observer.samples();
    println!("wide left join transient: {samples:?}");
    assert!(
        samples.allocations > 0 && samples.frees > 0,
        "missing transient join events"
    );
    assert!(
        samples.requested_headroom >= 0 && samples.usable_headroom >= 0,
        "wide left join transient ownership: {samples:?}"
    );
    check_join_phase("preparation", preparation.samples(), true);
    check_join_phase("prepared release", prepared_release.samples(), false);
    let finished_samples = finished_release.samples();
    assert!(finished_samples.requested_headroom >= 0 && finished_samples.usable_headroom >= 0);
    println!(
        "wide left join finished release: allocations={} frees={}; charge released",
        finished_samples.allocations, finished_samples.frees
    );
    // Test both early-drop paths: before the first step and after spill begins.
    // Each must restore the same memory and file counts as the complete run.
    for external in [false, true] {
        let preparation =
            super::transient_ownership::Observer::new(&db, memory, before.requested, before.usable);
        let query = preparation.during(|| db.prepare(SOURCE))?;
        let execution =
            super::transient_ownership::Observer::new(&db, memory, before.requested, before.usable);
        let mut result = execution.during(|| db.execute(&query, &cancel))?;
        if external {
            let mut steps = 0;
            loop {
                steps += 1;
                assert!(
                    steps < 200_000,
                    "wide left join did not reach external storage"
                );
                match execution.during(|| result.step()) {
                    QueryStep::Progress => (),
                    _ => panic!("wide left join must reach external storage before rows"),
                }
                if db.reserved_temp_bytes() > 0 {
                    break;
                }
            }
        }
        assert_eq!(
            db.reserved_memory_bytes(),
            memory + query.accounted_memory_bytes() + result.accounted_memory_bytes()
        );
        let release =
            super::transient_ownership::Observer::new(&db, memory, before.requested, before.usable);
        release.during(|| drop(result));
        assert_eq!(
            db.reserved_memory_bytes(),
            memory + query.accounted_memory_bytes()
        );
        assert_eq!(db.reserved_temp_bytes(), 0);
        let prepared_release =
            super::transient_ownership::Observer::new(&db, memory, before.requested, before.usable);
        prepared_release.during(|| drop(query));
        assert_eq!(Live::now(), before);
        assert_eq!(db.reserved_memory_bytes(), memory);
        check_join_phase("abandoned preparation", preparation.samples(), true);
        let executed = execution.samples();
        assert!(
            executed.allocations > 0
                && executed.requested_headroom >= 0
                && executed.usable_headroom >= 0,
            "abandoned execution: {executed:?}"
        );
        check_join_phase(
            if external {
                "external abandonment"
            } else {
                "immediate abandonment"
            },
            release.samples(),
            false,
        );
        check_join_phase(
            "abandoned prepared release",
            prepared_release.samples(),
            false,
        );
    }
    println!("wide left join lifecycle passed: preparation, finished release, two abandonments");
    super::transient_ownership::check_calibration(
        &db,
        matches!(control, WideJoinControl::DisabledObserver),
    );
    db.close()?;
    println!(
        "wide left join rows=11 steps={steps} minimum-usable-headroom={minimum_headroom} temporary={peak_temp} release=complete"
    );
    assert!(
        minimum_headroom >= 0,
        "wide left join usable ownership attribution"
    );
    println!("wide left join passed: 64 columns, 11 pairs; rows, ownership and release");
    Ok(())
}

// Refuse allocations only during prepare. Enumerating file descriptors itself
// allocates, so inspect them after disabling faults; check heap counters first.
fn check_failed_join_preparation(
    db: &Database,
    source: &str,
    before: Live,
    memory: u64,
    missing_observation: bool,
) {
    use super::{CALLS, REFUSED, workload};
    const ALLOCATION_LIMIT: usize = 32;
    let observer =
        super::transient_ownership::Observer::new(db, memory, before.requested, before.usable);
    let baseline = workload::arm(None, ALLOCATION_LIMIT);
    let prepared = observer.during(|| db.prepare(source));
    workload::suspend_faults();
    let census = CALLS.load(Ordering::Relaxed);
    assert!(census > 1 && census <= ALLOCATION_LIMIT);
    assert_eq!(REFUSED.load(Ordering::Relaxed), 0);
    let query = prepared.expect("healthy join preparation census");
    assert_eq!(query.result_column_count(), 64);
    drop(query);
    workload::finish("join preparation census", baseline);
    assert_eq!(Live::now(), before);
    assert_eq!(db.reserved_memory_bytes(), memory);
    check_join_phase("preparation census", observer.samples(), true);
    for prefix in 0..=census {
        let observer =
            super::transient_ownership::Observer::new(db, memory, before.requested, before.usable);
        let baseline = workload::arm(Some(prefix), ALLOCATION_LIMIT);
        let prepared = if missing_observation && prefix == 1 {
            db.prepare(source)
        } else {
            observer.during(|| db.prepare(source))
        };
        if prepared.is_err() {
            // Read counters without allocating while the returned error is live.
            assert_eq!(LIVE_REQUESTED.load(Ordering::Relaxed), before.requested);
            assert_eq!(LIVE_USABLE.load(Ordering::Relaxed), before.usable);
            assert_eq!(db.reserved_memory_bytes(), memory);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
        workload::suspend_faults();
        let calls = CALLS.load(Ordering::Relaxed);
        let refusals = REFUSED.load(Ordering::Relaxed);
        if prefix < census {
            let error = match prepared {
                Err(error) => error,
                Ok(_) => panic!("join preparation accepted a refused allocation"),
            };
            assert!(
                matches!(&error, Error::Resource { .. })
                    || matches!(&error, Error::Io { source, .. }
                        if source.kind() == std::io::ErrorKind::OutOfMemory),
                "join preparation allocation outcome: {error}"
            );
            assert!(calls > prefix && refusals > 0);
            // Include descriptor counts now that test allocations are allowed.
            assert_eq!(Live::now(), before);
            assert_eq!(db.reserved_memory_bytes(), memory);
            assert_eq!(db.reserved_temp_bytes(), 0);
            workload::finish("join preparation refusal", baseline);
            drop(error);
        } else {
            assert_eq!(calls, census);
            assert_eq!(refusals, 0);
            let query = prepared.expect("full-prefix join preparation control");
            assert_eq!(query.result_column_count(), 64);
            drop(query);
            workload::finish("join preparation control", baseline);
        }
        assert_eq!(Live::now(), before);
        assert_eq!(db.reserved_memory_bytes(), memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
        let samples = observer.samples();
        // A zero prefix allocates nothing. Every shorter-than-complete prefix
        // must free all buffers it obtained before the refusal.
        assert!(
            samples.allocations == prefix && (prefix == census || samples.frees == prefix),
            "missing failed preparation events: prefix={prefix}: {samples:?}"
        );
        assert!(
            samples.requested_headroom >= 0 && samples.usable_headroom >= 0,
            "failed join preparation ownership: prefix={prefix}: {samples:?}"
        );
        if prefix == 0 {
            println!("join preparation prefix=0 calls={calls} refusals={refusals} samples=none");
        } else {
            println!(
                "join preparation prefix={prefix} calls={calls} refusals={refusals} samples={samples:?}"
            );
        }
    }
    // EXP(1000) fails during preparation after join descriptors have been built.
    // Allocate the test's SQL string before observing those engine allocations.
    let late_source = format!("{source} |> WHERE l.id > EXP(1000)");
    let start = late_source.find("EXP(1000)").unwrap();
    let late_before = Live::now();
    let observer = super::transient_ownership::Observer::new(
        db,
        memory,
        late_before.requested,
        late_before.usable,
    );
    let error = match observer.during(|| db.prepare(&late_source)) {
        Err(error) => error,
        Ok(_) => panic!("late join preparation must fail"),
    };
    assert_eq!(Live::now(), late_before);
    assert_eq!(db.reserved_memory_bytes(), memory);
    assert_eq!(db.reserved_temp_bytes(), 0);
    drop(late_source);
    assert_eq!(Live::now(), before);
    let Error::ArithmeticOverflow { operation, span } = &error else {
        panic!("late join preparation outcome: {error}");
    };
    assert_eq!(*operation, "exponentiation");
    assert_eq!((span.start(), span.end()), (start, start + 9));
    assert!(workload::format_error(Some(&error)));
    drop(error);
    assert_eq!(Live::now(), before);
    assert_eq!(db.reserved_memory_bytes(), memory);
    assert_eq!(db.reserved_temp_bytes(), 0);
    let samples = observer.samples();
    assert_eq!(samples.allocations, samples.frees);
    check_join_phase("late preparation error", samples, true);
    println!(
        "wide left join preparation failures passed: prefixes=0..={census}; live errors, owned span and release"
    );
}

// Start with a prepared plan, then refuse allocations in execute(). A failed
// construction must release its buffers while leaving that plan intact.
fn check_failed_join_construction(
    db: &Database,
    source: &str,
    before: Live,
    memory: u64,
    missing_observation: bool,
) {
    use super::{CALLS, REFUSED, workload};
    const ALLOCATION_LIMIT: usize = 512;
    let cancel = CancellationToken::new();
    let query = db.prepare(source).unwrap();
    let prepared = Live::now();
    let prepared_memory = db.reserved_memory_bytes();
    assert_eq!(prepared_memory, memory + query.accounted_memory_bytes());
    let observer = super::transient_ownership::Observer::new(
        db,
        prepared_memory,
        prepared.requested,
        prepared.usable,
    );
    let baseline = workload::arm(None, ALLOCATION_LIMIT);
    let result = observer.during(|| db.execute(&query, &cancel));
    workload::suspend_faults();
    let census = CALLS.load(Ordering::Relaxed);
    assert!(census > 1 && census <= ALLOCATION_LIMIT);
    assert_eq!(REFUSED.load(Ordering::Relaxed), 0);
    drop(result.expect("healthy join construction census"));
    workload::finish("join construction census", baseline);
    assert_eq!(Live::now(), prepared);
    assert_eq!(db.reserved_memory_bytes(), prepared_memory);
    check_join_phase("construction census", observer.samples(), true);
    for prefix in 0..=census {
        let observer = super::transient_ownership::Observer::new(
            db,
            prepared_memory,
            prepared.requested,
            prepared.usable,
        );
        let baseline = workload::arm(Some(prefix), ALLOCATION_LIMIT);
        let result = if missing_observation && prefix == 1 {
            db.execute(&query, &cancel)
        } else {
            observer.during(|| db.execute(&query, &cancel))
        };
        if let Err(error) = &result {
            // Check memory release and error formatting while faults remain active;
            // descriptor enumeration must wait because it allocates test storage.
            assert_eq!(LIVE_REQUESTED.load(Ordering::Relaxed), prepared.requested);
            assert_eq!(LIVE_USABLE.load(Ordering::Relaxed), prepared.usable);
            assert_eq!(db.reserved_memory_bytes(), prepared_memory);
            assert_eq!(db.reserved_temp_bytes(), 0);
            assert!(workload::format_error(Some(error)));
        }
        workload::suspend_faults();
        let calls = CALLS.load(Ordering::Relaxed);
        let refusals = REFUSED.load(Ordering::Relaxed);
        if prefix < census {
            let error = match result {
                Err(error) => error,
                Ok(_) => panic!("join construction accepted a refused allocation"),
            };
            assert!(
                matches!(&error, Error::Resource { .. })
                    || matches!(&error, Error::Io { source, .. }
                        if source.kind() == std::io::ErrorKind::OutOfMemory),
                "join construction allocation outcome: {error}"
            );
            assert!(calls > prefix && refusals > 0);
            // Keep the error alive while checking that file handles were released.
            assert_eq!(Live::now(), prepared);
            workload::finish("join construction refusal", baseline);
            drop(error);
        } else {
            assert_eq!(calls, census);
            assert_eq!(refusals, 0);
            let result = result.expect("full-prefix join construction control");
            assert_eq!(
                db.reserved_memory_bytes(),
                prepared_memory + result.accounted_memory_bytes()
            );
            drop(result);
            workload::finish("join construction control", baseline);
        }
        assert_eq!(Live::now(), prepared);
        assert_eq!(db.reserved_memory_bytes(), prepared_memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
        let samples = observer.samples();
        assert!(
            samples.allocations == prefix && (prefix == census || samples.frees == prefix),
            "missing failed construction events: prefix={prefix}: {samples:?}"
        );
        assert!(
            samples.requested_headroom >= 0 && samples.usable_headroom >= 0,
            "failed join construction ownership: prefix={prefix}: {samples:?}"
        );
        if prefix == 0 {
            println!("join construction prefix=0 calls={calls} refusals={refusals} samples=none");
        } else {
            println!(
                "join construction prefix={prefix} calls={calls} refusals={refusals} samples={samples:?}"
            );
        }
    }
    drop(query);
    assert_eq!(Live::now(), before);
    assert_eq!(db.reserved_memory_bytes(), memory);
    assert_eq!(db.reserved_temp_bytes(), 0);
    println!(
        "wide left join construction failures passed: prefixes=0..={census}; live errors and release"
    );
}

// LOG10(0) must fail after the join has used temporary storage. A separate
// observer for each step proves that the failing step itself frees buffers.
// Dropping SQL text before execution also tests that the error owns its span.
fn check_failed_join_execution(
    db: &Database,
    source: &str,
    before: Live,
    memory: u64,
    missing_observation: bool,
) {
    use super::transient_ownership::Observer;
    for expression in [
        "LOG10(ABS(r.id-3))",
        "SAFE_DIVIDE(1, LOG10(ABS(r.id-3)))",
        "COALESCE(NULLIF(1, 1), LOG10(ABS(r.id-3)))",
    ] {
        // Replace the right-only text60 column with the failing expression to
        // stay at 64 outputs while keeping the other wide text fields.
        let sql = format!("# 雪\n{source} |> DROP text60 |> EXTEND {expression} AS failed_value");
        let start = sql.find(expression).unwrap();
        let end = start + expression.len();
        let query = db.prepare(&sql).unwrap();
        drop(sql);
        assert_eq!(query.result_column_count(), 64);
        let output = query.result_column(63).unwrap();
        assert_eq!(
            (output.data_type, output.nullable),
            (DataType::Double, true)
        );
        let prepared_live = Live::now();
        let cancel = CancellationToken::new();
        let execution = Observer::new(db, memory, before.requested, before.usable);
        let mut result = execution.during(|| db.execute(&query, &cancel)).unwrap();
        let samples = execution.samples();
        assert!(
            samples.allocations > 0,
            "missing failed-query construction events"
        );
        assert!(
            samples.requested_headroom >= 0 && samples.usable_headroom >= 0,
            "demanded construction: {samples:?}"
        );
        let mut peak_temp = 0;
        let mut failed = false;
        for step in 1..200_000 {
            assert_eq!(
                db.reserved_memory_bytes(),
                memory + query.accounted_memory_bytes() + result.accounted_memory_bytes(),
            );
            let observer = Observer::new(db, memory, before.requested, before.usable);
            match if missing_observation {
                result.step()
            } else {
                observer.during(|| result.step())
            } {
                QueryStep::Progress | QueryStep::Rows(_) => (),
                QueryStep::Failed(Error::ArithmeticDomain { operation, span }) => {
                    assert_eq!(*operation, "base-ten logarithm");
                    assert_eq!((span.start(), span.end()), (start, end));
                    failed = true;
                }
                _ => panic!("missing demanded join domain error"),
            }
            let samples = observer.samples();
            assert!(
                samples.requested_headroom >= 0 && samples.usable_headroom >= 0,
                "failed join step ownership: {samples:?}",
            );
            if failed {
                assert!(peak_temp > 0, "join error must follow external work");
                assert_eq!(Live::now(), prepared_live);
                assert_eq!(
                    result.accounted_memory_bytes(),
                    std::mem::size_of_val(&result) as u64
                );
                assert!(samples.frees > 0, "missing failed execution events");
                assert_eq!(db.reserved_temp_bytes(), 0);
                assert_eq!(
                    db.reserved_memory_bytes(),
                    memory + query.accounted_memory_bytes() + result.accounted_memory_bytes(),
                );
                println!(
                    "wide left join demanded failure: expression={expression}; step={step}; temporary={peak_temp}; {samples:?}"
                );
                break;
            }
            peak_temp = peak_temp.max(db.reserved_temp_bytes());
        }
        assert!(failed, "bounded join execution must reach its domain error");
        // Stepping again must return the same error without rebuilding execution.
        let retained = Live::now();
        let charge = db.reserved_memory_bytes();
        let repeated = Observer::new(db, memory, before.requested, before.usable);
        let QueryStep::Failed(Error::ArithmeticDomain { operation, span }) =
            repeated.during(|| result.step())
        else {
            panic!("failed result lost its terminal error");
        };
        assert_eq!(*operation, "base-ten logarithm");
        assert_eq!((span.start(), span.end()), (start, end));
        assert_eq!(repeated.samples().allocations, 0);
        assert_eq!(repeated.samples().frees, 0);
        assert_eq!(Live::now(), retained);
        assert_eq!(db.reserved_memory_bytes(), charge);
        let release = Observer::new(db, memory, before.requested, before.usable);
        let error = release
            .during(|| result.into_error())
            .expect("owned terminal error");
        assert_eq!(
            db.reserved_memory_bytes(),
            memory + query.accounted_memory_bytes()
        );
        let prepared_release = Observer::new(db, memory, before.requested, before.usable);
        prepared_release.during(|| drop(query));
        check_join_phase("failed prepared release", prepared_release.samples(), false);
        let Error::ArithmeticDomain { operation, span } = &error else {
            panic!("owned join domain error");
        };
        assert_eq!(*operation, "base-ten logarithm");
        assert_eq!((span.start(), span.end()), (start, end));
        assert_eq!(Live::now(), before);
        assert_eq!(db.reserved_memory_bytes(), memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert!(super::workload::format_error(Some(&error)));
        drop(error);
        assert_eq!(Live::now(), before);
        assert_eq!(db.reserved_memory_bytes(), memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
        let samples = release.samples();
        assert_eq!((samples.allocations, samples.frees), (0, 0));
        assert!(
            samples.requested_headroom >= 0 && samples.usable_headroom >= 0,
            "demanded construction: {samples:?}"
        );
    }
    println!(
        "wide left join execution failures passed: 3 demanded errors; external work, owned spans and release"
    );
}

// Zero events could make the memory inequalities pass without observing anything.
// Require frees, and allocations where expected, to detect a disabled observer.
fn check_join_phase(phase: &str, samples: super::transient_ownership::Samples, allocates: bool) {
    println!("wide left join {phase}: {samples:?}");
    assert!(
        samples.frees > 0 && (!allocates || samples.allocations > 0),
        "missing transient join lifecycle events: {phase}"
    );
    assert!(
        samples.requested_headroom >= 0 && samples.usable_headroom >= 0,
        "wide left join lifecycle ownership: {phase}: {samples:?}"
    );
}

// Each key has two source rows, so a self-join produces four pairs per key.
// Nullable amounts distinguish COUNT(*) from COUNT(amount) and SUM(amount).
pub(super) fn joined_shapes(
    root: &Path,
    wrong_attribution: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(root)?;
    let path = root.join("database");
    let cancel = CancellationToken::new();
    create_joined_sales(&path, &cancel)?;
    // Initialize stdout before measuring so its lazy allocation is not charged
    // to the query. Row checks below borrow values without collecting them.
    println!("joined shapes: nullable self-join, grouping, descending order");
    for budget in [2_200_000, 12_000_000] {
        let db = Database::open(&path, Config::new(budget, TEMP)?)?;
        let before = Live::now();
        let memory = db.reserved_memory_bytes();
        let query = db.prepare(
            "FROM sales AS s |> JOIN sales AS copies ON s.region = copies.region \
             |> AGGREGATE COUNT(*) AS n, COUNT(s.amount) AS present, SUM(s.amount) AS total \
             GROUP BY s.region |> ORDER BY region DESC",
        )?;
        let mut result = db.execute(&query, &cancel)?;
        let mut seen = 0;
        let mut progress = 0;
        let mut row_steps = 0;
        let mut finished = false;
        let mut minimum_headroom = i128::MAX;
        let mut peak_temp = 0;
        // Sample after execute and each step, including Finished before drop.
        // The parent process deadline catches a query that stops making progress.
        loop {
            let charge = query.accounted_memory_bytes() + result.accounted_memory_bytes();
            assert_eq!(db.reserved_memory_bytes(), memory + charge);
            let requested = LIVE_REQUESTED
                .load(Ordering::Relaxed)
                .checked_sub(before.requested)
                .unwrap();
            let usable = LIVE_USABLE
                .load(Ordering::Relaxed)
                .checked_sub(before.usable)
                .unwrap();
            assert!(
                requested as u64 <= charge,
                "joined requested allocation admission"
            );
            // Inflate attributed heap use for the negative control. Check the
            // deficit after consuming all rows and verifying release.
            let attributed = usable as u128 + u128::from(wrong_attribution) * u128::from(charge);
            minimum_headroom = minimum_headroom.min(i128::from(charge) - attributed as i128);
            peak_temp = peak_temp.max(db.reserved_temp_bytes());
            if finished {
                break;
            }
            match result.step() {
                QueryStep::Progress => progress += 1,
                QueryStep::Finished => finished = true,
                QueryStep::Failed(error) => panic!("joined query: {error:?}"),
                QueryStep::Rows(batch) => {
                    row_steps += 1;
                    assert_eq!(batch.column_count(), 4);
                    for row in 0..batch.len() {
                        assert!(seen < ROWS);
                        let key = (ROWS - 1 - seen) as i64;
                        let (present, total) = match key % 4 {
                            0 => (0, Value::Null),
                            1 => (2, Value::Int64(6)),
                            _ => (4, Value::Int64(8)),
                        };
                        assert_eq!(batch.value(row, 0), Some(Value::Int64(key)));
                        assert_eq!(batch.value(row, 1), Some(Value::Int64(4)));
                        assert_eq!(batch.value(row, 2), Some(Value::Int64(present)));
                        assert_eq!(batch.value(row, 3), Some(total));
                        seen += 1;
                    }
                }
            }
        }
        assert!(
            finished && seen == ROWS && progress > 0 && row_steps > 0,
            "joined incomplete: finished={finished} rows={seen} progress={progress} row_steps={row_steps}"
        );
        assert!(peak_temp > 0, "joined workload must exercise sorted inputs");
        drop(result);
        drop(query);
        assert_eq!(Live::now(), before);
        assert_eq!(db.reserved_memory_bytes(), memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
        db.close()?;
        println!(
            "joined budget={budget} rows={seen} progress={progress} row-steps={row_steps} minimum-usable-headroom={minimum_headroom} temporary={peak_temp} release=complete"
        );
        assert!(minimum_headroom >= 0, "joined usable ownership attribution");
    }
    println!("joined shapes passed: 2 budgets; complete rows, step ownership and release");
    Ok(())
}

fn create_joined_sales(path: &Path, cancel: &CancellationToken) -> Result<(), pipesql::Error> {
    // Loading needs its own budget; reopen under the smaller query budgets later.
    let db = Database::create_empty(path, Config::new(32_000_000, TEMP)?)?;
    db.declare_table(
        "sales",
        &["region", "amount"].map(|name| ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: name == "amount",
        }),
        cancel,
    )?;
    let mut append = db.begin_append(
        "sales",
        AppendLimits {
            batches: (2 * ROWS / 256) as u32,
            encoded_bytes: 1_000_000,
        },
        cancel,
    )?;
    for amount in [1, 3] {
        for start in (0..ROWS).step_by(256) {
            let regions: [i64; 256] = std::array::from_fn(|row| (ROWS - 1 - start - row) as i64);
            let mut validity = [0_u8; 256 / 8];
            for (row, region) in regions.iter().enumerate() {
                if region % 4 >= 2 || (region % 4 == 1 && amount == 3) {
                    validity[row / 8] |= 1 << (row % 8);
                }
            }
            append.write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&regions),
                        validity: &[255; 256 / 8],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&[amount; 256]),
                        validity: &validity,
                    },
                ],
                cancel,
            )?;
        }
    }
    append.commit(cancel)?;
    db.close()
}
