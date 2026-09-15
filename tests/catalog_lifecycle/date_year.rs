use super::*;

// Literal Gregorian boundaries, independently checked with Python's datetime.
// Epoch offsets are input data; expected years never call the engine decoder.
const DAYS: [i32; 17] = [
    -719_162, -718_798, -718_008, -682_944, -573_372, -25_568, -25_509, -25_508, -1, 0, 10_956,
    11_016, 11_322, 11_323, 16_801, 2_932_896, 0,
];
const YEARS: [i64; 16] = [
    1, 1, 4, 100, 400, 1899, 1900, 1900, 1969, 1970, 1999, 2000, 2000, 2001, 2016, 9999,
];

fn fixture() -> (Directory, Database) {
    let directory = Directory::new();
    let db = Database::create_empty(&directory.database(), config()).unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "calendar",
        &[
            ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "day",
                data_type: DataType::Date,
                nullable: true,
            },
        ],
        &cancel,
    )
    .unwrap();
    let dates = DAYS.map(|day| DateValue::from_days_since_unix_epoch(day).unwrap());
    let ids = std::array::from_fn::<_, 17, _>(|index| index as i64);
    let mut append = db.begin_append("calendar", limits(), &cancel).unwrap();
    append
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&ids),
                    validity: &[255, 255, 1],
                },
                ColumnInput {
                    values: ColumnValues::Date(&dates),
                    validity: &[255, 255, 0],
                },
            ],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    (directory, db)
}

#[test]
fn date_year_preserves_dates_across_scan_sort_and_reopen() {
    let (directory, db) = fixture();
    let expected = DAYS
        .into_iter()
        .enumerate()
        .map(|(index, day)| {
            if index == 16 {
                vec![Cell::Null, Cell::Null]
            } else {
                vec![Cell::Day(day), Cell::Integer(YEARS[index])]
            }
        })
        .collect::<Vec<_>>();
    let queries = [
        "FROM calendar |> SELECT id, day, EXTRACT(YEAR FROM day) AS y |> ORDER BY id |> SELECT day, y",
        "FROM calendar |> ORDER BY id |> SELECT day, EXTRACT(YEAR FROM day) AS y",
        "FROM (FROM calendar |> ORDER BY id) |> SELECT day, (extract(year from (day))) AS y",
    ];
    for sql in queries {
        let prepared = db.prepare(sql).unwrap();
        assert_eq!(
            prepared.result_column(1).unwrap().data_type,
            DataType::Int64
        );
        assert!(prepared.result_column(1).unwrap().nullable);
        drop(prepared);
        query(&db, sql, expected.clone());
    }
    db.close().unwrap();
    let db = Database::open(&directory.database(), config()).unwrap();
    query(&db, queries[0], expected);
}

#[test]
fn date_year_folds_owned_constants_and_reuses_date_shifts() {
    let (_directory, db) = fixture();
    let prepared = {
        let sql = String::from(
            "FROM calendar |> LIMIT 1 |> SELECT EXTRACT(YEAR FROM DATE '0001-01-01') AS a, EXTRACT(YEAR FROM DATE_ADD(DATE '1999-12-31', INTERVAL 1 DAY)) AS b, EXTRACT(YEAR FROM DATE_SUB(DATE_ADD(DATE '2000-02-29', INTERVAL 1 YEAR), INTERVAL 1 YEAR)) AS c, EXTRACT(YEAR FROM DATE '9999-12-31') AS d",
        );
        db.prepare(&sql).unwrap()
    };
    for index in 0..4 {
        let column = prepared.result_column(index).unwrap();
        assert_eq!(column.data_type, DataType::Int64);
        assert!(!column.nullable);
    }
    let cancel = CancellationToken::new();
    let mut result = db.execute(&prepared, &cancel).unwrap();
    assert_eq!(
        collect_unordered(&mut result),
        vec![vec![
            Cell::Integer(1),
            Cell::Integer(2000),
            Cell::Integer(2000),
            Cell::Integer(9999)
        ]]
    );
    drop(result);
    drop(prepared);
    let mut date = String::from("DATE '1999-12-31'");
    for _ in 0..8 {
        date = format!("DATE_ADD({date}, INTERVAL 1 DAY)");
    }
    query(
        &db,
        &format!("FROM calendar |> LIMIT 1 |> SELECT EXTRACT(YEAR FROM {date}) AS y"),
        integers(&[2000]),
    );
    date = format!("DATE_ADD({date}, INTERVAL 1 DAY)");
    assert!(matches!(
        db.prepare(&format!(
            "FROM calendar |> SELECT EXTRACT(YEAR FROM {date}) AS y"
        )),
        Err(Error::Parse { .. })
    ));
    for prefix in ["FROM calendar", "FROM calendar |> ORDER BY id"] {
        query(
            &db,
            &format!(
                "{prefix} |> SELECT DATE '2016-01-01' AS d |> SELECT EXTRACT(YEAR FROM d) AS y |> AGGREGATE SUM(y) AS total"
            ),
            integers(&[34_272]),
        );
    }
}

#[test]
fn date_year_composes_with_identity_demand_and_numeric_consumers() {
    let (_directory, db) = nullable_facts().unwrap();
    query(
        &db,
        "FROM facts AS f |> SET d = EXTRACT(YEAR FROM d) |> ORDER BY id |> SELECT f.d, d",
        vec![
            vec![Cell::Day(0), Cell::Integer(1970)],
            vec![Cell::Day(0), Cell::Integer(1970)],
            vec![Cell::Day(0), Cell::Integer(1970)],
            vec![Cell::Null, Cell::Null],
        ],
    );
    query(
        &db,
        "FROM facts |> EXTEND EXTRACT(YEAR FROM d) AS y |> WHERE id < 1 OR y = 1970 |> SELECT COALESCE(y, 1) AS chosen |> AGGREGATE SUM(chosen) AS total",
        integers(&[5910]),
    );
    query(
        &db,
        "FROM facts |> EXTEND EXTRACT(YEAR FROM d) AS y |> SELECT COALESCE(1, y) AS chosen, y + 1 AS next_year |> ORDER BY next_year NULLS FIRST",
        vec![
            vec![Cell::Integer(1), Cell::Null],
            vec![Cell::Integer(1), Cell::Integer(1971)],
            vec![Cell::Integer(1), Cell::Integer(1971)],
            vec![Cell::Integer(1), Cell::Integer(1971)],
        ],
    );
    query(
        &db,
        "FROM facts |> SELECT EXTRACT(YEAR FROM d) AS y |> AGGREGATE COUNT(*) AS entries, COUNT(y) AS present, SUM(y) AS total",
        vec![vec![
            Cell::Integer(4),
            Cell::Integer(3),
            Cell::Integer(5910),
        ]],
    );
}

#[test]
fn date_year_crosses_join_group_and_set_materialization() {
    let (_directory, db) = nullable_facts().unwrap();
    query(
        &db,
        "FROM facts AS a |> LEFT JOIN (FROM facts |> WHERE id = 0) AS b ON a.id = b.id |> ORDER BY a.id |> SELECT EXTRACT(YEAR FROM b.d) AS y",
        vec![
            vec![Cell::Integer(1970)],
            vec![Cell::Null],
            vec![Cell::Null],
            vec![Cell::Null],
        ],
    );
    query(
        &db,
        "FROM facts |> AGGREGATE COUNT(*) AS n GROUP BY d |> SELECT EXTRACT(YEAR FROM d) AS y |> ORDER BY y NULLS FIRST",
        vec![vec![Cell::Null], vec![Cell::Integer(1970)]],
    );
    query(
        &db,
        "FROM facts |> SELECT d |> UNION DISTINCT (FROM facts |> SELECT DATE '2000-02-29' AS d) |> SELECT EXTRACT(YEAR FROM d) AS y |> ORDER BY y NULLS FIRST",
        vec![
            vec![Cell::Null],
            vec![Cell::Integer(1970)],
            vec![Cell::Integer(2000)],
        ],
    );
    query(
        &db,
        "FROM facts |> SELECT EXTRACT(YEAR FROM d) AS y |> AGGREGATE COUNT(*) AS n GROUP AND ORDER BY y |> SELECT y, n",
        vec![
            vec![Cell::Null, Cell::Integer(1)],
            vec![Cell::Integer(1970), Cell::Integer(3)],
        ],
    );
}

#[test]
fn date_year_rejects_unsupported_parts_types_and_expressions_with_owned_spans() {
    let (_directory, db) = nullable_facts().unwrap();
    let baseline = db.reserved_memory_bytes();
    for (expression, token) in [
        ("EXTRACT(MONTH FROM d)", "MONTH"),
        ("EXTRACT(ISOYEAR FROM d)", "ISOYEAR"),
        ("EXTRACT(YEAR FROM i)", "i"),
        ("EXTRACT(YEAR FROM n)", "n"),
        ("EXTRACT(YEAR FROM s)", "s"),
        ("EXTRACT(YEAR FROM DATE '1900-02-29')", "'1900-02-29'"),
    ] {
        let sql = format!("FROM facts |> SELECT {expression} AS y");
        let start = sql.find(expression).unwrap() + expression.find(token).unwrap();
        let error = db.prepare(&sql).err().expect("unsupported extraction");
        drop(sql);
        match error {
            Error::Parse { span, .. } | Error::Bind { span, .. } => {
                assert_eq!((span.start(), span.end()), (start, start + token.len()))
            }
            other => panic!("{other}"),
        }
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    for expression in [
        "EXTRACT(YEAR FROM NULL)",
        "EXTRACT(YEAR FROM '2000-01-01')",
        "EXTRACT(YEAR FROM d, d)",
        "EXTRACT(YEAR FROM EXTRACT(YEAR FROM d))",
        "EXTRACT(YEAR FROM DATE_ADD(d, INTERVAL 1 YEAR))",
        "EXTRACT(YEAR FROM d) + 1",
        "EXTRACT(YEAR d)",
        "EXTRACT(YEAR FROM DATE_ADD(DATE '9999-12-31', INTERVAL 1 DAY))",
        "EXTRACT(YEAR FROM DATE_SUB(DATE '0001-01-01', INTERVAL 1 DAY))",
    ] {
        assert!(
            matches!(
                db.prepare(&format!("FROM facts |> SELECT {expression} AS y")),
                Err(Error::Parse { .. } | Error::Bind { .. })
            ),
            "{expression}"
        );
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
}

#[test]
fn date_year_cancellation_and_abandonment_release_owners() {
    let (_directory, db) = fixture();
    let baseline = db.reserved_memory_bytes();
    for sql in [
        "FROM calendar |> SELECT EXTRACT(YEAR FROM day) AS y",
        "FROM calendar |> ORDER BY id |> SELECT EXTRACT(YEAR FROM day) AS y",
        "FROM calendar |> SELECT EXTRACT(YEAR FROM day) AS y |> AGGREGATE COUNT(*) AS n GROUP BY y",
    ] {
        let prepared = db.prepare(sql).unwrap();
        let prepared_memory = db.reserved_memory_bytes();
        let cancel = CancellationToken::new();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        cancel.cancel();
        assert!(matches!(result.step(), QueryStep::Failed(Error::Cancelled)));
        assert!(matches!(result.step(), QueryStep::Failed(Error::Cancelled)));
        drop(result);
        assert_eq!(db.reserved_memory_bytes(), prepared_memory);
        let cancel = CancellationToken::new();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        let mut saw_rows = false;
        for _ in 0..4096 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Rows(_) => {
                    saw_rows = true;
                    break;
                }
                _ => panic!("expected a row before abandonment"),
            }
        }
        assert!(saw_rows);
        drop(result);
        assert_eq!(db.reserved_memory_bytes(), prepared_memory);
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}

#[test]
fn date_year_logical_plan_exposes_the_typed_conversion() {
    let (_directory, db) = fixture();
    let prepared = db
        .prepare("FROM calendar |> SELECT EXTRACT(YEAR FROM day) AS y")
        .unwrap();
    assert_eq!(
        prepared.logical_plan().to_string(),
        concat!(
            "logical plan\n",
            "r0 = source occurrence=0 columns=[c1, c2]\n",
            "r1 = select input=r0 columns=[c3]\n",
            "c3 = extract year c2 input=r0 type=INT64 nullable\n",
            "result = r1\n",
            "output[0] = c3 name=Some(\"y\") type=INT64 nullable\n",
        )
    );
}

#[test]
fn date_year_numeric_errors_keep_demand_and_owned_spans() {
    let (_directory, db) = fixture();
    let baseline = db.reserved_memory_bytes();
    let prefix = "FROM calendar |> EXTEND EXTRACT(YEAR FROM day) AS y |> EXTEND y + 9223372036854775807 AS bad";
    query(
        &db,
        &format!("{prefix} |> SELECT COALESCE(1, bad) AS chosen |> AGGREGATE SUM(chosen) AS total"),
        integers(&[17]),
    );
    query(
        &db,
        &format!("{prefix} |> WHERE id IS NOT NULL OR bad > 0 |> AGGREGATE COUNT(*) AS n"),
        integers(&[17]),
    );
    let sql = format!("{prefix} |> SELECT SAFE_DIVIDE(bad, 0) AS n");
    let start = sql.find("y + 9223372036854775807").unwrap();
    let end = start + "y + 9223372036854775807".len();
    let prepared = db.prepare(&sql).unwrap();
    drop(sql);
    let cancel = CancellationToken::new();
    let mut result = db.execute(&prepared, &cancel).unwrap();
    let mut failed = false;
    for _ in 0..4096 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Failed(Error::ArithmeticOverflow { .. }) => {
                failed = true;
                break;
            }
            _ => panic!("expected demanded year addition overflow"),
        }
    }
    assert!(failed);
    let error = result.into_error().unwrap();
    drop(prepared);
    let Error::ArithmeticOverflow {
        operation: "addition",
        span,
    } = error
    else {
        panic!("owned addition failure");
    };
    assert_eq!((span.start(), span.end()), (start, end));
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
}
