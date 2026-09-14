use super::order::{integers, query};
use super::*;

fn fixture(values: ColumnValues<'_>, validity: &[u8]) -> (Directory, Database) {
    let (kind, count) = match &values {
        ColumnValues::Int64(values) => (DataType::Int64, values.len()),
        ColumnValues::Double(values) => (DataType::Double, values.len()),
        _ => panic!("numeric fixture"),
    };
    let directory = Directory::new();
    let db = Database::create_empty(&directory.database(), config()).unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "casts",
        &[
            ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "x",
                data_type: kind,
                nullable: true,
            },
        ],
        &cancel,
    )
    .unwrap();
    let ids: Vec<i64> = (0..count as i64).collect();
    let mut id_validity = vec![0; count.div_ceil(8)];
    for row in 0..count {
        id_validity[row / 8] |= 1 << (row % 8);
    }
    let mut append = db.begin_append("casts", limits(), &cancel).unwrap();
    append
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&ids),
                    validity: &id_validity,
                },
                ColumnInput { values, validity },
            ],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    (directory, db)
}

#[test]
fn public_cast_rounds_integer_boundaries_and_preserves_nullable_schema() {
    let (directory, db) = fixture(
        ColumnValues::Int64(&[
            0,
            1,
            -1,
            9_007_199_254_740_991,
            9_007_199_254_740_992,
            9_007_199_254_740_993,
            9_007_199_254_740_994,
            9_007_199_254_740_995,
            -9_007_199_254_740_993,
            -9_007_199_254_740_995,
            i64::MIN,
            i64::MAX,
            i64::MAX,
        ]),
        &[255, 15],
    );
    let expected = [
        Some(0x0000_0000_0000_0000),
        Some(0x3ff0_0000_0000_0000),
        Some(0xbff0_0000_0000_0000),
        Some(0x433f_ffff_ffff_ffff),
        Some(0x4340_0000_0000_0000),
        Some(0x4340_0000_0000_0000),
        Some(0x4340_0000_0000_0001),
        Some(0x4340_0000_0000_0002),
        Some(0xc340_0000_0000_0000),
        Some(0xc340_0000_0000_0002),
        Some(0xc3e0_0000_0000_0000),
        Some(0x43e0_0000_0000_0000),
        None,
    ]
    .map(|bits| vec![bits.map_or(Cell::Null, Cell::Number)])
    .to_vec();
    for target in ["FLOAT64", "DOUBLE"] {
        for sql in [
            format!("FROM casts |> SELECT id, CAST(x AS {target}) AS n |> ORDER BY id |> SELECT n"),
            format!("FROM casts |> ORDER BY id |> SELECT CAST(x AS {target}) AS n"),
            format!("FROM (FROM casts |> ORDER BY id) |> SELECT CAST(x AS {target}) AS n"),
        ] {
            let prepared = db.prepare(&sql).unwrap();
            let column = prepared.result_column(0).unwrap();
            assert_eq!(column.data_type, DataType::Double);
            assert!(column.nullable);
            drop(prepared);
            query(&db, &sql, expected.clone());
        }
    }
    // Conversion can merge distinct INT64 grouping keys after precision is lost.
    query(
        &db,
        "FROM casts |> WHERE id >= 4 AND id <= 5 |> SELECT CAST(x AS FLOAT64) AS n |> AGGREGATE COUNT(*) AS entries GROUP BY n",
        vec![vec![Cell::Number(0x4340_0000_0000_0000), Cell::Integer(2)]],
    );
    db.close().unwrap();
    let db = Database::open(&directory.database(), config()).unwrap();
    query(
        &db,
        "FROM casts |> ORDER BY id |> SELECT CAST(x AS FLOAT64) AS n",
        expected,
    );
}

#[test]
fn public_cast_preserves_double_bits_through_scan_and_materialization() {
    let bits = [
        0x0000_0000_0000_0000,
        0x8000_0000_0000_0000,
        0x0000_0000_0000_0001,
        0x8000_0000_0000_0001,
        0x3ff0_0000_0000_0000,
        0xbff0_0000_0000_0000,
        0x7fef_ffff_ffff_ffff,
        0xffef_ffff_ffff_ffff,
        0x7ff0_0000_0000_0000,
        0xfff0_0000_0000_0000,
        0x7ff0_0000_0000_0042,
        0xfff8_0000_0000_0042,
        0x7ff0_0000_0000_0000,
    ];
    let values = bits.map(f64::from_bits);
    let (_directory, db) = fixture(ColumnValues::Double(&values), &[255, 15]);
    let expected = bits
        .into_iter()
        .enumerate()
        .map(|(index, bits)| {
            vec![if index == 12 {
                Cell::Null
            } else {
                Cell::Number(bits)
            }]
        })
        .collect::<Vec<_>>();
    for sql in [
        "FROM casts |> SELECT id, CAST(x AS DOUBLE) AS n |> ORDER BY id |> SELECT n",
        "FROM casts |> ORDER BY id |> SELECT CAST(x AS FLOAT64) AS n",
        "FROM casts |> ORDER BY id |> SELECT CAST(COALESCE(x, x) AS DOUBLE) AS n",
    ] {
        query(&db, sql, expected.clone());
    }
}

#[test]
fn public_cast_owns_literals_and_composes_with_typed_identities() {
    let (_directory, db) = super::null_predicate::fixture().unwrap();
    let prepared = {
        let sql = String::from(
            "FROM facts |> SELECT CAST(9007199254740993 AS FLOAT64) AS wide, CAST(1 AS DOUBLE) + 2 AS n, (CAST((CAST(1 AS DOUBLE) + 2) AS FLOAT64)) * 2 AS nested",
        );
        db.prepare(&sql).unwrap()
    };
    for index in 0..3 {
        let column = prepared.result_column(index).unwrap();
        assert_eq!(column.data_type, DataType::Double);
        assert!(!column.nullable);
    }
    let cancel = CancellationToken::new();
    let mut result = db.execute(&prepared, &cancel).unwrap();
    assert_eq!(
        collect(&mut result),
        vec![
            vec![
                Cell::Number(0x4340_0000_0000_0000),
                Cell::Number(0x4008_0000_0000_0000),
                Cell::Number(0x4018_0000_0000_0000),
            ];
            4
        ]
    );
    drop(result);
    drop(prepared);
    query(
        &db,
        "FROM facts |> SELECT CAST(SAFE_DIVIDE(i, 0) AS FLOAT64) AS n",
        vec![vec![Cell::Null]; 4],
    );
    let nonnull = db
        .prepare("FROM facts |> SELECT CAST(COALESCE(i, 9) AS DOUBLE) AS n")
        .unwrap();
    assert_eq!(
        nonnull.result_column(0).unwrap().data_type,
        DataType::Double
    );
    assert!(!nonnull.result_column(0).unwrap().nullable);
    drop(nonnull);
    query(
        &db,
        "FROM facts |> SELECT CAST(COALESCE(i, 9) AS DOUBLE) AS n",
        vec![
            vec![Cell::Number(0)],
            vec![Cell::Number(0x401c_0000_0000_0000)],
            vec![Cell::Number(0x4022_0000_0000_0000)],
            vec![Cell::Number(0x4022_0000_0000_0000)],
        ],
    );
    query(
        &db,
        "FROM facts AS f |> SET i = cAsT(i AS float64) |> ORDER BY id |> SELECT f.i, i",
        vec![
            vec![Cell::Integer(0), Cell::Number(0)],
            vec![Cell::Integer(7), Cell::Number(0x401c_0000_0000_0000)],
            vec![Cell::Null, Cell::Null],
            vec![Cell::Integer(9), Cell::Number(0x4022_0000_0000_0000)],
        ],
    );
    query(
        &db,
        "FROM facts |> SELECT CAST(i AS FLOAT64) AS n |> UNION ALL (FROM facts |> SELECT CAST(id AS DOUBLE) AS n) |> AGGREGATE COUNT(*) AS entries, COUNT(n) AS present, SUM(n) AS total",
        vec![vec![
            Cell::Integer(8),
            Cell::Integer(7),
            Cell::Number(0x4036_0000_0000_0000),
        ]],
    );
    query(
        &db,
        "FROM facts AS a |> LEFT JOIN (FROM facts |> WHERE id = 0) AS b ON a.id = b.id |> ORDER BY a.id |> SELECT CAST(b.i AS DOUBLE) AS n",
        vec![
            vec![Cell::Number(0)],
            vec![Cell::Null],
            vec![Cell::Null],
            vec![Cell::Null],
        ],
    );
    query(
        &db,
        "FROM facts |> SELECT BYTE_LENGTH(s) AS width |> SELECT CAST(width AS FLOAT64) AS n |> AGGREGATE SUM(n) AS total",
        vec![vec![Cell::Number(0x4022_0000_0000_0000)]],
    );
    query(
        &db,
        "FROM facts |> AGGREGATE SUM(CAST(i AS DOUBLE)) AS total",
        vec![vec![Cell::Number(0x4030_0000_0000_0000)]],
    );
}

#[test]
fn public_cast_rejects_wrong_grammar_types_and_targets_with_owned_spans() {
    let (_directory, db) = super::null_predicate::fixture().unwrap();
    let baseline = db.reserved_memory_bytes();
    for (expression, exact) in [
        ("CAST()", None),
        ("CAST(i)", None),
        ("CAST(i AS)", None),
        ("CAST(i AS INT64)", Some("INT64")),
        ("CAST(i AS STRING)", Some("STRING")),
        ("CAST(i AS DATE)", Some("DATE")),
        ("CAST(i AS BOOL)", Some("BOOL")),
        ("CAST(i AS FLOAT)", Some("FLOAT")),
        ("CAST(i AS FLOAT32)", Some("FLOAT32")),
        ("CAST(i AS NUMERIC)", Some("NUMERIC")),
        ("CAST(i AS BIGNUMERIC)", Some("BIGNUMERIC")),
        ("CAST(s AS DOUBLE)", Some("s")),
        ("CAST(d AS FLOAT64)", Some("d")),
        ("CAST(NULL AS DOUBLE)", None),
        ("CAST('1' AS FLOAT64)", None),
        ("CAST(i, i AS DOUBLE)", None),
        ("CAST((i AS DOUBLE))", None),
        ("CAST(i AS DOUBLE, DOUBLE)", None),
        ("CAST(i AS DOUBLE(3))", None),
        ("CAST(i AS 'DOUBLE')", None),
        ("CAST(i AS DOUBLE FORMAT 'x')", None),
        ("SAFE_CAST(i AS DOUBLE)", None),
        ("CAST(BYTE_LENGTH(s) AS DOUBLE)", None),
    ] {
        let sql = format!("FROM facts |> EXTEND {expression} AS unused |> LIMIT 0 |> SELECT id");
        let error = match db.prepare(&sql) {
            Ok(_) => panic!("unsupported cast: {sql}"),
            Err(error) => error,
        };
        let span = match &error {
            Error::Parse { span, .. } | Error::Bind { span, .. } => *span,
            other => panic!("{other}"),
        };
        assert!(span.start() <= span.end() && span.end() <= sql.len());
        assert!(sql.is_char_boundary(span.start()) && sql.is_char_boundary(span.end()));
        if let Some(exact) = exact {
            assert_eq!(&sql[span.start()..span.end()], exact, "{sql}");
        }
        drop(sql);
        assert!(!error.to_string().is_empty());
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
}

#[test]
fn public_cast_preserves_error_timing_and_conditional_demand() {
    let (_directory, db) = fixture(ColumnValues::Int64(&[i64::MAX, i64::MAX]), &[1]);
    for sql in [
        "FROM casts |> SELECT CAST(x AS FLOAT64) + 1 AS n",
        "FROM casts |> ORDER BY id |> SELECT CAST(x AS DOUBLE) + 1 AS n",
    ] {
        query(
            &db,
            sql,
            vec![vec![Cell::Number(0x43e0_0000_0000_0000)], vec![Cell::Null]],
        );
    }
    for sql in [
        "FROM casts |> SELECT COALESCE(1, CAST(x + 1 AS FLOAT64)) AS n",
        "FROM casts |> ORDER BY id |> SELECT COALESCE(1, CAST(x + 1 AS DOUBLE)) AS n",
    ] {
        query(&db, sql, vec![vec![Cell::Number(0x3ff0_0000_0000_0000)]; 2]);
    }
    query(
        &db,
        "FROM casts |> EXTEND CAST(x + 1 AS DOUBLE) AS n |> WHERE id < 0 AND n > 0 |> SELECT id",
        vec![],
    );
    query(
        &db,
        "FROM casts |> EXTEND CAST(x + 1 AS DOUBLE) AS n |> WHERE id >= 0 OR n > 0 |> AGGREGATE COUNT(*) AS entries",
        integers(&[2]),
    );
    for prefix in ["FROM casts", "FROM casts |> ORDER BY id"] {
        for expression in [
            "CAST(x + 1 AS DOUBLE)",
            "SAFE_DIVIDE(1, CAST(x + 1 AS FLOAT64))",
        ] {
            owned_addition_failure(
                &db,
                format!("{prefix} |> SELECT {expression} AS n"),
                expression,
            );
        }
    }
    let sql = "FROM casts |> WHERE id > CAST(9223372036854775807 + 1 AS DOUBLE) |> SELECT id";
    assert!(matches!(
        db.prepare(sql),
        Err(Error::ArithmeticOverflow {
            operation: "addition",
            ..
        })
    ));
}

fn owned_addition_failure(db: &Database, sql: String, expression: &str) {
    let baseline = db.reserved_memory_bytes();
    let start = sql.find(expression).unwrap();
    let end = start + expression.len();
    let prepared = db.prepare(&sql).unwrap();
    drop(sql);
    let cancel = CancellationToken::new();
    let mut result = db.execute(&prepared, &cancel).unwrap();
    let mut failed = false;
    for _ in 0..512 {
        match result.step() {
            QueryStep::Progress | QueryStep::Rows(_) => (),
            QueryStep::Failed(Error::ArithmeticOverflow {
                operation: "addition",
                ..
            }) => {
                failed = true;
                break;
            }
            QueryStep::Failed(error) => panic!("expected addition failure: {error}"),
            QueryStep::Finished => panic!("expected addition failure before completion"),
        }
    }
    assert!(failed);
    assert!(matches!(
        result.step(),
        QueryStep::Failed(Error::ArithmeticOverflow { .. })
    ));
    let error = result.into_error().unwrap();
    drop(prepared);
    let Error::ArithmeticOverflow { operation, span } = error else {
        panic!("owned addition failure");
    };
    assert_eq!(operation, "addition");
    assert_eq!((span.start(), span.end()), (start, end));
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
}

#[test]
fn public_cast_cancellation_and_abandonment_release_query_owners() {
    let (_directory, db) = super::null_predicate::fixture().unwrap();
    let baseline = db.reserved_memory_bytes();
    for sql in [
        "FROM facts |> SELECT CAST(i AS FLOAT64) AS n",
        "FROM facts |> ORDER BY id |> SELECT CAST(i AS DOUBLE) AS n",
    ] {
        let prepared = db.prepare(sql).unwrap();
        let retained = db.reserved_memory_bytes();
        // Cancel during progress, cancel after lending rows, then abandon rows.
        for mode in 0..3 {
            let cancel = CancellationToken::new();
            let mut result = db.execute(&prepared, &cancel).unwrap();
            let mut stopped = false;
            for _ in 0..512 {
                match result.step() {
                    QueryStep::Progress => {
                        if mode == 0 {
                            cancel.cancel();
                        }
                    }
                    QueryStep::Rows(batch) => {
                        assert!(!batch.is_empty());
                        if mode == 2 {
                            stopped = true;
                            break;
                        }
                        assert_eq!(mode, 1, "progress cancellation must precede output");
                        cancel.cancel();
                    }
                    QueryStep::Failed(Error::Cancelled) => {
                        assert!(cancel.is_cancelled());
                        assert!(matches!(result.step(), QueryStep::Failed(Error::Cancelled)));
                        stopped = true;
                        break;
                    }
                    QueryStep::Finished => panic!("expected cancellation or abandonment"),
                    QueryStep::Failed(error) => panic!("{error}"),
                }
            }
            assert!(stopped);
            drop(result);
            assert_eq!(db.reserved_memory_bytes(), retained);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    query(
        &db,
        "FROM facts |> AGGREGATE SUM(CAST(i AS DOUBLE)) AS total",
        vec![vec![Cell::Number(0x4030_0000_0000_0000)]],
    );
}
