use super::*;

#[test]
fn controller_drives_native_replay_sort_reduction_and_private_publication() {
    let rows = [
        (Some(3), Some(i64::MAX), f64::MAX),
        (Some(1), Some(10), 10.0),
        (Some(3), Some(i64::MAX), f64::MAX),
        (Some(1), Some(14), 14.0),
        (Some(3), Some(-i64::MAX), -f64::MAX),
        (None, None, -0.0),
        (Some(2), Some(8), 8.0),
    ];
    let directory = Directory::new();
    let database = database(&directory, &rows);
    let cancel = CancellationToken::new();
    let query = database.prepare(QUERY).unwrap();
    let baseline = database.reserved_memory_bytes();
    let mut canonical = None;
    for ordered in [false, true] {
        for hash_groups in [2, 4] {
            for arguments in [1, 3, BATCH_ROWS] {
                let mut running = database.execute(&query, &cancel).unwrap();
                let mut general = connect(&database, &mut running, hash_groups, arguments, ordered);
                assert!(general.reservation.bytes() > 0);
                let mut effects = Effects::default();
                let mut observed = Vec::new();
                let mut phases = Vec::new();
                let mut finished = false;
                for _ in 0..10_000 {
                    if !phases.contains(&general.phase) {
                        phases.push(general.phase);
                    }
                    match step(&mut general, &mut running, &cancel, &mut effects).unwrap() {
                        Advance::Rows => {
                            assert!(matches!(
                                general.phase,
                                Phase::EmitMemory(_) | Phase::EmitDisk
                            ));
                            observed.push(values(&mut running));
                        }
                        Advance::Progress => assert!(workspace(&mut running).output.is_empty()),
                        Advance::Finished => {
                            finished = true;
                            break;
                        }
                    }
                }
                assert!(
                    finished,
                    "controller makes bounded progress: {:?}",
                    general.phase
                );
                assert_eq!(observed.len(), 4);
                if ordered {
                    assert!(observed.windows(2).all(|pair| pair[0].0 < pair[1].0));
                }
                observed.sort_by_key(|row| row.0);
                assert_eq!(observed[0], (None, None, (-0.0_f64).to_bits(), 1));
                assert_eq!(observed[1], (Some(1), Some(24), 12.0_f64.to_bits(), 2));
                assert_eq!(observed[2], (Some(2), Some(8), 8.0_f64.to_bits(), 1));
                assert_eq!(observed[3].1, Some(i64::MAX));
                assert_eq!(observed[3].3, 3);
                if let Some(expected) = &canonical {
                    assert_eq!(&observed, expected);
                } else {
                    canonical = Some(observed);
                }
                if hash_groups == 2 {
                    assert!(phases.contains(&Phase::Create));
                    assert!(phases.iter().any(|phase| matches!(phase, Phase::Spill(_))));
                    assert!(phases.contains(&Phase::Sort) && phases.contains(&Phase::Reduce));
                    assert!(matches!(general.files, Files::Open(_)));
                    let Source::Declared(scan) = workspace(&mut running).scan.source_mut() else {
                        unreachable!()
                    };
                    assert!(matches!(
                        scan[0].restart_once(&cancel),
                        Err(Error::Resource {
                            required: 2,
                            limit: 1,
                            ..
                        })
                    ));
                } else {
                    assert!(
                        matches!(general.files, Files::Pending(_)),
                        "successful hash grouping creates no files"
                    );
                }
                drop(general);
                drop(running);
                assert_eq!(database.reserved_memory_bytes(), baseline);
                assert_eq!(database.reserved_temp_bytes(), 0);
            }
        }
    }
}

#[test]
fn empty_and_fully_filtered_input_produce_no_groups_on_either_path() {
    for empty in [false, true] {
        let directory = Directory::new();
        let rows: &[Row] = if empty {
            &[]
        } else {
            &[(Some(1), Some(2), 3.0)]
        };
        let database = database(&directory, rows);
        let cancel = CancellationToken::new();
        let query = database
            .prepare(&QUERY.replace("FROM facts", "FROM facts |> WHERE k < 0"))
            .unwrap();
        let baseline = database.reserved_memory_bytes();
        let mut running = database.execute(&query, &cancel).unwrap();
        let before = database.reserved_memory_bytes();
        let general = connect(&database, &mut running, 1, 1, true);
        let minimum = database.reserved_memory_bytes() - before - general.memory[0].memory_bytes();
        drop(general);
        drop(running);
        for skip_hash in [false, true] {
            let mut running = database.execute(&query, &cancel).unwrap();
            let pressure = skip_hash.then(|| {
                database
                    .reserve_memory(
                        database.config().memory_limit_bytes()
                            - database.reserved_memory_bytes()
                            - minimum,
                        "empty grouping minimum test",
                    )
                    .unwrap()
            });
            let mut general = connect(&database, &mut running, 1, 1, true);
            assert_eq!(general.memory.is_empty(), skip_hash);
            let mut done = false;
            for _ in 0..100 {
                match step(&mut general, &mut running, &cancel, &mut Effects::default()).unwrap() {
                    Advance::Rows => panic!("empty grouping cannot invent a group"),
                    Advance::Finished => {
                        done = true;
                        break;
                    }
                    Advance::Progress => (),
                }
            }
            assert!(done);
            drop(general);
            drop(running);
            drop(pressure);
            assert_eq!(database.reserved_memory_bytes(), baseline);
            assert_eq!(database.reserved_temp_bytes(), 0);
        }
    }
}

#[test]
fn projected_maximum_text_keys_fit_the_reserved_result_frame() {
    for (width, temporary) in [(10, 8_000_000), (MAX_COLUMNS, 16_000_000)] {
        let directory = Directory::new();
        let database = Database::create_empty(
            &directory.0.join("db"),
            crate::Config::new(16_000_000, temporary).unwrap(),
        )
        .unwrap();
        let cancel = CancellationToken::new();
        database
            .declare_table(
                "facts",
                &[crate::ColumnDeclaration {
                    name: "k",
                    data_type: DataType::String,
                    nullable: false,
                }],
                &cancel,
            )
            .unwrap();
        let first = "a".repeat(crate::batch::MAX_TEXT_BYTES);
        let second = "b".repeat(crate::batch::MAX_TEXT_BYTES);
        let mut append = database
            .begin_append(
                "facts",
                crate::AppendLimits {
                    batches: 1,
                    encoded_bytes: 200_000,
                },
                &cancel,
            )
            .unwrap();
        append
            .write(
                &[crate::ColumnInput {
                    values: crate::ColumnValues::String(&[&second, &first]),
                    validity: &[3],
                }],
                &cancel,
            )
            .unwrap();
        append.commit(&cancel).unwrap();
        let scan_query = database.prepare("FROM facts").unwrap();
        let count_query = database
            .prepare("FROM facts |> AGGREGATE COUNT(*) AS n")
            .unwrap();
        let baseline = database.reserved_memory_bytes();
        for hash_groups in [1, 2] {
            let plan = lower(
                &database,
                &scan_query,
                scan_query.snapshot.as_ref().unwrap().state(),
                0,
            )
            .unwrap();
            let (mut source_workspace, _) = declared::open(
                &database,
                &scan_query,
                &plan,
                &cancel,
                &mut Effects::default(),
            )
            .unwrap();
            let mut driver = InputDriver::new();
            let mut output = plan.output().clone();
            output.column_count = width;
            output.columns.fill(0);
            let types = [DataType::String; MAX_COLUMNS];
            let text = [Some(crate::batch::MAX_TEXT_BYTES); MAX_COLUMNS];
            let mut output_charge = database
                .reserve_memory(
                    Batch::required_bytes_with_text(&types[..width], &text[..width]).unwrap(),
                    "test projected output",
                )
                .unwrap();
            source_workspace.output =
                OwnedBatch::new_with_text(&types[..width], &text[..width], &mut output_charge)
                    .unwrap();
            let aggregate = AggregateState::new(
                &database.memory,
                count_query.plan.aggregates.first().unwrap(),
                count_query.plan.aggregate_demand(0),
                1,
                scan_query.plan.input_columns(),
            )
            .unwrap();
            let keys = schema(&[(DataType::String, false)]);
            let mut general = General::new(
                &database,
                aggregate,
                keys,
                &output,
                true,
                Limits {
                    arguments: 1,
                    run_bytes: RECORD_HEADER + 5 + crate::batch::MAX_TEXT_BYTES,
                    run_rows: 1,
                },
                (
                    hash_groups,
                    hash_groups * (5 + crate::batch::MAX_TEXT_BYTES),
                ),
            )
            .unwrap();
            assert!(general.output.max_bytes > MAX_ARGUMENT_RECORD_BYTES);
            let mut rows = 0;
            let mut done = false;
            for _ in 0..1000 {
                match drive_consumer(
                    &mut general,
                    &mut source_workspace,
                    &mut driver,
                    (plan.scan(), &output),
                    &cancel,
                    &mut Effects::default(),
                )
                .unwrap()
                {
                    Advance::Rows => {
                        for column in 0..width {
                            let Some(Value::String(value)) =
                                source_workspace.output.value(0, column)
                            else {
                                panic!("text result");
                            };
                            assert_eq!(value.as_str(), if rows == 0 { &first } else { &second });
                        }
                        rows += 1;
                    }
                    Advance::Finished => {
                        done = true;
                        break;
                    }
                    Advance::Progress => (),
                }
            }
            assert!(done);
            assert_eq!(rows, 2);
            drop(general);
            drop(source_workspace);
            drop(plan);
            drop(output_charge);
            assert_eq!(database.reserved_memory_bytes(), baseline);
            assert_eq!(database.reserved_temp_bytes(), 0);
        }
        let query = database
            .prepare(&format!(
                "FROM facts |> AGGREGATE COUNT(*) AS n GROUP AND ORDER BY k |> SELECT {}",
                vec!["k"; width].join(",")
            ))
            .unwrap();
        let baseline = database.reserved_memory_bytes();
        let initial = database.execute(&query, &cancel).unwrap();
        let Some(Aggregation::General(owner)) = initial.first_aggregate() else {
            unreachable!()
        };
        let minimum = initial.accounted_memory_bytes() + crate::catalog::MAX_BYTES as u64
            - owner[0].memory[0].memory_bytes();
        drop(initial);
        for disk in [false, true] {
            let pressure = disk.then(|| {
                database
                    .reserve_memory(
                        database.config().memory_limit_bytes() - baseline - minimum,
                        "wide public disk path",
                    )
                    .unwrap()
            });
            let mut result = database.execute(&query, &cancel).unwrap();
            let Some(Aggregation::General(owner)) = result.first_aggregate() else {
                unreachable!()
            };
            assert_eq!(owner[0].memory.is_empty(), disk);
            let mut rows = 0;
            let mut finished = false;
            let mut peak_temp = 0;
            for _ in 0..2000 {
                match result.step() {
                    QueryStep::Rows(batch) => {
                        assert_eq!(batch.column_count(), width);
                        for row in 0..batch.len() {
                            for column in 0..width {
                                let Some(Value::String(value)) = batch.value(row, column) else {
                                    panic!("public text output")
                                };
                                assert_eq!(
                                    value.as_str(),
                                    if rows == 0 { &first } else { &second }
                                );
                            }
                            rows += 1;
                        }
                    }
                    QueryStep::Progress => (),
                    QueryStep::Finished => {
                        finished = true;
                        break;
                    }
                    QueryStep::Failed(error) => panic!("public width={width} disk={disk}: {error}"),
                }
                peak_temp = peak_temp.max(database.reserved_temp_bytes());
            }
            assert!(finished);
            assert_eq!(rows, 2);
            assert_eq!(peak_temp != 0, disk);
            drop(result);
            drop(pressure);
            assert_eq!(database.reserved_memory_bytes(), baseline);
            assert_eq!(database.reserved_temp_bytes(), 0);
        }
    }
}

#[test]
fn repeated_aggregation_executes_through_public_preparation() {
    let directory = Directory::new();
    let database = database(
        &directory,
        &[
            (Some(2), Some(4), 4.0),
            (Some(1), Some(3), 3.0),
            (Some(2), Some(5), 5.0),
            (None, None, 0.0),
        ],
    );
    let cancel = CancellationToken::new();
    for (sql, expected) in [
        (
            "FROM facts |> AGGREGATE SUM(n) AS total GROUP BY k |> AGGREGATE AVG(total) AS average",
            6.0,
        ),
        (
            "FROM facts |> AGGREGATE SUM(n) AS total GROUP BY k |> SELECT total*2 AS doubled |> AGGREGATE AVG(doubled) AS average",
            12.0,
        ),
        (
            "FROM facts |> AGGREGATE SUM(n) AS total |> AGGREGATE AVG(total) AS average",
            12.0,
        ),
        (
            "FROM facts |> AGGREGATE SUM(n) AS total GROUP BY k |> AGGREGATE SUM(total) AS subtotal GROUP BY total |> AGGREGATE AVG(subtotal) AS average",
            6.0,
        ),
    ] {
        let baseline = database.reserved_memory_bytes();
        let query = database.prepare(sql).unwrap();
        let mut result = database.execute(&query, &cancel).unwrap();
        let mut rows = Vec::new();
        let mut finished = false;
        for _ in 0..16384 {
            match result.step() {
                QueryStep::Rows(batch) => {
                    for row in 0..batch.len() {
                        let Value::Double(value) = batch.value(row, 0).unwrap() else {
                            panic!("DOUBLE result");
                        };
                        rows.push(value);
                    }
                }
                QueryStep::Progress => (),
                QueryStep::Finished => {
                    finished = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("{sql}: {error}"),
            }
        }
        assert!(finished, "{sql}");
        assert_eq!(rows, [expected], "{sql}");
        drop(result);
        drop(query);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
}

#[test]
fn derived_inputs_bind_and_execute_independent_scopes() {
    let directory = Directory::new();
    let database = database(
        &directory,
        &[
            (Some(1), Some(10), 10.0),
            (Some(1), Some(20), 20.0),
            (Some(2), Some(90), 90.0),
        ],
    );
    let cancel = CancellationToken::new();
    let baseline = database.reserved_memory_bytes();
    for (sql, expected) in [
        (
            "FROM facts |> AGGREGATE SUM(n) AS total GROUP BY k |> AS totals |> JOIN facts AS a ON totals.k = a.k |> AGGREGATE SUM(total) AS weighted",
            Value::Int64(150),
        ),
        (
            "FROM (FROM facts |> AGGREGATE SUM(n) AS total GROUP BY k) AS totals |> AGGREGATE AVG(total) AS mean",
            Value::Double(60.0),
        ),
        (
            "FROM facts AS a |> JOIN (FROM facts |> AGGREGATE SUM(n) AS total GROUP BY k) AS totals ON a.k = totals.k |> AGGREGATE SUM(total) AS weighted",
            Value::Int64(150),
        ),
        (
            "FROM (FROM (FROM facts)) |> AGGREGATE COUNT(*) AS n",
            Value::Int64(3),
        ),
        (
            "FROM (FROM facts |> WHERE k = 1 |> AGGREGATE SUM(n) AS total GROUP BY k) AS a |> JOIN (FROM facts |> AGGREGATE SUM(n) AS total GROUP BY k) AS b ON a.k = b.k |> AGGREGATE SUM(a.total+b.total) AS n",
            Value::Int64(60),
        ),
        (
            "FROM facts AS a |> JOIN (FROM facts AS b |> JOIN facts AS c ON b.k = c.k |> SELECT b.k AS k) AS r ON a.k = r.k |> AGGREGATE COUNT(*) AS n",
            Value::Int64(9),
        ),
        (
            "FROM facts AS a |> JOIN (FROM facts AS a |> SELECT a.k AS k) AS b ON a.k = b.k |> AGGREGATE COUNT(*) AS n",
            Value::Int64(5),
        ),
        (
            "FROM (FROM facts AS a) AS b |> AGGREGATE SUM(b.n) AS total",
            Value::Int64(120),
        ),
        (
            "FROM (FROM facts |> SELECT (n+1)*2 AS x) |> AGGREGATE SUM(x) AS total",
            Value::Int64(246),
        ),
        (
            "FROM (FROM facts |> ORDER BY n DESC |> LIMIT 2) |> AGGREGATE SUM(n) AS total",
            Value::Int64(110),
        ),
    ] {
        let query = database
            .prepare(sql)
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
        let mut result = database.execute(&query, &cancel).unwrap();
        let mut seen = false;
        let mut done = false;
        for _ in 0..16384 {
            match result.step() {
                QueryStep::Rows(batch) => {
                    assert!(!seen && batch.len() == 1 && batch.column_count() == 1);
                    assert_eq!(batch.value(0, 0), Some(expected), "{sql}");
                    seen = true;
                }
                QueryStep::Progress => (),
                QueryStep::Finished => {
                    done = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("{sql}: {error}"),
            }
        }
        assert!(seen && done, "{sql}");
        drop(result);
        drop(query);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
    for sql in [
        "FROM (FROM facts AS a) |> SELECT a.n",
        "FROM (FROM facts) |> SELECT facts.n",
        "FROM facts AS a |> JOIN (FROM facts |> WHERE a.k = 1) AS b ON a.k = b.k",
        "FROM facts AS a |> JOIN (FROM facts) AS a ON a.k = a.k",
        "FROM (FROM facts |> SELECT k AS x,n AS x) |> SELECT x",
    ] {
        assert!(
            matches!(database.prepare(sql), Err(Error::Bind { .. })),
            "{sql}"
        );
        assert_eq!(database.reserved_memory_bytes(), baseline);
    }
    for sql in [
        "FROM (SELECT k FROM facts)",
        "FROM (FROM facts;)",
        "FROM (FROM facts",
        "FROM facts |> JOIN (FROM facts) AS b",
    ] {
        assert!(
            matches!(database.prepare(sql), Err(Error::Parse { .. })),
            "{sql}"
        );
        assert_eq!(database.reserved_memory_bytes(), baseline);
    }
    for (sql, ordered) in [
        ("FROM (FROM facts |> ORDER BY n)", false),
        ("FROM (FROM facts |> ORDER BY n |> LIMIT 2) AS t", false),
        ("FROM (FROM facts |> ORDER BY n) |> ORDER BY k", true),
    ] {
        let query = database.prepare(sql).unwrap();
        assert_eq!(
            query
                .plan
                .order_key(query.plan.final_relation(), 0)
                .unwrap()
                .is_some(),
            ordered
        );
        drop(query);
        assert_eq!(database.reserved_memory_bytes(), baseline);
    }
}

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[cfg_attr(
    all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"),
    ignore = "GNU aarch64 pthread minimum exceeds the 64-KiB reported-stack ceiling"
)]
fn derived_nesting_uses_bounded_reported_stack() {
    let directory = Directory::new();
    let database = database(&directory, &[]);
    let baseline = database.reserved_memory_bytes();
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(48 * 1024)
            .spawn_scoped(scope, || {
                let reported = pipesql_filesystem::test_current_thread_stack_bytes();
                assert!(
                    reported <= 65536,
                    "actual stack exceeds test ceiling: {reported}"
                );
                let mut sql = String::from("FROM facts");
                for depth in 0..=crate::frontend::MAX_STAGES {
                    let query = database
                        .prepare(&sql)
                        .unwrap_or_else(|error| panic!("depth={depth}: {error}"));
                    drop(query);
                    assert_eq!(database.reserved_memory_bytes(), baseline);
                    sql = format!("FROM ({sql})");
                }
                assert!(matches!(database.prepare(&sql), Err(Error::Parse { .. })));
                assert_eq!(database.reserved_memory_bytes(), baseline);
                sql = String::from("FROM facts");
                for depth in 0..=4 {
                    let query = database
                        .prepare(&sql)
                        .unwrap_or_else(|error| panic!("join depth={depth}: {error}"));
                    drop(query);
                    assert_eq!(database.reserved_memory_bytes(), baseline);
                    sql = format!(
                        "FROM facts AS a |> JOIN ({sql}) AS b ON a.k = b.k |> SELECT a.k AS k"
                    );
                }
                assert!(database.prepare(&sql).is_err());
                assert_eq!(database.reserved_memory_bytes(), baseline);
            })
            .unwrap()
            .join()
            .unwrap();
    });
}
