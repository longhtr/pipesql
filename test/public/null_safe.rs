//! Test IS DISTINCT FROM and its negation, including NULL operands.
//!
//! Unlike ordinary equality, these comparisons always return true or false:
//! two NULLs are not distinct, and one NULL differs from a non-NULL value.
//! Literal row IDs check that rule across types and query stages. Short-circuit
//! cases place arithmetic errors in branches that need not be evaluated.
//!
//! Numeric boundary cases distinguish exact integer comparisons from comparisons
//! that first convert to DOUBLE. They also check signed zero and NaN bits, then
//! repeat after reopening. A plan prepared before append must keep its empty view.

use super::*;

#[test]
fn public_null_safe_predicates_have_typed_two_valued_results() {
    let (_directory, db) = nullable_facts().unwrap();
    for (column, literal, distinct, same) in [
        ("n", "0.0", vec![0, 2, 3], vec![1]),
        ("n", "NULL", vec![1, 2, 3], vec![0]),
        ("i", "7", vec![0, 2, 3], vec![1]),
        ("i", "0.0", vec![1, 2, 3], vec![0]),
        ("i", "NULL", vec![0, 1, 3], vec![2]),
        ("i", "SAFE_DIVIDE(1, 0)", vec![0, 1, 3], vec![2]),
        ("s", "''", vec![0, 1, 3], vec![2]),
        ("s", "NULL", vec![0, 2, 3], vec![1]),
        ("d", "DATE '1970-01-01'", vec![3], vec![0, 1, 2]),
        ("d", "NULL", vec![0, 1, 2], vec![3]),
    ] {
        for (predicate, expected) in [
            (format!("{column} IS DISTINCT FROM {literal}"), &distinct),
            (format!("{column} IS NOT DISTINCT FROM {literal}"), &same),
            (format!("NOT ({column} IS DISTINCT FROM {literal})"), &same),
            (
                format!("NOT ({column} IS NOT DISTINCT FROM {literal})"),
                &distinct,
            ),
        ] {
            for source in [
                "FROM facts",
                "FROM (FROM facts)",
                "FROM facts |> ORDER BY id DESC",
            ] {
                query(
                    &db,
                    &format!("{source} |> WHERE {predicate} |> ORDER BY id |> SELECT id"),
                    integers(expected),
                );
            }
        }
    }
    query(
        &db,
        "FROM facts |> WHERE NOT (n=0.0) |> ORDER BY id |> SELECT id",
        integers(&[2, 3]),
    );
    query(
        &db,
        "FROM facts |> WHERE n IS DISTINCT FROM 0.0 AND (i IS NOT DISTINCT FROM NULL OR s IS NOT DISTINCT FROM 'é') |> ORDER BY id |> SELECT id",
        integers(&[2, 3]),
    );
    query(
        &db,
        "FROM facts |> AGGREGATE COUNT(*) AS n GROUP BY i |> WHERE i IS DISTINCT FROM 7 |> AGGREGATE SUM(n) AS n",
        integers(&[3]),
    );
    query(
        &db,
        "FROM facts AS a |> JOIN facts AS b ON a.id=b.id |> WHERE b.s IS NOT DISTINCT FROM NULL |> SELECT a.id",
        integers(&[1]),
    );
    query(
        &db,
        "FROM facts |> SELECT id |> UNION ALL (FROM facts |> WHERE i IS NOT DISTINCT FROM NULL |> SELECT id) |> WHERE id IS DISTINCT FROM 2 |> ORDER BY id",
        integers(&[0, 1, 3]),
    );
}

#[test]
fn public_null_safe_predicates_reject_invalid_literals_and_keep_short_circuit_demand() {
    let (_directory, db) = nullable_facts().unwrap();
    for suffix in [
        "i IS DISTINCT",
        "i IS DISTINCT FROM",
        "i IS NOT NOT DISTINCT FROM 1",
        "i IS DISTINCT FROM s",
        "i IS DISTINCT FROM '7'",
        "s IS DISTINCT FROM 7",
        "d IS DISTINCT FROM 0",
        "s IS DISTINCT FROM SAFE_DIVIDE(1, 0)",
        "i IS DISTINCT FROM 9223372036854775808",
        "n IS DISTINCT FROM 1e999",
        "missing IS DISTINCT FROM NULL",
    ] {
        let sql = format!("FROM facts |> WHERE {suffix}");
        assert!(
            matches!(
                db.prepare(&sql),
                Err(Error::Parse { .. } | Error::Bind { .. })
            ),
            "{sql}"
        );
    }
    query(
        &db,
        "FROM facts |> SELECT id, id*9223372036854775807 AS bad |> WHERE id<2 AND bad IS DISTINCT FROM NULL |> ORDER BY id |> SELECT id",
        integers(&[0, 1]),
    );
    query(
        &db,
        "FROM facts |> EXTEND DIV(id, 0) AS bad |> WHERE id IS DISTINCT FROM NULL OR bad IS DISTINCT FROM NULL |> ORDER BY id |> SELECT id",
        integers(&[0, 1, 2, 3]),
    );
    query(
        &db,
        "FROM facts |> EXTEND DIV(id, 0) AS bad |> WHERE id IS NOT DISTINCT FROM NULL AND bad IS DISTINCT FROM NULL |> SELECT id",
        vec![],
    );
}

#[test]
fn public_null_safe_numeric_boundaries_and_snapshots_survive_reopen() {
    let directory = Directory::new();
    let path = directory.database();
    let mut db = Database::create_empty(&path, config()).unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "samples",
        &[
            ColumnDeclaration {
                name: "i",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "d",
                data_type: DataType::Double,
                nullable: true,
            },
        ],
        &cancel,
    )
    .unwrap();
    let sql = "FROM samples |> WHERE i IS DISTINCT FROM NULL |> SELECT i";
    let empty = db.prepare(sql).unwrap();
    let mut append = db.begin_append("samples", limits(), &cancel).unwrap();
    append
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&[
                        9_007_199_254_740_993,
                        9_007_199_254_740_992,
                        0,
                        -1,
                    ]),
                    validity: &[15],
                },
                ColumnInput {
                    values: ColumnValues::Double(&[
                        -0.0,
                        f64::from_bits(0x7ff8_0000_0000_0042),
                        f64::INFINITY,
                        0.0,
                    ]),
                    validity: &[7],
                },
            ],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    assert!(collect_unordered(&mut db.execute(&empty, &cancel).unwrap()).is_empty());
    drop(empty);
    for reopen in [false, true] {
        if reopen {
            db.close().unwrap();
            db = Database::open(&path, config()).unwrap();
        }
        query(
            &db,
            sql,
            integers(&[9_007_199_254_740_993, 9_007_199_254_740_992, 0, -1]),
        );
        query(
            &db,
            "FROM samples |> WHERE i IS NOT DISTINCT FROM 9007199254740992 |> SELECT i",
            integers(&[9_007_199_254_740_992]),
        );
        // Both adjacent integers round to 2^53 as DOUBLE. The integer literal
        // above distinguishes them; this floating-point literal cannot.
        query(
            &db,
            "FROM samples |> WHERE i IS NOT DISTINCT FROM 9007199254740992.0 |> SELECT i",
            integers(&[9_007_199_254_740_993, 9_007_199_254_740_992]),
        );
        query(
            &db,
            "FROM samples |> WHERE d IS NOT DISTINCT FROM 0.0 |> SELECT d",
            vec![vec![Cell::Number((-0.0_f64).to_bits())]],
        );
        query(
            &db,
            "FROM samples |> WHERE d IS DISTINCT FROM 0.0 |> SELECT d",
            vec![
                vec![Cell::Number(0x7ff8_0000_0000_0042)],
                vec![Cell::Number(f64::INFINITY.to_bits())],
                vec![Cell::Null],
            ],
        );
    }
    db.close().unwrap();
}
