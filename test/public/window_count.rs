//! Test analytic COUNT: attach an input or partition count to every input row.
//!
//! Unlike an aggregate, this window expression keeps the rows it counts. Empty
//! input therefore produces no rows. LIMIT before the expression changes the
//! count; LIMIT afterward only changes how many counted rows reach the caller.
//! Literal answers check this ordering through filters, joins and grouped input.
//!
//! Returning only the count lets execution discard input values. Returning those
//! values too requires storage until counting finishes. A one-byte temporary
//! budget separates these paths; a larger fixture checks stored text, NULLs and
//! resource release. Other cases check when overflow must be evaluated and which
//! rows remain visible to queries prepared before an append and reclamation.
//! Keyed cases compare literal partition counts, including NULL, DATE, binary
//! text equality and prior-stage computed keys.

use super::*;

#[test]
fn partition_counts_keep_rows_and_group_null_keys() {
    let (_directory, db) = join_fixture();
    query(
        &db,
        "FROM facts |> EXTEND COUNT(*) OVER () AS total |> EXTEND COUNT(*) OVER (PARTITION BY k) AS n |> SELECT v, total, n |> ORDER BY v",
        [(10, 2), (20, 2), (30, 1), (40, 1)]
            .map(|(v, n)| vec![Cell::Integer(v), Cell::Integer(4), Cell::Integer(n)])
            .to_vec(),
    );
    query(
        &db,
        "FROM facts |> SELECT k, COUNT(*) OVER (PARTITION BY k) AS n |> ORDER BY k NULLS FIRST",
        vec![
            vec![Cell::Null, Cell::Integer(1)],
            vec![Cell::Integer(1), Cell::Integer(2)],
            vec![Cell::Integer(1), Cell::Integer(2)],
            vec![Cell::Integer(2), Cell::Integer(1)],
        ],
    );
    query(
        &db,
        "FROM facts |> SELECT COUNT(*) OVER (PARTITION BY k) AS n |> ORDER BY n",
        integers(&[1, 1, 2, 2]),
    );
    query(
        &db,
        "FROM facts AS f |> SELECT f.v+1 AS next, COUNT(*) OVER (PARTITION BY f.k) AS n, COUNT(*) OVER (PARTITION BY f.k) AS m |> ORDER BY next",
        [(11, 2), (21, 2), (31, 1), (41, 1)]
            .map(|(v, n)| vec![Cell::Integer(v), Cell::Integer(n), Cell::Integer(n)])
            .to_vec(),
    );
    query(
        &db,
        "FROM facts |> SELECT 9223372036854775807+v AS bad, v |> EXTEND COUNT(*) OVER (PARTITION BY bad) AS unused |> SELECT v |> ORDER BY v",
        integers(&[10, 20, 30, 40]),
    );
    query(
        &db,
        "FROM facts |> WHERE v<0 |> SELECT COUNT(*) OVER (PARTITION BY k) AS n",
        vec![],
    );
    query(
        &db,
        "FROM facts |> SELECT k, CASE WHEN v<30 THEN 0 ELSE 1 END AS category |> SELECT COUNT(*) OVER (PARTITION BY category, k) AS n |> ORDER BY n",
        integers(&[1, 1, 2, 2]),
    );
}

#[test]
fn partition_keys_use_date_equality_and_binary_text_equality() {
    let (_directory, db) = nullable_facts().unwrap();
    query(
        &db,
        "FROM facts |> SELECT id, COUNT(*) OVER (PARTITION BY d) AS n |> ORDER BY id",
        [3, 3, 3, 1]
            .into_iter()
            .enumerate()
            .map(|(id, n)| vec![Cell::Integer(id as i64), Cell::Integer(n)])
            .collect(),
    );
    let (_directory, db) = text_fixture();
    // Union copies every key, including NULL, empty text and the maximum-length
    // string. Composed and decomposed accents remain distinct binary keys.
    query(
        &db,
        "FROM texts |> UNION ALL (FROM texts) |> SELECT id, COUNT(*) OVER (PARTITION BY category, category) AS n |> ORDER BY id",
        (0..10)
            .flat_map(|id| std::iter::repeat_n(vec![Cell::Integer(id), Cell::Integer(2)], 2))
            .collect(),
    );
}

#[test]
fn count_without_input_values_runs_with_no_temporary_space() {
    let (directory, db) = join_fixture();
    db.close().unwrap();
    let db = Database::open(&directory.database(), Config::new(8_000_000, 1).unwrap()).unwrap();
    for (sql, expected) in [
        ("FROM facts |> SELECT COUNT(*) OVER () AS n", 4),
        (
            "FROM facts |> SELECT 9223372036854775807+1 AS unused, COUNT(*) OVER () AS n |> SELECT n",
            4,
        ),
        (
            "FROM facts |> SELECT v |> UNION ALL (FROM facts |> SELECT v) |> SELECT COUNT(*) OVER () AS n |> WHERE n=8 |> LIMIT 4",
            8,
        ),
    ] {
        query(&db, sql, integers(&[expected; 4]));
    }
    query(
        &db,
        "FROM facts |> SELECT COUNT(*) OVER () AS n |> WHERE n<0",
        vec![],
    );
    query(
        &db,
        "FROM facts |> SELECT 9223372036854775807+1 AS bad, COUNT(*) OVER () AS n |> LIMIT 0",
        vec![],
    );
    query(
        &db,
        "FROM facts |> SELECT COUNT(*) OVER () AS n, 'x' AS s, DATE '1970-01-01' AS d |> LIMIT 1",
        vec![vec![Cell::Integer(4), Cell::Text("x".into()), Cell::Day(0)]],
    );
    for (sql, arithmetic) in [
        (
            "FROM facts |> SELECT 9223372036854775807+1 AS bad, COUNT(*) OVER () AS n |> LIMIT 1",
            true,
        ),
        ("FROM facts |> SELECT v, COUNT(*) OVER () AS n", false),
        (
            "FROM facts |> SELECT COUNT(*) OVER (PARTITION BY k) AS n",
            false,
        ),
    ] {
        let baseline = db.reserved_memory_bytes();
        let prepared = db.prepare(sql).unwrap();
        let cancel = CancellationToken::new();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        let mut failed = false;
        for _ in 0..1000 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Failed(error) => {
                    if arithmetic {
                        assert!(matches!(error, Error::ArithmeticOverflow { .. }));
                    } else {
                        assert!(matches!(error, Error::Resource { limit: 1, .. }));
                    }
                    failed = true;
                    break;
                }
                _ => panic!("demanded expression or retained values must fail before rows"),
            }
        }
        assert!(failed);
        drop(result);
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }

    // Release a count-only result before output and after its first batch,
    // either by dropping it directly or by requesting cancellation first.
    for (after_row, cancelled) in [(false, false), (false, true), (true, false), (true, true)] {
        let baseline = db.reserved_memory_bytes();
        let prepared = db
            .prepare("FROM facts |> SELECT COUNT(*) OVER () AS n")
            .unwrap();
        let cancel = CancellationToken::new();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        if after_row {
            let mut seen = false;
            for _ in 0..1000 {
                match result.step() {
                    QueryStep::Progress => (),
                    QueryStep::Rows(batch) => {
                        assert_eq!(batch.value(0, 0), Some(Value::Int64(4)));
                        seen = true;
                        break;
                    }
                    _ => panic!("counter must publish its first row"),
                }
            }
            assert!(seen);
        }
        if cancelled {
            cancel.cancel();
            assert!(matches!(result.step(), QueryStep::Failed(Error::Cancelled)));
        }
        drop(result);
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }

    query(
        &db,
        "FROM facts |> SELECT COUNT(*) OVER () AS n",
        integers(&[4; 4]),
    );
}

#[test]
fn full_partition_count_preserves_rows_and_composes_with_producers() {
    let (_directory, db) = join_fixture();
    query(
        &db,
        "FROM facts |> SELECT COUNT(*) OVER () AS n",
        integers(&[4, 4, 4, 4]),
    );
    query(
        &db,
        "FROM facts |> WHERE v<0 |> SELECT COUNT(*) OVER () AS n",
        vec![],
    );
    query(
        &db,
        "FROM facts |> SELECT v+1 AS next, COUNT(*) OVER () AS n, COUNT(*) OVER () AS other |> ORDER BY next",
        [11, 21, 31, 41]
            .map(|v| vec![Cell::Integer(v), Cell::Integer(4), Cell::Integer(4)])
            .to_vec(),
    );
    query(
        &db,
        "FROM facts AS f |> EXTEND COUNT(*) OVER () AS n |> WHERE n=4 |> SELECT f.v, n+1 AS next |> ORDER BY v",
        [10, 20, 30, 40]
            .map(|v| vec![Cell::Integer(v), Cell::Integer(5)])
            .to_vec(),
    );
    query(
        &db,
        "FROM facts |> LIMIT 2 |> SELECT COUNT(*) OVER () AS n",
        integers(&[2, 2]),
    );
    query(
        &db,
        "FROM facts |> SELECT COUNT(*) OVER () AS n |> LIMIT 2",
        integers(&[4, 4]),
    );
    query(
        &db,
        "FROM facts |> SELECT k |> DISTINCT |> SELECT COUNT(*) OVER () AS n",
        integers(&[3, 3, 3]),
    );
    query(
        &db,
        "FROM facts |> AGGREGATE COUNT(*) AS n GROUP BY k |> SELECT COUNT(*) OVER () AS partition_rows |> AGGREGATE SUM(partition_rows) AS total",
        integers(&[9]),
    );
    query(
        &db,
        "FROM facts |> AGGREGATE SUM(9223372036854775807+v) AS unused GROUP BY k |> SELECT COUNT(*) OVER () AS n",
        integers(&[3, 3, 3]),
    );
    query(
        &db,
        "FROM facts |> SELECT v |> UNION ALL (FROM facts |> SELECT v) |> SELECT COUNT(*) OVER () AS n |> LIMIT 1",
        integers(&[8]),
    );
    query(
        &db,
        "FROM facts AS f |> JOIN dimensions AS d ON f.k=d.k |> SELECT COUNT(*) OVER () AS n |> LIMIT 1",
        integers(&[5]),
    );
    query(
        &db,
        "FROM facts |> EXTEND COUNT(*) OVER () AS n |> WHERE v<30 |> SELECT COUNT(*) OVER () AS m, n |> ORDER BY n",
        vec![vec![Cell::Integer(2), Cell::Integer(4)]; 2],
    );
}

#[test]
fn analytic_emission_preserves_demanded_errors_and_original_scope() {
    let (_directory, db) = join_fixture();
    // Counting all four rows does not require computing every projected value.
    // The first value fits INT64; requesting the second makes addition overflow.
    // A filter using that expression must evaluate it during counting instead.
    query(
        &db,
        "FROM facts |> SELECT 9223372036854775797+v AS overflow, COUNT(*) OVER () AS n |> LIMIT 1",
        vec![vec![Cell::Integer(i64::MAX), Cell::Integer(4)]],
    );
    query(
        &db,
        "FROM facts |> SELECT 9223372036854775797+v AS overflow, COUNT(*) OVER (PARTITION BY v) AS n |> LIMIT 1",
        vec![vec![Cell::Integer(i64::MAX), Cell::Integer(1)]],
    );
    query(
        &db,
        "FROM facts |> SELECT 9223372036854775797+v AS overflow, COUNT(*) OVER () AS n |> SELECT n",
        integers(&[4, 4, 4, 4]),
    );
    for sql in [
        "FROM facts |> SELECT 9223372036854775797+v AS overflow, COUNT(*) OVER () AS n |> LIMIT 2",
        "FROM facts |> EXTEND 9223372036854775797+v AS overflow |> WHERE overflow>0 |> SELECT COUNT(*) OVER () AS n |> LIMIT 1",
        "FROM facts |> SELECT 9223372036854775797+v AS overflow, COUNT(*) OVER (PARTITION BY v) AS n |> LIMIT 2",
        "FROM facts |> EXTEND 9223372036854775797+v AS overflow |> SELECT COUNT(*) OVER (PARTITION BY overflow) AS n |> LIMIT 1",
    ] {
        let prepared = db.prepare(sql).unwrap();
        let cancel = CancellationToken::new();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        let mut failed = false;
        for _ in 0..100_000 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Rows(_) => assert!(
                    !sql.contains("PARTITION BY overflow"),
                    "partition keys must be evaluated before output"
                ),
                QueryStep::Failed(Error::ArithmeticOverflow { span, .. }) => {
                    assert_eq!(&sql[span.start()..span.end()], "9223372036854775797+v");
                    failed = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("{sql}: {error}"),
                QueryStep::Finished => break,
            }
        }
        assert!(failed, "{sql}");
    }
    for sql in [
        "FROM facts |> SELECT COUNT(*) OVER () AS n, n+1 AS next",
        "FROM facts |> EXTEND COUNT(*) OVER () AS n, n+1 AS next",
        "FROM facts |> SET v=COUNT(*) OVER ()",
        "FROM facts |> SELECT COUNT(v) OVER ()",
        "FROM facts |> SELECT COUNT(*) OVER (ORDER BY v)",
        "FROM facts |> SELECT COUNT(*) OVER ()+1",
    ] {
        assert!(db.prepare(sql).is_err(), "{sql}");
    }
}

#[test]
fn analytic_snapshot_retains_typed_rows_across_append_and_reclamation() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.database(),
        Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    db.declare_table("facts", &declarations(), &cancel).unwrap();
    let sql = "FROM facts |> SELECT note, amount, number, day, COUNT(*) OVER () AS n";
    let empty = db.prepare(sql).unwrap();
    let note = ["雪", "", "ignored"];
    let amount = [i64::MIN, i64::MAX, 0];
    let numbers = [-0.0, f64::from_bits(0x7ff8_0000_0000_0042), f64::INFINITY];
    let days = [-719162, 0, 2932896].map(|n| DateValue::from_days_since_unix_epoch(n).unwrap());
    let mut expected = Vec::new();
    for row in 0..3 {
        expected.push(vec![
            if row == 2 {
                Cell::Null
            } else {
                Cell::Text(note[row].into())
            },
            Cell::Integer(amount[row]),
            Cell::Number(numbers[row].to_bits()),
            Cell::Day(days[row].days_since_unix_epoch()),
            Cell::Integer(3),
        ]);
    }
    expected.sort_unstable();
    let mut old = None;
    let mut old_count = None;
    for append_index in 0..2 {
        let mut append = db.begin_append("facts", limits(), &cancel).unwrap();
        append
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::String(&note),
                        validity: &[3],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&amount),
                        validity: &[7],
                    },
                    ColumnInput {
                        values: ColumnValues::Double(&numbers),
                        validity: &[7],
                    },
                    ColumnInput {
                        values: ColumnValues::Date(&days),
                        validity: &[7],
                    },
                ],
                &cancel,
            )
            .unwrap();
        append.commit(&cancel).unwrap();
        if append_index == 0 {
            old = Some(db.prepare(sql).unwrap());
            old_count = Some(
                db.prepare("FROM facts |> SELECT COUNT(*) OVER () AS n")
                    .unwrap(),
            );
        }
    }
    // Reclaim with the zero-row and three-row queries still alive. Fresh queries
    // below must see six rows; old counts and typed rows must retain their view.
    db.reclaim(&cancel).unwrap();
    assert!(collect_unordered(&mut db.execute(&empty, &cancel).unwrap()).is_empty());
    let old = old.unwrap();
    let old_count = old_count.unwrap();
    assert_eq!(
        collect_unordered(&mut db.execute(&old_count, &cancel).unwrap()),
        integers(&[3; 3])
    );
    let current_count = db
        .prepare("FROM facts |> SELECT COUNT(*) OVER () AS n")
        .unwrap();
    assert_eq!(
        collect_unordered(&mut db.execute(&current_count, &cancel).unwrap()),
        integers(&[6; 6])
    );
    assert_eq!(
        collect_unordered(&mut db.execute(&old, &cancel).unwrap()),
        expected
    );
    let current = db.prepare(sql).unwrap();
    for row in &mut expected {
        row[4] = Cell::Integer(6);
    }
    expected.extend(expected.clone());
    expected.sort_unstable();
    assert_eq!(
        collect_unordered(&mut db.execute(&current, &cancel).unwrap()),
        expected
    );
    drop(old);
    drop(old_count);
    drop(empty);
    db.reclaim(&cancel).unwrap();
    assert_eq!(
        collect_unordered(&mut db.execute(&current, &cancel).unwrap()),
        expected
    );
    assert_eq!(db.reserved_temp_bytes(), 0);
}

#[test]
fn analytic_spill_preserves_nullable_text_and_all_row_values() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.database(),
        Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "facts",
        &[
            ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "note",
                data_type: DataType::String,
                nullable: true,
            },
        ],
        &cancel,
    )
    .unwrap();
    let ids: Vec<_> = (0..600).collect();
    let text: Vec<_> = ids
        .iter()
        .map(|id| format!("row-{id}:雪{}\0", "x".repeat(100)))
        .collect();
    let borrowed: Vec<_> = text.iter().map(String::as_str).collect();
    let mut valid = [0; 75];
    for id in 0..600 {
        if id % 3 != 0 {
            valid[id / 8] |= 1 << (id % 8);
        }
    }
    let mut append = db.begin_append("facts", limits(), &cancel).unwrap();
    append
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&ids),
                    validity: &[255; 75],
                },
                ColumnInput {
                    values: ColumnValues::String(&borrowed),
                    validity: &valid,
                },
            ],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    let prepared = db
        .prepare("FROM facts |> EXTEND COUNT(*) OVER () AS n |> SELECT id, note, n")
        .unwrap();
    let baseline = db.reserved_memory_bytes();
    let mut result = db.execute(&prepared, &cancel).unwrap();
    let mut rows = Vec::new();
    let mut done = false;
    let mut disk = false;
    for _ in 0..100_000 {
        disk |= db.reserved_temp_bytes() > 0;
        assert_eq!(
            db.reserved_memory_bytes(),
            baseline + result.accounted_memory_bytes()
        );
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    rows.push(
                        (0..3)
                            .map(|column| owned_cell(batch.value(row, column).unwrap()))
                            .collect::<Vec<_>>(),
                    );
                }
            }
            QueryStep::Finished => {
                done = true;
                break;
            }
            QueryStep::Failed(error) => panic!("{error}"),
        }
    }
    assert!(done && disk);
    rows.sort_unstable();
    let expected: Vec<_> = ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            vec![
                Cell::Integer(*id),
                if id % 3 == 0 {
                    Cell::Null
                } else {
                    Cell::Text(text[index].clone())
                },
                Cell::Integer(600),
            ]
        })
        .collect();
    assert_eq!(rows, expected);
    drop(result);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
}
