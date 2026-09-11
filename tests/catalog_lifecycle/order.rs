use super::*;

pub(super) fn query(db: &Database, sql: &str, expected: Vec<Vec<Cell>>) {
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let prepared = db
        .prepare(sql)
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut result = db
        .execute(&prepared, &cancel)
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut rows = Vec::new();
    let mut done = false;
    for _ in 0..100_000 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    rows.push(
                        (0..batch.column_count())
                            .map(|column| owned_cell(batch.value(row, column).unwrap()))
                            .collect::<Vec<_>>(),
                    );
                }
            }
            QueryStep::Finished => {
                done = true;
                break;
            }
            QueryStep::Failed(error) => panic!("{sql}: {error}"),
        }
    }
    assert!(done, "bounded ordering fixture: {sql}");
    assert_eq!(rows, expected, "{sql}");
    drop(result);
    drop(prepared);
    assert_eq!(db.reserved_memory_bytes(), baseline, "{sql}");
    assert_eq!(db.reserved_temp_bytes(), 0, "{sql}");
}

pub(super) fn integers(values: &[i64]) -> Vec<Vec<Cell>> {
    values
        .iter()
        .map(|value| vec![Cell::Integer(*value)])
        .collect()
}

#[test]
fn public_order_preserves_hidden_keys_aliases_and_composed_producers() {
    let (_directory, db) = join_fixture();
    for sql in [
        "FROM facts |> ORDER BY k DESC NULLS FIRST,v DESC |> SELECT v",
        "FROM facts |> SELECT v AS k,k AS v |> ORDER BY 2 DESC NULLS FIRST,1 DESC |> SELECT k",
        "FROM facts |> ORDER BY k DESC NULLS FIRST,k ASC NULLS LAST,v DESC |> SELECT v",
    ] {
        query(&db, sql, integers(&[40, 30, 20, 10]));
    }
    query(
        &db,
        "FROM facts |> ORDER BY k DESC NULLS FIRST,v DESC |> SELECT v |> WHERE v != 20",
        integers(&[40, 30, 10]),
    );
    query(
        &db,
        "FROM facts |> ORDER BY v DESC |> ORDER BY k ASC NULLS LAST,v ASC |> SELECT v",
        integers(&[10, 20, 30, 40]),
    );
    query(
        &db,
        "FROM facts |> AGGREGATE SUM(v) AS total GROUP BY k |> ORDER BY total DESC,k ASC NULLS LAST |> SELECT k,total",
        vec![
            vec![Cell::Null, Cell::Integer(40)],
            vec![Cell::Integer(1), Cell::Integer(30)],
            vec![Cell::Integer(2), Cell::Integer(30)],
        ],
    );
    query(
        &db,
        "FROM facts |> ORDER BY v DESC |> AGGREGATE SUM(v) AS total,COUNT(*) AS n",
        vec![vec![Cell::Integer(100), Cell::Integer(4)]],
    );
    for prefix in ["FROM facts AS f", "FROM facts |> ORDER BY v DESC |> AS f"] {
        query(
            &db,
            &format!(
                "{prefix} |> JOIN dimensions AS d ON f.k = d.k |> ORDER BY f.v DESC,d.label |> SELECT f.v,d.label"
            ),
            [(30, "c"), (20, "a"), (20, "b"), (10, "a"), (10, "b")]
                .map(|(v, s)| vec![Cell::Integer(v), Cell::Text(s.to_owned())])
                .to_vec(),
        );
    }
    query(&db, "FROM facts |> WHERE v < 0 |> ORDER BY k", vec![]);
    db.close().unwrap();
}

#[test]
fn public_order_covers_scalar_domains_directions_nulls_and_raw_values() {
    let directory = Directory::new();
    let cancel = CancellationToken::new();
    let db = Database::create_empty(
        &directory.database(),
        Config::new(8_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let declarations = [
        ("id", DataType::Int64),
        ("number", DataType::Double),
        ("text", DataType::String),
        ("day", DataType::Date),
        ("payload", DataType::String),
    ]
    .map(|(name, data_type)| ColumnDeclaration {
        name,
        data_type,
        nullable: name != "id" && name != "payload",
    });
    db.declare_table("facts", &declarations, &cancel).unwrap();
    let numbers = [
        f64::INFINITY,
        -0.0,
        f64::from_bits(0x7ff8_0000_0000_0123),
        1.5,
        f64::NEG_INFINITY,
        0.0,
        -2.5,
        f64::from_bits(0xfff8_0000_0000_0456),
        42.0,
        0.0,
    ];
    let texts = ["é", "a", "雪", "", "é", "a", "z", "雪", "a", "ignored"];
    let day_numbers = [10, -1, 0, 10, -1, 0, 10, -1, 0, 99];
    let days = day_numbers.map(|day| DateValue::from_days_since_unix_epoch(day).unwrap());
    let long = "x".repeat(65_536);
    let payloads = ["0", "1", "2", "3", "4", "5", "6", "7", long.as_str(), "9"];
    let mut writer = db
        .begin_append(
            "facts",
            AppendLimits {
                batches: 1,
                encoded_bytes: 100_000,
            },
            &cancel,
        )
        .unwrap();
    writer
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]),
                    validity: &[255, 3],
                },
                ColumnInput {
                    values: ColumnValues::Double(&numbers),
                    validity: &[255, 1],
                },
                ColumnInput {
                    values: ColumnValues::String(&texts),
                    validity: &[255, 1],
                },
                ColumnInput {
                    values: ColumnValues::Date(&days),
                    validity: &[255, 1],
                },
                ColumnInput {
                    values: ColumnValues::String(&payloads),
                    validity: &[255, 3],
                },
            ],
            &cancel,
        )
        .unwrap();
    writer.commit(&cancel).unwrap();
    query(
        &db,
        "FROM facts |> ORDER BY id |> LIMIT 1 OFFSET 8 |> SELECT payload,payload AS duplicate",
        vec![vec![Cell::Text(long.clone()), Cell::Text(long.clone())]],
    );
    query(
        &db,
        "FROM facts |> ORDER BY id DESC |> LIMIT 1 |> SELECT number,day,text",
        vec![vec![Cell::Null, Cell::Null, Cell::Null]],
    );
    // Independent semantic ranks: all NaNs tie below negative infinity;
    // signed zeros tie; text follows UTF-8 byte order; dates follow day values.
    let ranks = [
        ("number", [6, 3, 0, 4, 1, 3, 2, 0, 5, 0]),
        ("text", [4, 1, 5, 0, 4, 1, 3, 5, 1, 0]),
        ("day", [2, 0, 1, 2, 0, 1, 2, 0, 1, 0]),
    ];
    for (name, rank) in ranks {
        for descending in [false, true] {
            for nulls_last in [false, true] {
                let mut ids: Vec<usize> = (0..10).collect();
                ids.sort_by_key(|id| {
                    (
                        if nulls_last { *id == 9 } else { *id != 9 },
                        if descending { -rank[*id] } else { rank[*id] },
                        *id,
                    )
                });
                let expected = ids
                    .iter()
                    .map(|id| {
                        vec![
                            Cell::Integer(*id as i64),
                            if *id == 9 {
                                Cell::Null
                            } else {
                                Cell::Number(numbers[*id].to_bits())
                            },
                            Cell::Text(payloads[*id].to_owned()),
                        ]
                    })
                    .collect();
                query(
                    &db,
                    &format!(
                        "FROM facts |> ORDER BY {name} {} NULLS {},id |> SELECT id,number,payload",
                        if descending { "DESC" } else { "ASC" },
                        if nulls_last { "LAST" } else { "FIRST" }
                    ),
                    expected,
                );
            }
        }
    }
    query(
        &db,
        "FROM facts |> ORDER BY number,id |> SELECT id",
        integers(&[9, 2, 7, 4, 6, 1, 5, 3, 8, 0]),
    );
    query(
        &db,
        "FROM facts |> ORDER BY number DESC,id |> SELECT id",
        integers(&[0, 8, 3, 1, 5, 6, 4, 2, 7, 9]),
    );
    db.close().unwrap();
}

#[test]
fn public_order_rejects_unsupported_and_ambiguous_keys_without_retaining_owners() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    for sql in [
        "FROM facts |> ORDER k",
        "FROM facts |> ORDER BY",
        "FROM facts |> ORDER BY k + 1",
        "FROM facts |> ORDER BY k COLLATE 'und:ci'",
        "FROM facts |> ORDER BY k NULLS MIDDLE",
        "FROM facts |> ORDER BY 1.0",
        "FROM facts |> ORDER BY 0",
        "FROM facts |> ORDER BY -1",
        "FROM facts |> ORDER BY 3",
        "FROM facts |> ORDER BY 18446744073709551616",
        "FROM facts |> SELECT v |> ORDER BY 2",
        "FROM facts |> ORDER BY missing",
        "FROM facts |> SELECT k AS x,v AS x |> ORDER BY x",
        "FROM facts |> SELECT k,k |> ORDER BY k",
        "FROM facts |> ORDER BY k |> SELECT v |> ORDER BY k",
        "FROM facts AS a |> ORDER BY b.k",
    ] {
        assert!(
            matches!(
                db.prepare(sql),
                Err(Error::Parse { .. } | Error::Bind { .. })
            ),
            "{sql}"
        );
        assert_eq!(db.reserved_memory_bytes(), baseline, "{sql}");
        assert_eq!(db.reserved_temp_bytes(), 0, "{sql}");
    }
    // An ordinal disambiguates duplicate display names.
    query(
        &db,
        "FROM facts |> SELECT v AS x,v AS x |> ORDER BY 2 DESC",
        [40, 30, 20, 10]
            .map(|v| vec![Cell::Integer(v), Cell::Integer(v)])
            .to_vec(),
    );
    db.close().unwrap();
}

#[test]
fn public_order_admits_all_visible_keys_and_refuses_excess_query_work() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.database(),
        Config::new(32_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let names: Vec<_> = (0..64).map(|n| format!("c{n}")).collect();
    let declarations: Vec<_> = names
        .iter()
        .map(|name| ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: false,
        })
        .collect();
    db.declare_table("wide", &declarations, &cancel).unwrap();
    let values = [1, 0];
    let inputs: Vec<_> = (0..64)
        .map(|_| ColumnInput {
            values: ColumnValues::Int64(&values),
            validity: &[3],
        })
        .collect();
    let mut writer = db
        .begin_append(
            "wide",
            AppendLimits {
                batches: 1,
                encoded_bytes: 10_000,
            },
            &cancel,
        )
        .unwrap();
    writer.write(&inputs, &cancel).unwrap();
    writer.commit(&cancel).unwrap();
    let keys = (1..=64)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    query(
        &db,
        &format!("FROM wide |> ORDER BY {keys}"),
        vec![vec![Cell::Integer(0); 64], vec![Cell::Integer(1); 64]],
    );
    let baseline = db.reserved_memory_bytes();
    for sql in [
        format!("FROM wide |> ORDER BY {}", ["1"; 81].join(",")),
        format!("FROM wide{}", " |> ORDER BY 1".repeat(17)),
        "FROM wide |> ORDER BY 65".to_owned(),
    ] {
        assert!(db.prepare(&sql).is_err(), "{sql}");
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    db.close().unwrap();
}
