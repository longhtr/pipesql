//! Public grouping contract tests.
use super::*;

const GROUPING_QUERY: &str =
    "FROM sales |> AGGREGATE COUNT(*) AS n,SUM(amount) AS total GROUP AND ORDER BY region";

#[test]
fn ordered_grouping_preserves_few_many_and_skewed_groups_across_memory_budgets() {
    for groups in [32, 4096] {
        for skewed in [false, true] {
            let directory = Directory::new();
            create_grouping_sales(&directory, groups, skewed);
            for memory in [2_000_000, 1_200_000] {
                let db = Database::open(
                    &directory.database(),
                    Config::new(memory, 8_000_000).unwrap(),
                )
                .unwrap();
                let cancel = CancellationToken::new();
                let query = db.prepare(GROUPING_QUERY).unwrap();
                let baseline = db.reserved_memory_bytes();
                let mut result = db.execute(&query, &cancel).unwrap();
                let mut next_region = 0;
                let mut peak_temp = 0;
                let mut finished = false;
                for _ in 0..200_000 {
                    match result.step() {
                        QueryStep::Rows(batch) => {
                            assert_eq!(batch.column_count(), 3);
                            for row in 0..batch.len() {
                                let base = 4096 / groups;
                                // The first pass distributes amount 1 evenly. The
                                // second distributes amount 3 evenly or all to 0.
                                let second = if skewed {
                                    if next_region == 0 { 4096 } else { 0 }
                                } else {
                                    base
                                };
                                assert!(next_region < groups, "extra group");
                                for (column, expected) in
                                    [next_region, base + second, base + 3 * second]
                                        .into_iter()
                                        .enumerate()
                                {
                                    assert!(
                                        matches!(
                                            batch.value(row, column),
                                            Some(Value::Int64(value)) if value == expected
                                        ),
                                        "groups={groups} skewed={skewed} memory={memory} region={next_region} column={column}"
                                    );
                                }
                                next_region += 1;
                            }
                        }
                        QueryStep::Progress => (),
                        QueryStep::Finished => {
                            finished = true;
                            break;
                        }
                        QueryStep::Failed(error) => panic!("grouping failed: {error}"),
                    }
                    peak_temp = peak_temp.max(db.reserved_temp_bytes());
                }
                assert!(finished, "query exceeded the fixture's step bound");
                assert_eq!(next_region, groups, "missing group");
                assert_eq!(
                    peak_temp > 0,
                    groups == 4096 && memory == 1_200_000,
                    "verify the intended grouping path"
                );
                drop(result);
                assert_eq!(db.reserved_memory_bytes(), baseline);
                assert_eq!(db.reserved_temp_bytes(), 0);
                drop(query);
                db.close().unwrap();
            }
        }
    }
}

fn create_grouping_sales(directory: &Directory, groups: i64, skewed: bool) {
    let db = Database::create_empty(
        &directory.database(),
        Config::new(32_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "sales",
        &["region", "amount"].map(|name| ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: false,
        }),
        &cancel,
    )
    .unwrap();
    let mut append = db
        .begin_append(
            "sales",
            AppendLimits {
                batches: 32,
                encoded_bytes: 1_000_000,
            },
            &cancel,
        )
        .unwrap();
    for amount in [1, 3] {
        for start in (0..4096).step_by(256) {
            let regions: [i64; 256] = std::array::from_fn(|row| {
                if skewed && amount == 3 {
                    0
                } else {
                    (4095 - start - row as i64) % groups
                }
            });
            append
                .write(
                    &[
                        ColumnInput {
                            values: ColumnValues::Int64(&regions),
                            validity: &[255; 32],
                        },
                        ColumnInput {
                            values: ColumnValues::Int64(&[amount; 256]),
                            validity: &[255; 32],
                        },
                    ],
                    &cancel,
                )
                .unwrap();
        }
    }
    append.commit(&cancel).unwrap();
    db.close().unwrap();
}

#[test]
fn declared_grouping_preserves_nullable_text_float_and_date_keys() {
    let directory = Directory::new();
    let db = Database::create_empty(&directory.database(), config()).unwrap();
    let cancel = CancellationToken::new();
    db.declare_table("facts", &declarations(), &cancel).unwrap();
    let sql = "FROM facts |> AGGREGATE COUNT(*) AS n,SUM(amount) AS total,AVG(amount) AS mean GROUP AND ORDER BY note,number,day";
    let empty = db.prepare(sql).unwrap();
    let days = [DateValue::from_days_since_unix_epoch(0).unwrap(); 6];
    let nan = f64::from_bits(0x7ff8_0000_0000_0123);
    let mut writer = db.begin_append("facts", limits(), &cancel).unwrap();
    writer
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::String(&["é", "é", "é", "a", "a", "ignored"]),
                    validity: &[31],
                },
                ColumnInput {
                    values: ColumnValues::Int64(&[i64::MAX, i64::MAX, -i64::MAX, 10, 14, 5]),
                    validity: &[63],
                },
                ColumnInput {
                    values: ColumnValues::Double(&[-0.0, 0.0, -0.0, nan, f64::NAN, -1.0]),
                    validity: &[63],
                },
                ColumnInput {
                    values: ColumnValues::Date(&days),
                    validity: &[63],
                },
            ],
            &cancel,
        )
        .unwrap();
    writer.commit(&cancel).unwrap();
    assert!(
        collect(&mut db.execute(&empty, &cancel).unwrap()).is_empty(),
        "old grouped query retains the empty generation"
    );
    let query = db.prepare(sql).unwrap();
    for (index, kind, nullable) in [
        (0, DataType::String, true),
        (1, DataType::Double, false),
        (2, DataType::Date, false),
        (3, DataType::Int64, false),
        (4, DataType::Int64, true),
        (5, DataType::Double, true),
    ] {
        let column = query.result_column(index).unwrap();
        assert_eq!(column.data_type, kind);
        assert_eq!(column.nullable, nullable);
    }
    let baseline = db.reserved_memory_bytes();
    assert_eq!(
        collect(&mut db.execute(&query, &cancel).unwrap()),
        vec![
            vec![
                Cell::Null,
                Cell::Number((-1.0_f64).to_bits()),
                Cell::Day(0),
                Cell::Integer(1),
                Cell::Integer(5),
                Cell::Number(5.0_f64.to_bits())
            ],
            vec![
                Cell::Text("a".into()),
                Cell::Number(nan.to_bits()),
                Cell::Day(0),
                Cell::Integer(2),
                Cell::Integer(24),
                Cell::Number(12.0_f64.to_bits())
            ],
            vec![
                Cell::Text("é".into()),
                Cell::Number((-0.0_f64).to_bits()),
                Cell::Day(0),
                Cell::Integer(3),
                Cell::Integer(i64::MAX),
                Cell::Number(((i64::MAX as f64) / 3.0).to_bits())
            ],
        ]
    );
    assert_eq!(db.reserved_memory_bytes(), baseline);
    let hidden = db.prepare("FROM facts |> SELECT note AS key,amount |> AGGREGATE SUM(amount*2) AS bad,COUNT(*) AS n GROUP BY key |> WHERE n > 1 |> SELECT n,key").unwrap();
    assert_eq!(
        collect(&mut db.execute(&hidden, &cancel).unwrap()),
        vec![
            vec![Cell::Integer(2), Cell::Text("a".into())],
            vec![Cell::Integer(3), Cell::Text("é".into())],
        ]
    );
    assert!(matches!(
        db.prepare(
            "FROM facts |> SELECT note AS a,note AS b |> AGGREGATE COUNT(*) AS n GROUP BY a,b"
        ),
        Err(Error::Bind { .. })
    ));
}

#[test]
fn declared_grouping_admits_nine_distinct_keys_with_typed_output() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.database(),
        Config::new(8_000_000, 2_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let names = ["a", "b", "c", "d", "e", "f", "g", "h", "i"];
    let declarations = names.map(|name| ColumnDeclaration {
        name,
        data_type: DataType::Int64,
        nullable: false,
    });
    db.declare_table("facts", &declarations, &cancel).unwrap();
    let mut writer = db.begin_append("facts", limits(), &cancel).unwrap();
    let inputs = names.map(|_| ColumnInput {
        values: ColumnValues::Int64(&[2, 1, 2]),
        validity: &[7],
    });
    writer.write(&inputs, &cancel).unwrap();
    writer.commit(&cancel).unwrap();
    let query = db
        .prepare("FROM facts |> AGGREGATE COUNT(*) AS n GROUP AND ORDER BY a,b,c,d,e,f,g,h,i")
        .unwrap();
    assert_eq!(query.result_column_count(), 10);
    let mut result = db.execute(&query, &cancel).unwrap();
    let mut observed = Vec::new();
    for _ in 0..1000 {
        match result.step() {
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let mut values = Vec::new();
                    for column in 0..10 {
                        let Some(Value::Int64(value)) = batch.value(row, column) else {
                            panic!("typed key/count");
                        };
                        values.push(value);
                    }
                    observed.push(values);
                }
            }
            QueryStep::Finished => break,
            QueryStep::Failed(error) => panic!("nine-key query failed: {error}"),
            QueryStep::Progress => (),
        }
    }
    assert!(matches!(result.step(), QueryStep::Finished));
    assert_eq!(observed, vec![vec![1; 10], vec![2; 10]]);
}
