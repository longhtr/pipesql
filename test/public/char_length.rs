//! Test CHAR_LENGTH as a count of Unicode scalar values, not displayed symbols.
//!
//! One visible symbol may contain several scalars: a flag uses two regional
//! indicators, while the woman-technologist emoji includes a joining character.
//! Literal byte/count pairs make this distinction testable without using Rust's
//! character counter to calculate the expected answers.
//!
//! Queries compare stored and decoded literal text across sorting, joins,
//! grouping, sets and reopening. Shared argument-error and cancellation cases
//! live in `byte_length`; these cases keep the character-count expectations here.

use super::*;

#[test]
fn char_length_counts_scalars_across_scan_sort_and_reopen() {
    let (directory, db) = text_fixture();
    let maximum = "😀".repeat(16_384);
    let cancel = CancellationToken::new();
    let mut append = db.begin_append("texts", limits(), &cancel).unwrap();
    append
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&[10, 11, 12, 13, 14]),
                    validity: &[31],
                },
                ColumnInput {
                    values: ColumnValues::String(&["雪", "😀", "🇻🇳", "👩‍💻", &maximum]),
                    validity: &[31],
                },
            ],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    // Each pair is (UTF-8 bytes, Unicode scalars). The final value reaches the
    // stored byte limit with four-byte scalars, so its byte count is four times its scalar count.
    let expected = [
        Some((0, 0)),
        Some((1, 1)),
        Some((2, 1)),
        Some((3, 2)),
        Some((9, 9)),
        Some((10, 10)),
        Some((4, 4)),
        None,
        Some((1, 1)),
        Some((65_536, 65_536)),
        Some((3, 1)),
        Some((4, 1)),
        Some((8, 2)),
        Some((11, 3)),
        Some((65_536, 16_384)),
    ]
    .into_iter()
    .map(|pair| match pair {
        Some((bytes, characters)) => vec![Cell::Integer(bytes), Cell::Integer(characters)],
        None => vec![Cell::Null, Cell::Null],
    })
    .collect::<Vec<_>>();
    let queries = [
        "FROM texts |> SELECT id, BYTE_LENGTH(category) AS bytes, CHAR_LENGTH(category) AS characters |> ORDER BY id |> SELECT bytes, characters",
        "FROM texts |> ORDER BY id |> SELECT BYTE_LENGTH(category) AS bytes, CHAR_LENGTH(category) AS characters",
        "FROM (FROM texts |> ORDER BY id) |> SELECT BYTE_LENGTH(category) AS bytes, CHAR_LENGTH(category) AS characters",
    ];
    for sql in queries {
        let prepared = db.prepare(sql).unwrap();
        for index in 0..2 {
            let column = prepared.result_column(index).unwrap();
            assert_eq!(column.data_type, DataType::Int64);
            assert!(column.nullable);
        }
        drop(prepared);
        query(&db, sql, expected.clone());
    }
    db.close().unwrap();
    let db = Database::open(&directory.database(), config()).unwrap();
    query(&db, queries[0], expected);
}

#[test]
fn char_length_literals_fold_owned_decoded_scalars() {
    let (_directory, db) = nullable_facts().unwrap();
    // Drop the source before executing. The escaped combining accent must
    // contribute one scalar after decoding, not six source characters.
    let prepared = {
        let sql = String::from(
            "FROM facts |> SELECT CHAR_LENGTH('é') AS a, (CHAR_LENGTH(('e\\u0301'))) AS b, CHAR_LENGTH('') AS c, CHAR_LENGTH('😀😀😀😀😀😀😀😀') AS d, CHAR_LENGTH('\\000') AS e",
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
        collect_unordered(&mut result),
        vec![
            vec![
                Cell::Integer(1),
                Cell::Integer(2),
                Cell::Integer(0),
                Cell::Integer(8),
                Cell::Integer(1)
            ];
            4
        ]
    );
    drop(result);
    drop(prepared);
    for sql in [
        "FROM facts |> SELECT '雪' AS s |> SELECT CHAR_LENGTH(s) AS n |> SELECT n + 1 AS n |> AGGREGATE SUM(n) AS total",
        "FROM facts |> ORDER BY id |> SELECT '雪' AS s |> SELECT CHAR_LENGTH(s) AS n |> SELECT n + 1 AS n |> AGGREGATE SUM(n) AS total",
    ] {
        query(&db, sql, integers(&[8]));
    }
    for expression in [
        "CHARACTER_LENGTH(s)",
        "LENGTH(s)",
        "OCTET_LENGTH(s)",
        "CHAR_LENGTH('😀😀😀😀😀😀😀😀a')",
    ] {
        let sql = format!("FROM facts |> SELECT {expression} AS n");
        assert!(matches!(
            db.prepare(&sql),
            Err(Error::Parse { .. }) | Err(Error::Bind { .. })
        ));
    }
}

#[test]
fn char_length_preserves_identity_and_conditional_composition() {
    let (_directory, db) = nullable_facts().unwrap();
    query(
        &db,
        "FROM facts AS f |> SET s = CHAR_LENGTH(s) |> ORDER BY id |> SELECT f.s, s",
        vec![
            vec![Cell::Text("present".into()), Cell::Integer(7)],
            vec![Cell::Null, Cell::Null],
            vec![Cell::Text("".into()), Cell::Integer(0)],
            vec![Cell::Text("é".into()), Cell::Integer(1)],
        ],
    );
    query(
        &db,
        "FROM facts |> EXTEND CHAR_LENGTH(s) AS width |> WHERE id < 2 OR width = 1 |> ORDER BY id |> SELECT COALESCE(width, 99) AS chosen",
        integers(&[7, 99, 1]),
    );
    query(
        &db,
        "FROM facts |> EXTEND CHAR_LENGTH(s) AS width |> SELECT COALESCE(1, width) AS chosen, width + 2 AS shifted |> ORDER BY shifted NULLS FIRST",
        vec![
            vec![Cell::Integer(1), Cell::Null],
            vec![Cell::Integer(1), Cell::Integer(2)],
            vec![Cell::Integer(1), Cell::Integer(3)],
            vec![Cell::Integer(1), Cell::Integer(9)],
        ],
    );
    query(
        &db,
        "FROM facts |> SELECT CHAR_LENGTH(s) AS n |> AGGREGATE COUNT(*) AS entries, COUNT(n) AS present, SUM(n) AS total",
        vec![vec![Cell::Integer(4), Cell::Integer(3), Cell::Integer(8)]],
    );
}

#[test]
fn char_length_crosses_join_group_and_set_materialization() {
    let (_directory, db) = nullable_facts().unwrap();
    query(
        &db,
        "FROM facts AS a |> LEFT JOIN (FROM facts |> WHERE id = 0) AS b ON a.id = b.id |> ORDER BY a.id |> SELECT CHAR_LENGTH(b.s) AS n",
        vec![
            vec![Cell::Integer(7)],
            vec![Cell::Null],
            vec![Cell::Null],
            vec![Cell::Null],
        ],
    );
    query(
        &db,
        "FROM facts |> AGGREGATE COUNT(*) AS n GROUP BY s |> SELECT CHAR_LENGTH(s) AS characters |> ORDER BY characters NULLS FIRST",
        vec![
            vec![Cell::Null],
            vec![Cell::Integer(0)],
            vec![Cell::Integer(1)],
            vec![Cell::Integer(7)],
        ],
    );
    query(
        &db,
        "FROM facts |> SELECT s |> UNION DISTINCT (FROM facts |> SELECT '雪' AS s) |> SELECT CHAR_LENGTH(s) AS n |> ORDER BY n NULLS FIRST",
        vec![
            vec![Cell::Null],
            vec![Cell::Integer(0)],
            vec![Cell::Integer(1)],
            vec![Cell::Integer(1)],
            vec![Cell::Integer(7)],
        ],
    );
}
