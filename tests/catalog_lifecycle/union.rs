use super::*;

#[test]
fn public_union_composes_with_spill_and_temporary_refusal() {
    const ROWS: usize = 2048;
    const BATCH: usize = 64;
    let directory = Directory::new();
    let cancel = CancellationToken::new();
    let db = Database::create_empty(
        &directory.database(),
        Config::new(32_000_000, 16_000_000).unwrap(),
    )
    .unwrap();
    db.declare_table(
        "words",
        &[ColumnDeclaration {
            name: "k",
            data_type: DataType::String,
            nullable: false,
        }],
        &cancel,
    )
    .unwrap();
    let keys: Vec<_> = (0..ROWS)
        .map(|i| format!("{i:04}{}", "界".repeat(300)))
        .collect();
    let mut writer = db
        .begin_append(
            "words",
            AppendLimits {
                batches: (ROWS / BATCH) as u32,
                encoded_bytes: 4_000_000,
            },
            &cancel,
        )
        .unwrap();
    for batch in keys.chunks(BATCH) {
        let text: Vec<_> = batch.iter().map(String::as_str).collect();
        writer
            .write(
                &[ColumnInput {
                    values: ColumnValues::String(&text),
                    validity: &[255; BATCH / 8],
                }],
                &cancel,
            )
            .unwrap();
    }
    writer.commit(&cancel).unwrap();
    db.close().unwrap();
    for (sql, grouped, copies) in [
        (
            "FROM words |> ORDER BY k |> UNION ALL (FROM words |> ORDER BY k)",
            false,
            2,
        ),
        (
            "FROM words |> UNION ALL (FROM words) |> AGGREGATE COUNT(*) AS n GROUP BY k",
            true,
            1,
        ),
        ("FROM words |> UNION DISTINCT (FROM words)", false, 1),
    ] {
        for temp_limit in [16_000_000, 1] {
            let db = Database::open(
                &directory.database(),
                Config::new(4_000_000, temp_limit).unwrap(),
            )
            .unwrap();
            let baseline = db.reserved_memory_bytes();
            let prepared = db.prepare(sql).unwrap();
            let mut result = db.execute(&prepared, &cancel).unwrap();
            let mut actual = Vec::new();
            let mut peak_temp = 0;
            let mut terminal = false;
            for _ in 0..200_000 {
                let step = result.step();
                peak_temp = peak_temp.max(db.reserved_temp_bytes());
                assert!(db.reserved_memory_bytes() <= 4_000_000);
                match step {
                    QueryStep::Progress => (),
                    QueryStep::Rows(batch) => {
                        for row in 0..batch.len() {
                            actual.push(
                                (0..batch.column_count())
                                    .map(|column| owned_cell(batch.value(row, column).unwrap()))
                                    .collect::<Vec<_>>(),
                            );
                        }
                    }
                    QueryStep::Finished => {
                        assert_ne!(temp_limit, 1);
                        terminal = true;
                        break;
                    }
                    QueryStep::Failed(Error::Resource {
                        owner: "database temporary storage",
                        ..
                    }) => {
                        assert_eq!(temp_limit, 1);
                        terminal = true;
                        break;
                    }
                    QueryStep::Failed(error) => panic!("{sql}: {error}"),
                }
            }
            assert!(terminal, "bounded spill fixture: {sql}");
            if temp_limit != 1 {
                assert!(peak_temp > 0, "this case must exercise spill: {sql}");
                let mut expected: Vec<Vec<Cell>> = if grouped {
                    keys.iter()
                        .map(|key| vec![Cell::Text(key.clone()), Cell::Integer(2)])
                        .collect()
                } else {
                    keys.iter()
                        .cycle()
                        .take(ROWS * copies)
                        .map(|key| vec![Cell::Text(key.clone())])
                        .collect()
                };
                actual.sort_unstable();
                expected.sort_unstable();
                assert_eq!(actual, expected);
            }
            drop(result);
            drop(prepared);
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
            db.close().unwrap();
        }
    }
}

#[test]
fn public_union_distinct_compares_complete_positional_rows() {
    let (_directory, db) = join_fixture();
    for (sql, expected) in [
        (
            "FROM facts |> SELECT k AS x |> UNION DISTINCT (FROM facts |> SELECT k AS y), (FROM facts |> SELECT 2 AS z) |> ORDER BY x NULLS FIRST",
            vec![
                vec![Cell::Null],
                vec![Cell::Integer(1)],
                vec![Cell::Integer(2)],
            ],
        ),
        (
            "FROM facts |> SELECT k, v |> UNION DISTINCT (FROM facts |> SELECT k, v) |> SELECT k |> ORDER BY k NULLS FIRST",
            vec![
                vec![Cell::Null],
                vec![Cell::Integer(1)],
                vec![Cell::Integer(1)],
                vec![Cell::Integer(2)],
            ],
        ),
        (
            "FROM facts |> SELECT v AS x |> UNION DISTINCT (FROM facts |> SELECT v AS x |> UNION ALL (FROM facts |> SELECT v AS x)), (FROM facts |> SELECT v+1 AS x) |> ORDER BY x",
            integers(&[10, 11, 20, 21, 30, 31, 40, 41]),
        ),
        (
            "FROM facts |> UNION DISTINCT (FROM facts) |> AS u |> JOIN dimensions AS d ON u.k=d.k |> SELECT u.v |> ORDER BY v",
            integers(&[10, 10, 20, 20, 30]),
        ),
        (
            "FROM facts |> WHERE v<0 |> SELECT v*9223372036854775807 AS v |> UNION DISTINCT (FROM facts |> SELECT v) |> ORDER BY v",
            integers(&[10, 20, 30, 40]),
        ),
        (
            "FROM facts |> WHERE v<0 |> UNION DISTINCT (FROM facts |> WHERE v<0) |> AGGREGATE COUNT(*) AS n",
            integers(&[0]),
        ),
    ] {
        query(&db, sql, expected);
    }
    for sql in [
        "FROM facts |> SELECT k AS x |> UNION DISTINCT (FROM facts |> SELECT k AS y) |> SELECT y",
        "FROM facts |> UNION DISTINCT (FROM facts) |> SELECT facts.k",
        "FROM facts |> UNION DISTINCT (FROM dimensions)",
    ] {
        assert!(matches!(db.prepare(sql), Err(Error::Bind { .. })), "{sql}");
    }
}

#[test]
fn public_union_distinct_demands_projected_away_fields_before_limit() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    for sql in [
        "FROM facts |> SELECT v, v*9223372036854775807 AS unused |> UNION DISTINCT (FROM facts |> SELECT v, 1 AS unused) |> SELECT v |> LIMIT 1",
        "FROM facts |> SELECT v, 1 AS unused |> UNION DISTINCT (FROM facts |> SELECT v, v*9223372036854775807 AS unused) |> SELECT v |> LIMIT 1",
    ] {
        let cancel = CancellationToken::new();
        let prepared = db.prepare(sql).unwrap();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        let mut failed = false;
        for _ in 0..2000 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Rows(_) | QueryStep::Finished => {
                    panic!("deduplication skipped a demanded value: {sql}")
                }
                QueryStep::Failed(Error::ArithmeticOverflow { span, .. }) => {
                    assert_eq!(&sql[span.start()..span.end()], "v*9223372036854775807");
                    failed = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("wrong failure: {error}"),
            }
        }
        assert!(failed);
        drop(result);
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}

#[test]
fn public_union_streams_positional_duplicates_and_composed_branches() {
    let (_directory, db) = join_fixture();
    for (sql, expected) in [
        (
            "FROM facts |> SELECT v AS x |> UNION ALL (FROM facts |> SELECT v AS y) |> ORDER BY x",
            integers(&[10, 10, 20, 20, 30, 30, 40, 40]),
        ),
        (
            "FROM facts |> SELECT v AS x |> UNION ALL (FROM facts |> SELECT k AS y) |> ORDER BY x",
            [vec![Cell::Null]]
                .into_iter()
                .chain(integers(&[1, 1, 2, 10, 20, 30, 40]))
                .collect(),
        ),
        (
            "FROM facts |> SELECT v AS x |> UNION ALL (FROM facts |> SELECT v+1 AS x |> UNION ALL (FROM facts |> SELECT v+2 AS x)) |> AGGREGATE COUNT(*) AS n",
            integers(&[12]),
        ),
        (
            "FROM facts |> SELECT k |> UNION ALL (FROM facts |> SELECT k) |> DISTINCT |> ORDER BY k",
            vec![
                vec![Cell::Null],
                vec![Cell::Integer(1)],
                vec![Cell::Integer(2)],
            ],
        ),
        (
            "FROM facts |> SELECT v |> UNION ALL (FROM facts |> WHERE v<0 |> SELECT v) |> ORDER BY v",
            integers(&[10, 20, 30, 40]),
        ),
        (
            "FROM facts |> WHERE v<0 |> SELECT v |> UNION ALL (FROM facts |> SELECT v) |> ORDER BY v",
            integers(&[10, 20, 30, 40]),
        ),
        (
            "FROM facts |> SELECT v |> UNION ALL (FROM facts |> SELECT v) |> SET v=v+1 |> WHERE v>30 |> ORDER BY v",
            integers(&[31, 31, 41, 41]),
        ),
        (
            "FROM facts |> SELECT v AS x, v AS y |> UNION ALL (FROM facts |> SELECT v, k) |> SELECT y |> ORDER BY y",
            [vec![Cell::Null]]
                .into_iter()
                .chain(integers(&[1, 1, 2, 10, 20, 30, 40]))
                .collect(),
        ),
    ] {
        query(&db, sql, expected);
    }
}

#[test]
fn public_union_limits_preserve_branch_demand_and_join_composition() {
    let (_directory, db) = join_fixture();
    for (sql, expected) in [
        (
            "FROM facts |> SELECT v |> UNION ALL (FROM facts |> SELECT v*9223372036854775807 AS v) |> LIMIT 1",
            integers(&[10]),
        ),
        (
            "FROM facts |> SELECT v, v*9223372036854775807 AS unused |> UNION ALL (FROM facts |> SELECT v, v*9223372036854775807 AS unused) |> SELECT v |> ORDER BY v",
            integers(&[10, 10, 20, 20, 30, 30, 40, 40]),
        ),
        (
            "FROM facts |> SELECT k, v |> UNION ALL (FROM facts |> SELECT k, v) |> AS u |> JOIN dimensions AS d ON u.k=d.k |> SELECT u.v |> ORDER BY v",
            integers(&[10, 10, 10, 10, 20, 20, 20, 20, 30, 30]),
        ),
        (
            "FROM dimensions AS d |> JOIN (FROM facts |> UNION ALL (FROM facts) |> LIMIT 3) AS u ON d.k=u.k |> SELECT u.v |> ORDER BY v",
            integers(&[10, 10, 20, 20, 30]),
        ),
    ] {
        query(&db, sql, expected);
    }
}

#[test]
fn public_union_propagates_demanded_branch_errors_with_original_spans() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    for sql in [
        "FROM facts |> SELECT v |> UNION ALL (FROM facts |> SELECT v*9223372036854775807 AS v)",
        "FROM facts |> WHERE v<0 |> SELECT v |> UNION ALL (FROM facts |> SELECT v*9223372036854775807 AS v)",
        "FROM facts |> SELECT v |> UNION ALL (FROM facts |> SELECT v*9223372036854775807 AS bad |> ORDER BY bad |> SELECT 1 AS v)",
    ] {
        let cancel = CancellationToken::new();
        let prepared = db.prepare(sql).unwrap();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        let mut failed = false;
        for _ in 0..2000 {
            match result.step() {
                QueryStep::Progress | QueryStep::Rows(_) => (),
                QueryStep::Finished => panic!("demanded overflow disappeared: {sql}"),
                QueryStep::Failed(Error::ArithmeticOverflow { span, .. }) => {
                    assert_eq!(&sql[span.start()..span.end()], "v*9223372036854775807");
                    failed = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("wrong failure: {error}"),
            }
        }
        assert!(failed);
        drop(result);
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}

#[test]
fn public_union_cancellation_releases_each_scheduled_prefix() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    for (sql, expected_rows) in [
        ("FROM facts |> UNION ALL (FROM facts) |> SELECT v", 8),
        ("FROM facts |> UNION DISTINCT (FROM facts) |> SELECT v", 4),
        (
            "FROM facts |> UNION DISTINCT (FROM facts) |> AGGREGATE COUNT(*) AS n",
            1,
        ),
        (
            "FROM facts |> ORDER BY v |> UNION ALL (FROM facts |> ORDER BY v) |> SELECT v",
            8,
        ),
        (
            "FROM facts |> AGGREGATE COUNT(*) AS n |> UNION ALL (FROM facts |> AGGREGATE COUNT(*) AS n)",
            2,
        ),
    ] {
        let prepared = db.prepare(sql).unwrap();
        let retained = db.reserved_memory_bytes();
        let mut prefixes = 0;
        // Stop when the first uncancelled execution completes: every earlier
        // scheduler boundary has then been cancelled in a fresh execution.
        for cancel_after in 0..1000 {
            let cancel = CancellationToken::new();
            let mut result = db.execute(&prepared, &cancel).unwrap();
            let mut terminal = false;
            let mut completed = false;
            let mut rows = 0;
            for step in 0..2000 {
                if step == cancel_after {
                    cancel.cancel();
                }
                match result.step() {
                    QueryStep::Progress => (),
                    QueryStep::Rows(batch) => rows += batch.len(),
                    QueryStep::Finished => {
                        completed = true;
                        terminal = true;
                        break;
                    }
                    QueryStep::Failed(Error::Cancelled) => {
                        assert!(cancel.is_cancelled());
                        terminal = true;
                        break;
                    }
                    QueryStep::Failed(error) => panic!("{sql}: {error}"),
                }
            }
            assert!(terminal);
            drop(result);
            assert_eq!(db.reserved_memory_bytes(), retained);
            assert_eq!(db.reserved_temp_bytes(), 0);
            if completed {
                assert_eq!(
                    rows, expected_rows,
                    "{sql}: complete output after cancellation"
                );
                // A producer that has finished its work can report completion
                // after late cancellation. Still sweep the remaining boundary.
                if !cancel.is_cancelled() {
                    break;
                }
            }
            prefixes += 1;
        }
        assert!(
            prefixes > 4 && prefixes < 999,
            "bounded complete cancellation sweep"
        );
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
}

#[test]
fn public_union_keeps_typed_bytes_and_one_snapshot_across_branches() {
    let directory = Directory::new();
    let db = Database::create_empty(&directory.database(), config()).unwrap();
    let cancel = CancellationToken::new();
    for name in ["first", "second"] {
        db.declare_table(name, &declarations(), &cancel).unwrap();
    }
    let sql =
        "FROM first |> UNION ALL (FROM second |> SELECT note AS renamed, amount, number, day)";
    let empty = db.prepare(sql).unwrap();
    let dates = [
        DateValue::from_days_since_unix_epoch(-719_162).unwrap(),
        DateValue::from_days_since_unix_epoch(2_932_896).unwrap(),
    ];
    let append = |name| {
        let mut writer = db.begin_append(name, limits(), &cancel).unwrap();
        writer
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::String(&["雪é", "ignored"]),
                        validity: &[1],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&[i64::MIN, i64::MAX]),
                        validity: &[3],
                    },
                    ColumnInput {
                        values: ColumnValues::Double(&[
                            -0.0,
                            f64::from_bits(0x7ff8_0000_0000_0042),
                        ]),
                        validity: &[3],
                    },
                    ColumnInput {
                        values: ColumnValues::Date(&dates),
                        validity: &[3],
                    },
                ],
                &cancel,
            )
            .unwrap();
        writer.commit(&cancel).unwrap();
    };
    append("first");
    append("second");
    assert!(collect_unordered(&mut db.execute(&empty, &cancel).unwrap()).is_empty());
    drop(empty);
    let prepared = db.prepare(sql).unwrap();
    assert_eq!(prepared.result_column(0).unwrap().name, Some("note"));
    let mut running = db.execute(&prepared, &cancel).unwrap();
    append("second");
    let pair = [
        vec![
            Cell::Text("雪é".into()),
            Cell::Integer(i64::MIN),
            Cell::Number((-0.0_f64).to_bits()),
            Cell::Day(-719_162),
        ],
        vec![
            Cell::Null,
            Cell::Integer(i64::MAX),
            Cell::Number(0x7ff8_0000_0000_0042),
            Cell::Day(2_932_896),
        ],
    ];
    let mut expected: Vec<_> = pair.iter().cycle().take(4).cloned().collect();
    expected.sort_unstable();
    assert_eq!(collect_unordered(&mut running), expected);
    drop(running);
    assert_eq!(
        collect_unordered(&mut db.execute(&prepared, &cancel).unwrap()),
        expected
    );
    drop(prepared);
    let fresh = db.prepare(sql).unwrap();
    let mut expected: Vec<_> = pair.iter().cycle().take(6).cloned().collect();
    expected.sort_unstable();
    assert_eq!(
        collect_unordered(&mut db.execute(&fresh, &cancel).unwrap()),
        expected
    );
    drop(fresh);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close().unwrap();
}

#[test]
fn public_union_distinct_keeps_typed_representatives_and_prepared_snapshot() {
    let directory = Directory::new();
    let db = Database::create_empty(&directory.database(), config()).unwrap();
    let cancel = CancellationToken::new();
    db.declare_table("typed", &declarations(), &cancel).unwrap();
    let baseline = db.reserved_memory_bytes();
    let sql = "FROM typed |> UNION DISTINCT (FROM typed)";
    let empty = db.prepare(sql).unwrap();
    let first_day = DateValue::from_days_since_unix_epoch(-719_162).unwrap();
    let last_day = DateValue::from_days_since_unix_epoch(2_932_896).unwrap();
    let mut writer = db.begin_append("typed", limits(), &cancel).unwrap();
    writer
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::String(&["ignored", "ignored", "雪", "雪"]),
                    validity: &[12],
                },
                ColumnInput {
                    values: ColumnValues::Int64(&[i64::MIN, i64::MIN, i64::MAX, i64::MAX]),
                    validity: &[15],
                },
                ColumnInput {
                    values: ColumnValues::Double(&[
                        -0.0,
                        0.0,
                        f64::from_bits(0x7ff8_0000_0000_0042),
                        f64::from_bits(0x7ff8_0000_0000_0099),
                    ]),
                    validity: &[15],
                },
                ColumnInput {
                    values: ColumnValues::Date(&[first_day, first_day, last_day, last_day]),
                    validity: &[15],
                },
            ],
            &cancel,
        )
        .unwrap();
    writer.commit(&cancel).unwrap();
    assert!(collect_unordered(&mut db.execute(&empty, &cancel).unwrap()).is_empty());
    drop(empty);
    let prepared = db.prepare(sql).unwrap();
    let mut running = db.execute(&prepared, &cancel).unwrap();
    let mut writer = db.begin_append("typed", limits(), &cancel).unwrap();
    writer
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::String(&["new"]),
                    validity: &[1],
                },
                ColumnInput {
                    values: ColumnValues::Int64(&[0]),
                    validity: &[1],
                },
                ColumnInput {
                    values: ColumnValues::Double(&[1.5]),
                    validity: &[1],
                },
                ColumnInput {
                    values: ColumnValues::Date(
                        &[DateValue::from_days_since_unix_epoch(0).unwrap()],
                    ),
                    validity: &[1],
                },
            ],
            &cancel,
        )
        .unwrap();
    writer.commit(&cancel).unwrap();

    // Compare all non-floating fields literally. Floating representatives may
    // retain either original encoding within each not-distinct class; they must
    // not acquire a new encoding during deduplication.
    let check = |mut rows: Vec<Vec<Cell>>, fresh: bool| {
        assert_eq!(rows.len(), if fresh { 3 } else { 2 });
        rows.sort_unstable_by(|left, right| left[1].cmp(&right[1]));
        let Cell::Number(bits) = rows[0][2] else {
            panic!("zero representative is not DOUBLE")
        };
        assert!([(-0.0_f64).to_bits(), 0.0_f64.to_bits()].contains(&bits));
        rows[0][2] = Cell::Number(0);
        let last = rows.last_mut().unwrap();
        let Cell::Number(bits) = last[2] else {
            panic!("NaN representative is not DOUBLE")
        };
        assert!([0x7ff8_0000_0000_0042, 0x7ff8_0000_0000_0099].contains(&bits));
        last[2] = Cell::Number(0x7ff8_0000_0000_0042);
        let mut expected = vec![vec![
            Cell::Null,
            Cell::Integer(i64::MIN),
            Cell::Number(0),
            Cell::Day(-719_162),
        ]];
        if fresh {
            expected.push(vec![
                Cell::Text("new".into()),
                Cell::Integer(0),
                Cell::Number(1.5_f64.to_bits()),
                Cell::Day(0),
            ]);
        }
        expected.push(vec![
            Cell::Text("雪".into()),
            Cell::Integer(i64::MAX),
            Cell::Number(0x7ff8_0000_0000_0042),
            Cell::Day(2_932_896),
        ]);
        assert_eq!(rows, expected);
    };
    check(collect_unordered(&mut running), false);
    drop(running);
    check(
        collect_unordered(&mut db.execute(&prepared, &cancel).unwrap()),
        false,
    );
    drop(prepared);
    let fresh = db.prepare(sql).unwrap();
    check(
        collect_unordered(&mut db.execute(&fresh, &cancel).unwrap()),
        true,
    );
    drop(fresh);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close().unwrap();
}
