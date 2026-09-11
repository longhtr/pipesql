use super::order::{integers, query};
use super::*;

fn fixture() -> (Directory, Database) {
    let directory = Directory::new();
    let db = Database::create_empty(&directory.database(), config()).unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "texts",
        &[
            ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "category",
                data_type: DataType::String,
                nullable: true,
            },
        ],
        &cancel,
    )
    .unwrap();
    let long = "a".repeat(65_536);
    let values = [
        "",
        "a",
        "é",
        "e\u{301}",
        "line\nnext",
        "line\\nnext",
        "it's",
        "ignored",
        "\0",
        &long,
    ];
    let mut append = db
        .begin_append(
            "texts",
            AppendLimits {
                batches: 1,
                encoded_bytes: 100_000,
            },
            &cancel,
        )
        .unwrap();
    append
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]),
                    validity: &[255, 3],
                },
                ColumnInput {
                    values: ColumnValues::String(&values),
                    validity: &[127, 3],
                },
            ],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    (directory, db)
}

#[test]
fn public_text_predicates_preserve_literal_values_and_composed_inputs() {
    let (_directory, db) = fixture();
    for (literal, id) in [
        ("''", 0),
        ("'a'", 1),
        ("'é'", 2),
        (r"'e\u0301'", 3),
        (r"'line\nnext'", 4),
        (r"'line\\nnext'", 5),
        (r"'it\'s'", 6),
        (r"'\000'", 8),
    ] {
        query(
            &db,
            &format!("FROM texts |> WHERE category = {literal} |> SELECT id"),
            integers(&[id]),
        );
    }
    for (comparison, ids) in [
        ("<", vec![0, 8]),
        ("<=", vec![0, 1, 8]),
        ("=", vec![1]),
        ("!=", vec![0, 2, 3, 4, 5, 6, 8, 9]),
        (">=", vec![1, 2, 3, 4, 5, 6, 9]),
        (">", vec![2, 3, 4, 5, 6, 9]),
    ] {
        query(
            &db,
            &format!("FROM texts |> WHERE category {comparison} 'a' |> ORDER BY id |> SELECT id"),
            integers(&ids),
        );
    }
    for sql in [
        "FROM texts |> ORDER BY id DESC |> WHERE category BETWEEN 'a' AND 'e' |> SELECT id",
        "FROM texts |> ORDER BY id DESC |> SELECT category AS c,id |> WHERE c >= 'a' AND c <= 'e' |> SELECT id",
        "FROM (FROM texts |> ORDER BY id DESC |> LIMIT 10) |> WHERE category >= 'a' AND category <= 'e' |> ORDER BY id DESC |> SELECT id",
    ] {
        query(&db, sql, integers(&[9, 1]));
    }
    query(
        &db,
        "FROM texts |> AGGREGATE COUNT(*) AS n GROUP AND ORDER BY category |> WHERE category = 'é' |> SELECT n",
        integers(&[1]),
    );
    query(
        &db,
        "FROM texts AS a |> JOIN texts AS b ON a.id = b.id |> WHERE b.category = 'é' |> SELECT a.id",
        integers(&[2]),
    );
    let baseline = db.reserved_memory_bytes();
    for sql in [
        "FROM texts |> WHERE category = 1",
        "FROM texts |> WHERE id = '1'",
        "FROM texts |> WHERE category = DATE '2026-01-01'",
        "FROM texts |> WHERE category = r'a'",
        "FROM texts |> WHERE category = '''a'''",
        "FROM texts |> WHERE category = 'a' 'b'",
        "FROM texts |> WHERE category = 'a''b'",
        "FROM texts |> WHERE category = '\\400'",
    ] {
        assert!(
            matches!(
                db.prepare(sql),
                Err(Error::Parse { .. } | Error::Bind { .. })
            ),
            "{sql}"
        );
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
}

#[test]
fn text_literal_ownership_cancellation_and_healed_reuse() {
    let (_directory, db) = fixture();
    let baseline = db.reserved_memory_bytes();
    let mut source = String::from(
        "FROM texts |> WHERE category = 'é' OR category = 'absent' |> ORDER BY id |> WHERE NOT category IS NULL |> SELECT id",
    );
    let prepared = db.prepare(&source).unwrap();
    source.clear();
    source.push_str(&"x".repeat(4096));
    drop(source);
    for prefix in 0..64 {
        let cancel = CancellationToken::new();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        let mut complete = false;
        let mut seen = 0;
        for step in 0..512 {
            if step == prefix {
                cancel.cancel();
            }
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Rows(batch) => {
                    assert_eq!(batch.len(), 1);
                    assert_eq!(batch.value(0, 0), Some(Value::Int64(2)));
                    seen += 1;
                }
                QueryStep::Finished => {
                    assert_eq!(seen, 1);
                    complete = true;
                    break;
                }
                QueryStep::Failed(error) => {
                    assert!(matches!(error, Error::Cancelled));
                    complete = true;
                    break;
                }
            }
        }
        assert!(complete);
        drop(result);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(
            db.reserved_memory_bytes(),
            baseline + prepared.accounted_memory_bytes()
        );
    }
    drop(prepared);
    query(
        &db,
        "FROM texts |> WHERE category = 'é' |> SELECT id",
        integers(&[2]),
    );
    assert_eq!(db.reserved_memory_bytes(), baseline);
}
