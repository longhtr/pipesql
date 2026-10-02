//! Check numeric searched CASE through query preparation and execution.
//!
//! Literal answers distinguish first-match selection, NULL conditions and common
//! numeric result types. Failing expressions in unchosen arms must remain
//! undemanded, including dependencies defined by an earlier projection.
//!
//! Demanded failures must preserve their error and release execution ownership.
//! Preparation rejects incomplete syntax, unsupported types and oversized CASE
//! expressions. Cancellation and abandoned results check the same ownership boundary
//! without requiring the query to produce a final row.

use super::*;

#[test]
fn public_case_selects_first_true_arm_and_composes() {
    let (_directory, db) = join_fixture();
    for (sql, expected) in [
        (
            "FROM facts |> SELECT CASE WHEN v<=20 THEN 1 WHEN v<=30 THEN 2 ELSE 3 END AS class |> ORDER BY class",
            integers(&[1, 1, 2, 3]),
        ),
        (
            "FROM facts |> SELECT CASE WHEN k IS NULL THEN 0 WHEN k=1 THEN v ELSE -1 END AS n |> ORDER BY n",
            integers(&[-1, 0, 10, 20]),
        ),
        (
            "FROM facts |> SELECT CASE WHEN k=1 THEN v END AS n |> ORDER BY n NULLS FIRST",
            vec![
                vec![Cell::Null],
                vec![Cell::Null],
                vec![Cell::Integer(10)],
                vec![Cell::Integer(20)],
            ],
        ),
        (
            "FROM facts |> SELECT CASE WHEN k IS NOT NULL THEN 1 ELSE 0 END AS class, v |> AGGREGATE SUM(v) AS total GROUP AND ORDER BY class",
            vec![
                vec![Cell::Integer(0), Cell::Integer(40)],
                vec![Cell::Integer(1), Cell::Integer(60)],
            ],
        ),
        (
            "FROM facts AS f |> LEFT JOIN dimensions AS d ON f.k=d.k |> SELECT CASE WHEN d.k IS NULL THEN f.v ELSE 0 END AS n |> AGGREGATE SUM(n) AS total",
            integers(&[40]),
        ),
        (
            "FROM facts |> SELECT CASE WHEN k=1 THEN CASE WHEN v=10 THEN 11 ELSE 22 END ELSE CASE WHEN k IS NULL THEN 44 ELSE 33 END END AS n |> ORDER BY n",
            integers(&[11, 22, 33, 44]),
        ),
        (
            "FROM facts |> SELECT CASE WHEN CASE WHEN k IS NULL THEN 0 ELSE k END=1 THEN 9 ELSE 8 END AS n |> ORDER BY n",
            integers(&[8, 8, 9, 9]),
        ),
    ] {
        query(&db, sql, expected);
    }
}

#[test]
fn public_case_preserves_types_nulls_and_skipped_errors() {
    let (_directory, db) = join_fixture();
    for (expression, kind, nullable, expected) in [
        ("NULL", DataType::Int64, true, Cell::Null),
        ("CAST(NULL AS DOUBLE)", DataType::Double, true, Cell::Null),
        ("COALESCE(NULL, NULL)", DataType::Int64, true, Cell::Null),
        ("NULLIF(1, NULL)", DataType::Int64, true, Cell::Integer(1)),
        (
            "CASE WHEN 1=1 THEN 7 ELSE DIV(1, 0) END",
            DataType::Int64,
            false,
            Cell::Integer(7),
        ),
        (
            "CASE WHEN 1=0 THEN DIV(1, 0) ELSE 8 END",
            DataType::Int64,
            false,
            Cell::Integer(8),
        ),
        (
            "CASE WHEN 1=1 THEN 7 WHEN DIV(1, 0)=0 THEN 8 ELSE 9 END",
            DataType::Int64,
            false,
            Cell::Integer(7),
        ),
        (
            "CASE WHEN NULL=1 THEN 7 WHEN 1=NULL THEN 8 ELSE 9 END",
            DataType::Int64,
            false,
            Cell::Integer(9),
        ),
        (
            "CASE WHEN 1=1 THEN 7 ELSE 0.0 END",
            DataType::Double,
            false,
            Cell::Number(7.0_f64.to_bits()),
        ),
        (
            "CASE WHEN 1=0 THEN 7.0 ELSE 8 END",
            DataType::Double,
            false,
            Cell::Number(8.0_f64.to_bits()),
        ),
        (
            "CASE WHEN 1=0 THEN 7.0 END",
            DataType::Double,
            true,
            Cell::Null,
        ),
        (
            "CASE WHEN 1=1 THEN NULL ELSE 8 END",
            DataType::Int64,
            true,
            Cell::Null,
        ),
        (
            "CASE WHEN 1=0 THEN NULL END",
            DataType::Int64,
            true,
            Cell::Null,
        ),
        (
            "COALESCE(CASE WHEN 1=0 THEN 1 END, 5)",
            DataType::Int64,
            false,
            Cell::Integer(5),
        ),
        (
            "COALESCE(6, CASE WHEN DIV(1, 0)=0 THEN 1 END)",
            DataType::Int64,
            false,
            Cell::Integer(6),
        ),
        (
            "CASE WHEN COALESCE(NULL, 1)=1 THEN COALESCE(7, DIV(1, 0)) ELSE 0 END",
            DataType::Int64,
            false,
            Cell::Integer(7),
        ),
        (
            "CASE WHEN 1=1 THEN 9007199254740993 ELSE 0 END",
            DataType::Int64,
            false,
            Cell::Integer(9_007_199_254_740_993),
        ),
        (
            "CASE WHEN 9007199254740993=9007199254740992 THEN 0 ELSE 1 END",
            DataType::Int64,
            false,
            Cell::Integer(1),
        ),
    ] {
        let sql = format!("FROM facts |> SELECT {expression} AS n |> LIMIT 1");
        let prepared = db.prepare(&sql).unwrap();
        let column = prepared.result_column(0).unwrap();
        assert_eq!(
            (column.data_type, column.nullable),
            (kind, nullable),
            "{sql}"
        );
        drop(prepared);
        query(&db, &sql, vec![vec![expected]]);
    }
    query(
        &db,
        "FROM facts |> SELECT DIV(v, 0) AS bad, v |> SELECT CASE WHEN v>0 THEN v ELSE bad END AS n |> ORDER BY n",
        integers(&[10, 20, 30, 40]),
    );
    query(
        &db,
        "FROM facts |> SELECT DIV(v, 0) AS bad, v |> SELECT CASE WHEN v>0 THEN v WHEN bad=0 THEN bad END AS n |> ORDER BY n",
        integers(&[10, 20, 30, 40]),
    );
}

#[test]
fn public_case_reports_demanded_errors_and_releases_execution() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    for (sql, source) in [
        (
            "FROM facts |> SELECT CASE WHEN DIV(v, 0)=0 THEN 1 ELSE 2 END AS n",
            "CASE WHEN DIV(v, 0)=0 THEN 1 ELSE 2 END",
        ),
        (
            "FROM facts |> SELECT CASE WHEN v>0 THEN DIV(v, 0) ELSE 2 END AS n",
            "CASE WHEN v>0 THEN DIV(v, 0) ELSE 2 END",
        ),
        (
            "FROM facts |> SELECT CASE WHEN v<0 THEN 1 ELSE DIV(v, 0) END AS n",
            "CASE WHEN v<0 THEN 1 ELSE DIV(v, 0) END",
        ),
        (
            "FROM facts |> SELECT DIV(v, 0) AS bad, v |> SELECT CASE WHEN v>0 THEN bad ELSE 2 END AS n",
            "DIV(v, 0)",
        ),
    ] {
        let prepared = db.prepare(sql).unwrap();
        let cancel = CancellationToken::new();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        let mut failed = false;
        for _ in 0..10_000 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Failed(Error::DivisionByZero { span, .. }) => {
                    assert_eq!(&sql[span.start()..span.end()], source);
                    failed = true;
                    break;
                }
                _ => panic!("CASE did not report its demanded error: {sql}"),
            }
        }
        assert!(failed);
        assert!(matches!(result.step(), QueryStep::Failed(_)));
        drop(result);
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
        query(&db, &format!("{sql} |> LIMIT 0"), vec![]);
    }
    query(
        &db,
        "FROM facts |> SELECT CASE WHEN v>0 THEN v END AS n |> ORDER BY n",
        integers(&[10, 20, 30, 40]),
    );
}

#[test]
fn public_case_rejects_incomplete_unsupported_and_oversized_expressions() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    for expression in [
        "CASE END",
        "CASE k WHEN 1 THEN 2 END",
        "CASE WHEN k THEN 2 END",
        "CASE WHEN k=1 THEN END",
        "CASE WHEN k=1 THEN 2 ELSE END",
        "CASE WHEN k=1 THEN 2",
        "CASE WHEN k=1 ELSE 2 END",
        "CASE WHEN k=1 THEN 2 ELSE 3 WHEN k=2 THEN 4 END",
        "CASE WHEN k=1 THEN 2 ELSE 3 ELSE 4 END",
        "CASE WHEN k=1 AND v=10 THEN 2 END",
        "CASE WHEN k=1 OR v=10 THEN 2 END",
        "CASE WHEN NOT k=1 THEN 2 END",
        "CASE WHEN TRUE THEN 2 END",
        "CASE WHEN k=1 THEN 'text' ELSE 'other' END",
        "CASE WHEN k=1 THEN DATE '2000-01-01' END",
        "CASE WHEN missing=1 THEN 2 END",
        "CASE WHEN k=1 THEN 2 ELSE missing END",
        "CASE WHEN k=1 THEN 2 ELSE 9223372036854775808 END",
        "(CASE WHEN k=1 THEN 2)",
        "COALESCE(CASE WHEN k=1 THEN 2, 3)",
        "CASE WHEN k IS 1 THEN 2 END",
        "CASE WHEN k IS NOT THEN 2 END",
    ] {
        assert!(
            matches!(
                db.prepare(&format!("FROM facts |> SELECT {expression} AS n")),
                Err(Error::Parse { .. } | Error::Bind { .. })
            ),
            "{expression}"
        );
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    // Each arm adds two comparison operands, a result and a CASE operation;
    // the shared fallback adds one. Seven fit (29 operations), eight do not.
    for arms in [7, 8] {
        let expression = format!("CASE {} ELSE 3 END", "WHEN k=1 THEN 2 ".repeat(arms));
        let sql = format!("FROM facts |> SELECT {expression} AS n |> ORDER BY n");
        if arms == 7 {
            query(&db, &sql, integers(&[2, 2, 3, 3]));
        } else {
            assert!(matches!(db.prepare(&sql), Err(Error::Parse { .. })));
        }
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
}

#[test]
fn public_case_cancellation_and_abandonment_release_query_owners() {
    let (_directory, db) = join_fixture();
    for sql in [
        "FROM facts |> SELECT CASE WHEN k IS NULL THEN 0 ELSE v END AS n",
        "FROM facts |> SELECT CASE WHEN k=1 THEN v ELSE 0 END AS n |> ORDER BY n",
        "FROM facts |> AGGREGATE SUM(CASE WHEN k=1 THEN v ELSE 0 END) AS total GROUP BY k",
    ] {
        assert_cancel_and_drop_release(&db, sql);
    }
}
