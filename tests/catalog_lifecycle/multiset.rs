use super::order::{integers, query};
use super::*;
use std::collections::BTreeMap;

#[test]
fn public_multiset_preserves_typed_classes_original_bits_and_snapshots() {
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
    let first = DateValue::from_days_since_unix_epoch(-719_162).unwrap();
    let last = DateValue::from_days_since_unix_epoch(2_932_896).unwrap();
    let nan_a = f64::from_bits(0x7ff8_0000_0000_0042);
    let nan_b = f64::from_bits(0x7ff8_0000_0000_0099);
    let nan_right = f64::from_bits(0x7ff8_0000_0000_0077);
    let append = |name: &str, number: &[f64]| {
        let mut writer = db.begin_append(name, limits(), &cancel).unwrap();
        writer
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::String(&[
                            "zero", "nan", "雪", "inf", "neg", "ignored",
                        ]),
                        validity: &[0x1f],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&[
                            i64::MIN,
                            i64::MAX,
                            9_007_199_254_740_993,
                            0,
                            0,
                            0,
                        ]),
                        validity: &[0x1f],
                    },
                    ColumnInput {
                        values: ColumnValues::Double(number),
                        validity: &[0x1f],
                    },
                    ColumnInput {
                        values: ColumnValues::Date(&[first, last, first, last, first, first]),
                        validity: &[0x1f],
                    },
                ],
                &cancel,
            )
            .unwrap();
        writer.commit(&cancel).unwrap();
    };
    append(
        "left_rows",
        &[-0.0, nan_a, 1.5, f64::INFINITY, f64::NEG_INFINITY, 0.0],
    );
    append(
        "left_rows",
        &[0.0, nan_b, 1.5, f64::INFINITY, f64::NEG_INFINITY, 0.0],
    );
    let right = [0.0, nan_right, 1.5, f64::INFINITY, f64::NEG_INFINITY, 0.0];
    append("right_rows", &right);
    let baseline = db.reserved_memory_bytes();
    let except = db
        .prepare("FROM left_rows |> EXCEPT ALL (FROM right_rows)")
        .unwrap();
    let intersect = db
        .prepare("FROM left_rows |> INTERSECT ALL (FROM right_rows)")
        .unwrap();
    let mut running = db.execute(&intersect, &cancel).unwrap();
    // The prepared inputs retain one right occurrence even after two appends.
    append("right_rows", &right);
    append("right_rows", &right);
    let normalized = |mut rows: Vec<Vec<Cell>>| {
        for row in &mut rows {
            if row[0] == Cell::Text("zero".into()) {
                assert!(
                    [
                        Cell::Number((-0.0_f64).to_bits()),
                        Cell::Number(0.0_f64.to_bits())
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
            Cell::Text("雪".into()),
            Cell::Integer(9_007_199_254_740_993),
            Cell::Number(1.5_f64.to_bits()),
            Cell::Day(-719_162),
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
    assert_eq!(normalized(collect(&mut running)), expected);
    drop(running);
    for prepared in [&except, &intersect] {
        assert_eq!(
            normalized(collect(&mut db.execute(prepared, &cancel).unwrap())),
            expected
        );
    }
    drop(except);
    drop(intersect);
    query(
        &db,
        "FROM left_rows |> EXCEPT ALL (FROM right_rows)",
        vec![],
    );
    let fresh = db
        .prepare("FROM left_rows |> INTERSECT ALL (FROM right_rows)")
        .unwrap();
    let doubled: Vec<_> = expected
        .into_iter()
        .flat_map(|row| [row.clone(), row])
        .collect();
    let fresh_rows = collect(&mut db.execute(&fresh, &cancel).unwrap());
    for (label, mut bits) in [
        ("zero", vec![(-0.0_f64).to_bits(), 0.0_f64.to_bits()]),
        ("nan", vec![nan_a.to_bits(), nan_b.to_bits()]),
    ] {
        let mut observed: Vec<_> = fresh_rows
            .iter()
            .filter(|row| row[0] == Cell::Text(label.into()))
            .map(|row| match row[2] {
                Cell::Number(bits) => bits,
                _ => panic!("DOUBLE representative"),
            })
            .collect();
        observed.sort_unstable();
        bits.sort_unstable();
        assert_eq!(observed, bits, "each left occurrence keeps its own bits");
    }
    assert_eq!(normalized(fresh_rows), doubled);
    drop(fresh);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
}

#[test]
fn public_multiset_matches_independent_complete_row_counts() {
    let (_directory, db) = join_fixture();
    let source = [(Some(1), 10), (Some(1), 20), (Some(2), 30), (None, 40)];
    let baseline = db.reserved_memory_bytes();
    for complete in [false, true] {
        let projection = if complete {
            "k, DIV(v, 30) AS bucket"
        } else {
            "k"
        };
        // Count literal source rows without using the engine's ordering,
        // serialized records, comparison functions, or merge decisions.
        let counts = |limit| {
            let mut counts = BTreeMap::<Vec<Cell>, usize>::new();
            for (key, value) in source {
                if value <= limit {
                    let mut row = vec![key.map_or(Cell::Null, Cell::Integer)];
                    if complete {
                        row.push(Cell::Integer(value / 30));
                    }
                    *counts.entry(row).or_default() += 1;
                }
            }
            counts
        };
        for left in [0, 10, 25, 40, 50] {
            for right in [0, 10, 25, 40, 50] {
                for operator in ["EXCEPT ALL", "INTERSECT ALL"] {
                    let right_counts = counts(right);
                    let mut expected = vec![];
                    for (row, left_count) in counts(left) {
                        let right_count = right_counts.get(&row).copied().unwrap_or(0);
                        let copies = if operator == "EXCEPT ALL" {
                            left_count.saturating_sub(right_count)
                        } else {
                            left_count.min(right_count)
                        };
                        expected.extend(std::iter::repeat_n(row, copies));
                    }
                    let sql = format!(
                        "FROM facts |> WHERE v<={left} |> SELECT {projection} |> {operator} (FROM facts |> WHERE v<={right} |> SELECT {projection})"
                    );
                    query(&db, &sql, expected);
                    assert_eq!(db.reserved_memory_bytes(), baseline);
                    assert_eq!(db.reserved_temp_bytes(), 0);
                }
            }
        }
    }
}

#[test]
fn public_multiset_composes_and_matches_arguments_left_to_right() {
    let (_directory, db) = join_fixture();
    for (sql, expected) in [
        (
            "FROM facts |> SELECT k AS x |> EXCEPT ALL (FROM facts |> WHERE v=10 |> SELECT k AS y) |> ORDER BY x NULLS FIRST",
            vec![
                vec![Cell::Null],
                vec![Cell::Integer(1)],
                vec![Cell::Integer(2)],
            ],
        ),
        (
            "FROM facts |> SELECT k |> EXCEPT ALL (FROM facts |> WHERE v=10 |> SELECT k), (FROM facts |> WHERE v=20 |> SELECT k), |> ORDER BY k NULLS FIRST",
            vec![vec![Cell::Null], vec![Cell::Integer(2)]],
        ),
        (
            "FROM facts |> SELECT k |> INTERSECT ALL (FROM facts |> SELECT k), (FROM facts |> WHERE v<=20 |> SELECT k), |> ORDER BY k",
            integers(&[1, 1]),
        ),
        (
            "FROM facts |> SELECT k |> EXCEPT ALL (FROM facts |> SELECT k |> EXCEPT ALL (FROM facts |> WHERE v=10 |> SELECT k)) |> ORDER BY k",
            integers(&[1]),
        ),
        (
            "FROM facts |> SELECT k |> UNION ALL (FROM facts |> SELECT k) |> INTERSECT ALL (FROM facts |> SELECT k) |> AGGREGATE COUNT(*) AS n",
            integers(&[4]),
        ),
        (
            "FROM facts |> SELECT k |> INTERSECT ALL (FROM facts |> SELECT k) |> AS f |> LEFT JOIN dimensions AS d ON f.k=d.k |> SELECT f.k |> ORDER BY k NULLS FIRST",
            vec![
                vec![Cell::Null],
                vec![Cell::Integer(1)],
                vec![Cell::Integer(1)],
                vec![Cell::Integer(1)],
                vec![Cell::Integer(1)],
                vec![Cell::Integer(2)],
            ],
        ),
    ] {
        query(&db, sql, expected);
    }
    for operator in ["EXCEPT ALL", "INTERSECT ALL"] {
        for (left, right) in [("k", "k"), ("k", "v"), ("v", "k"), ("v", "v")] {
            let sql = format!(
                "FROM facts |> SELECT {left} AS x |> {operator} (FROM facts |> SELECT {right} AS y)"
            );
            let prepared = db.prepare(&sql).unwrap();
            let column = prepared.result_column(0).unwrap();
            assert_eq!(column.name, Some("x"));
            assert_eq!(column.data_type, DataType::Int64);
            let nullable = left == "k" && (operator == "EXCEPT ALL" || right == "k");
            assert_eq!(column.nullable, nullable, "{sql}");
        }
        for template in [
            "FROM facts |> {operator} (FROM dimensions)",
            "FROM facts |> {operator} (FROM facts |> SELECT k)",
            "FROM facts |> {operator} (FROM facts) |> SELECT facts.k",
            "FROM facts |> {operator} (FROM dimensions |> SELECT facts.k)",
            "FROM facts |> SELECT k AS x |> {operator} (FROM facts |> SELECT k AS y) |> SELECT y",
        ] {
            let sql = template.replace("{operator}", operator);
            assert!(matches!(db.prepare(&sql), Err(Error::Bind { .. })), "{sql}");
        }
    }
}
