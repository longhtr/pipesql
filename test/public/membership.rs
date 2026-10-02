//! Test IN and NOT IN lists, including duplicates and NULL entries.
//!
//! A matching non-NULL value makes IN true. Without a match, a NULL operand or
//! list entry makes the answer unknown; NOT preserves unknown, so WHERE rejects
//! the row. A local set-membership model checks these rules independently of
//! the engine's Boolean plan. Literal cases cover other types and query stages.
//!
//! Invalid lists must fail preparation even inside a branch execution could skip.
//! Overflow cases check which branches are evaluated and identify the failing
//! expression by its source span. Cancellation and early drop must release query
//! reservations and allow a later execution to succeed.

use super::*;

#[test]
fn literal_membership_preserves_duplicates_null_and_negation() {
    let (_directory, db) = nullable_facts().unwrap();
    for (predicate, expected) in [
        ("id IN (1, 3, 1)", vec![1, 3]),
        ("id IN (NULL, 1, 3, NULL)", vec![1, 3]),
        ("NOT id IN (1, 3)", vec![0, 2]),
        ("id NOT IN (1, 3, 1)", vec![0, 2]),
        ("NOT id NOT IN (1, 3)", vec![1, 3]),
        ("id NOT IN (1, NULL, 3)", vec![]),
        ("NOT id NOT IN (1, NULL, 3)", vec![1, 3]),
        ("id NOT IN (1, 3) AND id=2 OR id=1", vec![1, 2]),
        ("NOT id IN (1, NULL, 3)", vec![]),
        ("id IN (NULL)", vec![]),
        ("NOT id IN (NULL)", vec![]),
        ("id IN (1, NULL) OR id=2", vec![1, 2]),
        ("NOT (id IN (1, NULL) AND id=2)", vec![0, 1, 3]),
    ] {
        query(
            &db,
            &format!("FROM facts |> WHERE {predicate} |> ORDER BY id |> SELECT id"),
            integers(&expected),
        );
    }
}

#[test]
fn membership_checks_types_and_composes_with_producers() {
    let (_directory, db) = nullable_facts().unwrap();
    for (predicate, expected) in [
        ("i IN (7, 9, 7)", vec![1, 3]),
        ("NOT i IN (7)", vec![0, 3]),
        ("n IN (0, -0.0)", vec![1]),
        ("NOT n IN (0)", vec![2, 3]),
        ("n NOT IN (0, -0.0)", vec![2, 3]),
        ("n NOT IN (0, NULL)", vec![]),
        ("i NOT IN (7)", vec![0, 3]),
        ("s NOT IN ('')", vec![0, 3]),
        ("d NOT IN (DATE '1970-01-02')", vec![0, 1, 2]),
        ("NOT n IN (0, NULL)", vec![]),
        ("s IN ('é', '', 'é')", vec![2, 3]),
        ("NOT s IN ('')", vec![0, 3]),
        ("d IN (DATE '1970-01-01', NULL)", vec![0, 1, 2]),
    ] {
        for source in [
            "FROM facts",
            "FROM (FROM facts)",
            "FROM facts |> ORDER BY id DESC",
            "FROM facts |> LIMIT 4",
            "FROM facts AS a |> JOIN (FROM facts |> SELECT id AS other) AS b ON a.id=b.other",
            "FROM facts |> UNION ALL (FROM facts |> LIMIT 0)",
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
        "FROM facts |> AGGREGATE COUNT(*) AS n GROUP AND ORDER BY s |> WHERE s IN ('', 'é', NULL) |> SELECT n",
        integers(&[1, 1]),
    );
    let baseline = db.reserved_memory_bytes();
    for predicate in [
        "id IN ()",
        "id IN (1, )",
        "id IN (, 1)",
        "id IN (1 2)",
        "id IN (1",
        "id IN (1))",
        "id NOT NOT IN (1)",
        "id NOT = 1",
        "id NOT IS NULL",
        "id NOT",
        "id NOT IN ()",
        "id NOT IN (1, )",
        "id NOT IN (i)",
        "id NOT IN (FROM facts |> SELECT id)",
        "id >= 0 OR s NOT IN (1)",
        "id IN UNNEST (1)",
        "id IN (FROM facts |> SELECT id)",
        "id IN (i)",
        "id+1 IN (1)",
        "id=0 OR missing IN (NULL)",
        "id>=0 OR s IN (NULL, 1)",
        "id>=0 OR i IN (NULL, '7')",
        "id>=0 OR d IN (DATE '2000-02-30')",
        "id>=0 OR i IN (9223372036854775808)",
        "id = NULL",
        "id = (NULL)",
    ] {
        let sql = format!("FROM facts |> WHERE {predicate}");
        let error = db.prepare(&sql).err().expect(&sql);
        let span = match error {
            Error::Parse { span, .. } | Error::Bind { span, .. } => span,
            error => panic!("{sql}: {error}"),
        };
        assert!(span.start() <= span.end() && span.end() <= sql.len());
        assert_eq!(db.reserved_memory_bytes(), baseline, "{sql}");
    }
}

#[test]
fn membership_matches_independent_nullable_set_model() {
    let directory = Directory::new();
    let db = Database::create_empty(&directory.database(), config()).unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "pairs",
        &["a", "b", "id"].map(|name| ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: name != "id",
        }),
        &cancel,
    )
    .unwrap();
    let values = [None, Some(0), Some(7)];
    let mut columns = [[0; 9]; 3];
    let mut validity = [[0_u8; 2]; 3];
    for row in 0..9 {
        for (column, value) in [values[row / 3], values[row % 3], Some(row as i64)]
            .into_iter()
            .enumerate()
        {
            if let Some(value) = value {
                columns[column][row] = value;
                validity[column][row / 8] |= 1 << (row % 8);
            }
        }
    }
    let mut append = db.begin_append("pairs", limits(), &cancel).unwrap();
    append
        .write(
            &std::array::from_fn::<_, 3, _>(|column| ColumnInput {
                values: ColumnValues::Int64(&columns[column]),
                validity: &validity[column],
            }),
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();

    // None represents SQL unknown. Search the values directly, then account
    // for NULL; do not reproduce the engine's expansion into Boolean branches.
    let member = |value: Option<i64>, candidates: &[Option<i64>]| {
        value
            .map(|v| candidates.contains(&Some(v)))
            .and_then(|found| {
                if found {
                    Some(true)
                } else if candidates.contains(&None) {
                    None
                } else {
                    Some(false)
                }
            })
    };
    let and = |a, b| match (a, b) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    };
    let or = |a, b| match (a, b) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    };
    for (list, candidates) in [
        ("7", vec![Some(7)]),
        ("NULL, 7, 7", vec![None, Some(7), Some(7)]),
        ("0, 7", vec![Some(0), Some(7)]),
        ("NULL", vec![None]),
    ] {
        for form in 0..8 {
            let left = format!("a IN ({list})");
            let expression = match form {
                0 => left,
                1 => format!("NOT {left}"),
                2 => format!("{left} AND b IN (7)"),
                3 => format!("{left} OR b IN (7)"),
                4 => format!("NOT ({left} AND b IN (7))"),
                5 => format!("NOT ({left} OR b IN (7))"),
                6 => format!("a NOT IN ({list})"),
                7 => format!("NOT a NOT IN ({list})"),
                _ => unreachable!(),
            };
            let expected: Vec<_> = (0..9)
                .filter(|row| {
                    let a = member(values[row / 3], &candidates);
                    let b = member(values[row % 3], &[Some(7)]);
                    let truth = match form {
                        0 => a,
                        1 | 6 => a.map(|v| !v),
                        7 => a,
                        2 => and(a, b),
                        3 => or(a, b),
                        4 => and(a, b).map(|v| !v),
                        5 => or(a, b).map(|v| !v),
                        _ => unreachable!(),
                    };
                    truth == Some(true)
                })
                .map(|row| row as i64)
                .collect();
            query(
                &db,
                &format!("FROM pairs |> WHERE {expression} |> ORDER BY id |> SELECT id"),
                integers(&expected),
            );
        }
    }
}

#[test]
fn membership_preserves_conditional_demand_and_release() {
    let (_directory, db) = nullable_facts().unwrap();
    for (sql, expected) in [
        (
            "FROM facts |> SELECT id, id*9223372036854775807 AS bad |> WHERE id IN (2, 3) OR bad IN (0, 9223372036854775807) |> ORDER BY id |> SELECT id",
            vec![0, 1, 2, 3],
        ),
        (
            "FROM facts |> SELECT id, i, id*9223372036854775807 AS bad |> WHERE id=2 |> WHERE NOT (i IN (0, NULL) OR bad IN (0)) |> SELECT id",
            vec![],
        ),
        (
            "FROM facts |> SELECT id, i, id*9223372036854775807 AS bad |> WHERE i IN (NULL) AND bad IN (0) |> SELECT id",
            vec![],
        ),
        (
            "FROM facts |> AGGREGATE SUM(9223372036854775807) AS s, COUNT(*) AS n |> WHERE n IN (4) OR s IN (0) |> SELECT n",
            vec![4],
        ),
    ] {
        query(&db, sql, integers(&expected));
    }
    for predicate in [
        "bad IN (0) OR id IN (2)",
        "bad NOT IN (0) OR id IN (2)",
        "i NOT IN (0, NULL) OR bad NOT IN (0)",
        "NOT (i IN (0, NULL) AND bad IN (0))",
    ] {
        let sql = format!(
            "FROM facts |> SELECT id, i, id*9223372036854775807 AS bad |> WHERE id=2 |> WHERE {predicate} |> SELECT id"
        );
        let baseline = db.reserved_memory_bytes();
        let prepared = db.prepare(&sql).unwrap();
        let cancel = CancellationToken::new();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        let mut failed = false;
        for _ in 0..8192 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Failed(Error::ArithmeticOverflow { span, .. }) => {
                    assert_eq!(&sql[span.start()..span.end()], "id*9223372036854775807");
                    failed = true;
                    break;
                }
                _ => panic!("demanded overflow hidden: {sql}"),
            }
        }
        assert!(failed, "{sql}");
        drop(result);
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}

#[test]
fn membership_retains_stage_bounds_cancellation_and_early_drop() {
    let (_directory, db) = nullable_facts().unwrap();
    // SELECT uses one stage and each list item uses another. Fifteen repeated
    // items fill the 16-stage limit; equal values must not evade that limit.
    let negated_limit = format!(
        "FROM facts |> SELECT id |> WHERE id NOT IN ({})",
        ["0"; 15].join(", ")
    );
    query(&db, &negated_limit, integers(&[1, 2, 3]));
    assert!(matches!(
        db.prepare(&format!(
            "FROM facts |> SELECT id |> WHERE id NOT IN ({})",
            ["0"; 16].join(", ")
        )),
        Err(Error::Parse { .. })
    ));
    let at_limit = format!(
        "FROM facts |> SELECT id |> WHERE id IN ({})",
        ["0"; 15].join(", ")
    );
    query(&db, &at_limit, integers(&[0]));
    let over_limit = format!(
        "FROM facts |> SELECT id |> WHERE id IN ({})",
        ["0"; 16].join(", ")
    );
    assert!(matches!(db.prepare(&over_limit), Err(Error::Parse { .. })));
    let baseline = db.reserved_memory_bytes();
    let prepared = db
        .prepare("FROM facts |> WHERE id NOT IN (0, 2) |> ORDER BY id |> SELECT id")
        .unwrap();
    let parked = db.reserved_memory_bytes();
    for cancel_after in [0, 1] {
        let cancel = CancellationToken::new();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        for _ in 0..cancel_after {
            assert!(matches!(result.step(), QueryStep::Progress));
        }
        cancel.cancel();
        assert!(matches!(result.step(), QueryStep::Failed(Error::Cancelled)));
        drop(result);
        assert_eq!(db.reserved_memory_bytes(), parked);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    let cancel = CancellationToken::new();
    let mut result = db.execute(&prepared, &cancel).unwrap();
    assert!(matches!(result.step(), QueryStep::Progress));
    drop(result);
    assert_eq!(db.reserved_memory_bytes(), parked);
    assert_eq!(db.reserved_temp_bytes(), 0);
    drop(prepared);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    query(&db, &at_limit, integers(&[0]));
}
