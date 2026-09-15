//! Public joins contract tests.
use super::*;

#[test]
fn self_join_preserves_independent_occurrences_and_duplicate_pairs() {
    assert_query_rows(
        "FROM facts AS left_row \
         |> JOIN facts AS right_row ON left_row.k = right_row.k \
         |> SELECT left_row.v, right_row.v",
        [(10, 10), (10, 20), (20, 10), (20, 20), (30, 30)]
            .map(|(left, right)| vec![Cell::Integer(left), Cell::Integer(right)])
            .to_vec(),
    );
}

#[test]
fn join_consumes_projected_relation() {
    assert_query_rows(
        "FROM facts |> SELECT k AS key, v |> AS projected \
         |> JOIN dimensions AS dimension ON projected.key = dimension.k \
         |> SELECT projected.v, dimension.label",
        [(10, "a"), (10, "b"), (20, "a"), (20, "b"), (30, "c")]
            .map(|(value, label)| vec![Cell::Integer(value), Cell::Text(label.to_owned())])
            .to_vec(),
    );
}

#[test]
fn join_consumes_aggregated_relation() {
    assert_query_rows(
        "FROM facts |> AGGREGATE SUM(v) AS total GROUP BY k |> AS grouped \
         |> JOIN dimensions AS dimension ON grouped.k = dimension.k \
         |> SELECT grouped.total, dimension.label",
        ["a", "b", "c"]
            .map(|label| vec![Cell::Integer(30), Cell::Text(label.to_owned())])
            .to_vec(),
    );
}

#[test]
fn joins_compose_with_further_joins_filters_and_aggregation() {
    assert_query_rows(
        "FROM facts AS f |> JOIN dimensions AS d ON f.k = d.k \
         |> JOIN facts AS g ON d.k = g.k |> WHERE g.v >= 20 \
         |> SELECT f.v, g.v, d.label",
        [
            (10, 20, "a"),
            (10, 20, "b"),
            (20, 20, "a"),
            (20, 20, "b"),
            (30, 30, "c"),
        ]
        .map(|(a, b, label)| {
            vec![
                Cell::Integer(a),
                Cell::Integer(b),
                Cell::Text(label.to_owned()),
            ]
        })
        .to_vec(),
    );
    for (suffix, count) in [("", 5), (" |> WHERE f.v > 100", 0)] {
        assert_query_rows(
            &format!(
                "FROM facts AS f |> JOIN dimensions AS d ON f.k = d.k{suffix} |> AGGREGATE COUNT(*) AS n"
            ),
            vec![vec![Cell::Integer(count)]],
        );
    }
    assert_query_rows(
        "FROM facts AS f |> JOIN dimensions AS d ON d.k = f.k \
         |> AGGREGATE SUM(f.v) AS total, COUNT(*) AS n GROUP BY d.label",
        [("a", 30, 2), ("b", 30, 2), ("c", 30, 1)]
            .map(|(label, total, n)| {
                vec![
                    Cell::Text(label.to_owned()),
                    Cell::Integer(total),
                    Cell::Integer(n),
                ]
            })
            .to_vec(),
    );
    for sql in [
        "FROM facts AS f |> WHERE f.v > 100 |> JOIN dimensions AS d ON f.k = d.k",
        "FROM facts AS f |> WHERE f.v = 40 |> JOIN dimensions AS d ON f.k = d.k",
    ] {
        assert_query_rows(sql, vec![]);
    }
}

#[test]
fn joins_keep_one_snapshot_across_appends_threads_and_reopen() {
    for left_join in [false, true] {
        check_join_snapshots(false, left_join);
    }
}

#[test]
fn join_snapshots_fit_reported_stack_allowance() {
    for left_join in [false, true] {
        check_join_snapshots(true, left_join);
    }
}

fn check_join_snapshots(small_stack: bool, left_join: bool) {
    let (directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let sql = if left_join {
        "FROM facts AS f |> LEFT JOIN dimensions AS d ON f.k=d.k |> AGGREGATE COUNT(*) AS n"
    } else {
        "FROM facts AS f |> JOIN dimensions AS d ON f.k=d.k |> AGGREGATE COUNT(*) AS n"
    };
    let [old_rows, middle_rows, fresh_rows] = if left_join { [6, 8, 11] } else { [5, 7, 10] };
    let old = db.prepare(sql).unwrap();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(1);
    let timeout = std::time::Duration::from_secs(30);
    let thread = if small_stack {
        std::thread::Builder::new().stack_size(pipesql_filesystem::TEST_SMALL_STACK_REQUEST_BYTES)
    } else {
        std::thread::Builder::new()
    };
    std::thread::scope(|scope| {
        let (reader_db, reader_query, reader_cancel) = (&db, &old, &cancel);
        let worker = thread
            .spawn_scoped(scope, move || {
                if small_stack {
                    pipesql_filesystem::test_assert_small_stack();
                }
                let mut result = reader_db.execute(reader_query, reader_cancel).unwrap();
                assert!(matches!(result.step(), QueryStep::Progress));
                ready_tx.send(()).unwrap();
                resume_rx.recv_timeout(timeout).unwrap();
                assert_eq!(
                    collect_unordered(&mut result),
                    vec![vec![Cell::Integer(old_rows)]]
                );
            })
            .unwrap();
        ready_rx.recv_timeout(timeout).unwrap();
        let mut append = db.begin_append("facts", limits(), &cancel).unwrap();
        append
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&[1]),
                        validity: &[1],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&[99]),
                        validity: &[1],
                    },
                ],
                &cancel,
            )
            .unwrap();
        append.commit(&cancel).unwrap();
        let middle = db.prepare(sql).unwrap();
        let mut append = db.begin_append("dimensions", limits(), &cancel).unwrap();
        append
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&[1]),
                        validity: &[1],
                    },
                    ColumnInput {
                        values: ColumnValues::String(&["late"]),
                        validity: &[1],
                    },
                ],
                &cancel,
            )
            .unwrap();
        append.commit(&cancel).unwrap();
        resume_tx.send(()).unwrap();
        // Scratch bootstrap may refuse competing starts. Complete this reader
        // before starting another join; its live pin crossed both publications.
        worker.join().unwrap();
        assert_eq!(
            collect_unordered(&mut db.execute(&middle, &cancel).unwrap()),
            vec![vec![Cell::Integer(middle_rows)]]
        );
        let fresh = db.prepare(sql).unwrap();
        assert_eq!(
            collect_unordered(&mut db.execute(&fresh, &cancel).unwrap()),
            vec![vec![Cell::Integer(fresh_rows)]]
        );
    });
    drop(old);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    let config = db.config();
    db.close().unwrap();
    let db = Database::open(&directory.database(), config).unwrap();
    let fresh = db.prepare(sql).unwrap();
    assert_eq!(
        collect_unordered(&mut db.execute(&fresh, &cancel).unwrap()),
        vec![vec![Cell::Integer(fresh_rows)]]
    );
    drop(fresh);
    db.close().unwrap();
}

#[test]
fn typed_join_matches_an_independent_row_oracle_through_spill() {
    let directory = Directory::new();
    let cancel = CancellationToken::new();
    let path = directory.database();
    let db = Database::create_empty(&path, Config::new(16_000_000, 16_000_000).unwrap()).unwrap();
    let keys = [
        Some(-0.0),
        Some(0.0),
        Some(f64::NAN),
        Some(f64::from_bits(0x7ff8000000000042)),
        Some(f64::INFINITY),
        Some(f64::NEG_INFINITY),
        Some(1.0),
        None,
    ];
    let days = [0, 0, 1, 1, 2, 3, 3, 0];
    let maximum = "雪".repeat(21_845) + "x";
    assert_eq!(maximum.len(), 65_536);
    let texts = [
        Some(maximum.as_str()),
        Some(maximum.as_str()),
        Some("a\0雪"),
        Some("a\0雪"),
        Some(""),
        Some("tail"),
        None,
        Some("tail"),
    ];
    let names = ["id", "k", "day", "s"];
    let kinds = [
        DataType::Int64,
        DataType::Double,
        DataType::Date,
        DataType::String,
    ];
    let schema = std::array::from_fn::<_, 4, _>(|i| ColumnDeclaration {
        name: names[i],
        data_type: kinds[i],
        nullable: i != 0,
    });
    db.declare_table("typed", &schema, &cancel).unwrap();
    // One row per unit also crosses source-unit and maximum-value boundaries.
    for row in 0..keys.len() {
        let mut writer = db
            .begin_append(
                "typed",
                AppendLimits {
                    batches: 1,
                    encoded_bytes: 100_000,
                },
                &cancel,
            )
            .unwrap();
        writer
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&[row as i64]),
                        validity: &[1],
                    },
                    ColumnInput {
                        values: ColumnValues::Double(&[keys[row].unwrap_or(0.0)]),
                        validity: &[u8::from(keys[row].is_some())],
                    },
                    ColumnInput {
                        values: ColumnValues::Date(&[DateValue::from_days_since_unix_epoch(
                            days[row],
                        )
                        .unwrap()]),
                        validity: &[1],
                    },
                    ColumnInput {
                        values: ColumnValues::String(&[texts[row].unwrap_or("")]),
                        validity: &[u8::from(texts[row].is_some())],
                    },
                ],
                &cancel,
            )
            .unwrap();
        writer.commit(&cancel).unwrap();
    }
    db.close().unwrap();
    for cap in [16_000_000, 4_000_000, 3_000_000] {
        let db = Database::open(&path, Config::new(cap, 16_000_000).unwrap()).unwrap();
        let baseline = db.reserved_memory_bytes();
        for modifier in ["", "LEFT "] {
            for key in ["k", "day", "s"] {
                let sql = format!(
                    "FROM typed AS l |> {modifier}JOIN typed AS r ON r.{key} = l.{key} |> SELECT r.s, l.id, r.id, l.k, r.day, l.s"
                );
                let query = db.prepare(&sql).unwrap();
                if cap == 3_000_000 {
                    assert!(matches!(
                        db.execute(&query, &cancel),
                        Err(Error::Resource { .. })
                    ));
                    drop(query);
                    assert_eq!(db.reserved_memory_bytes(), baseline);
                    assert_eq!(db.reserved_temp_bytes(), 0);
                    continue;
                }
                let mut result = db.execute(&query, &cancel).unwrap();
                let mut expected = vec![];
                for left in 0..keys.len() {
                    let before = expected.len();
                    for right in 0..keys.len() {
                        let equal = match key {
                            "k" => keys[left].zip(keys[right]).is_some_and(|(l, r)| l == r),
                            "day" => days[left] == days[right],
                            "s" => texts[left].zip(texts[right]).is_some_and(|(l, r)| l == r),
                            _ => unreachable!(),
                        };
                        if equal {
                            expected.push(vec![
                                texts[right].map_or(Cell::Null, |v| Cell::Text(v.to_owned())),
                                Cell::Integer(left as i64),
                                Cell::Integer(right as i64),
                                keys[left].map_or(Cell::Null, |v| Cell::Number(v.to_bits())),
                                Cell::Day(days[right]),
                                texts[left].map_or(Cell::Null, |v| Cell::Text(v.to_owned())),
                            ]);
                        }
                    }
                    if modifier == "LEFT " && expected.len() == before {
                        expected.push(vec![
                            Cell::Null,
                            Cell::Integer(left as i64),
                            Cell::Null,
                            keys[left].map_or(Cell::Null, |v| Cell::Number(v.to_bits())),
                            Cell::Null,
                            texts[left].map_or(Cell::Null, |v| Cell::Text(v.to_owned())),
                        ]);
                    }
                }
                expected.sort_unstable();
                let mut actual = vec![];
                let mut done = false;
                let mut peak_temp = 0;
                for _ in 0..20_000 {
                    assert_eq!(
                        db.reserved_memory_bytes(),
                        baseline + query.accounted_memory_bytes() + result.accounted_memory_bytes()
                    );
                    peak_temp = peak_temp.max(db.reserved_temp_bytes());
                    match result.step() {
                        QueryStep::Rows(batch) => {
                            for row in 0..batch.len() {
                                actual.push(
                                    (0..6)
                                        .map(|column| owned_cell(batch.value(row, column).unwrap()))
                                        .collect::<Vec<_>>(),
                                );
                            }
                        }
                        QueryStep::Finished => {
                            done = true;
                            break;
                        }
                        QueryStep::Failed(error) => panic!("{key}, cap {cap}: {error}"),
                        QueryStep::Progress => (),
                    }
                }
                assert!(done && peak_temp > 65_536);
                actual.sort_unstable();
                assert_eq!(actual, expected, "{key}, cap {cap}");
                drop(result);
                drop(query);
                assert_eq!(db.reserved_memory_bytes(), baseline);
                assert_eq!(db.reserved_temp_bytes(), 0);
            }
        }
        db.close().unwrap();
    }
}

#[test]
fn range_aliases_preserve_values_through_projection_filter_and_aggregation() {
    for query in [
        "FROM facts AS f |> WHERE f.v >= 20 |> SELECT f.v",
        "FROM facts AS f |> SELECT f.v AS value |> AS projected \
         |> WHERE projected.value >= 20 |> SELECT projected.value",
    ] {
        assert_query_rows(
            query,
            [20, 30, 40]
                .map(|value| vec![Cell::Integer(value)])
                .to_vec(),
        );
    }
    assert_query_rows(
        "FROM facts AS f |> AGGREGATE SUM(f.v) AS total GROUP BY f.k |> AS grouped \
         |> WHERE grouped.total >= 30 |> SELECT grouped.total",
        [30, 30, 40]
            .map(|value| vec![Cell::Integer(value)])
            .to_vec(),
    );
}

#[test]
fn left_join_retains_null_keys_and_duplicate_matches() {
    for modifier in ["LEFT", "LEFT OUTER"] {
        let mut expected = [(10, "a"), (10, "b"), (20, "a"), (20, "b"), (30, "c")]
            .map(|(v, label)| vec![Cell::Integer(v), Cell::Text(label.to_owned())])
            .to_vec();
        expected.push(vec![Cell::Integer(40), Cell::Null]);
        assert_query_rows(
            &format!(
                "FROM facts AS f |> {modifier} JOIN dimensions AS d ON f.k=d.k |> SELECT f.v, d.label"
            ),
            expected,
        );
    }
    assert_query_rows(
        "FROM facts AS f |> LEFT JOIN (FROM dimensions |> WHERE k=2) AS d ON f.k=d.k |> SELECT f.v, d.label",
        vec![
            vec![Cell::Integer(10), Cell::Null],
            vec![Cell::Integer(20), Cell::Null],
            vec![Cell::Integer(30), Cell::Text("c".to_owned())],
            vec![Cell::Integer(40), Cell::Null],
        ],
    );
}

#[test]
fn left_join_empty_inputs_and_post_join_filters() {
    assert_query_rows(
        "FROM facts AS f |> LEFT JOIN (FROM dimensions |> WHERE k>100) AS d ON f.k=d.k |> SELECT f.v, d.label",
        [10, 20, 30, 40]
            .map(|v| vec![Cell::Integer(v), Cell::Null])
            .to_vec(),
    );
    assert_query_rows(
        "FROM facts AS f |> WHERE v>100 |> LEFT JOIN dimensions AS d ON f.k=d.k",
        vec![],
    );
    assert_query_rows(
        "FROM facts AS f |> LEFT JOIN dimensions AS d ON f.k=d.k |> WHERE d.label IS NULL |> SELECT f.v",
        vec![vec![Cell::Integer(40)]],
    );
    assert_query_rows(
        "FROM facts AS f |> LEFT JOIN dimensions AS d ON f.k=d.k |> WHERE d.label='absent' |> SELECT f.v",
        vec![],
    );
    assert_query_rows(
        "FROM facts AS f |> LEFT JOIN dimensions AS d ON f.k=d.k |> AGGREGATE COUNT(*) AS n, COUNT(d.label) AS matches, SUM(f.v) AS total",
        vec![vec![Cell::Integer(6), Cell::Integer(5), Cell::Integer(130)]],
    );
}

#[test]
fn left_join_nested_producers_preserve_nulls_and_expression_demand() {
    assert_query_rows(
        "FROM facts AS f |> LEFT JOIN dimensions AS d ON f.k=d.k |> LEFT JOIN facts AS g ON d.k=g.k |> AGGREGATE COUNT(*) AS n, COUNT(g.v) AS present, SUM(f.v) AS total",
        vec![vec![
            Cell::Integer(10),
            Cell::Integer(9),
            Cell::Integer(190),
        ]],
    );
    assert_query_rows(
        "FROM facts AS f |> LEFT JOIN (FROM dimensions AS d |> LEFT JOIN facts AS g ON d.k=g.k |> SELECT d.k, g.v) AS r ON f.k=r.k |> AGGREGATE COUNT(*) AS n, SUM(r.v+1) AS total",
        vec![vec![Cell::Integer(10), Cell::Integer(159)]],
    );
    assert_query_rows(
        "FROM facts AS f |> LEFT JOIN (FROM facts |> AGGREGATE SUM(v+9223372036854775807) AS unused GROUP BY k) AS r ON f.k=r.k |> SELECT f.v",
        [10, 20, 30, 40].map(|v| vec![Cell::Integer(v)]).to_vec(),
    );
    assert_query_rows(
        "FROM facts AS f |> WHERE f.k IS NULL |> LEFT JOIN facts AS r ON f.k=r.k |> SELECT r.v/0 AS missing",
        vec![vec![Cell::Null]],
    );

    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let sql = "FROM facts AS f |> LEFT JOIN (FROM facts |> AGGREGATE SUM(v+9223372036854775807) AS demanded GROUP BY k) AS r ON f.k=r.k |> AGGREGATE COUNT(r.demanded) AS n";
    let query = db.prepare(sql).unwrap();
    let mut result = db.execute(&query, &cancel).unwrap();
    let mut failed = false;
    for _ in 0..4096 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Failed(Error::ArithmeticOverflow { span, .. }) => {
                assert_eq!(&sql[span.start()..span.end()], "SUM(v+9223372036854775807)");
                failed = true;
                break;
            }
            _ => panic!("demanded argument must fail before output"),
        }
    }
    assert!(failed);
    assert!(matches!(
        result.step(),
        QueryStep::Failed(Error::ArithmeticOverflow { .. })
    ));
    drop(result);
    drop(query);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    let healthy = db
        .prepare(
            "FROM facts AS f |> LEFT JOIN dimensions AS d ON f.k=d.k |> AGGREGATE COUNT(*) AS n",
        )
        .unwrap();
    assert_eq!(
        collect_unordered(&mut db.execute(&healthy, &cancel).unwrap()),
        vec![vec![Cell::Integer(6)]]
    );
    drop(healthy);
    assert_eq!(db.reserved_memory_bytes(), baseline);
}
