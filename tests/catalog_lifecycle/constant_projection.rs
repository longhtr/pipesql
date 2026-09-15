use super::*;

#[test]
fn select_materializes_owned_string_and_date_constants() {
    let (_directory, db) = nullable_facts().unwrap();
    query(
        &db,
        "FROM facts |> SELECT '雪' AS label, DATE '1970-01-02' AS day",
        vec![vec![Cell::Text("雪".into()), Cell::Day(1)]; 4],
    );
}

#[test]
fn extend_constants_preserve_input_values_and_row_count() {
    let (_directory, db) = nullable_facts().unwrap();
    query(
        &db,
        "FROM facts |> EXTEND 'source' AS label, DATE_ADD(DATE '1970-01-01', INTERVAL 1 DAY) AS day |> ORDER BY id |> SELECT id, label, day",
        (0..4)
            .map(|id| vec![Cell::Integer(id), Cell::Text("source".into()), Cell::Day(1)])
            .collect(),
    );
}

#[test]
fn set_constants_preserve_original_range_values() {
    let (_directory, db) = nullable_facts().unwrap();
    query(
        &db,
        "FROM facts AS f |> SET s='replacement', d=DATE '1969-12-31' |> ORDER BY id |> SELECT f.s, s, d",
        [
            Cell::Text("present".into()),
            Cell::Null,
            Cell::Text("".into()),
            Cell::Text("é".into()),
        ]
        .into_iter()
        .map(|original| vec![original, Cell::Text("replacement".into()), Cell::Day(-1)])
        .collect(),
    );
}

#[test]
fn constants_compose_with_filters_grouping_and_union() {
    let (_directory, db) = nullable_facts().unwrap();
    query(
        &db,
        "FROM facts |> SELECT '雪' AS label, DATE '1970-01-02' AS day |> WHERE label IN ('雪', NULL) AND day = DATE '1970-01-02' |> AGGREGATE COUNT(*) AS n GROUP BY label, day |> SELECT label, day, n",
        vec![vec![
            Cell::Text("雪".into()),
            Cell::Day(1),
            Cell::Integer(4),
        ]],
    );
    query(
        &db,
        "FROM facts |> SELECT 'left' AS label |> UNION ALL (FROM facts |> SELECT 'right' AS label) |> AGGREGATE COUNT(*) AS n GROUP BY label |> ORDER BY label",
        vec![
            vec![Cell::Text("left".into()), Cell::Integer(4)],
            vec![Cell::Text("right".into()), Cell::Integer(4)],
        ],
    );
    query(
        &db,
        "FROM facts |> SELECT ('') AS label, (DATE_SUB(DATE '2000-03-01', INTERVAL 1 DAY)) AS day |> AGGREGATE MIN(label) AS lo, MAX(day) AS hi",
        vec![vec![Cell::Text("".into()), Cell::Day(11016)]],
    );
}

#[test]
fn malformed_constants_fail_preparation_even_when_undemanded() {
    let (_directory, db) = nullable_facts().unwrap();
    let baseline = db.reserved_memory_bytes();
    for constant in [
        "DATE '2023-02-29'",
        "DATE_ADD(DATE '9999-12-31', INTERVAL 1 DAY)",
        "DATE_ADD(DATE '2000-01-01', INTERVAL 1 WEEK)",
        "DATE_ADD(DATE '2000-01-01', INTERVAL 9223372036854775808 DAY)",
        "'123456789012345678901234567890123'", // 33 source bytes
        "'\\uD800'",
        "NULL",
        "'a'+'b'",
    ] {
        let sql = format!("FROM facts |> EXTEND {constant} AS unused |> LIMIT 0 |> SELECT id");
        assert!(db.prepare(&sql).is_err(), "{sql}");
        assert_eq!(db.reserved_memory_bytes(), baseline, "{sql}");
    }
}

#[test]
fn prepared_constants_outlive_source_and_release_on_early_drop() {
    let (_directory, db) = nullable_facts().unwrap();
    let baseline = db.reserved_memory_bytes();
    let prepared = {
        let source =
            String::from("FROM facts |> SELECT '\\u96EA' AS label, DATE '0001-01-01' AS day");
        db.prepare(&source).unwrap()
    };
    let cancel = CancellationToken::new();
    let mut result = db.execute(&prepared, &cancel).unwrap();
    let mut observed = false;
    for _ in 0..100_000 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                assert_eq!(
                    owned_cell(batch.value(0, 0).unwrap()),
                    Cell::Text("雪".into())
                );
                assert_eq!(owned_cell(batch.value(0, 1).unwrap()), Cell::Day(-719162));
                observed = true;
                break;
            }
            QueryStep::Failed(error) => panic!("{error}"),
            QueryStep::Finished => panic!("expected constant rows"),
        }
    }
    assert!(observed);
    drop(result);
    drop(prepared);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
}

#[test]
fn maximum_literal_and_output_width_are_materialized() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.database(),
        Config::new(16_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "facts",
        &[ColumnDeclaration {
            name: "id",
            data_type: DataType::Int64,
            nullable: false,
        }],
        &cancel,
    )
    .unwrap();
    let mut append = db.begin_append("facts", limits(), &cancel).unwrap();
    append
        .write(
            &[ColumnInput {
                values: ColumnValues::Int64(&[1; 512]),
                validity: &[u8::MAX; 64],
            }],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    let literal = "12345678901234567890123456789012";
    let columns = vec![format!("'{literal}'"); 64];
    query(
        &db,
        &format!("FROM facts |> SELECT {}", columns.join(", ")),
        vec![vec![Cell::Text(literal.into()); 64]; 512],
    );
    let too_wide = format!("FROM facts |> SELECT {}, ''", columns.join(", "));
    assert!(db.prepare(&too_wide).is_err());
}
