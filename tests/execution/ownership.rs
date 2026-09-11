use super::{Q1, Q6, ROW, TempDir, config};
use pipesql::{CancellationToken, Database, QueryStep, Value};
use std::fs;

#[test]
fn stock_public_load_reopen_prepare_execute_and_release() {
    let temp = TempDir::new();
    let database_path = temp.0.join("database");
    let input_path = temp.0.join("lineitem.tbl");
    fs::write(&input_path, ROW).expect("write input");
    let mut database = Database::create(&database_path, config()).expect("create");
    database
        .load_lineitem(&input_path, &CancellationToken::new())
        .expect("load");
    database.close().expect("close loaded database");

    let database = Database::open(&database_path, config()).expect("open");

    let resident = database.reserved_memory_bytes();
    let prepared = database.prepare(Q6).expect("prepare");
    let prepared_bytes = prepared.accounted_memory_bytes();
    let cancellation = CancellationToken::new();
    let mut result = database.execute(&prepared, &cancellation).expect("execute");
    let mut rows = 0;
    let mut finished = false;
    for _ in 0..1024 {
        match result.step() {
            QueryStep::Rows(batch) => {
                rows += batch.len();
                assert_eq!(batch.len(), 1);
                assert_eq!(batch.column_count(), 1);
                assert_eq!(batch.value(0, 0), Some(Value::Double(8.0)));
                assert_eq!(batch.value(0, 1), None);
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("Q6 failed: {error:?}"),
        }
    }
    assert!(finished);
    assert_eq!(rows, 1);
    assert!(matches!(result.step(), QueryStep::Finished));
    drop(result);
    assert_eq!(database.reserved_memory_bytes(), resident + prepared_bytes);
    drop(prepared);
    assert_eq!(database.reserved_memory_bytes(), resident);

    let prepared = database.prepare(Q1).expect("prepare Q1");
    assert_eq!(prepared.result_column_count(), 10);
    let cancellation = CancellationToken::new();
    let mut result = database
        .execute(&prepared, &cancellation)
        .expect("execute Q1");
    let mut rows = 0;
    let mut finished = false;
    for _ in 0..1024 {
        match result.step() {
            QueryStep::Rows(batch) => {
                rows += batch.len();
                assert_eq!(batch.len(), 1);
                assert_eq!(batch.column_count(), 10);
                let Some(Value::String(return_flag)) = batch.value(0, 0) else {
                    panic!("Q1 returnflag is not STRING");
                };
                let Some(Value::String(line_status)) = batch.value(0, 1) else {
                    panic!("Q1 linestatus is not STRING");
                };
                assert_eq!(return_flag.as_str(), "R");
                assert_eq!(line_status.as_str(), "F");
                assert_eq!(batch.value(0, 2), Some(Value::Double(1.0)));
                assert_eq!(batch.value(0, 3), Some(Value::Double(100.0)));
                assert_eq!(batch.value(0, 9), Some(Value::Int64(1)));
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("Q1 failed: {error:?}"),
        }
    }
    assert!(finished);
    assert_eq!(rows, 1);
    assert!(matches!(result.step(), QueryStep::Finished));
    drop(result);
    drop(prepared);
    assert_eq!(database.reserved_memory_bytes(), resident);
    database.close().expect("close");
}

#[test]
fn arithmetic_errors_own_byte_ranges_after_query_and_source_drop() {
    let temp = TempDir::new();
    let input = temp.0.join("input.tbl");
    fs::write(&input, ROW.repeat(2)).unwrap();
    let mut database = Database::create(&temp.0.join("database"), config()).unwrap();
    let cancel = CancellationToken::new();
    database.load_lineitem(&input, &cancel).unwrap();
    let baseline = database.reserved_memory_bytes();
    for (sql, fragment, operation, prepare_failure) in [
        (
            "# 雪\nFROM lineitem |> WHERE l_quantity < (9223372036854775807 + 1)",
            "(9223372036854775807 + 1)",
            "addition",
            true,
        ),
        (
            "FROM lineitem |> LIMIT 0 OFFSET +(9223372036854775807 + 1)",
            "+(9223372036854775807 + 1)",
            "addition",
            true,
        ),
        (
            "FROM lineitem |> AGGREGATE SUM(l_quantity) AS safe,AVG(9223372036854775807 * 2) AS bad |> SELECT bad",
            "AVG(9223372036854775807 * 2)",
            "multiplication",
            false,
        ),
        (
            "# 雪\nFROM lineitem |> AGGREGATE SUM(9223372036854775807 * 2) AS unused,AVG(9223372036854775807 * 2) AS mean |> SELECT mean",
            "AVG(9223372036854775807 * 2)",
            "multiplication",
            false,
        ),
        (
            "FROM lineitem |> AGGREGATE AVG(9223372036854775807) AS mean,SUM(9223372036854775807) AS total |> LIMIT 1 |> SELECT total",
            "SUM(9223372036854775807)",
            "SUM",
            false,
        ),
        (
            "FROM lineitem |> AGGREGATE AVG(1.0e308) AS mean,SUM(1.0e308) AS total",
            "SUM(1.0e308)",
            "SUM",
            false,
        ),
        (
            "FROM lineitem |> AGGREGATE SUM(1.0e308*2.0) AS unused,AVG(1.0e308*2.0) AS mean |> SELECT mean",
            "AVG(1.0e308*2.0)",
            "multiplication",
            false,
        ),
    ] {
        let source = sql.to_owned();
        let start = source.find(fragment).unwrap();
        let expected = start..start + fragment.len();
        let error = if prepare_failure {
            let error = database.prepare(&source).err().unwrap();
            drop(source);
            error
        } else {
            let query = database.prepare(&source).unwrap();
            drop(source);
            let mut result = database.execute(&query, &cancel).unwrap();
            let mut failed = false;
            for _ in 0..10_000 {
                match result.step() {
                    QueryStep::Progress => (),
                    QueryStep::Failed(_) => {
                        failed = true;
                        break;
                    }
                    _ => panic!("expected arithmetic failure before rows"),
                }
            }
            assert!(failed);
            let error = result.into_error().unwrap();
            drop(query);
            error
        };
        assert!(
            matches!(&error, pipesql::Error::ArithmeticOverflow { operation: actual, span } if *actual == operation && (span.start()..span.end()) == expected)
        );
        assert_eq!(
            error.to_string(),
            format!(
                "arithmetic overflow during {operation} at bytes {}..{}",
                expected.start, expected.end
            )
        );
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
}
