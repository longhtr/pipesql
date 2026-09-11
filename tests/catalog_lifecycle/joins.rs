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
         |> SELECT f.v,g.v,d.label",
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
         |> AGGREGATE SUM(f.v) AS total,COUNT(*) AS n GROUP BY d.label",
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
#[cfg_attr(
    all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"),
    ignore = "GNU aarch64 has a 128-KiB pthread minimum; this test requires at most 64 KiB"
)]
fn joins_keep_one_snapshot_across_appends_threads_and_reopen() {
    let (directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let sql = "FROM facts AS f |> JOIN dimensions AS d ON f.k = d.k |> AGGREGATE COUNT(*) AS n";
    let old = db.prepare(sql).unwrap();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(1);
    let timeout = std::time::Duration::from_secs(30);
    std::thread::scope(|scope| {
        let (reader_db, reader_query, reader_cancel) = (&db, &old, &cancel);
        let worker = std::thread::Builder::new()
            .stack_size(48 * 1024)
            .spawn_scoped(scope, move || {
                let stack = pipesql_filesystem::test_current_thread_stack_bytes();
                assert!(stack <= 65_536, "reported stack {stack}");
                let mut result = reader_db.execute(reader_query, reader_cancel).unwrap();
                assert!(matches!(result.step(), QueryStep::Progress));
                ready_tx.send(()).unwrap();
                resume_rx.recv_timeout(timeout).unwrap();
                assert_eq!(collect(&mut result), vec![vec![Cell::Integer(5)]]);
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
            collect(&mut db.execute(&middle, &cancel).unwrap()),
            vec![vec![Cell::Integer(7)]]
        );
        let fresh = db.prepare(sql).unwrap();
        assert_eq!(
            collect(&mut db.execute(&fresh, &cancel).unwrap()),
            vec![vec![Cell::Integer(10)]]
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
        collect(&mut db.execute(&fresh, &cancel).unwrap()),
        vec![vec![Cell::Integer(10)]]
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
        for key in ["k", "day", "s"] {
            let sql = format!(
                "FROM typed AS l |> JOIN typed AS r ON r.{key} = l.{key} |> SELECT r.s,l.id,r.id,l.k,r.day,l.s"
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
