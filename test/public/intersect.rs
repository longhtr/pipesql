//! Test INTERSECT DISTINCT: keep one copy of each row found in both inputs.
//!
//! Literal answers and an independent standard-library set check complete-row
//! matching. The result uses the left column names. A column can contain NULL only
//! if both input columns allow NULL, because a row must match on both sides.
//!
//! Typed cases check that matching NaNs and signed zeros produces an original
//! left-hand value. Queries prepared before an append must keep their old inputs.
//! This module also tests input errors for EXCEPT and INTERSECT, with both ALL and
//! DISTINCT: evaluating either input can fail before the first output row. Those
//! checks verify the error's SQL span, released resources and subsequent reuse.

use super::*;

#[test]
fn public_intersect_compares_complete_rows_and_composes() {
    let (_directory, db) = join_fixture();
    for (sql, expected) in [
        (
            "FROM facts |> SELECT k AS x |> INTERSECT DISTINCT (FROM facts |> WHERE v=10 |> SELECT k AS y) |> ORDER BY x",
            integers(&[1]),
        ),
        (
            "FROM facts |> SELECT k |> INTERSECT DISTINCT (FROM facts |> SELECT k), (FROM facts |> WHERE v>=20 |> SELECT k), |> ORDER BY k NULLS FIRST",
            vec![
                vec![Cell::Null],
                vec![Cell::Integer(1)],
                vec![Cell::Integer(2)],
            ],
        ),
        (
            "FROM facts |> INTERSECT DISTINCT (FROM facts |> WHERE v=10) |> SELECT k",
            integers(&[1]),
        ),
        (
            "FROM facts |> INTERSECT DISTINCT (FROM facts |> WHERE v<0) |> AGGREGATE COUNT(*) AS n",
            integers(&[0]),
        ),
        (
            "FROM facts |> SELECT k |> INTERSECT DISTINCT (FROM facts |> SELECT k |> EXCEPT DISTINCT (FROM facts |> WHERE v=10 |> SELECT k)) |> ORDER BY k NULLS FIRST",
            vec![vec![Cell::Null], vec![Cell::Integer(2)]],
        ),
        (
            "FROM facts |> INTERSECT DISTINCT (FROM facts |> WHERE v>=20) |> AGGREGATE SUM(v) AS total",
            integers(&[90]),
        ),
    ] {
        query(&db, sql, expected);
    }
    query(
        &db,
        "FROM facts |> INTERSECT DISTINCT (FROM facts |> WHERE v>=20) |> AS f |> LEFT JOIN dimensions AS d ON f.k=d.k |> SELECT f.v |> ORDER BY v",
        integers(&[20, 20, 30, 40]),
    );
    for sql in [
        "FROM facts |> INTERSECT DISTINCT (FROM dimensions)",
        "FROM facts |> INTERSECT DISTINCT (FROM facts |> SELECT k)",
        "FROM facts |> INTERSECT DISTINCT (FROM facts) |> SELECT facts.k",
        "FROM facts |> INTERSECT DISTINCT (FROM dimensions |> SELECT facts.k)",
        "FROM facts |> SELECT k AS x |> INTERSECT DISTINCT (FROM facts |> SELECT k AS y) |> SELECT y",
    ] {
        assert!(matches!(db.prepare(sql), Err(Error::Bind { .. })), "{sql}");
    }
}

#[test]
fn public_intersect_infers_nullability_from_both_inputs() {
    let (_directory, db) = join_fixture();
    for (left, right, nullable) in [
        ("k", "k", true),
        ("k", "v", false),
        ("v", "k", false),
        ("v", "v", false),
    ] {
        let sql = format!(
            "FROM facts |> SELECT {left} AS x |> INTERSECT DISTINCT (FROM facts |> SELECT {right} AS y)"
        );
        let prepared = db.prepare(&sql).unwrap();
        let column = prepared.result_column(0).unwrap();
        assert_eq!(column.name, Some("x"));
        assert_eq!(column.data_type, DataType::Int64);
        assert_eq!(column.nullable, nullable, "{sql}");
        drop(prepared);
    }
    for sql in [
        "FROM facts |> SELECT k AS x |> INTERSECT DISTINCT (FROM facts |> SELECT DIV(v, 10)) |> ORDER BY x",
        "FROM facts |> SELECT DIV(v, 10) AS x |> INTERSECT DISTINCT (FROM facts |> SELECT k) |> ORDER BY x",
    ] {
        query(&db, sql, integers(&[1, 2]));
    }
    query(
        &db,
        "FROM facts |> SELECT k AS a, k AS b |> INTERSECT DISTINCT (FROM facts |> WHERE v=10 |> SELECT k, k) |> SELECT b",
        integers(&[1]),
    );
}

#[test]
fn public_intersect_matches_an_independent_complete_row_set_oracle() {
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
        // Dividing by 20 gives the two k = 1 rows different buckets. Comparing
        // only k would therefore produce the wrong two-column result.
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
                let expected: Vec<_> = rows(left).intersection(&rows(right)).cloned().collect();
                let sql = format!(
                    "FROM facts |> WHERE v<={left} |> SELECT {projection} |> INTERSECT DISTINCT (FROM facts |> WHERE v<={right} |> SELECT {projection})"
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
fn public_intersect_preserves_typed_equality_bits_and_pinned_inputs() {
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
    for name in ["left_rows", "right_rows"] {
        db.declare_table(name, &schema, &cancel).unwrap();
    }
    let baseline = db.reserved_memory_bytes();
    let sql = "FROM left_rows |> INTERSECT DISTINCT (FROM right_rows)";
    let empty = db.prepare(sql).unwrap();
    let first = DateValue::from_days_since_unix_epoch(-719_162).unwrap();
    let last = DateValue::from_days_since_unix_epoch(2_932_896).unwrap();
    let epoch = DateValue::from_days_since_unix_epoch(0).unwrap();
    let nan_a = f64::from_bits(0x7ff8_0000_0000_0042);
    let nan_b = f64::from_bits(0x7ff8_0000_0000_0099);
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
                    "zero", "zero", "nan", "nan", "雪", "inf", "ignored", "neg",
                ]),
                validity: &[0xbf],
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
                    0,
                ]),
                validity: &[0xbf],
            },
            ColumnInput {
                values: ColumnValues::Double(&[
                    -0.0,
                    0.0,
                    nan_a,
                    nan_b,
                    1.5,
                    f64::INFINITY,
                    0.0,
                    f64::NEG_INFINITY,
                ]),
                validity: &[0xbf],
            },
            ColumnInput {
                values: ColumnValues::Date(&[first, first, last, last, epoch, last, epoch, first]),
                validity: &[0xbf],
            },
        ],
    );
    let right_empty = db.prepare(sql).unwrap();
    append(
        "right_rows",
        &[
            ColumnInput {
                values: ColumnValues::String(&["zero", "nan", "inf", "ignored", "neg"]),
                validity: &[0x17],
            },
            ColumnInput {
                values: ColumnValues::Int64(&[i64::MIN, i64::MAX, 0, 0, 0]),
                validity: &[0x17],
            },
            ColumnInput {
                values: ColumnValues::Double(&[
                    0.0,
                    f64::from_bits(0x7ff8_0000_0000_0077),
                    f64::INFINITY,
                    0.0,
                    f64::NEG_INFINITY,
                ]),
                validity: &[0x17],
            },
            ColumnInput {
                values: ColumnValues::Date(&[first, last, last, epoch, first]),
                validity: &[0x17],
            },
        ],
    );
    for prepared in [&empty, &right_empty] {
        assert!(collect_unordered(&mut db.execute(prepared, &cancel).unwrap()).is_empty());
    }
    drop(empty);
    drop(right_empty);
    let prepared = db.prepare(sql).unwrap();
    for position in 0..4 {
        assert!(prepared.result_column(position).unwrap().nullable);
    }
    let mut running = db.execute(&prepared, &cancel).unwrap();
    // The snow row gains a match on the right, visible only to new queries.
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
    // Accept either left NaN, but reject the different NaN stored on the right.
    // Then normalize equivalent values to compare rows without choosing which
    // left occurrence the engine must keep.
    let normalized = |mut rows: Vec<Vec<Cell>>| {
        for row in &mut rows {
            if row[0] == Cell::Text("zero".into()) {
                assert!(
                    [
                        Cell::Number(0.0_f64.to_bits()),
                        Cell::Number((-0.0_f64).to_bits())
                    ]
                    .contains(&row[2])
                );
                row[2] = Cell::Number(0.0_f64.to_bits());
            } else if row[0] == Cell::Text("nan".into()) {
                assert!(
                    [Cell::Number(nan_a.to_bits()), Cell::Number(nan_b.to_bits())]
                        .contains(&row[2])
                );
                row[2] = Cell::Number(nan_a.to_bits());
            }
        }
        rows.sort_unstable();
        rows
    };
    let mut expected = vec![
        vec![Cell::Null; 4],
        vec![
            Cell::Text("zero".into()),
            Cell::Integer(i64::MIN),
            Cell::Number(0.0_f64.to_bits()),
            Cell::Day(-719_162),
        ],
        vec![
            Cell::Text("nan".into()),
            Cell::Integer(i64::MAX),
            Cell::Number(nan_a.to_bits()),
            Cell::Day(2_932_896),
        ],
        vec![
            Cell::Text("inf".into()),
            Cell::Integer(0),
            Cell::Number(f64::INFINITY.to_bits()),
            Cell::Day(2_932_896),
        ],
        vec![
            Cell::Text("neg".into()),
            Cell::Integer(0),
            Cell::Number(f64::NEG_INFINITY.to_bits()),
            Cell::Day(-719_162),
        ],
    ];
    expected.sort_unstable();
    assert_eq!(normalized(collect_unordered(&mut running)), expected);
    drop(running);
    assert_eq!(
        normalized(collect_unordered(
            &mut db.execute(&prepared, &cancel).unwrap()
        )),
        expected
    );
    drop(prepared);
    expected.push(vec![
        Cell::Text("雪".into()),
        Cell::Integer(9_007_199_254_740_993),
        Cell::Number(1.5_f64.to_bits()),
        Cell::Day(0),
    ]);
    expected.sort_unstable();
    let fresh = db.prepare(sql).unwrap();
    assert_eq!(
        normalized(collect_unordered(&mut db.execute(&fresh, &cancel).unwrap())),
        expected
    );
    drop(fresh);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close().unwrap();
}

#[test]
fn public_sorted_set_demands_both_complete_inputs_before_emitting() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    for operator in [
        "EXCEPT DISTINCT",
        "INTERSECT DISTINCT",
        "EXCEPT ALL",
        "INTERSECT ALL",
    ] {
        // Test failure on each side, including a failing right input with an
        // empty left input. LIMIT 1 still needs the complete row comparisons;
        // LIMIT 0 below skips execution and must avoid the error entirely.
        for template in [
            "FROM facts |> SELECT v, v*9223372036854775807 AS unused |> {operator} (FROM facts |> SELECT v, 1 AS unused) |> SELECT v |> LIMIT 1",
            "FROM facts |> SELECT v, 1 AS unused |> {operator} (FROM facts |> SELECT v, v*9223372036854775807 AS unused) |> SELECT v |> LIMIT 1",
            "FROM facts |> WHERE v<0 |> SELECT v, 1 AS unused |> {operator} (FROM facts |> SELECT v, v*9223372036854775807 AS unused) |> SELECT v |> LIMIT 1",
        ] {
            let sql = template.replace("{operator}", operator);
            let cancel = CancellationToken::new();
            let prepared = db.prepare(&sql).unwrap();
            let mut result = db.execute(&prepared, &cancel).unwrap();
            let mut failed = false;
            for _ in 0..2000 {
                match result.step() {
                    QueryStep::Progress => (),
                    QueryStep::Rows(_) | QueryStep::Finished => {
                        panic!("set operation skipped demanded input: {sql}")
                    }
                    QueryStep::Failed(Error::ArithmeticOverflow { span, .. }) => {
                        assert_eq!(&sql[span.start()..span.end()], "v*9223372036854775807");
                        failed = true;
                        break;
                    }
                    QueryStep::Failed(error) => panic!("wrong failure: {error}"),
                }
            }
            assert!(failed, "bounded set failure: {sql}");
            assert!(matches!(
                result.step(),
                QueryStep::Failed(Error::ArithmeticOverflow { .. })
            ));
            drop(result);
            drop(prepared);
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
            query(&db, "FROM facts |> AGGREGATE COUNT(*) AS n", integers(&[4]));
            query(&db, &format!("{sql} |> LIMIT 0"), vec![]);
        }
    }
}
