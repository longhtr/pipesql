use super::order::{integers, query};
use super::*;

#[test]
fn byte_length_counts_utf8_bytes_nulls_and_maximal_values() {
    let (directory, db) = super::text_filter::fixture();
    let expected = [
        Some(0),
        Some(1),
        Some(2),
        Some(3),
        Some(9),
        Some(10),
        Some(4),
        None,
        Some(1),
        Some(65_536),
    ]
    .into_iter()
    .map(|value| vec![value.map_or(Cell::Null, Cell::Integer)])
    .collect::<Vec<_>>();
    for sql in [
        "FROM texts |> SELECT id, BYTE_LENGTH(category) AS n |> ORDER BY id |> SELECT n",
        "FROM texts |> ORDER BY id |> SELECT BYTE_LENGTH(category) AS n",
        "FROM (FROM texts |> ORDER BY id) |> SELECT BYTE_LENGTH(category) AS n",
    ] {
        let prepared = db.prepare(sql).unwrap();
        let column = prepared.result_column(0).unwrap();
        assert_eq!(column.data_type, DataType::Int64);
        assert!(column.nullable);
        drop(prepared);
        query(&db, sql, expected.clone());
    }
    db.close().unwrap();
    let db = Database::open(&directory.database(), config()).unwrap();
    query(
        &db,
        "FROM texts |> SELECT id, BYTE_LENGTH(category) AS n |> ORDER BY id |> SELECT n",
        expected,
    );
}

#[test]
fn byte_length_literals_own_decoded_values_and_compose_with_constants() {
    let (_directory, db) = super::null_predicate::fixture().unwrap();
    let prepared = {
        let sql = String::from(
            "FROM facts |> SELECT BYTE_LENGTH('é') AS a, (BYTE_LENGTH(('\\u96EA'))) AS b, BYTE_LENGTH('') AS c, BYTE_LENGTH('12345678901234567890123456789012') AS d, BYTE_LENGTH('\\000') AS e",
        );
        db.prepare(&sql).unwrap()
    };
    for index in 0..5 {
        let column = prepared.result_column(index).unwrap();
        assert_eq!(column.data_type, DataType::Int64);
        assert!(!column.nullable);
    }
    let cancel = CancellationToken::new();
    let mut result = db.execute(&prepared, &cancel).unwrap();
    assert_eq!(
        collect(&mut result),
        vec![
            vec![
                Cell::Integer(2),
                Cell::Integer(3),
                Cell::Integer(0),
                Cell::Integer(32),
                Cell::Integer(1)
            ];
            4
        ]
    );
    drop(result);
    drop(prepared);
    for sql in [
        "FROM facts |> SELECT '雪' AS s |> SELECT BYTE_LENGTH(s) AS n |> SELECT n + 1 AS n |> AGGREGATE SUM(n) AS total",
        "FROM facts |> ORDER BY id |> SELECT '雪' AS s |> SELECT BYTE_LENGTH(s) AS n |> SELECT n + 1 AS n |> AGGREGATE SUM(n) AS total",
    ] {
        query(&db, sql, integers(&[16]));
    }
}

#[test]
fn byte_length_preserves_range_inputs_and_conditional_dependencies() {
    let (_directory, db) = super::null_predicate::fixture().unwrap();
    query(
        &db,
        "FROM facts AS f |> SET s = BYTE_LENGTH(s) |> ORDER BY id |> SELECT f.s, s",
        vec![
            vec![Cell::Text("present".into()), Cell::Integer(7)],
            vec![Cell::Null, Cell::Null],
            vec![Cell::Text("".into()), Cell::Integer(0)],
            vec![Cell::Text("é".into()), Cell::Integer(2)],
        ],
    );
    query(
        &db,
        "FROM facts |> EXTEND BYTE_LENGTH(s) AS width |> WHERE id < 2 OR width = 2 |> ORDER BY id |> SELECT COALESCE(width, 99) AS chosen",
        integers(&[7, 99, 2]),
    );
    query(
        &db,
        "FROM facts |> EXTEND BYTE_LENGTH(s) AS width |> SELECT COALESCE(1, width) AS chosen, width + 2 AS shifted |> ORDER BY shifted NULLS FIRST",
        vec![
            vec![Cell::Integer(1), Cell::Null],
            vec![Cell::Integer(1), Cell::Integer(2)],
            vec![Cell::Integer(1), Cell::Integer(4)],
            vec![Cell::Integer(1), Cell::Integer(9)],
        ],
    );
    query(
        &db,
        "FROM facts |> SELECT BYTE_LENGTH(s) AS n |> AGGREGATE COUNT(*) AS entries, COUNT(n) AS present, SUM(n) AS total",
        vec![vec![Cell::Integer(4), Cell::Integer(3), Cell::Integer(9)]],
    );
}

#[test]
fn byte_length_crosses_join_group_and_set_materialization() {
    let (_directory, db) = super::null_predicate::fixture().unwrap();
    query(
        &db,
        "FROM facts AS a |> LEFT JOIN (FROM facts |> WHERE id = 0) AS b ON a.id = b.id |> ORDER BY a.id |> SELECT BYTE_LENGTH(b.s) AS n",
        vec![
            vec![Cell::Integer(7)],
            vec![Cell::Null],
            vec![Cell::Null],
            vec![Cell::Null],
        ],
    );
    query(
        &db,
        "FROM facts |> AGGREGATE COUNT(*) AS n GROUP BY s |> SELECT BYTE_LENGTH(s) AS bytes |> ORDER BY bytes NULLS FIRST",
        vec![
            vec![Cell::Null],
            vec![Cell::Integer(0)],
            vec![Cell::Integer(2)],
            vec![Cell::Integer(7)],
        ],
    );
    query(
        &db,
        "FROM facts |> SELECT s |> UNION DISTINCT (FROM facts |> SELECT '雪' AS s) |> SELECT BYTE_LENGTH(s) AS n |> ORDER BY n NULLS FIRST",
        vec![
            vec![Cell::Null],
            vec![Cell::Integer(0)],
            vec![Cell::Integer(2)],
            vec![Cell::Integer(3)],
            vec![Cell::Integer(7)],
        ],
    );
}

#[test]
fn byte_length_rejects_unsupported_forms_with_owned_spans() {
    let (_directory, db) = super::null_predicate::fixture().unwrap();
    let baseline = db.reserved_memory_bytes();
    for expression in [
        "BYTE_LENGTH()",
        "BYTE_LENGTH(s, s)",
        "BYTE_LENGTH(id)",
        "BYTE_LENGTH(n)",
        "BYTE_LENGTH(d)",
        "BYTE_LENGTH(NULL)",
        "BYTE_LENGTH(1)",
        "BYTE_LENGTH(BYTE_LENGTH(s))",
        "BYTE_LENGTH(s) + 1",
        "1 + BYTE_LENGTH(s)",
        "BYTE_LENGTH('123456789012345678901234567890123')",
        "BYTE_LENGTH('\\uD800')",
    ] {
        let sql = format!("FROM facts |> EXTEND {expression} AS unused |> LIMIT 0 |> SELECT id");
        let error = match db.prepare(&sql) {
            Ok(_) => panic!("unsupported expression accepted: {sql}"),
            Err(error) => error,
        };
        let span = match error {
            Error::Parse { span, .. } | Error::Bind { span, .. } => span,
            other => panic!("unexpected rejection: {other}"),
        };
        assert!(
            span.start() <= span.end() && span.end() <= sql.len(),
            "{sql}"
        );
        assert!(sql.is_char_boundary(span.start()) && sql.is_char_boundary(span.end()));
        if matches!(
            expression,
            "BYTE_LENGTH(id)" | "BYTE_LENGTH(n)" | "BYTE_LENGTH(d)"
        ) {
            let argument = expression
                .strip_prefix("BYTE_LENGTH(")
                .unwrap()
                .strip_suffix(')')
                .unwrap();
            assert_eq!(&sql[span.start()..span.end()], argument);
        }
        drop(sql);
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    assert!(matches!(
        db.prepare("FROM facts |> AGGREGATE SUM(BYTE_LENGTH(s)) AS n"),
        Err(Error::Parse { .. }) | Err(Error::Bind { .. })
    ));
    assert_eq!(db.reserved_memory_bytes(), baseline);
}

#[test]
fn byte_length_cancellation_and_abandonment_release_query_owners() {
    let (_directory, db) = super::text_filter::fixture();
    let baseline = db.reserved_memory_bytes();
    for sql in [
        "FROM texts |> SELECT BYTE_LENGTH(category) AS width",
        "FROM texts |> ORDER BY id |> SELECT BYTE_LENGTH(category) AS width",
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
        "FROM texts |> SELECT BYTE_LENGTH(category) AS width |> WHERE width = 65536",
        integers(&[65_536]),
    );
}
