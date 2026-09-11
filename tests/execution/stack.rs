use super::{Q1, Q6, ROW, TempDir, config};
use pipesql::{CancellationToken, Database, QueryStep, Value};
use std::fs;

#[test]
#[cfg_attr(
    all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"),
    ignore = "GNU aarch64 has a 128-KiB pthread minimum; this test requires at most 64 KiB"
)]
fn loaded_open_and_queries_fit_the_reported_stack_allowance() {
    let temp = TempDir::new();
    let path = temp.0.join("database");
    let input = temp.0.join("lineitem.tbl");
    fs::write(&input, ROW).unwrap();
    let mut database = Database::create(&path, config()).unwrap();
    database
        .load_lineitem(&input, &CancellationToken::new())
        .unwrap();
    database.close().unwrap();

    std::thread::Builder::new()
        // Observe the native extent too: requested and reported sizes differ.
        .stack_size(48 * 1024)
        .spawn(move || {
            let reported = pipesql_filesystem::test_current_thread_stack_bytes();
            assert!(reported > 0 && reported <= 65_536);
            let database = Database::open(&path, config()).unwrap();
            let resident = database.reserved_memory_bytes();
            let deep_limit = format!("FROM lineitem{}", " |> LIMIT 1 OFFSET 0".repeat(16));
            let q1_limit = format!("{} |> LIMIT 1", Q1.trim().trim_end_matches(';'));
            for source in [Q6, Q1, &deep_limit, &q1_limit] {
                let query = database.prepare(source).unwrap();
                let cancellation = CancellationToken::new();
                let mut result = database.execute(&query, &cancellation).unwrap();
                let mut rows = 0;
                let mut finished = false;
                for _ in 0..1024 {
                    match result.step() {
                        QueryStep::Rows(batch) => rows += batch.len(),
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
            // Constants and aggregate arguments share parser storage, including
            // BETWEEN's two bounds. Exercise their independent programs through
            // the public runtime on the same measured small stack.
            let source = "FROM lineitem |> WHERE l_quantity BETWEEN (3-3) AND (1+1) |> WHERE l_shipdate >= DATE_ADD(DATE '1993-12-31',INTERVAL 1 DAY) |> AGGREGATE SUM((l_quantity+2)*3) AS s,AVG(l_quantity*4) AS a,COUNT(*) AS n |> WHERE s > (2*4) |> LIMIT (2-1) OFFSET (3-3)";
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
        })
        .unwrap()
        .join()
        .unwrap();
}
