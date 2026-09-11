use super::order::{integers, query};
use super::*;

#[test]
fn boolean_filters_preserve_null_nan_precedence_and_producer_composition() {
    let (_directory, db) = super::null_predicate::fixture().unwrap();
    for (predicate, expected) in [
        ("NOT n < 0", vec![1, 2, 3]),
        ("n >= 0", vec![1, 3]),
        ("NOT NOT n < 0", vec![]),
        ("n < 0 OR n IS NULL", vec![0]),
        ("NOT (n < 0 AND id < 0)", vec![0, 1, 2, 3]),
        ("NOT (n < 0 OR id < 0)", vec![1, 2, 3]),
        ("s IS NULL OR id=0", vec![0, 1]),
        ("id=0 OR id=1 AND s IS NOT NULL", vec![0]),
        ("(id=0 OR id=1) AND s IS NOT NULL", vec![0]),
        ("NOT (s IS NULL OR s='')", vec![0, 3]),
        ("NOT s BETWEEN '' AND 'present'", vec![3]),
        ("NOT d < DATE '1970-01-01'", vec![0, 1, 2]),
        ("NOT (i > 0 AND id > 1)", vec![0, 1]),
    ] {
        for source in [
            "FROM facts",
            "FROM (FROM facts)",
            "FROM facts |> ORDER BY id DESC",
            "FROM facts |> LIMIT 4",
            "FROM facts AS a |> JOIN (FROM facts |> SELECT id AS other) AS b ON a.id=b.other",
        ] {
            query(
                &db,
                &format!("{source} |> WHERE {predicate} |> ORDER BY id |> SELECT id"),
                integers(&expected),
            );
        }
    }
    query(
        &db,
        "FROM facts |> AGGREGATE SUM(i) AS x, COUNT(*) AS n |> WHERE NOT x < 0 OR n=0 |> SELECT n",
        integers(&[4]),
    );
    query(
        &db,
        "FROM facts |> AGGREGATE COUNT(*) AS n GROUP AND ORDER BY s |> WHERE NOT (s IS NULL OR s='') |> SELECT n",
        integers(&[1, 1]),
    );
    for predicate in [
        "NOT",
        "()",
        "(id=0",
        "id=0 OR",
        "OR id=0",
        "id=0 AND OR id=1",
        "NOT (id=0))",
        "id=0 OR missing=1",
        "id=0 OR s=1",
    ] {
        assert!(
            matches!(
                db.prepare(&format!("FROM facts |> WHERE {predicate}")),
                Err(Error::Parse { .. } | Error::Bind { .. })
            ),
            "{predicate}"
        );
    }
}

#[test]
fn boolean_filters_preserve_conditional_computed_demand() {
    let (_directory, db) = super::null_predicate::fixture().unwrap();
    for predicate in [
        "n<0 AND bad>0",
        "NOT (n>=0 OR n IS NULL OR NOT n<0) AND bad>0",
    ] {
        query(
            &db,
            &format!("FROM facts |> SELECT n,id*9223372036854775807 AS bad |> WHERE {predicate}"),
            vec![],
        );
    }
    query(
        &db,
        "FROM facts |> SELECT id,i,id*9223372036854775807 AS bad |> WHERE id=2 |> WHERE NOT (i>0 OR bad>0) |> SELECT id",
        vec![],
    );
    query(
        &db,
        "FROM facts |> SELECT id,id*9223372036854775807 AS bad |> WHERE id>=2 OR bad>=0 |> ORDER BY id |> SELECT id",
        integers(&[0, 1, 2, 3]),
    );
    query(
        &db,
        "FROM facts |> AGGREGATE SUM(9223372036854775807) AS s,COUNT(*) AS n |> WHERE n>0 OR s>0 |> SELECT n",
        integers(&[4]),
    );
    let baseline = db.reserved_memory_bytes();
    let failures = ["bad>0 AND n<0", "n<0 OR bad>0", "NOT (i>0 AND bad>0)"].map(|predicate| format!("FROM facts |> SELECT id,n,i,id*9223372036854775807 AS bad |> WHERE id=2 |> WHERE {predicate} |> SELECT id"));
    for sql in failures.iter().map(String::as_str).chain(["FROM facts |> AGGREGATE SUM(9223372036854775807) AS s,COUNT(*) AS n |> WHERE n<0 OR s>0 |> SELECT n"]) {
        let prepared = db.prepare(sql).unwrap();
        let cancel = CancellationToken::new();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        let mut failed = false;
        for _ in 0..8192 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Rows(_) | QueryStep::Finished => {
                    panic!("demanded overflow hidden: {sql}")
                }
                QueryStep::Failed(error) => {
                    assert!(matches!(error, Error::ArithmeticOverflow { .. }));
                    failed = true;
                    break;
                }
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
fn boolean_filters_match_independent_three_valued_model() {
    let directory = Directory::new();
    let db = Database::create_empty(&directory.database(), config()).unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "truth",
        &['a', 'b', 'c', 'i'].map(|name| ColumnDeclaration {
            name: match name {
                'a' => "a",
                'b' => "b",
                'c' => "c",
                _ => "i",
            },
            data_type: DataType::Int64,
            nullable: name != 'i',
        }),
        &cancel,
    )
    .unwrap();
    let values = [Some(false), Some(true), None];
    let mut columns = [[0_i64; 27]; 4];
    let mut validity = [[0_u8; 4]; 4];
    for row in 0..27 {
        for column in 0..3 {
            let value = values[(row / [9, 3, 1][column]) % 3];
            columns[column][row] = i64::from(value == Some(true));
            if value.is_some() {
                validity[column][row / 8] |= 1 << (row % 8);
            }
        }
        columns[3][row] = row as i64;
        validity[3][row / 8] |= 1 << (row % 8);
    }
    let mut append = db
        .begin_append(
            "truth",
            AppendLimits {
                batches: 1,
                encoded_bytes: 20_000,
            },
            &cancel,
        )
        .unwrap();
    append
        .write(
            &std::array::from_fn::<_, 4, _>(|index| ColumnInput {
                values: ColumnValues::Int64(&columns[index]),
                validity: &validity[index],
            }),
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();

    type Expression = (String, [Option<bool>; 27]);
    let unary = |input: &Expression| -> Expression {
        (format!("NOT ({})", input.0), input.1.map(|v| v.map(|b| !b)))
    };
    let binary = |a: &Expression, b: &Expression, and: bool| -> Expression {
        (
            format!("({}) {} ({})", a.0, if and { "AND" } else { "OR" }, b.0),
            std::array::from_fn(|row| match (a.1[row], b.1[row], and) {
                (Some(false), _, true) | (_, Some(false), true) => Some(false),
                (Some(true), Some(true), true) => Some(true),
                (Some(true), _, false) | (_, Some(true), false) => Some(true),
                (Some(false), Some(false), false) => Some(false),
                _ => None,
            }),
        )
    };
    let leaves: Vec<Expression> = ["a", "b", "c"]
        .iter()
        .enumerate()
        .map(|(column, name)| {
            (
                format!("{name}=1"),
                std::array::from_fn(|row| values[(row / [9, 3, 1][column]) % 3]),
            )
        })
        .collect();
    let mut level = leaves.clone();
    level.extend(leaves.iter().map(unary));
    for a in &leaves {
        for b in &leaves {
            for and in [true, false] {
                level.push(binary(a, b, and));
            }
        }
    }
    let check = |expression: Expression| {
        let expected: Vec<_> = expression
            .1
            .iter()
            .enumerate()
            .filter_map(|(row, v)| (*v == Some(true)).then_some(row as i64))
            .collect();
        query(
            &db,
            &format!(
                "FROM truth |> WHERE {} |> ORDER BY i |> SELECT i",
                expression.0
            ),
            integers(&expected),
        );
    };
    for leaf in &leaves {
        check(leaf.clone());
    }
    for a in &level {
        check(unary(a));
    }
    for a in &level {
        for b in &level {
            for and in [true, false] {
                check(binary(a, b, and));
            }
        }
    }
}

#[test]
#[cfg_attr(
    all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"),
    ignore = "GNU aarch64 has a 128-KiB pthread minimum; this test requires at most 64 KiB"
)]
fn boolean_scan_scratch_is_optional_bounded_and_small_stack_safe() {
    let (_directory, db) = super::null_predicate::fixture().unwrap();
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(48 * 1024)
            .spawn_scoped(scope, || {
                assert!(pipesql_filesystem::test_current_thread_stack_bytes() <= 65_536);
                let cancel = CancellationToken::new();
                for (projection, column, rows) in [("id", "id", 4096_u64), ("id+0 AS x", "x", 256)]
                {
                    let mut charges = [0; 2];
                    for (index, predicate) in [
                        format!("{column}>=0 AND {column}<=3"),
                        format!("{column}=0 OR {column}=1"),
                    ]
                    .iter()
                    .enumerate()
                    {
                        let prepared = db
                            .prepare(&format!(
                                "FROM facts |> SELECT {projection} |> WHERE {predicate}"
                            ))
                            .unwrap();
                        let baseline = db.reserved_memory_bytes();
                        let result = db.execute(&prepared, &cancel).unwrap();
                        charges[index] = db.reserved_memory_bytes() - baseline;
                        drop(result);
                        assert_eq!(db.reserved_memory_bytes(), baseline);
                    }
                    assert_eq!(charges[1] - charges[0], rows * 5 + 2 * 4096);
                }
                let source = format!(
                    "FROM facts |> WHERE {}id<0{} |> SELECT id",
                    "(".repeat(70),
                    ")".repeat(70)
                );
                query(&db, &source, vec![]);
                query(
                    &db,
                    &format!(
                        "FROM facts |> WHERE {}id<0 |> SELECT id",
                        "NOT ".repeat(100)
                    ),
                    vec![],
                );
            })
            .unwrap()
            .join()
            .unwrap();
    });
}
