//! Public grouping contract tests.
use super::*;

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
