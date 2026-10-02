//! Check when grouping failures stop output and release query resources.
//!
//! A later group's overflowing SUM must prevent any group from being returned.
//! Filtering that group out first must avoid demanding its SUM. These cases check
//! both memory and disk paths and require the error to name the SQL expression.
//!
//! Healthy runs identify phase kinds and I/O positions for cancellation, drop and
//! injected-error cases. Result-file mutations separately challenge reserved
//! header bytes, checksums and truncation; read and flush failures must also stop
//! the controller. A failed controller must remain failed without further I/O.
//!
//! Component tests drop their owners before checking released reservations.
//! Public-result tests check that failure releases the workspace while retaining
//! only the result handle. Composition cases cancel repeated aggregates and a
//! derived-table join, then run a fresh query to check that the database remains
//! usable. Expected successful results come from the literal fixture rows.

use super::*;

#[test]
fn limits_preserve_group_validation_and_projection_boundaries() {
    enum Expected {
        Rows(&'static [(i64, Option<i64>)]),
        Overflow(&'static str, &'static str),
    }
    use Expected::{Overflow, Rows};

    let directory = Directory::new();
    let database = database(
        &directory,
        &[
            (Some(1), Some(7), 0.0),
            (Some(2), None, 0.0),
            (Some(9), Some(i64::MAX), 0.0),
            (Some(9), Some(i64::MAX), 0.0),
        ],
    );
    let prefix = "FROM facts |> AGGREGATE SUM(n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k";
    // The first group is safe, the second is NULL, and the last SUM overflows.
    // Grouping validates retained values before any output. An ordinary filter
    // can reject the last group; a following LIMIT cannot undo that validation.
    for (suffix, expected) in [
        // Text emits one group per batch: missing prevalidation cannot hide
        // behind a numeric batch that happens to include the overflowing group.
        (
            "SELECT k, total, 'marker' AS marker |> LIMIT 1",
            Overflow("SUM", "SUM(n)"),
        ),
        ("SELECT k, total |> LIMIT 1", Overflow("SUM", "SUM(n)")),
        ("SELECT k, total |> LIMIT 0", Rows(&[])),
        (
            "SELECT k, total |> LIMIT 0 OFFSET 1",
            Overflow("SUM", "SUM(n)"),
        ),
        (
            "WHERE k < 9 |> SELECT k, total |> LIMIT 2",
            Rows(&[(1, Some(7)), (2, None)]),
        ),
        (
            "WHERE k < 9 |> SELECT k, total |> LIMIT 1 OFFSET 1",
            Rows(&[(2, None)]),
        ),
        (
            "WHERE k < 9 |> SELECT k, total |> LIMIT 0 OFFSET 1",
            Rows(&[]),
        ),
        (
            "WHERE nrows < 2 |> SELECT k, total |> LIMIT 2",
            Rows(&[(1, Some(7)), (2, None)]),
        ),
        ("LIMIT 1 |> SELECT k, 0 AS v", Rows(&[(1, Some(0))])),
        ("LIMIT 1 |> SELECT k, total", Overflow("SUM", "SUM(n)")),
        (
            "LIMIT 1 |> WHERE k < 9 |> SELECT k, total",
            Overflow("SUM", "SUM(n)"),
        ),
        (
            "SELECT k, CASE WHEN k < 9 THEN total ELSE 0 END AS v |> LIMIT 3",
            Rows(&[(1, Some(7)), (2, None), (9, Some(0))]),
        ),
        ("SELECT k, 0 AS v |> LIMIT 1 OFFSET 4", Rows(&[])),
        (
            "SELECT k, 9223372036854775807 + (k - 1) AS v |> LIMIT 1",
            Overflow("addition", "9223372036854775807 + (k - 1)"),
        ),
        (
            "LIMIT 1 |> SELECT k, 9223372036854775807 + (k - 1) AS v",
            Rows(&[(1, Some(i64::MAX))]),
        ),
        (
            "EXTEND 9223372036854775807 + (k - 1) AS v |> WHERE v > 0 |> SELECT k, 0 AS v |> LIMIT 1",
            Overflow("addition", "9223372036854775807 + (k - 1)"),
        ),
        (
            "EXTEND 9223372036854775807 + (k - 1) AS unused |> SELECT k, 0 AS v |> LIMIT 1",
            Rows(&[(1, Some(0))]),
        ),
    ] {
        let sql = format!("{prefix} |> {suffix}");
        let query = database.prepare(&sql).unwrap();
        let dormant = suffix == "SELECT k, total |> LIMIT 0";
        for disk in [false, true] {
            let baseline = database.reserved_memory_bytes();
            let cancel = CancellationToken::new();
            let mut result = database.execute(&query, &cancel).unwrap();
            first_hash_limits(&mut result, if disk { 0 } else { 4 }, 4 * 9);
            let (mut hashed, mut written, mut terminal) = (false, false, false);
            let mut actual = Vec::new();
            for _ in 0..10_000 {
                if let Some(Aggregation::General(owner)) = result.first_aggregate() {
                    hashed |= matches!(owner[0].phase, Phase::Hash);
                    if let Files::Open(files) = &owner[0].files {
                        written |=
                            (0..2).any(|slot| files.test_file(slot).metadata().unwrap().len() > 0);
                    }
                }
                match result.step() {
                    QueryStep::Progress => (),
                    QueryStep::Rows(batch) => {
                        assert!(
                            matches!(expected, Rows(_)),
                            "group validation precedes all output: {sql}"
                        );
                        assert_eq!(batch.column_count(), 2);
                        for row in 0..batch.len() {
                            let Some(Value::Int64(key)) = batch.value(row, 0) else {
                                panic!("integer group key: {sql}");
                            };
                            let value = match batch.value(row, 1) {
                                Some(Value::Int64(value)) => Some(value),
                                Some(Value::Null) => None,
                                _ => panic!("nullable integer result: {sql}"),
                            };
                            actual.push((key, value));
                        }
                    }
                    QueryStep::Finished => {
                        let Rows(rows) = expected else {
                            panic!("missing demanded error: {sql}, disk={disk}");
                        };
                        assert_eq!(actual, rows, "{sql}, disk={disk}");
                        terminal = true;
                        break;
                    }
                    QueryStep::Failed(Error::ArithmeticOverflow { operation, span }) => {
                        let Overflow(wanted_operation, expression) = expected else {
                            panic!("undemanded overflow: {sql}, disk={disk}");
                        };
                        assert_eq!(*operation, wanted_operation);
                        assert_eq!(&sql[span.start()..span.end()], expression);
                        assert!(actual.is_empty(), "group validation precedes all output");
                        assert!(matches!(
                            result.step(),
                            QueryStep::Failed(Error::ArithmeticOverflow { .. })
                        ));
                        terminal = true;
                        break;
                    }
                    QueryStep::Failed(error) => panic!("{sql}, disk={disk}: {error}"),
                }
            }
            assert!(terminal, "{sql}, disk={disk}");
            assert_eq!(hashed, !disk && !dormant, "{sql}");
            assert_eq!(written, disk && !dormant, "{sql}");
            drop(result);
            assert_eq!(database.reserved_memory_bytes(), baseline);
            assert_eq!(database.reserved_temp_bytes(), 0);
        }
    }
}

#[test]
fn later_overflow_cannot_publish_earlier_groups_and_filters_preserve_demand() {
    let rows = [
        (Some(1), Some(7), 7.0),
        (Some(9), Some(i64::MAX), 1.0),
        (Some(9), Some(i64::MAX), 2.0),
    ];
    let directory = Directory::new();
    let database = database(&directory, &rows);
    let cancel = CancellationToken::new();
    for filtered in [false, true] {
        let source = if filtered {
            format!("{QUERY} |> WHERE nrows < 2")
        } else {
            QUERY.to_owned()
        };
        let query = database.prepare(&source).unwrap();
        let baseline = database.reserved_memory_bytes();
        for hash_groups in [1, 2] {
            let mut running = database.execute(&query, &cancel).unwrap();
            let mut general = connect(&database, &mut running, hash_groups, 1, true);
            let mut effects = Effects::default();
            let mut published = 0;
            let mut terminal = false;
            for _ in 0..1000 {
                match step(&mut general, &mut running, &cancel, &mut effects) {
                    Ok(Advance::Rows) => {
                        for row in 0..workspace(&mut running).output.len() {
                            published += 1;
                            assert_eq!(values(&mut running, row).0, Some(1));
                        }
                    }
                    Ok(Advance::Progress) => (),
                    Ok(Advance::Finished) => {
                        assert!(filtered);
                        terminal = true;
                        break;
                    }
                    Err(Error::ArithmeticOverflow { operation, span }) => {
                        assert_eq!(operation, "SUM");
                        assert_eq!(&source[span.start()..span.end()], "SUM(n)");
                        assert!(!filtered);
                        assert_eq!(published, 0, "a later bad group prevents all publication");
                        let before = effects.count();
                        assert!(step(&mut general, &mut running, &cancel, &mut effects).is_err());
                        assert_eq!(effects.count(), before);
                        assert!(workspace(&mut running).output.is_empty());
                        terminal = true;
                        break;
                    }
                    Err(error) => panic!("unexpected failure: {error:?}"),
                }
            }
            assert!(terminal);
            assert_eq!(published, usize::from(filtered));
            drop(general);
            drop(running);
            assert_eq!(database.reserved_memory_bytes(), baseline);
            assert_eq!(database.reserved_temp_bytes(), 0);
        }
    }
}

#[test]
fn controller_cancellation_at_each_phase_is_terminal_and_reconciles_owners() {
    let directory = Directory::new();
    let database = database(
        &directory,
        &[
            (Some(2), Some(4), 4.0),
            (Some(1), Some(3), 3.0),
            (Some(3), Some(5), 5.0),
        ],
    );
    let cancel = CancellationToken::new();
    let query = database.prepare(QUERY).unwrap();
    let baseline = database.reserved_memory_bytes();
    for hash_groups in [1, 3] {
        let mut running = database.execute(&query, &cancel).unwrap();
        let mut general = connect(&database, &mut running, hash_groups, 1, true);
        let mut phases = Vec::new();
        for _ in 0..2000 {
            if general.phase == Phase::Done {
                break;
            }
            // Exercise each phase kind once; indexes within Capture, Check and
            // EmitMemory do not create additional cases in this sweep.
            let phase = std::mem::discriminant(&general.phase);
            if !phases.contains(&phase) {
                phases.push(phase);
            }
            step(&mut general, &mut running, &cancel, &mut Effects::default()).unwrap();
        }
        assert_eq!(general.phase, Phase::Done);
        let expected: &[Phase] = if hash_groups == 1 {
            &[
                Phase::Read,
                Phase::Await,
                Phase::Capture(0),
                Phase::Hash,
                Phase::Create,
                Phase::Encode(0),
                Phase::Push(0),
                Phase::Spill(0),
                Phase::Sort,
                Phase::Reduce,
                Phase::Flush,
                Phase::EmitDisk,
            ]
        } else {
            &[
                Phase::Read,
                Phase::Await,
                Phase::Capture(0),
                Phase::Hash,
                Phase::Order,
                Phase::Check(0),
                Phase::EmitMemory(0),
            ]
        };
        assert_eq!(phases.len(), expected.len());
        for phase in expected {
            assert!(phases.contains(&std::mem::discriminant(phase)));
        }
        drop(general);
        drop(running);
        for phase in phases {
            let mut running = database.execute(&query, &cancel).unwrap();
            let mut general = connect(&database, &mut running, hash_groups, 1, true);
            let mut effects = Effects::default();
            let mut reached = false;
            for _ in 0..2000 {
                if std::mem::discriminant(&general.phase) == phase {
                    let cancelled = CancellationToken::new();
                    cancelled.cancel();
                    let before = effects.count();
                    assert!(matches!(
                        step(&mut general, &mut running, &cancelled, &mut effects),
                        Err(Error::Cancelled)
                    ));
                    assert!(step(&mut general, &mut running, &cancel, &mut effects).is_err());
                    assert_eq!(effects.count(), before);
                    assert!(workspace(&mut running).output.is_empty());
                    reached = true;
                    break;
                }
                step(&mut general, &mut running, &cancel, &mut effects).unwrap();
            }
            assert!(reached);
            drop(general);
            drop(running);
            assert_eq!(database.reserved_memory_bytes(), baseline);
            assert_eq!(database.reserved_temp_bytes(), 0);
        }
    }
}

#[test]
fn checked_result_spool_rejects_corruption_truncation_and_io_failure() {
    let directory = Directory::new();
    let database = database(
        &directory,
        &[(Some(2), Some(4), 4.0), (Some(1), Some(3), 3.0)],
    );
    let cancel = CancellationToken::new();
    let query = database.prepare(QUERY).unwrap();
    let baseline = database.reserved_memory_bytes();
    for fault in 0..10 {
        let mut running = database.execute(&query, &cancel).unwrap();
        let mut general = connect(&database, &mut running, 1, 1, true);
        let mut effects = Effects::default();
        let target = if fault == 4 {
            Phase::Flush
        } else {
            Phase::EmitDisk
        };
        for _ in 0..1000 {
            if general.phase == target {
                break;
            }
            assert!(!matches!(
                step(&mut general, &mut running, &cancel, &mut effects).unwrap(),
                Advance::Rows
            ));
        }
        assert_eq!(general.phase, target);
        let slot = general.sort.spool_slot();
        let result_end = general.result_end;
        let Files::Open(scratch) = &mut general.files else {
            unreachable!()
        };
        // Keep header validation, payload checksum and missing bytes as distinct
        // failures. Fault 1 retains the original checksum; faults 6-9 repair it
        // so malformed typed payloads must fail after checksum validation.
        match fault {
            0 => scratch
                .write(slot, 24, &[1], &cancel, &mut effects)
                .unwrap(),
            1 => scratch
                .write(slot, RECORD_HEADER as u64, &[0xff], &cancel, &mut effects)
                .unwrap(),
            2 => scratch.reset(slot, &cancel, &mut effects).unwrap(),
            3 | 4 => {
                effects = Effects::with_faults(Faults {
                    fail_at: Some(0),
                    ..Faults::default()
                })
            }
            5 => {
                // Both rows have four present eight-byte values. Damage the
                // second record after a valid row has filled private batch slots.
                let row_bytes = (RECORD_HEADER + 4 * 9) as u64;
                assert_eq!(result_end, 2 * row_bytes);
                scratch
                    .write(slot, row_bytes + 24, &[1], &cancel, &mut effects)
                    .unwrap();
            }
            6..=9 => {
                let row_bytes = RECORD_HEADER + 4 * 9;
                assert_eq!(result_end, (2 * row_bytes) as u64);
                let mut record = vec![0; row_bytes];
                scratch
                    .read(slot, &mut record, row_bytes as u64, &cancel, &mut effects)
                    .unwrap();
                match fault {
                    6 => record[RECORD_HEADER] = 2,
                    7 => {
                        // COUNT is non-nullable. Encode a NULL without its word.
                        record[RECORD_HEADER + 3 * 9] = 0;
                        record.truncate(RECORD_HEADER + 3 * 9 + 1);
                    }
                    8 => {
                        record.pop();
                    }
                    9 => {
                        // A NULL key makes room below the schema's maximum size
                        // for a trailing byte after four otherwise valid cells.
                        record[RECORD_HEADER] = 0;
                        record.drain(RECORD_HEADER + 1..RECORD_HEADER + 9);
                        record.push(0);
                    }
                    _ => unreachable!(),
                }
                let length = record.len() as u32;
                record[16..20].copy_from_slice(&length.to_le_bytes());
                record[28..32].fill(0);
                let checksum = format::crc32c(&record);
                record[28..32].copy_from_slice(&checksum.to_le_bytes());
                scratch
                    .write(slot, row_bytes as u64, &record, &cancel, &mut effects)
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let failure = step(&mut general, &mut running, &cancel, &mut effects)
            .err()
            .expect("damaged output must fail before publishing its batch");
        if fault >= 6 {
            let expected = [
                "group value validity",
                "group value validity",
                "truncated group record",
                "group result trailing bytes",
            ][fault - 6];
            assert!(matches!(failure, Error::Corrupt(reason) if reason == expected));
        }
        assert!(workspace(&mut running).output.is_empty());
        let before = effects.count();
        assert!(step(&mut general, &mut running, &cancel, &mut effects).is_err());
        assert_eq!(effects.count(), before);
        drop(general);
        drop(running);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
}

#[test]
fn public_grouping_faults_release_the_complete_query_owner() {
    let directory = Directory::new();
    let database = database(
        &directory,
        &[
            (Some(2), Some(4), 4.0),
            (Some(1), Some(3), 3.0),
            (Some(3), Some(5), 5.0),
        ],
    );
    let query = database.prepare(
        "FROM facts |> AGGREGATE SUM(n) AS total, AVG(d) AS mean, COUNT(*) AS nrows GROUP AND ORDER BY k",
    ).unwrap();
    let baseline = database.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let result = database.execute(&query, &cancel).unwrap();
    let Some(Aggregation::General(owner)) = result.first_aggregate() else {
        unreachable!()
    };
    assert!(owner[0].hash_pending && owner[0].memory.is_empty());
    let disk_peak = result.accounted_memory_bytes() + crate::storage::catalog::MAX_BYTES as u64;
    drop(result);

    #[derive(Debug)]
    enum Fault {
        Cancel(usize, bool),
        Drop(usize),
        Io(u64),
    }
    for disk in [false, true] {
        let pressure = disk.then(|| {
            database
                .reserve_memory(
                    database.config().memory_limit_bytes() - baseline - disk_peak,
                    "public query disk path",
                )
                .unwrap()
        });
        let pressured = database.reserved_memory_bytes();
        let mut result = database.execute(&query, &cancel).unwrap();
        if disk {
            first_hash_limits(&mut result, 0, 0);
        }
        let Some(Aggregation::General(owner)) = result.first_aggregate() else {
            unreachable!()
        };
        assert!(owner[0].memory.is_empty());
        assert_eq!(owner[0].hash_pending, !disk);
        let mut phases = Vec::new();
        let mut faults = Vec::new();
        let mut effects = Effects::default();
        let mut done = false;
        let mut keys = Vec::new();
        for index in 0..2000 {
            let Some(Aggregation::General(owner)) = result.first_aggregate() else {
                unreachable!()
            };
            let phase = std::mem::discriminant(&owner[0].phase);
            if !phases.contains(&phase) {
                phases.push(phase);
                faults.push(Fault::Cancel(index, owner[0].phase == Phase::Done));
                faults.push(Fault::Drop(index));
            }
            // Start disk I/O injection after scratch creation. Creation has
            // separate scratch tests; this sweep checks the public result's
            // cleanup for later I/O failures.
            let files_open = matches!(owner[0].files, Files::Open(_));
            let before = effects.count();
            match result.step_with_effects(&mut effects) {
                QueryStep::Rows(batch) => {
                    for row in 0..batch.len() {
                        let Some(Value::Int64(key)) = batch.value(row, 0) else {
                            panic!("key")
                        };
                        keys.push(key);
                    }
                }
                QueryStep::Progress => (),
                QueryStep::Finished => done = true,
                QueryStep::Failed(error) => panic!("control: {error}"),
            }
            if files_open || !disk {
                for effect in before..effects.count() {
                    faults.push(Fault::Io(effect));
                }
            }
            if done {
                break;
            }
        }
        assert!(done);
        assert_eq!(keys, [1, 2, 3]);
        assert!(phases.len() >= if disk { 9 } else { 6 });
        assert_eq!(result.accounted_memory_bytes(), RESULT_BYTES);
        drop(result);
        assert_eq!(database.reserved_memory_bytes(), pressured);
        assert_eq!(database.reserved_temp_bytes(), 0);

        for fault in &faults {
            let token = CancellationToken::new();
            let mut result = database.execute(&query, &token).unwrap();
            if disk {
                first_hash_limits(&mut result, 0, 0);
            }
            let mut effects = Effects::with_faults(Faults {
                fail_at: match fault {
                    Fault::Io(at) => Some(*at),
                    _ => None,
                },
                ..Faults::default()
            });
            let mut reached = false;
            for index in 0..2000 {
                if matches!(fault, Fault::Drop(at) if *at == index) {
                    reached = true;
                    break;
                }
                if matches!(fault, Fault::Cancel(at, _) if *at == index) {
                    token.cancel();
                }
                match result.step_with_effects(&mut effects) {
                    QueryStep::Failed(error) => {
                        assert!(
                            match fault {
                                Fault::Cancel(_, terminal) =>
                                    !terminal && matches!(error, Error::Cancelled),
                                Fault::Io(_) => matches!(error, Error::Io { .. }),
                                Fault::Drop(_) => false,
                            },
                            "{fault:?}: {error}"
                        );
                        let count = effects.count();
                        assert!(matches!(
                            result.step_with_effects(&mut effects),
                            QueryStep::Failed(_)
                        ));
                        assert_eq!(effects.count(), count);
                        assert_eq!(result.accounted_memory_bytes(), RESULT_BYTES);
                        assert_eq!(database.reserved_memory_bytes(), pressured + RESULT_BYTES);
                        assert_eq!(database.reserved_temp_bytes(), 0);
                        reached = true;
                        break;
                    }
                    QueryStep::Finished => {
                        assert!(
                            matches!(fault, Fault::Cancel(at, true) if *at == index),
                            "unreached fault: {fault:?}"
                        );
                        let count = effects.count();
                        assert!(matches!(
                            result.step_with_effects(&mut effects),
                            QueryStep::Finished
                        ));
                        assert_eq!(effects.count(), count);
                        assert_eq!(result.accounted_memory_bytes(), RESULT_BYTES);
                        assert_eq!(database.reserved_memory_bytes(), pressured + RESULT_BYTES);
                        assert_eq!(database.reserved_temp_bytes(), 0);
                        reached = true;
                        break;
                    }
                    QueryStep::Rows(_) | QueryStep::Progress => (),
                }
            }
            assert!(reached, "{fault:?}");
            drop(result);
            assert_eq!(database.reserved_memory_bytes(), pressured);
            assert_eq!(database.reserved_temp_bytes(), 0);
        }
        if disk {
            let occupied = database.config().temp_limit_bytes();
            database.temporary.reserve(occupied).unwrap();
            let mut result = database.execute(&query, &cancel).unwrap();
            if disk {
                first_hash_limits(&mut result, 0, 0);
            }
            let mut failed = false;
            for _ in 0..2000 {
                match result.step() {
                    QueryStep::Failed(Error::Resource {
                        owner: "database temporary storage",
                        ..
                    }) => {
                        failed = true;
                        break;
                    }
                    QueryStep::Progress => (),
                    _ => panic!(
                        "disk query must refuse before publishing under full temporary pressure"
                    ),
                }
            }
            assert!(failed);
            assert_eq!(database.reserved_memory_bytes(), pressured + RESULT_BYTES);
            assert_eq!(database.reserved_temp_bytes(), occupied);
            drop(result);
            database.temporary.release(occupied);
        }
        println!(
            "public grouped owner: disk={disk}, phases={}, fault cuts={}",
            phases.len(),
            faults.len()
        );
        drop(pressure);
    }
    assert_eq!(database.reserved_memory_bytes(), baseline);
}

#[test]
fn repeated_aggregation_cancels_at_observed_controller_phases() {
    fn use_disk_for_second_group(result: &mut QueryResult<'_, '_>) {
        let State::Running(runtime) = &mut result.state else {
            unreachable!()
        };
        let Aggregation::General(owner) = &mut runtime.aggregates[1] else {
            unreachable!()
        };
        // Select disk processing explicitly before input starts. This isolates
        // cancellation across both output paths from hash-capacity decisions.
        set_hash_limits(&mut owner[0], 0, 0);
    }

    let directory = Directory::new();
    let database = database(
        &directory,
        &[
            (Some(1), Some(10), 10.0),
            (Some(1), Some(20), 20.0),
            (Some(2), Some(90), 90.0),
        ],
    );
    let query = database.prepare("FROM facts |> AGGREGATE SUM(n) AS total GROUP BY k |> AGGREGATE SUM(total) AS subtotal GROUP BY total |> AGGREGATE AVG(subtotal) AS mean").unwrap();
    let baseline = database.reserved_memory_bytes();
    let healthy = CancellationToken::new();
    let mut reference = database.execute(&query, &healthy).unwrap();
    use_disk_for_second_group(&mut reference);
    let mut boundaries = Vec::new();
    let mut memory_output = false;
    let mut disk_output = false;
    let mut reached = false;
    for prefix in 0..8192 {
        let State::Running(runtime) = &reference.state else {
            unreachable!()
        };
        for (index, aggregation) in runtime.aggregates.iter().enumerate() {
            if let Aggregation::General(owner) = aggregation {
                let phase = owner[0].phase;
                let kind = std::mem::discriminant(&phase);
                if !boundaries.iter().any(|&(i, k, _)| i == index && k == kind) {
                    boundaries.push((index, kind, prefix));
                }
                memory_output |= matches!(phase, Phase::EmitMemory(_));
                disk_output |= matches!(phase, Phase::EmitDisk);
            }
        }
        match reference.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                assert_eq!(batch.len(), 1);
                assert_eq!(batch.value(0, 0), Some(Value::Double(60.0)));
                reached = true;
                break;
            }
            _ => panic!("expected completed aggregate output"),
        }
    }
    assert!(reached && memory_output && disk_output);
    drop(reference);
    assert_eq!(database.reserved_memory_bytes(), baseline);
    // Several controllers can reach new phase kinds at the same scheduler step.
    // One cancellation at that prefix covers the shared boundary.
    let mut prefixes: Vec<_> = boundaries.iter().map(|&(_, _, prefix)| prefix).collect();
    prefixes.sort_unstable();
    prefixes.dedup();
    println!(
        "observed {} controller phases at {} distinct cancellation prefixes",
        boundaries.len(),
        prefixes.len()
    );
    for prefix in prefixes {
        let cancel = CancellationToken::new();
        let mut result = database.execute(&query, &cancel).unwrap();
        use_disk_for_second_group(&mut result);
        for _ in 0..prefix {
            assert!(matches!(result.step(), QueryStep::Progress));
        }
        cancel.cancel();
        let mut failed = false;
        for _ in 0..16 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Failed(Error::Cancelled) => {
                    failed = true;
                    break;
                }
                _ => panic!("prefix {prefix}: cancellation must precede output"),
            }
        }
        assert!(failed, "prefix {prefix}: cancellation must terminate");
        assert!(matches!(result.step(), QueryStep::Failed(Error::Cancelled)));
        drop(result);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
    let mut healed = database.execute(&query, &healthy).unwrap();
    use_disk_for_second_group(&mut healed);
    let mut seen = false;
    let mut done = false;
    for _ in 0..8192 {
        match healed.step() {
            QueryStep::Rows(batch) => {
                assert!(!seen && batch.len() == 1);
                assert_eq!(batch.value(0, 0), Some(Value::Double(60.0)));
                seen = true;
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                done = true;
                break;
            }
            QueryStep::Failed(error) => panic!("healed query: {error}"),
        }
    }
    assert!(seen && done);
    drop(healed);
    assert_eq!(database.reserved_memory_bytes(), baseline);
    assert_eq!(database.reserved_temp_bytes(), 0);
}

#[test]
fn derived_join_cancels_at_every_observed_step_and_heals() {
    let directory = Directory::new();
    let database = database(
        &directory,
        &[
            (Some(1), Some(10), 10.0),
            (Some(1), Some(20), 20.0),
            (Some(2), Some(90), 90.0),
        ],
    );
    let query = database.prepare("FROM (FROM facts |> AGGREGATE SUM(n) AS total GROUP BY k) AS a |> JOIN (FROM facts |> WHERE k > 0) AS b ON a.k = b.k |> AGGREGATE SUM(a.total) AS weighted").unwrap();
    let baseline = database.reserved_memory_bytes();
    let healthy = CancellationToken::new();
    let mut progress = 0;
    for pass in 0..2 {
        let mut result = database.execute(&query, &healthy).unwrap();
        let mut seen = false;
        let mut done = false;
        for _ in 0..4096 {
            match result.step() {
                QueryStep::Progress => {
                    if pass == 0 {
                        progress += 1;
                    }
                }
                QueryStep::Rows(batch) => {
                    assert!(!seen && batch.len() == 1 && batch.column_count() == 1);
                    assert_eq!(batch.value(0, 0), Some(Value::Int64(150)));
                    seen = true;
                }
                QueryStep::Finished => {
                    done = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("derived join: {error}"),
            }
        }
        assert!(seen && done);
        drop(result);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
        if pass == 1 {
            break;
        }
        for prefix in 0..=progress {
            let cancel = CancellationToken::new();
            let mut result = database.execute(&query, &cancel).unwrap();
            for _ in 0..prefix {
                assert!(matches!(result.step(), QueryStep::Progress));
            }
            cancel.cancel();
            let mut failed = false;
            for _ in 0..16 {
                match result.step() {
                    QueryStep::Progress => (),
                    QueryStep::Failed(Error::Cancelled) => {
                        failed = true;
                        break;
                    }
                    _ => panic!("prefix {prefix}: cancellation must precede output"),
                }
            }
            assert!(failed);
            assert!(matches!(result.step(), QueryStep::Failed(Error::Cancelled)));
            drop(result);
            assert_eq!(database.reserved_memory_bytes(), baseline);
            assert_eq!(database.reserved_temp_bytes(), 0);
        }
    }
    println!("derived join cancellation prefixes={}", progress + 1);
}
