//! Public aggregates contract tests.
use super::*;

#[test]
fn declared_global_aggregates_preserve_types_null_counts_and_pinned_inputs() {
    let directory = Directory::new();
    let path = directory.database();
    let db = Database::create_empty(&path, config()).unwrap();
    let cancel = CancellationToken::new();
    let declarations = [
        ColumnDeclaration {
            name: "n",
            data_type: DataType::Int64,
            nullable: true,
        },
        ColumnDeclaration {
            name: "d",
            data_type: DataType::Double,
            nullable: true,
        },
        ColumnDeclaration {
            name: "other",
            data_type: DataType::Int64,
            nullable: true,
        },
    ];
    db.declare_table("facts", &declarations, &cancel).unwrap();
    let sql = "FROM facts |> AGGREGATE COUNT(*) AS nrows, SUM(n) AS total, AVG(n) AS mean, SUM(d) AS ds, AVG(d) AS dm, SUM(other) AS os, AVG(other) AS om";
    let empty = db.prepare(sql).unwrap();
    assert_eq!(empty.result_column(0).unwrap().data_type, DataType::Int64);
    assert!(!empty.result_column(0).unwrap().nullable);
    assert_eq!(empty.result_column(1).unwrap().data_type, DataType::Int64);
    assert!(empty.result_column(1).unwrap().nullable);
    assert_eq!(empty.result_column(2).unwrap().data_type, DataType::Double);
    assert_eq!(
        collect(&mut db.execute(&empty, &cancel).unwrap()),
        vec![vec![
            Cell::Integer(0),
            Cell::Null,
            Cell::Null,
            Cell::Null,
            Cell::Null,
            Cell::Null,
            Cell::Null,
        ]]
    );
    const LARGE: i64 = 9_007_199_254_740_993;
    for (n, n_valid, d, d_valid, other, other_valid) in [
        (
            [LARGE, 5, i64::MIN],
            3,
            [f64::NAN, 10.0, 20.0],
            6,
            [1, i64::MAX, 3],
            5,
        ),
        (
            [-5, i64::MAX, 2],
            5,
            [30.0, f64::INFINITY, f64::MAX],
            1,
            [i64::MIN, 7, i64::MAX],
            2,
        ),
    ] {
        let mut writer = db.begin_append("facts", limits(), &cancel).unwrap();
        writer
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&n),
                        validity: &[n_valid],
                    },
                    ColumnInput {
                        values: ColumnValues::Double(&d),
                        validity: &[d_valid],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&other),
                        validity: &[other_valid],
                    },
                ],
                &cancel,
            )
            .unwrap();
        writer.commit(&cancel).unwrap();
    }
    // The old query still aggregates its empty generation after both appends.
    assert_eq!(
        collect(&mut db.execute(&empty, &cancel).unwrap())[0][0],
        Cell::Integer(0)
    );
    let query = db.prepare(sql).unwrap();
    let expected = vec![vec![
        Cell::Integer(6),
        Cell::Integer(LARGE + 2),
        Cell::Number(2_251_799_813_685_249.0_f64.to_bits()),
        Cell::Number(60.0_f64.to_bits()),
        Cell::Number(20.0_f64.to_bits()),
        Cell::Integer(11),
        Cell::Number((11.0_f64 / 3.0).to_bits()),
    ]];
    let baseline = db.reserved_memory_bytes();
    assert_eq!(collect(&mut db.execute(&query, &cancel).unwrap()), expected);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    let filtered = db.prepare("FROM facts |> SELECT n AS amount |> WHERE amount > 0 |> AGGREGATE SUM(amount) AS total, COUNT(*) AS nrows |> WHERE total = 9007199254741000 |> SELECT nrows,total").unwrap();
    assert_eq!(
        collect(&mut db.execute(&filtered, &cancel).unwrap()),
        vec![vec![Cell::Integer(3), Cell::Integer(LARGE + 7)]]
    );
    let none = db.prepare("FROM facts |> WHERE n < -100 |> AGGREGATE SUM(n) AS total, AVG(d) AS mean, COUNT(*) AS nrows").unwrap();
    assert_eq!(
        collect(&mut db.execute(&none, &cancel).unwrap()),
        vec![vec![Cell::Null, Cell::Null, Cell::Integer(0)]]
    );
    let grouped = db
        .prepare("FROM facts |> AGGREGATE COUNT(*) AS n GROUP BY other")
        .unwrap();
    assert_eq!(
        collect(&mut db.execute(&grouped, &cancel).unwrap()),
        vec![
            vec![Cell::Null, Cell::Integer(3)],
            vec![Cell::Integer(1), Cell::Integer(1)],
            vec![Cell::Integer(3), Cell::Integer(1)],
            vec![Cell::Integer(7), Cell::Integer(1)],
        ]
    );
    drop(grouped);
    db.declare_table("empty_values", &declarations, &cancel)
        .unwrap();
    let mut writer = db.begin_append("empty_values", limits(), &cancel).unwrap();
    writer
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&[i64::MAX; 3]),
                    validity: &[0],
                },
                ColumnInput {
                    values: ColumnValues::Double(&[f64::NAN; 3]),
                    validity: &[0],
                },
                ColumnInput {
                    values: ColumnValues::Int64(&[i64::MIN; 3]),
                    validity: &[0],
                },
            ],
            &cancel,
        )
        .unwrap();
    writer.commit(&cancel).unwrap();
    let nulls = db.prepare("FROM empty_values |> AGGREGATE COUNT(*) AS nrows,SUM(n) AS total,AVG(n) AS mean,SUM(d) AS ds,AVG(d) AS dm").unwrap();
    assert_eq!(
        collect(&mut db.execute(&nulls, &cancel).unwrap()),
        vec![vec![
            Cell::Integer(3),
            Cell::Null,
            Cell::Null,
            Cell::Null,
            Cell::Null
        ]]
    );
    drop(nulls);
    let token = CancellationToken::new();
    let mut result = db.execute(&query, &token).unwrap();
    assert!(matches!(result.step(), QueryStep::Progress));
    token.cancel();
    assert!(matches!(result.step(), QueryStep::Failed(Error::Cancelled)));
    drop(result);
    drop((empty, query, filtered, none));
    db.close().unwrap();
    let db = Database::open(&path, config()).unwrap();
    let query = db.prepare(sql).unwrap();
    assert_eq!(collect(&mut db.execute(&query, &cancel).unwrap()), expected);
    drop(query);
    db.close().unwrap();
    let query_path = directory.0.join("aggregate.sql");
    std::fs::write(&query_path, sql).unwrap();
    let cli = std::process::Command::new(env!("CARGO_BIN_EXE_pipesql"))
        .arg("query")
        .arg("--database")
        .arg(&path)
        .arg("--query-file")
        .arg(&query_path)
        .args([
            "--memory-limit-bytes",
            "4000000",
            "--temp-limit-bytes",
            "2000000",
        ])
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    let stdout = String::from_utf8(cli.stdout).unwrap();
    assert!(stdout.contains("status=queried"));
    let rows: Vec<_> = stdout
        .lines()
        .filter(|line| line.starts_with("row="))
        .collect();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].starts_with("row=int64:6|int64:9007199254740995|"));
}

#[test]
fn declared_integer_sum_overflow_is_final_and_demanded() {
    let directory = Directory::new();
    let db = Database::create_empty(&directory.database(), config()).unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "facts",
        &[ColumnDeclaration {
            name: "n",
            data_type: DataType::Int64,
            nullable: true,
        }],
        &cancel,
    )
    .unwrap();
    let mut writer = db.begin_append("facts", limits(), &cancel).unwrap();
    writer
        .write(
            &[ColumnInput {
                values: ColumnValues::Int64(&[i64::MAX, i64::MAX, 0]),
                validity: &[3],
            }],
            &cancel,
        )
        .unwrap();
    writer.commit(&cancel).unwrap();
    for (sql, fragment, operation) in [
        ("FROM facts |> AGGREGATE SUM(n) AS total", "SUM(n)", "SUM"),
        (
            "FROM facts |> AGGREGATE AVG(n) AS mean, SUM(n) AS total |> WHERE total > 0 |> SELECT mean",
            "SUM(n)",
            "SUM",
        ),
        (
            "FROM facts |> AGGREGATE SUM(n+1) AS total",
            "SUM(n+1)",
            "addition",
        ),
        (
            "# 雪\nFROM facts |> AGGREGATE SUM(n+1) AS unused,AVG(n+1) AS mean |> SELECT mean",
            "AVG(n+1)",
            "addition",
        ),
        (
            "FROM facts |> AGGREGATE AVG(n) AS mean,SUM(n) AS total |> ORDER BY total |> LIMIT 1 |> SELECT mean",
            "SUM(n)",
            "SUM",
        ),
        (
            "FROM facts |> AGGREGATE SUM(n) AS safe,SUM(n*2) AS bad |> ORDER BY bad |> LIMIT 1 |> SELECT safe",
            "SUM(n*2)",
            "multiplication",
        ),
    ] {
        let query = db.prepare(sql).unwrap();
        let baseline = db.reserved_memory_bytes();
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut failed = false;
        for _ in 0..128 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Failed(Error::ArithmeticOverflow {
                    operation: actual,
                    span,
                }) => {
                    assert_eq!(*actual, operation);
                    assert_eq!(span.start(), sql.find(fragment).unwrap());
                    assert_eq!(&sql[span.start()..span.end()], fragment);
                    failed = true;
                    break;
                }
                _ => panic!("overflow must fail before output for {sql}"),
            }
        }
        assert!(failed);
        drop(result);
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    let query = db.prepare("FROM facts |> AGGREGATE SUM(n) AS total, AVG(n) AS mean, COUNT(*) AS nrows |> SELECT mean,nrows").unwrap();
    assert_eq!(
        collect(&mut db.execute(&query, &cancel).unwrap()),
        vec![vec![
            Cell::Number((i64::MAX as f64).to_bits()),
            Cell::Integer(3)
        ]]
    );
    drop(query);
    let mut writer = db.begin_append("facts", limits(), &cancel).unwrap();
    writer
        .write(
            &[ColumnInput {
                values: ColumnValues::Int64(&[-i64::MAX]),
                validity: &[1],
            }],
            &cancel,
        )
        .unwrap();
    writer.commit(&cancel).unwrap();
    let query = db
        .prepare("FROM facts |> AGGREGATE SUM(n) AS total")
        .unwrap();
    assert_eq!(
        collect(&mut db.execute(&query, &cancel).unwrap()),
        vec![vec![Cell::Integer(i64::MAX)]]
    );
}

#[test]
fn declared_nullable_double_aggregates_preserve_exceptional_values() {
    for (values, validity, sql, expected) in [
        (
            [f64::MAX, f64::NAN, f64::MAX, f64::INFINITY],
            5,
            "FROM facts |> AGGREGATE SUM(v) AS total,AVG(v) AS mean |> SELECT mean",
            f64::MAX,
        ),
        (
            [f64::MAX, f64::MAX, 0.0, f64::NAN],
            11,
            "FROM facts |> AGGREGATE SUM(v) AS total",
            f64::NAN,
        ),
        (
            [f64::MAX, f64::MAX, -f64::MAX, f64::NAN],
            7,
            "FROM facts |> AGGREGATE SUM(v) AS total",
            f64::MAX,
        ),
        (
            [f64::INFINITY, f64::MAX, f64::NAN, 0.0],
            3,
            "FROM facts |> AGGREGATE SUM(v) AS total",
            f64::INFINITY,
        ),
        (
            [-0.0, f64::NAN, f64::MAX, 0.0],
            1,
            "FROM facts |> AGGREGATE SUM(v) AS total",
            -0.0,
        ),
    ] {
        let directory = Directory::new();
        let db = Database::create_empty(&directory.database(), config()).unwrap();
        let cancel = CancellationToken::new();
        db.declare_table(
            "facts",
            &[ColumnDeclaration {
                name: "v",
                data_type: DataType::Double,
                nullable: true,
            }],
            &cancel,
        )
        .unwrap();
        let mut writer = db.begin_append("facts", limits(), &cancel).unwrap();
        writer
            .write(
                &[ColumnInput {
                    values: ColumnValues::Double(&values),
                    validity: &[validity],
                }],
                &cancel,
            )
            .unwrap();
        writer.commit(&cancel).unwrap();
        let query = db.prepare(sql).unwrap();
        let output = collect(&mut db.execute(&query, &cancel).unwrap());
        let Cell::Number(bits) = output[0][0] else {
            panic!("expected DOUBLE");
        };
        if expected.is_nan() {
            assert!(f64::from_bits(bits).is_nan());
        } else {
            assert_eq!(bits, expected.to_bits());
        }
    }
}
