use super::*;

fn canonical(mut row: Vec<Cell>) -> Vec<Cell> {
    for cell in &mut row {
        if let Cell::Number(bits) = cell {
            let value = f64::from_bits(*bits);
            if value.is_nan() {
                *bits = f64::NAN.to_bits();
            } else if value == 0.0 {
                *bits = 0;
            }
        }
    }
    row
}

fn unordered(db: &Database, sql: &str, expected: Vec<Vec<Cell>>) {
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let prepared = db
        .prepare(sql)
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut result = db.execute(&prepared, &cancel).unwrap();
    let mut rows = Vec::new();
    let mut finished = false;
    for _ in 0..1_000_000 {
        match result.step() {
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    rows.push(canonical(
                        (0..batch.column_count())
                            .map(|column| owned_cell(batch.value(row, column).unwrap()))
                            .collect(),
                    ));
                }
            }
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Progress => (),
            QueryStep::Failed(error) => panic!("{sql}: {error}"),
        }
    }
    assert!(finished, "{sql}: bounded fixture did not finish");
    rows.sort();
    let mut expected: Vec<_> = expected.into_iter().map(canonical).collect();
    expected.sort();
    assert_eq!(rows, expected, "{sql}");
    drop(result);
    drop(prepared);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
}

#[test]
fn public_distinct_scalar_equivalence_empty_input_and_composition() {
    let directory = Directory::new();
    let cancel = CancellationToken::new();
    let db = Database::create_empty(
        &directory.database(),
        Config::new(8_000_000, 16_000_000).unwrap(),
    )
    .unwrap();
    let columns = [
        ("n", DataType::Double),
        ("i", DataType::Int64),
        ("t", DataType::String),
        ("d", DataType::Date),
    ]
    .map(|(name, data_type)| ColumnDeclaration {
        name,
        data_type,
        nullable: true,
    });
    db.declare_table("facts", &columns, &cancel).unwrap();
    unordered(&db, "FROM facts |> DISTINCT", vec![]);
    unordered(
        &db,
        "FROM facts |> DISTINCT |> AGGREGATE COUNT(*) AS n",
        order::integers(&[0]),
    );
    let numbers = [
        -0.0,
        0.0,
        f64::from_bits(0x7ff8_0000_0000_0123),
        f64::from_bits(0xfff8_0000_0000_0456),
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::from_bits(1),
        f64::MAX,
        -1.0,
        42.0,
    ];
    let integers = [i64::MIN, i64::MIN, i64::MAX, i64::MAX, 5, 6, 7, 8, 9, 0];
    let texts = [
        "雪",
        "雪",
        "",
        "",
        "é",
        "e\u{301}",
        "a\0b",
        "z",
        "null payload",
        "ignored",
    ];
    let days = [DateValue::from_days_since_unix_epoch(-1).unwrap(); 10];
    let validity = [255, 1];
    // Repeated rows cross batch/run boundaries. The reference is a set of
    // public typed values with explicit IEEE equivalence classes, no row codec.
    let mut expected = std::collections::BTreeSet::new();
    for index in 0..10 {
        expected.insert(if index == 9 {
            vec![Cell::Null; 4]
        } else {
            canonical(vec![
                Cell::Number(numbers[index].to_bits()),
                Cell::Integer(integers[index]),
                Cell::Text(texts[index].to_owned()),
                Cell::Day(-1),
            ])
        });
    }
    let mut writer = db
        .begin_append(
            "facts",
            AppendLimits {
                batches: 40,
                encoded_bytes: 100_000,
            },
            &cancel,
        )
        .unwrap();
    for _ in 0..40 {
        writer
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::Double(&numbers),
                        validity: &validity,
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&integers),
                        validity: &validity,
                    },
                    ColumnInput {
                        values: ColumnValues::String(&texts),
                        validity: &validity,
                    },
                    ColumnInput {
                        values: ColumnValues::Date(&days),
                        validity: &validity,
                    },
                ],
                &cancel,
            )
            .unwrap();
    }
    writer.commit(&cancel).unwrap();
    let expected: Vec<_> = expected.into_iter().collect();
    unordered(&db, "FROM facts |> DISTINCT", expected.clone());
    unordered(
        &db,
        "FROM (FROM facts |> DISTINCT) AS f |> DISTINCT",
        expected.clone(),
    );
    unordered(
        &db,
        "FROM facts |> DISTINCT |> WHERE i IS NULL OR i < 0",
        expected
            .iter()
            .filter(|row| matches!(row[1], Cell::Null | Cell::Integer(i64::MIN)))
            .cloned()
            .collect(),
    );
    unordered(
        &db,
        "FROM facts |> DISTINCT |> AGGREGATE COUNT(*) AS n",
        order::integers(&[expected.len() as i64]),
    );
    unordered(
        &db,
        "FROM facts |> AGGREGATE COUNT(*) AS n GROUP BY i |> SELECT n |> DISTINCT",
        order::integers(&[40, 80]),
    );
    unordered(
        &db,
        "FROM facts |> SELECT i |> DISTINCT |> SELECT i+0 AS x |> DISTINCT",
        expected.iter().map(|row| vec![row[1].clone()]).collect(),
    );
    unordered(
        &db,
        "FROM facts |> SELECT i |> DISTINCT |> ORDER BY i |> LIMIT 2",
        vec![vec![Cell::Null], vec![Cell::Integer(i64::MIN)]],
    );
    // DISTINCT demands every grouping field, even if its consumer drops it.
    // In contrast, a dropped expression after DISTINCT is never demanded.
    unordered(
        &db,
        "FROM facts |> DISTINCT |> SELECT i,i+1 AS unused |> SELECT i",
        expected.iter().map(|row| vec![row[1].clone()]).collect(),
    );
    for sql in [
        "FROM facts |> SELECT i,i+1 AS overflow |> DISTINCT |> SELECT i",
        "FROM facts |> WHERE i > 0 |> AGGREGATE SUM(i) AS overflow |> DISTINCT |> SELECT 1 AS constant",
    ] {
        let baseline = db.reserved_memory_bytes();
        let query = db.prepare(sql).unwrap();
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut failed = false;
        for _ in 0..100_000 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Failed(Error::ArithmeticOverflow { .. }) => {
                    failed = true;
                    break;
                }
                _ => panic!("{sql}: demanded overflow must precede DISTINCT output"),
            }
        }
        assert!(failed);
        drop(result);
        drop(query);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    db.close().unwrap();
}

#[test]
#[cfg_attr(
    all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"),
    ignore = "GNU aarch64 has a 128-KiB pthread minimum; this test requires at most 64 KiB"
)]
fn public_distinct_uses_all_64_columns_before_projection_and_repeats() {
    let directory = Directory::new();
    let cancel = CancellationToken::new();
    let db = Database::create_empty(
        &directory.database(),
        Config::new(64_000_000, 64_000_000).unwrap(),
    )
    .unwrap();
    let names: Vec<_> = (0..64).map(|index| format!("c{index}")).collect();
    let declarations: Vec<_> = names
        .iter()
        .map(|name| ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: false,
        })
        .collect();
    db.declare_table("wide", &declarations, &cancel).unwrap();
    // Only the last field distinguishes the third row. Pruning that field
    // through DISTINCT would incorrectly remove one of the projected zeros.
    let values: Vec<_> = (0..64)
        .map(|column| [0_i64, 0, i64::from(column == 63)])
        .collect();
    let inputs: Vec<_> = values
        .iter()
        .map(|values| ColumnInput {
            values: ColumnValues::Int64(values),
            validity: &[7],
        })
        .collect();
    let mut writer = db.begin_append("wide", limits(), &cancel).unwrap();
    writer.write(&inputs, &cancel).unwrap();
    writer.commit(&cancel).unwrap();

    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(48 * 1024)
            .spawn_scoped(scope, || {
                assert!(pipesql_filesystem::test_current_thread_stack_bytes() <= 65_536);
                order::query(
                    &db,
                    "FROM wide |> DISTINCT |> SELECT c0",
                    order::integers(&[0, 0]),
                );
                // A bodyless stage can remap 64 identities without consuming 64 tokens.
                // Exercise the complete stage budget, not the aggregate-output budget.
                let sql = format!("FROM wide{}", " |> DISTINCT".repeat(16));
                let baseline = db.reserved_memory_bytes();
                let prepared = db.prepare(&sql).unwrap();
                let mut result = db.execute(&prepared, &cancel).unwrap();
                let mut rows = collect(&mut result);
                rows.sort();
                let zero = vec![Cell::Integer(0); 64];
                let mut one = zero.clone();
                one[63] = Cell::Integer(1);
                assert_eq!(rows, vec![zero, one]);
                drop(result);
                drop(prepared);
                assert_eq!(db.reserved_memory_bytes(), baseline);
                assert_eq!(db.reserved_temp_bytes(), 0);
            })
            .unwrap()
            .join()
            .unwrap();
    });
    db.close().unwrap();
}

#[test]
fn public_distinct_preserves_visible_ranges_and_duplicate_outputs() {
    let (_directory, db) = join_fixture();
    order::query(
        &db,
        "FROM facts AS f |> DISTINCT |> SELECT f.k AS k,f.k AS duplicate |> DISTINCT |> AS d |> ORDER BY d.k |> SELECT d.k,d.duplicate",
        vec![
            vec![Cell::Null, Cell::Null],
            vec![Cell::Integer(1), Cell::Integer(1)],
            vec![Cell::Integer(2), Cell::Integer(2)],
        ],
    );
    db.close().unwrap();
}

#[test]
fn public_distinct_preserves_64_maximum_byte_text_fields() {
    let directory = Directory::new();
    let cancel = CancellationToken::new();
    let db = Database::create_empty(
        &directory.database(),
        Config::new(128_000_000, 64_000_000).unwrap(),
    )
    .unwrap();
    let names: Vec<_> = (0..64).map(|column| format!("c{column}")).collect();
    let columns: Vec<_> = names
        .iter()
        .map(|name| ColumnDeclaration {
            name,
            data_type: DataType::String,
            nullable: true,
        })
        .collect();
    db.declare_table("wide", &columns, &cancel).unwrap();
    let text = "é".repeat(32_768);
    assert_eq!(text.len(), 65_536);
    let values = [text.as_str(); 3];
    let last = [text.as_str(), text.as_str(), "雪"];
    let inputs: Vec<_> = (0..64)
        .map(|column| ColumnInput {
            values: ColumnValues::String(if column == 63 { &last } else { &values }),
            validity: &[7],
        })
        .collect();
    let mut append = db
        .begin_append(
            "wide",
            AppendLimits {
                batches: 1,
                encoded_bytes: 16_000_000,
            },
            &cancel,
        )
        .unwrap();
    append.write(&inputs, &cancel).unwrap();
    append.commit(&cancel).unwrap();
    let full = vec![Cell::Text(text); 64];
    let mut shorter = full.clone();
    shorter[63] = Cell::Text("雪".to_owned());
    unordered(&db, "FROM wide |> DISTINCT", vec![full, shorter]);
    db.close().unwrap();
}

#[test]
fn public_distinct_keeps_pinned_inputs_through_append_reclaim_and_joins() {
    let (directory, db) = join_fixture();
    let cancel = CancellationToken::new();
    let sql = "FROM facts |> SELECT k |> DISTINCT";
    let old = db.prepare(sql).unwrap();
    let mut append = db.begin_append("facts", limits(), &cancel).unwrap();
    append
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&[3, 3]),
                    validity: &[3],
                },
                ColumnInput {
                    values: ColumnValues::Int64(&[50, 50]),
                    validity: &[3],
                },
            ],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    db.reclaim(&cancel).unwrap();
    let mut result = db.execute(&old, &cancel).unwrap();
    assert_eq!(
        collect(&mut result),
        vec![
            vec![Cell::Null],
            vec![Cell::Integer(1)],
            vec![Cell::Integer(2)]
        ]
    );
    drop(result);
    let current = vec![
        vec![Cell::Null],
        vec![Cell::Integer(1)],
        vec![Cell::Integer(2)],
        vec![Cell::Integer(3)],
    ];
    unordered(&db, sql, current.clone());
    for sql in [
        "FROM (FROM facts |> SELECT k |> DISTINCT) AS f |> JOIN (FROM dimensions |> SELECT k |> DISTINCT) AS d ON f.k=d.k |> SELECT f.k",
        "FROM facts AS f |> JOIN dimensions AS d ON f.k=d.k |> SELECT f.k |> DISTINCT",
        "FROM facts |> SELECT k |> DISTINCT |> AS f |> JOIN dimensions AS d ON f.k=d.k |> SELECT f.k |> DISTINCT",
    ] {
        unordered(&db, sql, order::integers(&[1, 2]));
    }
    drop(old);
    db.close().unwrap();
    let reopened = Database::open(
        &directory.database(),
        Config::new(8_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    unordered(&reopened, sql, current);
    reopened.close().unwrap();
}
