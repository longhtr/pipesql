use super::{Q1, ROW, TempDir, config};
use pipesql::{CancellationToken, Database, QueryStep, Value};
use std::{fs, io::Write};

#[test]
fn every_admitted_key_pair_survives_public_load_reopen_scan_and_grouping() {
    // Derive the fixture from the input contract, independently of engine
    // key indexing: printable ASCII includes space and excludes the delimiter.
    let keys: Vec<u8> = (b' '..=b'~').filter(|byte| *byte != b'|').collect();
    assert_eq!(keys.len(), 94);
    let temp = TempDir::new();
    let path = temp.0.join("database");
    let input = temp.0.join("keys.tbl");
    let mut file = std::io::BufWriter::new(fs::File::create(&input).unwrap());
    let mut expected = Vec::new();
    for &flag in &keys {
        for &status in &keys {
            writeln!(
                file,
                "1|2|3|4|1|100|0.08|8|{}|{}|1994-01-01|12|13|14|15|16|",
                char::from(flag),
                char::from(status)
            )
            .unwrap();
            expected.push((flag, status));
        }
    }
    file.flush().unwrap();
    drop(file);
    let mut database = Database::create(&path, config()).unwrap();
    database
        .load_lineitem(&input, &CancellationToken::new())
        .unwrap();
    database.close().unwrap();
    let database = Database::open(&path, config()).unwrap();
    let resident = database.reserved_memory_bytes();
    for (sql, ordered) in [
        ("FROM lineitem |> SELECT l_returnflag,l_linestatus", false),
        (Q1, true),
    ] {
        let query = database.prepare(sql).unwrap();
        let cancellation = CancellationToken::new();
        let mut result = database.execute(&query, &cancellation).unwrap();
        let mut actual = Vec::new();
        let mut finished = false;
        for _ in 0..100_000 {
            match result.step() {
                QueryStep::Rows(batch) => {
                    for row in 0..batch.len() {
                        let (Some(Value::String(flag)), Some(Value::String(status))) =
                            (batch.value(row, 0), batch.value(row, 1))
                        else {
                            panic!("STRING keys");
                        };
                        actual.push((flag.as_str().as_bytes()[0], status.as_str().as_bytes()[0]));
                        if ordered {
                            assert_eq!(batch.value(row, 9), Some(Value::Int64(1)));
                        }
                    }
                }
                QueryStep::Progress => (),
                QueryStep::Finished => {
                    finished = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("loaded key failed to query: {error:?}"),
            }
        }
        assert!(finished);
        if !ordered {
            actual.sort_unstable();
        }
        assert_eq!(actual, expected);
    }
    assert_eq!(database.reserved_memory_bytes(), resident);
}

#[test]
fn legacy_limit_composes_across_batches_and_empty_aggregation() {
    let temp = TempDir::new();
    let path = temp.0.join("database");
    let input = temp.0.join("lineitem.tbl");
    fs::write(&input, ROW.repeat(600)).unwrap();
    let mut database = Database::create(&path, config()).unwrap();
    let cancel = CancellationToken::new();
    // Exercise the uninitialized namespace and the same graph after publication.
    for loaded in [false, true] {
        if loaded {
            database.load_lineitem(&input, &cancel).unwrap();
        }
        let baseline = database.reserved_memory_bytes();
        for (sql, expected) in [
            (
                "FROM lineitem |> LIMIT 257 OFFSET 255 |> AGGREGATE COUNT(*) AS n",
                Some(if loaded { 257 } else { 0 }),
            ),
            (
                "FROM lineitem |> LIMIT 599 |> LIMIT 4 OFFSET 597 |> AGGREGATE COUNT(*) AS n",
                Some(if loaded { 2 } else { 0 }),
            ),
            (
                "FROM lineitem |> LIMIT 0 OFFSET 9223372036854775807 |> AGGREGATE COUNT(*) AS n",
                Some(0),
            ),
            ("FROM lineitem |> AGGREGATE COUNT(*) AS n |> LIMIT 0", None),
            (
                "FROM lineitem |> AGGREGATE COUNT(*) AS n |> LIMIT 1",
                Some(if loaded { 600 } else { 0 }),
            ),
            (
                "FROM lineitem |> AGGREGATE COUNT(*) AS n |> LIMIT 1 OFFSET 1",
                None,
            ),
            (
                "FROM lineitem |> LIMIT 0 |> AGGREGATE COUNT(*) AS n GROUP BY l_returnflag |> LIMIT 1",
                None,
            ),
            (
                "FROM lineitem |> SELECT l_returnflag,l_quantity |> LIMIT 3 |> SELECT l_quantity,l_returnflag |> AGGREGATE COUNT(*) AS n GROUP BY l_returnflag |> LIMIT 1 |> SELECT n",
                if loaded { Some(3) } else { None },
            ),
        ] {
            let query = database.prepare(sql).unwrap();
            let mut result = database.execute(&query, &cancel).unwrap();
            let mut values = vec![];
            let mut finished = false;
            for _ in 0..2000 {
                match result.step() {
                    QueryStep::Progress => (),
                    QueryStep::Rows(batch) => {
                        for row in 0..batch.len() {
                            let Some(Value::Int64(value)) = batch.value(row, 0) else {
                                panic!("{sql}");
                            };
                            values.push(value);
                        }
                    }
                    QueryStep::Finished => {
                        finished = true;
                        break;
                    }
                    QueryStep::Failed(error) => panic!("{sql}: {error}"),
                }
            }
            assert!(finished, "{sql}");
            assert_eq!(values, expected.into_iter().collect::<Vec<_>>(), "{sql}");
            drop(result);
            drop(query);
            assert_eq!(database.reserved_memory_bytes(), baseline, "{sql}");
            assert_eq!(database.reserved_temp_bytes(), 0, "{sql}");
        }
        // SUM must resolve the consumer's schema after LIMIT and projection,
        // rather than reusing the scan's opposite physical column order.
        let sql = "FROM lineitem |> SELECT l_extendedprice,l_quantity |> LIMIT 3 |> SELECT l_quantity,l_extendedprice |> AGGREGATE SUM(l_quantity) AS total,SUM(l_extendedprice) AS total_price |> LIMIT 1 |> SELECT total_price,total";
        let query = database.prepare(sql).unwrap();
        let mut result = database.execute(&query, &cancel).unwrap();
        let mut rows = 0;
        let mut finished = false;
        for _ in 0..2000 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Rows(batch) => {
                    assert_eq!(batch.len(), 1);
                    assert_eq!(batch.column_count(), 2);
                    for (column, value) in [300.0, 3.0].into_iter().enumerate() {
                        assert_eq!(
                            batch.value(0, column),
                            Some(if loaded {
                                Value::Double(value)
                            } else {
                                Value::Null
                            })
                        );
                    }
                    rows += batch.len();
                }
                QueryStep::Finished => {
                    finished = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("reordered legacy SUM: {error}"),
            }
        }
        assert!(finished);
        assert_eq!(rows, 1);
        drop(result);
        drop(query);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
    database.close().unwrap();
}
