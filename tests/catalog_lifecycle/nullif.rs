use super::order::{integers, query};
use super::*;

#[test]
fn public_nullif_normalizes_sentinels_and_composes() {
    let (_directory, db) = join_fixture();
    for (sql, expected) in [
        (
            "FROM facts |> SELECT NULLIF(k, 1) AS n |> ORDER BY n NULLS FIRST",
            vec![
                vec![Cell::Null],
                vec![Cell::Null],
                vec![Cell::Null],
                vec![Cell::Integer(2)],
            ],
        ),
        (
            "FROM facts |> AGGREGATE SUM(NULLIF(v, 20)) AS total, COUNT(NULLIF(k, 1)) AS present, COUNT(*) AS nrows",
            vec![vec![Cell::Integer(80), Cell::Integer(1), Cell::Integer(4)]],
        ),
        (
            "FROM facts |> SELECT NULLIF(v, 20) AS n |> ORDER BY n NULLS FIRST |> AGGREGATE SUM(n) AS total",
            integers(&[80]),
        ),
        (
            "FROM facts |> AGGREGATE SUM(NULLIF(v, 20)) AS total GROUP BY k |> ORDER BY k NULLS FIRST",
            vec![
                vec![Cell::Null, Cell::Integer(40)],
                vec![Cell::Integer(1), Cell::Integer(10)],
                vec![Cell::Integer(2), Cell::Integer(30)],
            ],
        ),
        (
            "FROM facts |> SELECT NULLIF(v, v) AS n |> SELECT COALESCE(n, 10) AS restored",
            integers(&[10, 10, 10, 10]),
        ),
        (
            "FROM facts |> SELECT NULLIF(v, COALESCE(v, DIV(v, 0))) AS n",
            vec![vec![Cell::Null]; 4],
        ),
        (
            "FROM facts |> SELECT COALESCE(v, NULLIF(DIV(v, 0), 0)) AS n |> ORDER BY n",
            integers(&[10, 20, 30, 40]),
        ),
        (
            "FROM facts |> SELECT NULLIF(k, 1) AS k |> INTERSECT ALL (FROM facts |> SELECT NULLIF(k, 1) AS k) |> AGGREGATE COUNT(*) AS nrows, COUNT(k) AS present",
            vec![vec![Cell::Integer(4), Cell::Integer(1)]],
        ),
        (
            "FROM facts AS f |> LEFT JOIN dimensions AS d ON f.k=d.k |> SELECT NULLIF(f.v, 20) AS n |> AGGREGATE SUM(n) AS total",
            integers(&[90]),
        ),
        (
            "FROM facts |> WHERE v<0 |> AGGREGATE SUM(NULLIF(v, 0)) AS total, COUNT(NULLIF(v, 0)) AS present",
            vec![vec![Cell::Null, Cell::Integer(0)]],
        ),
    ] {
        query(&db, sql, expected);
    }
    for (expression, kind, expected) in [
        (
            "NULLIF(9007199254740993, 9007199254740992)",
            DataType::Int64,
            Cell::Integer(9_007_199_254_740_993),
        ),
        (
            "NULLIF(9007199254740993, 9007199254740992.0)",
            DataType::Double,
            Cell::Null,
        ),
        (
            "NULLIF(1, NULLIF(2.0, 2.0))",
            DataType::Double,
            Cell::Number(1.0_f64.to_bits()),
        ),
        ("NULLIF(-0.0, 0.0)", DataType::Double, Cell::Null),
        (
            "NULLIF(-0.0, 1.0)",
            DataType::Double,
            Cell::Number((-0.0_f64).to_bits()),
        ),
    ] {
        let sql = format!("FROM facts |> SELECT {expression} AS n |> LIMIT 1");
        let prepared = db.prepare(&sql).unwrap();
        let column = prepared.result_column(0).unwrap();
        assert_eq!(column.name, Some("n"));
        assert_eq!(column.data_type, kind);
        assert!(column.nullable);
        drop(prepared);
        query(&db, &sql, vec![vec![expected]]);
    }
}

#[test]
fn public_nullif_preserves_argument_error_order_and_outer_demand() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    for (sql, overflow, source) in [
        (
            "FROM facts |> SELECT v*9223372036854775807 AS bad, k |> SELECT NULLIF(DIV(k, 0), bad) AS n",
            false,
            "NULLIF(DIV(k, 0), bad)",
        ),
        (
            "FROM facts |> WHERE k IS NULL |> SELECT v*9223372036854775807 AS bad, k |> SELECT NULLIF(DIV(k, 0), bad) AS n",
            true,
            "v*9223372036854775807",
        ),
        (
            "FROM facts |> SELECT NULLIF(NULLIF(v, v), DIV(v, 0)) AS n",
            false,
            "NULLIF(NULLIF(v, v), DIV(v, 0))",
        ),
    ] {
        let cancel = CancellationToken::new();
        let prepared = db.prepare(sql).unwrap();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        let mut failed = false;
        for _ in 0..10_000 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Failed(error) => {
                    let span = match error {
                        Error::ArithmeticOverflow { span, .. } if overflow => span,
                        Error::DivisionByZero { span, .. } if !overflow => span,
                        _ => panic!("wrong NULLIF failure: {error}"),
                    };
                    assert_eq!(&sql[span.start()..span.end()], source);
                    failed = true;
                    break;
                }
                _ => panic!("NULLIF skipped its demanded error: {sql}"),
            }
        }
        assert!(failed, "bounded NULLIF failure");
        assert!(matches!(result.step(), QueryStep::Failed(_)));
        drop(result);
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
        query(&db, &format!("{sql} |> LIMIT 0"), vec![]);
        query(&db, "FROM facts |> AGGREGATE COUNT(*) AS n", integers(&[4]));
    }
    query(
        &db,
        "FROM facts |> EXTEND NULLIF(DIV(v, 0), 0) AS unused |> SELECT v |> ORDER BY v",
        integers(&[10, 20, 30, 40]),
    );
}

#[test]
fn public_nullif_retains_stored_double_bits_and_prepared_snapshots() {
    let directory = Directory::new();
    let path = directory.database();
    let db = Database::create_empty(&path, config()).unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "samples",
        &[ColumnDeclaration {
            name: "v",
            data_type: DataType::Double,
            nullable: true,
        }],
        &cancel,
    )
    .unwrap();
    let sql = "FROM samples |> SELECT NULLIF(v, 0.0) AS n";
    let empty = db.prepare(sql).unwrap();
    let bits = [
        0x7ff8_0000_0000_0042,
        0xfff8_0000_0000_0007,
        f64::INFINITY.to_bits(),
        f64::NEG_INFINITY.to_bits(),
        (-0.0_f64).to_bits(),
        0,
        1,
        1.0_f64.to_bits(),
    ];
    let values = bits.map(f64::from_bits);
    let mut append = db.begin_append("samples", limits(), &cancel).unwrap();
    append
        .write(
            &[ColumnInput {
                values: ColumnValues::Double(&values),
                validity: &[0x7f],
            }],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    assert!(collect(&mut db.execute(&empty, &cancel).unwrap()).is_empty());
    drop(empty);
    let expected = vec![
        vec![Cell::Number(bits[0])],
        vec![Cell::Number(bits[1])],
        vec![Cell::Number(bits[2])],
        vec![Cell::Number(bits[3])],
        vec![Cell::Null],
        vec![Cell::Null],
        vec![Cell::Number(1)],
        vec![Cell::Null],
    ];
    query(&db, sql, expected.clone());
    // NaNs do not equal themselves; every other present value does.
    let self_expected = vec![
        vec![Cell::Number(bits[0])],
        vec![Cell::Number(bits[1])],
        vec![Cell::Null],
        vec![Cell::Null],
        vec![Cell::Null],
        vec![Cell::Null],
        vec![Cell::Null],
        vec![Cell::Null],
    ];
    query(
        &db,
        "FROM samples |> SELECT NULLIF(v, v) AS n",
        self_expected.clone(),
    );
    db.close().unwrap();
    let db = Database::open(&path, config()).unwrap();
    query(&db, sql, expected);
    query(
        &db,
        "FROM samples |> SELECT NULLIF(v, v) AS n",
        self_expected,
    );
    db.close().unwrap();
}
