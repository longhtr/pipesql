//! Contract-authored row model; no engine binding, equality or grouping helpers.
use super::*;
use std::collections::BTreeMap;

type Row = (Option<i64>, Option<i64>);

fn cell(value: Option<i64>) -> Cell {
    value.map_or(Cell::Null, Cell::Integer)
}

fn sum(values: impl Iterator<Item = Option<i64>>) -> Cell {
    // These small fixtures cannot overflow i64, including their joined sums.
    let values: Vec<_> = values.flatten().collect();
    if values.is_empty() {
        Cell::Null
    } else {
        Cell::Integer(values.into_iter().sum())
    }
}

fn pairs<'a>(left: &'a [Row], right: &'a [Row]) -> Vec<(&'a Row, &'a Row)> {
    let mut result = Vec::new();
    for l in left {
        for r in right {
            if let (Some(a), Some(b)) = (l.0, r.0)
                && a == b
            {
                result.push((l, r));
            }
        }
    }
    result
}

fn append(db: &Database, name: &str, rows: &[Row]) {
    let cancel = CancellationToken::new();
    db.declare_table(
        name,
        &["k", "v"].map(|name| ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: true,
        }),
        &cancel,
    )
    .unwrap();
    if rows.is_empty() {
        return;
    }
    assert!(rows.len() <= 8);
    let columns = [0, 1].map(|column| {
        rows.iter()
            .map(|row| if column == 0 { row.0 } else { row.1 })
            .collect::<Vec<_>>()
    });
    let values = columns.each_ref().map(|column| {
        column
            .iter()
            .map(|value| value.unwrap_or(0))
            .collect::<Vec<_>>()
    });
    let valid = columns.each_ref().map(|column| {
        [column.iter().enumerate().fold(0_u8, |bits, (i, value)| {
            bits | (u8::from(value.is_some()) << i)
        })]
    });
    let mut writer = db.begin_append(name, limits(), &cancel).unwrap();
    writer
        .write(
            &[0, 1].map(|i| ColumnInput {
                values: ColumnValues::Int64(&values[i]),
                validity: &valid[i],
            }),
            &cancel,
        )
        .unwrap();
    writer.commit(&cancel).unwrap();
}

fn check(db: &Database, sql: &str, mut expected: Vec<Vec<Cell>>, context: &str) {
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let query = db
        .prepare(sql)
        .unwrap_or_else(|error| panic!("{context}: {sql}: {error}"));
    let mut result = db.execute(&query, &cancel).unwrap();
    let actual = collect(&mut result);
    expected.sort_unstable();
    assert_eq!(actual, expected, "{context}: {sql}");
    drop(result);
    drop(query);
    assert_eq!(db.reserved_memory_bytes(), baseline, "{context}: {sql}");
    assert_eq!(db.reserved_temp_bytes(), 0, "{context}: {sql}");
}

#[test]
fn generated_join_compositions_match_independent_multisets() {
    let mut cases = 0;
    let mut observed_empty = false;
    let mut observed_full_product = false;
    let mut observed_nonempty_null_sum = false;
    // Cross these dimensions: key distribution does not determine either NULL
    // pattern, payload values, source direction or query transformation.
    for keys in 0..3 {
        for null_keys in 0..3 {
            for null_values in 0..3 {
                let make_rows = |side: usize| -> Vec<Row> {
                    (0..8)
                        .map(|i| {
                            let key = match keys {
                                0 => i as i64 + 20 * side as i64, // Disjoint.
                                1 => ((i * (side + 1)) % 5) as i64,
                                2 => 0, // Full duplicate cross product.
                                _ => unreachable!(),
                            };
                            let key = match null_keys {
                                0 => Some(key),
                                1 if !(i + side).is_multiple_of(3) => Some(key),
                                _ => None,
                            };
                            // Include integers that a DOUBLE identity would round.
                            let value = [9_007_199_254_740_993, -3, 0, 7][(i + side) % 4];
                            let value = if null_values == 0
                                || (null_values == 1 && (i + 2 * side).is_multiple_of(3))
                            {
                                Some(value)
                            } else {
                                None
                            };
                            (key, value)
                        })
                        .collect()
                };
                let rows = [make_rows(0), make_rows(1)];
                let directory = Directory::new();
                let db = Database::create_empty(&directory.database(), config()).unwrap();
                append(&db, "a", &rows[0]);
                append(&db, "b", &rows[1]);
                append(&db, "empty", &[]);
                for (left, right, l, r) in [
                    ("a", "b", &rows[0], &rows[1]),
                    ("b", "a", &rows[1], &rows[0]),
                ] {
                    let context = format!(
                        "keys={keys},null_keys={null_keys},null_values={null_values},left={left}"
                    );
                    let joined = pairs(l, r);
                    observed_empty |= joined.is_empty();
                    observed_full_product |= joined.len() == 64;
                    let expected: Vec<_> = joined
                        .iter()
                        .map(|(l, r)| vec![cell(r.1), cell(l.1), cell(l.1)])
                        .collect();
                    // Same multiset under operand reversal, source projection,
                    // range renaming and duplicated physical output positions.
                    for prefix in [
                        format!(
                            "FROM {left} AS l |> JOIN {right} AS r ON l.k = r.k |> SELECT r.v,l.v,l.v"
                        ),
                        format!(
                            "FROM {left} |> SELECT v AS value,k AS key |> AS x |> JOIN {right} AS y ON y.k = x.key |> SELECT y.v,x.value,x.value"
                        ),
                    ] {
                        check(&db, &prefix, expected.clone(), &context);
                        cases += 1;
                    }
                    let filtered: Vec<_> = joined
                        .iter()
                        .filter(|(l, r)| l.1.is_some_and(|v| v >= 0) && r.1.is_some_and(|v| v < 8))
                        .collect();
                    check(
                        &db,
                        &format!(
                            "FROM {left} AS l |> WHERE l.v >= 0 |> JOIN {right} AS r ON r.k = l.k |> WHERE r.v < 8 |> SELECT l.v,r.v"
                        ),
                        filtered
                            .iter()
                            .map(|(l, r)| vec![cell(l.1), cell(r.1)])
                            .collect(),
                        &context,
                    );
                    cases += 1;
                    let total = sum(joined.iter().map(|(l, _)| l.1));
                    observed_nonempty_null_sum |= !joined.is_empty() && total == Cell::Null;
                    check(
                        &db,
                        &format!(
                            "FROM {left} AS l |> JOIN {right} AS r ON l.k = r.k |> AGGREGATE SUM(l.v) AS total,COUNT(*) AS n |> SELECT n,total"
                        ),
                        vec![vec![Cell::Integer(joined.len() as i64), total]],
                        &context,
                    );
                    cases += 1;
                    let mut groups: BTreeMap<Option<i64>, Vec<Option<i64>>> = BTreeMap::new();
                    for (l, r) in &joined {
                        groups.entry(r.1).or_default().push(l.1);
                    }
                    check(
                        &db,
                        &format!(
                            "FROM {left} AS l |> JOIN {right} AS r ON l.k = r.k |> AGGREGATE SUM(l.v) AS total,COUNT(*) AS n GROUP BY r.v |> WHERE n > 1 |> SELECT total,v,n"
                        ),
                        groups
                            .iter()
                            .filter(|(_, values)| values.len() > 1)
                            .map(|(key, values)| {
                                vec![
                                    sum(values.iter().copied()),
                                    cell(*key),
                                    Cell::Integer(values.len() as i64),
                                ]
                            })
                            .collect(),
                        &context,
                    );
                    cases += 1;
                    let mut groups: BTreeMap<Option<i64>, Vec<Option<i64>>> = BTreeMap::new();
                    for row in l {
                        groups.entry(row.0).or_default().push(row.1);
                    }
                    let mut expected = vec![];
                    for (key, values) in &groups {
                        for row in r {
                            if key.is_some() && *key == row.0 {
                                expected.push(vec![cell(row.1), sum(values.iter().copied())]);
                            }
                        }
                    }
                    check(
                        &db,
                        &format!(
                            "FROM {left} |> AGGREGATE SUM(v) AS total GROUP BY k |> AS g |> JOIN {right} AS r ON g.k = r.k |> SELECT r.v,g.total"
                        ),
                        expected,
                        &context,
                    );
                    cases += 1;
                    for prefix in [
                        format!("FROM empty AS l |> JOIN {right} AS r ON l.k = r.k"),
                        format!("FROM {left} AS l |> JOIN empty AS r ON l.k = r.k"),
                    ] {
                        check(
                            &db,
                            &(prefix + " |> AGGREGATE SUM(l.v) AS total,COUNT(*) AS n"),
                            vec![vec![Cell::Null, Cell::Integer(0)]],
                            &context,
                        );
                        cases += 1;
                    }
                }
                db.close().unwrap();
            }
        }
    }
    assert_eq!(cases, 432);
    assert!(observed_empty && observed_full_product && observed_nonempty_null_sum);
}

fn overflow(db: &Database, sql: &str) {
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let query = db.prepare(sql).unwrap();
    let mut result = db.execute(&query, &cancel).unwrap();
    let mut failed = false;
    for _ in 0..1024 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Failed(Error::ArithmeticOverflow { .. }) => {
                failed = true;
                break;
            }
            _ => panic!("demanded overflow must precede any output: {sql}"),
        }
    }
    assert!(failed, "overflow must terminate: {sql}");
    assert!(matches!(
        result.step(),
        QueryStep::Failed(Error::ArithmeticOverflow { .. })
    ));
    drop(result);
    drop(query);
    assert_eq!(db.reserved_memory_bytes(), baseline, "{sql}");
    assert_eq!(db.reserved_temp_bytes(), 0, "{sql}");
}

#[test]
fn joins_and_ordering_preserve_aggregate_demand_and_overflow_boundaries() {
    let directory = Directory::new();
    let db = Database::create_empty(&directory.database(), config()).unwrap();
    append(
        &db,
        "a",
        &[
            (Some(0), Some(i64::MAX)),
            (Some(0), Some(i64::MAX)),
            (Some(1), Some(1)),
            (Some(2), None),
        ],
    );
    append(
        &db,
        "b",
        &[(Some(0), Some(0)), (Some(0), Some(1)), (Some(1), Some(2))],
    );
    // Hidden sorting keys retain aggregate demand even across later filtering
    // or replacement ordering. An unused aggregate still remains undemanded.
    for expression in ["v", "v+1"] {
        let prefix =
            format!("FROM a |> AGGREGATE SUM({expression}) AS total,COUNT(*) AS n GROUP BY k");
        check(
            &db,
            &(prefix.clone() + " |> ORDER BY k |> SELECT n"),
            vec![
                vec![Cell::Integer(2)],
                vec![Cell::Integer(1)],
                vec![Cell::Integer(1)],
            ],
            "order without sum demand",
        );
        for suffix in [
            " |> ORDER BY total |> SELECT n",
            " |> ORDER BY total |> SELECT n |> WHERE n < 0",
            " |> ORDER BY total |> SELECT n |> ORDER BY n",
        ] {
            overflow(&db, &(prefix.clone() + suffix));
        }
    }
    // Dropping SUM removes both its argument and final-narrowing demand even
    // when the aggregate's other outputs cross a join boundary.
    for expression in ["v", "v+1"] {
        let prefix = format!(
            "FROM a |> AGGREGATE SUM({expression}) AS total,COUNT(*) AS n GROUP BY k |> AS g |> JOIN b AS r ON g.k = r.k"
        );
        check(
            &db,
            &(prefix.clone() + " |> SELECT r.v,g.n"),
            vec![
                vec![Cell::Integer(0), Cell::Integer(2)],
                vec![Cell::Integer(1), Cell::Integer(2)],
                vec![Cell::Integer(2), Cell::Integer(1)],
            ],
            "unused pre-join sum",
        );
        overflow(&db, &(prefix + " |> SELECT g.total"));
    }
    check(
        &db,
        "FROM a |> AGGREGATE SUM(v) AS total,AVG(v) AS mean GROUP BY k |> AS g |> JOIN b AS r ON g.k = r.k |> SELECT r.v,g.mean",
        vec![
            vec![Cell::Integer(0), Cell::Number((i64::MAX as f64).to_bits())],
            vec![Cell::Integer(1), Cell::Number((i64::MAX as f64).to_bits())],
            vec![Cell::Integer(2), Cell::Number(1.0_f64.to_bits())],
        ],
        "shared AVG demand before join",
    );
    overflow(
        &db,
        "FROM a |> AGGREGATE SUM(v) AS total GROUP BY k |> AS g |> JOIN b AS r ON g.total = r.k |> SELECT r.v",
    );
    check(
        &db,
        "FROM a |> AGGREGATE SUM(v) AS total,COUNT(*) AS n GROUP BY k |> WHERE n < 0 |> AS g |> JOIN b AS r ON g.total = r.k |> SELECT r.v",
        vec![],
        "group rejected before final narrowing and join key",
    );
    let joined = "FROM a AS l |> JOIN b AS r ON l.k = r.k";
    // Five pairs reach the aggregate. Only the key-1 pair survives v < 2.
    check(
        &db,
        &format!("{joined} |> WHERE l.v < 2 |> AGGREGATE SUM(l.v+1) AS total,COUNT(*) AS n"),
        vec![vec![Cell::Integer(2), Cell::Integer(1)]],
        "join filter suppresses scalar argument",
    );
    check(
        &db,
        &format!("{joined} |> WHERE r.v < 0 |> AGGREGATE SUM(l.v+1) AS total,COUNT(*) AS n"),
        vec![vec![Cell::Null, Cell::Integer(0)]],
        "empty join selection suppresses scalar argument",
    );
    check(
        &db,
        &format!("{joined} |> AGGREGATE SUM(l.v+1) AS total,COUNT(*) AS n |> SELECT n"),
        vec![vec![Cell::Integer(5)]],
        "unused argument after join",
    );
    for expression in ["l.v", "l.v+1"] {
        overflow(
            &db,
            &format!("{joined} |> AGGREGATE SUM({expression}) AS total"),
        );
    }
    append(
        &db,
        "bounded",
        &[(Some(0), Some(i64::MAX)), (Some(1), Some(-i64::MAX))],
    );
    check(
        &db,
        "FROM bounded |> AGGREGATE SUM(v) AS total",
        vec![vec![Cell::Integer(0)]],
        "source sum fits",
    );
    let multiplied = "FROM bounded AS l |> JOIN b AS r ON l.k = r.k";
    check(
        &db,
        &format!("{multiplied} |> AGGREGATE SUM(l.v) AS total"),
        vec![vec![Cell::Integer(i64::MAX)]],
        "joined sum cancels before final narrowing",
    );
    overflow(
        &db,
        &format!("{multiplied} |> WHERE r.v < 2 |> AGGREGATE SUM(l.v) AS total"),
    );
    let aggregated = format!("{joined} |> AGGREGATE SUM(l.v) AS total,COUNT(*) AS n");
    check(
        &db,
        &format!("{aggregated} |> WHERE n < 0 |> WHERE total > 0 |> SELECT n"),
        vec![],
        "ordered predicate rejects before final sum demand",
    );
    overflow(
        &db,
        &format!("{aggregated} |> WHERE total > 0 |> WHERE n < 0 |> SELECT n"),
    );
    db.close().unwrap();
}
