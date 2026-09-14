use super::{Q1, Q6, ROW, TempDir, config};
use pipesql::{CancellationToken, Database, QueryStep, Value};
use std::{fs, path::Path};

#[test]
fn loaded_open_and_queries_preserve_results_and_release_owners() {
    let temp = loaded_database();
    let path = temp.0.join("database");
    std::thread::spawn(move || check_loaded_queries(&path))
        .join()
        .unwrap();
}

#[test]
fn loaded_open_and_queries_fit_the_reported_stack_allowance() {
    let temp = loaded_database();
    let path = temp.0.join("database");
    std::thread::Builder::new()
        // Observe the native extent too: requested and reported sizes differ.
        .stack_size(pipesql_filesystem::TEST_SMALL_STACK_REQUEST_BYTES)
        .spawn(move || {
            pipesql_filesystem::test_assert_small_stack();
            check_loaded_queries(&path);
        })
        .unwrap()
        .join()
        .unwrap();
}

fn loaded_database() -> TempDir {
    let temp = TempDir::new();
    let path = temp.0.join("database");
    let input = temp.0.join("lineitem.tbl");
    fs::write(&input, ROW).unwrap();
    let mut database = Database::create(&path, config()).unwrap();
    database
        .load_lineitem(&input, &CancellationToken::new())
        .unwrap();
    database.close().unwrap();

    temp
}

fn check_loaded_queries(path: &Path) {
    let database = Database::open(path, config()).unwrap();
    let resident = database.reserved_memory_bytes();
    let deep_limit = format!("FROM lineitem{}", " |> LIMIT 1 OFFSET 0".repeat(16));
    let q1_limit = format!("{} |> LIMIT 1", Q1.trim().trim_end_matches(';'));
    let byte_lengths =
        "FROM lineitem |> SELECT BYTE_LENGTH(l_returnflag) AS width, BYTE_LENGTH('é') AS unicode";
    let conditional_lengths = "FROM lineitem |> AGGREGATE COUNT(*) AS entries GROUP BY l_returnflag |> SELECT BYTE_LENGTH(l_returnflag) AS width, BYTE_LENGTH('é') AS unicode |> SELECT COALESCE(width, 9223372036854775807 + 1) AS width, unicode";
    let char_lengths = byte_lengths.replace("BYTE_LENGTH", "CHAR_LENGTH");
    let conditional_char_lengths = conditional_lengths.replace("BYTE_LENGTH", "CHAR_LENGTH");
    for source in [
        Q6,
        Q1,
        &deep_limit,
        &q1_limit,
        byte_lengths,
        conditional_lengths,
        &char_lengths,
        &conditional_char_lengths,
    ] {
        let query = database.prepare(source).unwrap();
        let cancellation = CancellationToken::new();
        let mut result = database.execute(&query, &cancellation).unwrap();
        let mut rows = 0;
        let mut finished = false;
        for _ in 0..1024 {
            match result.step() {
                QueryStep::Rows(batch) => {
                    if source == byte_lengths || source == conditional_lengths {
                        assert_eq!(batch.column_count(), 2);
                        assert_eq!(batch.value(0, 0), Some(Value::Int64(1)));
                        assert_eq!(batch.value(0, 1), Some(Value::Int64(2)));
                    }
                    if source == char_lengths || source == conditional_char_lengths {
                        assert_eq!(batch.column_count(), 2);
                        assert_eq!(batch.value(0, 0), Some(Value::Int64(1)));
                        assert_eq!(batch.value(0, 1), Some(Value::Int64(1)));
                    }
                    rows += batch.len();
                }
                QueryStep::Progress => (),
                QueryStep::Finished => {
                    finished = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("query failed: {error:?}"),
            }
        }
        assert!(finished);
        assert_eq!(rows, 1);
    }
    // Full-width literals exercise owned prepared values and UTF-8
    // batch admission on the same small stack as ordinary legacy queries.
    let text = "12345678901234567890123456789012";
    let sql = format!(
        "FROM lineitem |> SELECT {}",
        vec![format!("'{text}'"); 64].join(", ")
    );
    {
        let query = database.prepare(&sql).unwrap();
        let cancel = CancellationToken::new();
        let mut result = database.execute(&query, &cancel).unwrap();
        let mut rows = 0;
        let mut finished = false;
        for _ in 0..1024 {
            match result.step() {
                QueryStep::Rows(batch) => {
                    assert_eq!(batch.column_count(), 64);
                    for row in 0..batch.len() {
                        for column in 0..64 {
                            let Some(Value::String(actual)) = batch.value(row, column) else {
                                panic!("literal text");
                            };
                            assert_eq!(actual.as_str(), text);
                        }
                    }
                    rows += batch.len();
                }
                QueryStep::Progress => (),
                QueryStep::Finished => {
                    finished = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("constant projection: {error}"),
            }
        }
        assert!(finished);
        assert_eq!(rows, 1);
    }
    // Constants and aggregate arguments share parser storage, including
    // BETWEEN's two bounds. Exercise their independent programs through
    // the public runtime.
    let source = "FROM lineitem |> WHERE l_quantity BETWEEN (3-3) AND (1+1) |> WHERE l_shipdate >= DATE_ADD(DATE '1993-12-31', INTERVAL 1 DAY) |> AGGREGATE SUM((COALESCE(l_quantity, 1/0)+2)*3) AS s, AVG(l_quantity*4) AS a, COUNT(*) AS n |> SELECT COALESCE(s, 1/0) AS s, a, n |> WHERE s > (2*4) |> LIMIT (2-1) OFFSET (3-3)";
    {
        let query = database.prepare(source).unwrap();
        let cancellation = CancellationToken::new();
        let mut result = database.execute(&query, &cancellation).unwrap();
        let mut rows = 0;
        let mut finished = false;
        for _ in 0..1024 {
            match result.step() {
                QueryStep::Rows(batch) => {
                    assert_eq!(batch.len(), 1);
                    assert_eq!(batch.value(0, 0), Some(Value::Double(9.0)));
                    assert_eq!(batch.value(0, 1), Some(Value::Double(4.0)));
                    assert_eq!(batch.value(0, 2), Some(Value::Int64(1)));
                    rows += batch.len();
                }
                QueryStep::Progress => (),
                QueryStep::Finished => {
                    finished = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("mixed expressions failed: {error:?}"),
            }
        }
        assert!(finished);
        assert_eq!(rows, 1);
    }
    assert_eq!(database.reserved_memory_bytes(), resident);
    database.close().unwrap();
}
