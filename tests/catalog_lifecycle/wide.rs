//! Public wide contract tests.
use super::*;

#[test]
fn scan_memory_upper_bound_admits_a_full_text_schema() {
    let directory = Directory::new();
    let cancellation = CancellationToken::new();
    let db = Database::create_empty(
        &directory.database(),
        Config::new(64_000_000, 2_000_000).unwrap(),
    )
    .unwrap();
    let names: Vec<_> = (0..64).map(|index| format!("c{index}")).collect();
    let columns: Vec<_> = names
        .iter()
        .map(|name| ColumnDeclaration {
            name,
            data_type: DataType::String,
            nullable: true,
        })
        .collect();
    db.declare_table("wide", &columns, &cancellation).unwrap();
    let query = db.prepare("FROM wide").unwrap();
    let prior_ownership = db.reserved_memory_bytes();
    drop(query);
    db.close().unwrap();

    // Leave exactly the advertised scan allowance after the database and query
    // are admitted. Even an empty table must admit its complete scan workspace.
    let allowance = Database::execution_memory_requirement_bytes();
    let db = Database::open(
        &directory.database(),
        Config::new(prior_ownership.checked_add(allowance).unwrap(), 2_000_000).unwrap(),
    )
    .unwrap();
    let query = db.prepare("FROM wide").unwrap();
    assert_eq!(db.reserved_memory_bytes(), prior_ownership);
    let mut result = db.execute(&query, &cancellation).unwrap();
    assert!(result.accounted_memory_bytes() <= allowance);
    assert!(collect(&mut result).is_empty());
    drop(result);
    assert_eq!(db.reserved_memory_bytes(), prior_ownership);
    assert_eq!(db.reserved_temp_bytes(), 0);
    drop(query);
    db.close().unwrap();
}

#[test]
#[cfg_attr(
    all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"),
    ignore = "GNU aarch64 has a 128-KiB pthread minimum; this test requires at most 64 KiB"
)]
fn complete_declared_schema_preserves_late_columns_and_wide_outputs() {
    let directory = Directory::new();
    let path = directory.database();
    let worker = std::thread::Builder::new()
        .stack_size(48 * 1024)
        .spawn(move || {
            let stack = pipesql_filesystem::test_current_thread_stack_bytes();
            assert!(stack <= 65_536, "reported stack {stack}");
            let config = Config::new(64_000_000, 16_000_000).unwrap();
            let cancel = CancellationToken::new();
            let db = Database::create_empty(&path, config).unwrap();
            let names: Vec<_> = (0..64).map(|i| format!("c{i}")).collect();
            let kinds = [
                DataType::Int64,
                DataType::Double,
                DataType::Date,
                DataType::String,
            ];
            let schema: Vec<_> = names
                .iter()
                .enumerate()
                .map(|(i, name)| ColumnDeclaration {
                    name,
                    data_type: kinds[i % 4],
                    nullable: true,
                })
                .collect();
            db.declare_table("wide", &schema, &cancel).unwrap();
            let integers: Vec<_> = (0..64)
                .map(|i| [i * 100, i * 100 + 1, i * 100 + 2])
                .collect();
            let numbers: Vec<_> = (0..64)
                .map(|i| [i as f64, i as f64 + 0.25, i as f64 + 0.5])
                .collect();
            let dates: Vec<_> = (0..64)
                .map(|i| {
                    std::array::from_fn::<_, 3, _>(|r| {
                        DateValue::from_days_since_unix_epoch(i * 10 + r as i32).unwrap()
                    })
                })
                .collect();
            let texts: Vec<_> = (0..64)
                .map(|i| {
                    [
                        format!("欄-{i}-0"),
                        format!("欄-{i}-1"),
                        format!("欄-{i}-2"),
                    ]
                })
                .collect();
            let borrowed: Vec<_> = texts
                .iter()
                .map(|v| v.each_ref().map(String::as_str))
                .collect();
            let input: Vec<_> = (0..64)
                .map(|i| ColumnInput {
                    values: match i % 4 {
                        0 => ColumnValues::Int64(&integers[i]),
                        1 => ColumnValues::Double(&numbers[i]),
                        2 => ColumnValues::Date(&dates[i]),
                        _ => ColumnValues::String(&borrowed[i]),
                    },
                    validity: if i % 2 == 0 { &[0b111] } else { &[0b101] },
                })
                .collect();
            let mut append = db
                .begin_append(
                    "wide",
                    AppendLimits {
                        batches: 1,
                        encoded_bytes: 1_000_000,
                    },
                    &cancel,
                )
                .unwrap();
            append.write(&input, &cancel).unwrap();
            append.commit(&cancel).unwrap();
            db.close().unwrap();
            let db = Database::open(&path, config).unwrap();
            let expected: Vec<Vec<Cell>> = (0..3)
                .map(|r| {
                    (0..64)
                        .map(|i| {
                            if r == 1 && i % 2 != 0 {
                                return Cell::Null;
                            }
                            match i % 4 {
                                0 => Cell::Integer((i * 100 + r) as i64),
                                1 => Cell::Number((i as f64 + r as f64 / 4.0).to_bits()),
                                2 => Cell::Day(i * 10 + r),
                                _ => Cell::Text(format!("欄-{i}-{r}")),
                            }
                        })
                        .collect()
                })
                .collect();
            let baseline = db.reserved_memory_bytes();
            for sql in [
                "FROM wide".to_owned(),
                format!("FROM wide |> SELECT {}", names.join(",")),
            ] {
                let q = db.prepare(&sql).unwrap();
                assert_eq!(q.result_column_count(), 64);
                for i in 0..64 {
                    assert_eq!(q.result_column(i).unwrap().data_type, kinds[i % 4]);
                }
                let mut result = db.execute(&q, &cancel).unwrap();
                assert_eq!(collect(&mut result), expected);
            }
            let q = db
                .prepare("FROM wide |> WHERE c60 >= 6001 |> SELECT c63 AS last,c60,c61,c62")
                .unwrap();
            let mut result = db.execute(&q, &cancel).unwrap();
            let mut selected: Vec<Vec<Cell>> = expected
                .iter()
                .skip(1)
                .map(|row| [63, 60, 61, 62].iter().map(|&i| row[i].clone()).collect())
                .collect();
            selected.sort_unstable();
            assert_eq!(collect(&mut result), selected);
            drop(result);
            drop(q);
            let q = db
                .prepare("FROM wide |> AGGREGATE SUM(c60) AS total")
                .unwrap();
            assert_eq!(
                collect(&mut db.execute(&q, &cancel).unwrap()),
                vec![vec![Cell::Integer(18003)]]
            );
            drop(q);
            order::query(
                &db,
                "FROM wide |> SELECT c60+1 AS x,c61*2 AS y |> WHERE x >= 6002 |> ORDER BY x",
                vec![
                    vec![Cell::Integer(6002), Cell::Null],
                    vec![Cell::Integer(6003), Cell::Number(123.0_f64.to_bits())],
                ],
            );
            order::query(
                &db,
                "FROM wide |> SELECT c60+1 AS x |> AGGREGATE SUM(x) AS s |> SELECT s+1 AS total",
                vec![vec![Cell::Integer(18007)]],
            );
            let arguments = (0..16)
                .map(|i| format!("c{}", i * 4))
                .collect::<Vec<_>>()
                .join("+");
            let q = db
                .prepare(&format!("FROM wide |> AGGREGATE SUM({arguments}) AS total"))
                .unwrap();
            assert_eq!(
                collect(&mut db.execute(&q, &cancel).unwrap()),
                vec![vec![Cell::Integer(144048)]]
            );
            drop(q);
            assert!(matches!(
                db.prepare(&format!("FROM wide |> SELECT {}", vec!["c0"; 65].join(","))),
                Err(Error::Parse { .. })
            ));
            let grouped = format!(
                "FROM wide |> AGGREGATE COUNT(*) AS n GROUP BY c60 |> SELECT {}",
                vec!["c60"; 64].join(",")
            );
            let q = db.prepare(&grouped).unwrap();
            assert_eq!(
                collect(&mut db.execute(&q, &cancel).unwrap()),
                (6000..6003)
                    .map(|v| vec![Cell::Integer(v); 64])
                    .collect::<Vec<_>>()
            );
            drop(q);
            let q = db.prepare("FROM wide").unwrap();
            let stopped = CancellationToken::new();
            let mut result = db.execute(&q, &stopped).unwrap();
            stopped.cancel();
            assert!(matches!(result.step(), QueryStep::Failed(Error::Cancelled)));
            drop(result);
            drop(q);
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
            db.close().unwrap();
            let db = Database::open(&path, Config::new(1_000_000, 16_000_000).unwrap()).unwrap();
            let baseline = db.reserved_memory_bytes();
            let q = db.prepare("FROM wide").unwrap();
            assert!(matches!(
                db.execute(&q, &cancel),
                Err(Error::Resource { .. })
            ));
            drop(q);
            let q = db.prepare("FROM wide |> SELECT c60").unwrap();
            assert_eq!(
                collect(&mut db.execute(&q, &cancel).unwrap()),
                (6000..6003)
                    .map(|v| vec![Cell::Integer(v)])
                    .collect::<Vec<_>>()
            );
            drop(q);
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
            db.close().unwrap();
        })
        .unwrap();
    worker.join().unwrap();
}
