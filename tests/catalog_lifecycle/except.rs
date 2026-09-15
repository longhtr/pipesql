//! Check set difference over complete positional rows.
//!
//! Literal results and a standard-library set model specify which left rows
//! survive. Separate cases check left-side names/nullability, typed NULLs and
//! floating equivalence without production comparison helpers. Appending to the
//! right table must affect fresh queries while prepared queries keep both pinned
//! inputs. Shared demanded-error cases live in `intersect.rs`.

use super::*;

#[test]
fn public_except_compares_complete_positional_rows_and_composes() {
    let (_directory, db) = join_fixture();
    for (sql, expected) in [
        (
            "FROM facts |> SELECT k AS x |> EXCEPT DISTINCT (FROM facts |> WHERE v=10 |> SELECT k AS y) |> ORDER BY x",
            vec![vec![Cell::Null], vec![Cell::Integer(2)]],
        ),
        (
            "FROM facts |> SELECT k AS x |> EXCEPT DISTINCT (FROM facts |> WHERE v=10 |> SELECT k), (FROM facts |> WHERE v=40 |> SELECT k), |> ORDER BY x",
            integers(&[2]),
        ),
        (
            "FROM facts |> SELECT k AS x |> EXCEPT DISTINCT (FROM facts |> SELECT k |> EXCEPT DISTINCT (FROM facts |> WHERE v=10 |> SELECT k)) |> ORDER BY x",
            integers(&[1]),
        ),
        (
            "FROM facts |> SELECT k AS x |> EXCEPT DISTINCT (FROM facts |> WHERE v<0 |> SELECT k) |> ORDER BY x",
            vec![
                vec![Cell::Null],
                vec![Cell::Integer(1)],
                vec![Cell::Integer(2)],
            ],
        ),
        (
            "FROM facts |> EXCEPT DISTINCT (FROM facts) |> AGGREGATE COUNT(*) AS n",
            integers(&[0]),
        ),
        (
            "FROM facts |> EXCEPT DISTINCT (FROM facts |> WHERE v=10) |> SELECT k |> ORDER BY k",
            vec![
                vec![Cell::Null],
                vec![Cell::Integer(1)],
                vec![Cell::Integer(2)],
            ],
        ),
        (
            "FROM facts |> EXCEPT DISTINCT (FROM facts |> WHERE v=10) |> AS f |> LEFT JOIN dimensions AS d ON f.k=d.k |> SELECT f.v |> ORDER BY v",
            integers(&[20, 20, 30, 40]),
        ),
        (
            "FROM facts |> EXCEPT DISTINCT (FROM facts |> WHERE v=10) |> AGGREGATE SUM(v) AS total",
            integers(&[90]),
        ),
    ] {
        query(&db, sql, expected);
    }
    for sql in [
        "FROM facts |> EXCEPT DISTINCT (FROM dimensions)",
        "FROM facts |> EXCEPT DISTINCT (FROM facts |> SELECT k)",
        "FROM facts |> EXCEPT DISTINCT (FROM facts) |> SELECT facts.k",
        "FROM facts |> EXCEPT DISTINCT (FROM dimensions |> SELECT facts.k)",
        "FROM facts |> SELECT k AS x |> EXCEPT DISTINCT (FROM facts |> SELECT k AS y) |> SELECT y",
    ] {
        assert!(matches!(db.prepare(sql), Err(Error::Bind { .. })), "{sql}");
    }
}

#[test]
fn public_except_preserves_left_nullability_and_repeated_positions() {
    let (_directory, db) = join_fixture();
    let sql =
        "FROM facts |> SELECT v AS x |> EXCEPT DISTINCT (FROM facts |> SELECT k) |> ORDER BY x";
    let prepared = db.prepare(sql).unwrap();
    let column = prepared.result_column(0).unwrap();
    assert_eq!(column.name, Some("x"));
    assert_eq!(column.data_type, DataType::Int64);
    assert!(!column.nullable);
    drop(prepared);
    query(&db, sql, integers(&[10, 20, 30, 40]));
    query(
        &db,
        "FROM facts |> SELECT k AS a, k AS b |> EXCEPT DISTINCT (FROM facts |> WHERE v=10 |> SELECT k, v) |> ORDER BY a",
        vec![
            vec![Cell::Null, Cell::Null],
            vec![Cell::Integer(1), Cell::Integer(1)],
            vec![Cell::Integer(2), Cell::Integer(2)],
        ],
    );
    query(
        &db,
        "FROM facts |> SELECT k AS a, k AS b |> EXCEPT DISTINCT (FROM facts |> WHERE v=10 |> SELECT k, k) |> SELECT b |> ORDER BY b",
        vec![vec![Cell::Null], vec![Cell::Integer(2)]],
    );
}

#[test]
fn public_except_preserves_typed_values_and_both_snapshot_inputs() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.database(),
        Config::new(8_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    for name in ["left_rows", "right_rows"] {
        db.declare_table(name, &declarations(), &cancel).unwrap();
    }
    let baseline = db.reserved_memory_bytes();
    let sql = "FROM left_rows |> EXCEPT DISTINCT (FROM right_rows)";
    let empty = db.prepare(sql).unwrap();
    let first = DateValue::from_days_since_unix_epoch(-719_162).unwrap();
    let last = DateValue::from_days_since_unix_epoch(2_932_896).unwrap();
    let epoch = DateValue::from_days_since_unix_epoch(0).unwrap();
    let append = |name, columns: &[ColumnInput<'_>]| {
        let mut writer = db.begin_append(name, limits(), &cancel).unwrap();
        writer.write(columns, &cancel).unwrap();
        writer.commit(&cancel).unwrap();
    };
    append(
        "left_rows",
        &[
            ColumnInput {
                values: ColumnValues::String(&[
                    "zero", "zero", "nan", "nan", "雪", "infinity", "ignored",
                ]),
                validity: &[63],
            },
            ColumnInput {
                values: ColumnValues::Int64(&[
                    i64::MIN,
                    i64::MIN,
                    i64::MAX,
                    i64::MAX,
                    9_007_199_254_740_993,
                    0,
                    0,
                ]),
                validity: &[127],
            },
            ColumnInput {
                values: ColumnValues::Double(&[
                    -0.0,
                    0.0,
                    f64::from_bits(0x7ff8_0000_0000_0042),
                    f64::from_bits(0x7ff8_0000_0000_0099),
                    1.5,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                ]),
                validity: &[127],
            },
            ColumnInput {
                values: ColumnValues::Date(&[first, first, last, last, epoch, last, first]),
                validity: &[127],
            },
        ],
    );
    append(
        "right_rows",
        &[
            ColumnInput {
                values: ColumnValues::String(&["zero", "nan"]),
                validity: &[3],
            },
            ColumnInput {
                values: ColumnValues::Int64(&[i64::MIN, i64::MAX]),
                validity: &[3],
            },
            ColumnInput {
                values: ColumnValues::Double(&[0.0, f64::from_bits(0x7ff8_0000_0000_0077)]),
                validity: &[3],
            },
            ColumnInput {
                values: ColumnValues::Date(&[first, last]),
                validity: &[3],
            },
        ],
    );
    assert!(collect_unordered(&mut db.execute(&empty, &cancel).unwrap()).is_empty());
    drop(empty);
    let prepared = db.prepare(sql).unwrap();
    let mut running = db.execute(&prepared, &cancel).unwrap();
    append(
        "right_rows",
        &[
            ColumnInput {
                values: ColumnValues::String(&["雪"]),
                validity: &[1],
            },
            ColumnInput {
                values: ColumnValues::Int64(&[9_007_199_254_740_993]),
                validity: &[1],
            },
            ColumnInput {
                values: ColumnValues::Double(&[1.5]),
                validity: &[1],
            },
            ColumnInput {
                values: ColumnValues::Date(&[epoch]),
                validity: &[1],
            },
        ],
    );
    let snow = vec![
        Cell::Text("雪".into()),
        Cell::Integer(9_007_199_254_740_993),
        Cell::Number(1.5_f64.to_bits()),
        Cell::Day(0),
    ];
    let mut expected = vec![
        snow.clone(),
        vec![
            Cell::Text("infinity".into()),
            Cell::Integer(0),
            Cell::Number(f64::INFINITY.to_bits()),
            Cell::Day(2_932_896),
        ],
        vec![
            Cell::Null,
            Cell::Integer(0),
            Cell::Number(f64::NEG_INFINITY.to_bits()),
            Cell::Day(-719_162),
        ],
    ];
    expected.sort_unstable();
    assert_eq!(collect_unordered(&mut running), expected);
    drop(running);
    assert_eq!(
        collect_unordered(&mut db.execute(&prepared, &cancel).unwrap()),
        expected
    );
    drop(prepared);
    expected.retain(|row| row != &snow);
    let fresh = db.prepare(sql).unwrap();
    assert_eq!(
        collect_unordered(&mut db.execute(&fresh, &cancel).unwrap()),
        expected
    );
    drop(fresh);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close().unwrap();
}

#[test]
fn public_except_matches_an_independent_complete_row_set_oracle() {
    use std::collections::BTreeSet;
    let (_directory, db) = join_fixture();
    let source = [(Some(1), 10), (Some(1), 20), (Some(2), 30), (None, 40)];
    let cancel = CancellationToken::new();
    let baseline = db.reserved_memory_bytes();
    for complete in [false, true] {
        let projection = if complete {
            "k, DIV(v, 20) AS bucket"
        } else {
            "k"
        };
        // The standard-library set oracle uses materialized integer/NULL rows;
        // it neither sorts engine records nor calls production equality code.
        let rows = |limit| -> BTreeSet<Vec<Cell>> {
            source
                .iter()
                .filter(|(_, value)| *value <= limit)
                .map(|(key, value)| {
                    let mut row = vec![key.map_or(Cell::Null, Cell::Integer)];
                    if complete {
                        row.push(Cell::Integer(value / 20));
                    }
                    row
                })
                .collect()
        };
        for left in [0, 10, 25, 40, 50] {
            for right in [0, 10, 25, 40, 50] {
                let expected: Vec<_> = rows(left).difference(&rows(right)).cloned().collect();
                let sql = format!(
                    "FROM facts |> WHERE v<={left} |> SELECT {projection} |> EXCEPT DISTINCT (FROM facts |> WHERE v<={right} |> SELECT {projection})"
                );
                let prepared = db.prepare(&sql).unwrap();
                assert_eq!(
                    collect_unordered(&mut db.execute(&prepared, &cancel).unwrap()),
                    expected,
                    "{sql}"
                );
                drop(prepared);
                assert_eq!(db.reserved_memory_bytes(), baseline);
                assert_eq!(db.reserved_temp_bytes(), 0);
            }
        }
    }
}

#[test]
fn public_except_compares_typed_nulls_in_every_scalar_position() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.database(),
        Config::new(8_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let schema = declarations().map(|mut column| {
        column.nullable = true;
        column
    });
    let day = DateValue::from_days_since_unix_epoch(-719_162).unwrap();
    for (table, validity) in [("left_rows", 2), ("right_rows", 0)] {
        db.declare_table(table, &schema, &cancel).unwrap();
        let mut writer = db.begin_append(table, limits(), &cancel).unwrap();
        writer
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::String(&["ignored", "雪"]),
                        validity: &[validity],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&[0, i64::MIN]),
                        validity: &[validity],
                    },
                    ColumnInput {
                        values: ColumnValues::Double(&[0.0, f64::from_bits(0x7ff8_0000_0000_0042)]),
                        validity: &[validity],
                    },
                    ColumnInput {
                        values: ColumnValues::Date(&[day, day]),
                        validity: &[validity],
                    },
                ],
                &cancel,
            )
            .unwrap();
        writer.commit(&cancel).unwrap();
    }
    let baseline = db.reserved_memory_bytes();
    let value = vec![
        Cell::Text("雪".into()),
        Cell::Integer(i64::MIN),
        Cell::Number(0x7ff8_0000_0000_0042),
        Cell::Day(-719_162),
    ];
    for (sql, mut expected) in [
        (
            "FROM left_rows |> EXCEPT DISTINCT (FROM right_rows)",
            vec![value.clone()],
        ),
        (
            "FROM right_rows |> EXCEPT DISTINCT (FROM left_rows)",
            vec![],
        ),
        (
            "FROM left_rows |> EXCEPT DISTINCT (FROM right_rows |> WHERE amount IS NOT NULL)",
            vec![vec![Cell::Null; 4], value],
        ),
    ] {
        let prepared = db.prepare(sql).unwrap();
        for position in 0..4 {
            assert!(prepared.result_column(position).unwrap().nullable);
        }
        expected.sort_unstable();
        assert_eq!(
            collect_unordered(&mut db.execute(&prepared, &cancel).unwrap()),
            expected,
            "{sql}"
        );
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    db.close().unwrap();
}
