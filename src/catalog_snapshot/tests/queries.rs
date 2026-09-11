//! Catalog scan values, pinned readers, demand, cancellation, and result ownership.
use super::{Fixture, append_columns, declarations, object};
use crate::catalog;
use crate::catalog_schema::{self, TableId};
use crate::effects::Effects;
use crate::namespace::UNITS_NAME;
use crate::native_unit::{self, InputColumn, InputValues};
use crate::table_data;
use crate::{CancellationToken, Database, Error};
use std::fs;

#[test]
fn catalog_native_text_feeds_bounded_result_ownership() {
    use crate::batch::Batch;
    use crate::frontend::DataType;
    use crate::{StringValue, Value};
    let parent = Fixture::directory();
    let path = parent.0.join("text-results");
    let database = Database::create_catalog_with_effects(
        &path,
        crate::Config::new(2_000_000, 1_000_000).unwrap(),
        &mut Effects::default(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let table = TableId::new(17).unwrap();
    database
        .catalog_writer()
        .unwrap()
        .create_table(
            "facts",
            table,
            &declarations(),
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    database
        .catalog_writer()
        .unwrap()
        .append(table, &append_columns(), &cancel, &mut Effects::default())
        .unwrap();
    let snapshot = database.catalog_snapshot().unwrap();
    let mut catalog_bytes = [0; catalog::MAX_BYTES];
    let catalog = snapshot
        .read_catalog(&mut catalog_bytes, &cancel, &mut Effects::default())
        .unwrap()
        .unwrap();
    let objects = path.join(UNITS_NAME);
    let mut schema_bytes = [0; catalog_schema::MAX_BYTES];
    let schema = catalog
        .read_schema(
            &objects,
            0,
            &mut schema_bytes,
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let mut index = catalog
        .open_data(&objects, 0, &cancel, &mut Effects::default())
        .unwrap()
        .unwrap();
    let reference = index
        .next(&cancel, &mut Effects::default())
        .unwrap()
        .unwrap();
    let unit = native_unit::read(
        &objects,
        database.database_identity(),
        reference,
        &schema,
        &cancel,
        &mut Effects::default(),
    )
    .unwrap();
    let mut payload = vec![0; native_unit::MAX_COLUMN_BYTES];
    let column = unit
        .read_column(
            catalog_schema::ColumnId::new(29).unwrap(),
            &mut payload,
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let types = [DataType::String];
    let capacities = [Some(6)];
    let bytes = Batch::required_bytes_with_text(&types, &capacities).unwrap();
    let charge = database
        .reserve_memory(bytes, "native text result test")
        .unwrap();
    let mut batch =
        Batch::new_with_text(&types, &capacities, database.config().memory_limit_bytes()).unwrap();
    for row in 0..4 {
        let value = column
            .string(row)
            .unwrap()
            .map_or(Value::Null, |s| Value::String(StringValue::new(s)));
        batch.set(row, 0, value).unwrap();
    }
    batch.publish_rows(4);
    // Reuse the original storage buffer: result text belongs to the batch.
    payload.fill(0);
    for (row, expected) in [Some("雪"), Some(""), Some("abc"), None]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            batch.value(row, 0),
            Some(expected.map_or(Value::Null, |s| Value::String(StringValue::new(s))))
        );
    }
    assert!(matches!(
        batch.set(0, 0, Value::String(StringValue::new("x"))),
        Err(Error::Resource {
            required: 7,
            limit: 6,
            ..
        })
    ));
    assert_eq!(
        batch.value(0, 0),
        Some(Value::String(StringValue::new("雪")))
    );
    drop(batch);
    drop(charge);
    assert_eq!(
        database.reserved_memory_bytes(),
        crate::catalog_snapshot::REGISTRY_BYTES + database.path_memory_bytes()
    );
}

#[test]
#[cfg_attr(
    all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"),
    ignore = "GNU aarch64 pthread minimum exceeds the 64-KiB reported-stack ceiling"
)]
fn catalog_readers_move_with_snapshot_and_cached_index_page() {
    struct Readers<'db> {
        payload: native_unit::ColumnBuffer,
        index: table_data::Cursor,
        unit: native_unit::Unit,
        snapshot: crate::catalog_snapshot::Snapshot<'db>,
    }
    let parent = Fixture::directory();
    let path = parent.0.join("owned-readers");
    let database = Database::create_catalog_with_effects(
        &path,
        crate::Config::new(2_000_000, 1_000_000).unwrap(),
        &mut Effects::default(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let table = TableId::new(17).unwrap();
    database
        .catalog_writer()
        .unwrap()
        .create_table(
            "facts",
            table,
            &declarations(),
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    for _ in 0..2 {
        database
            .catalog_writer()
            .unwrap()
            .append(table, &append_columns(), &cancel, &mut Effects::default())
            .unwrap();
    }
    let bytes = (std::mem::size_of::<Readers<'_>>() + native_unit::MAX_COLUMN_BYTES) as u64;
    let charge = database
        .reserve_memory(bytes, "retained native readers")
        .unwrap();
    let mut owner = crate::resources::allocate(1, 1, "retained reader allocation", bytes).unwrap();
    let readers = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(48 * 1024)
            .spawn_scoped(scope, || {
                let reported = pipesql_filesystem::test_current_thread_stack_bytes();
                assert!(
                    reported > 0 && reported <= 65_536,
                    "native reader stack: {reported}"
                );
                let snapshot = database.catalog_snapshot().unwrap();
                let mut catalog_bytes = [0; catalog::MAX_BYTES];
                let catalog = snapshot
                    .read_catalog(&mut catalog_bytes, &cancel, &mut Effects::default())
                    .unwrap()
                    .unwrap();
                let objects = path.join(UNITS_NAME);
                let mut schema_bytes = [0; catalog_schema::MAX_BYTES];
                let schema = catalog
                    .read_schema(
                        &objects,
                        0,
                        &mut schema_bytes,
                        &cancel,
                        &mut Effects::default(),
                    )
                    .unwrap();
                let mut index = catalog
                    .open_data(&objects, 0, &cancel, &mut Effects::default())
                    .unwrap()
                    .unwrap();
                let first = index
                    .next(&cancel, &mut Effects::default())
                    .unwrap()
                    .unwrap();
                let unit = native_unit::read(
                    &objects,
                    database.database_identity(),
                    first,
                    &schema,
                    &cancel,
                    &mut Effects::default(),
                )
                .unwrap();
                let mut bytes = crate::resources::allocate(
                    native_unit::MAX_COLUMN_BYTES,
                    native_unit::MAX_COLUMN_BYTES,
                    "retained column allocation",
                    database.config().memory_limit_bytes(),
                )
                .unwrap();
                bytes.resize(native_unit::MAX_COLUMN_BYTES, 0);
                let mut payload = native_unit::ColumnBuffer::new(bytes).unwrap();
                payload
                    .read(
                        &unit,
                        catalog_schema::ColumnId::new(29).unwrap(),
                        &cancel,
                        &mut Effects::default(),
                    )
                    .unwrap();
                Readers {
                    payload,
                    index,
                    unit,
                    snapshot,
                }
            })
            .unwrap()
            .join()
            .unwrap()
    });
    owner.push(readers);
    database
        .catalog_writer()
        .unwrap()
        .append(table, &append_columns(), &cancel, &mut Effects::default())
        .unwrap();
    let readers = &mut owner[0];
    assert_eq!(readers.snapshot.generation(), 3);
    assert_eq!(database.generation(), 4);
    assert_eq!(
        readers.payload.column().unwrap().string(0).unwrap(),
        Some("雪")
    );
    // Refill through the moved file owner, retaining the same payload allocation.
    readers
        .payload
        .read(
            &readers.unit,
            catalog_schema::ColumnId::new(29).unwrap(),
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    assert_eq!(
        readers.payload.column().unwrap().string(0).unwrap(),
        Some("雪")
    );
    let mut effects = Effects::default();
    assert_eq!(
        readers
            .index
            .next(&cancel, &mut effects)
            .unwrap()
            .unwrap()
            .rows(),
        4
    );
    assert!(readers.index.next(&cancel, &mut effects).unwrap().is_none());
    assert_eq!(
        effects.count(),
        0,
        "the moved cursor retains its verified page"
    );
    drop(owner);
    drop(charge);
    assert_eq!(
        database.reserved_memory_bytes(),
        crate::catalog_snapshot::REGISTRY_BYTES + database.path_memory_bytes()
    );
}

#[test]
fn catalog_query_scans_pinned_generations_and_reopen() {
    use crate::{QueryStep, Value};

    fn collect(result: &mut crate::QueryResult<'_, '_>) -> Vec<(Option<String>, u64)> {
        let mut values = Vec::new();
        for _ in 0..256 {
            match result.step() {
                QueryStep::Progress => {}
                QueryStep::Rows(batch) => {
                    assert_eq!(batch.column_count(), 2);
                    for row in 0..batch.len() {
                        let text = match batch.value(row, 0).unwrap() {
                            Value::Null => None,
                            Value::String(text) => Some(text.as_str().to_owned()),
                            value => panic!("unexpected text: {value:?}"),
                        };
                        let Value::Double(number) = batch.value(row, 1).unwrap() else {
                            panic!("DOUBLE");
                        };
                        values.push((text, number.to_bits()));
                    }
                }
                QueryStep::Finished => {
                    values.sort();
                    return values;
                }
                QueryStep::Failed(error) => panic!("query failed: {error:?}"),
            }
        }
        panic!("query did not finish within its finite test budget");
    }
    let parent = Fixture::directory();
    let path = parent.0.join("native-query");
    let config = crate::Config::new(4_000_000, 2_000_000).unwrap();
    let db = Database::create_catalog_with_effects(&path, config, &mut Effects::default()).unwrap();
    let table = TableId::new(17).unwrap();
    let cancel = CancellationToken::new();
    db.catalog_writer()
        .unwrap()
        .create_table(
            "facts",
            table,
            &declarations(),
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let sql = "FROM facts |> SELECT note AS text, amount AS n |> WHERE n > 0";
    let empty = crate::frontend::prepare_catalog(&db, sql).unwrap();
    let first = db
        .catalog_writer()
        .unwrap()
        .append(table, &append_columns(), &cancel, &mut Effects::default())
        .unwrap();
    let old = crate::frontend::prepare_catalog(&db, sql).unwrap();
    let mut running = db.execute(&old, &cancel).unwrap();
    let second = db
        .catalog_writer()
        .unwrap()
        .append(table, &append_columns(), &cancel, &mut Effects::default())
        .unwrap();
    let mut expected = vec![
        (Some("雪".to_owned()), 1_f64.to_bits()),
        (None, 42_f64.to_bits()),
    ];
    expected.sort();
    assert_eq!(collect(&mut running), expected);
    assert!(collect(&mut db.execute(&empty, &cancel).unwrap()).is_empty());
    assert_eq!(collect(&mut db.execute(&old, &cancel).unwrap()), expected);
    drop(running);
    drop(old);
    drop(empty);
    expected.extend(expected.clone());
    expected.sort();
    let current = crate::frontend::prepare_catalog(&db, sql).unwrap();
    assert_eq!(
        collect(&mut db.execute(&current, &cancel).unwrap()),
        expected
    );
    drop(current);
    assert_eq!(
        db.reserved_memory_bytes(),
        crate::catalog_snapshot::REGISTRY_BYTES + db.path_memory_bytes()
    );
    db.close().unwrap();
    let db = Database::open(&path, config).unwrap();
    for receipt in [first, second] {
        assert!(matches!(
            db.resolve_catalog(receipt.transaction(), &mut Effects::default())
                .unwrap(),
            crate::CommitResolution::Durable(_)
        ));
    }
    let reopened = crate::frontend::prepare_catalog(&db, sql).unwrap();
    assert_eq!(
        collect(&mut db.execute(&reopened, &cancel).unwrap()),
        expected
    );
}

#[test]
#[cfg_attr(
    all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"),
    ignore = "GNU aarch64 pthread minimum exceeds the 64-KiB reported-stack ceiling"
)]
fn catalog_query_text_boundaries_cancellation_and_memory() {
    use crate::{QueryStep, Value};
    let parent = Fixture::directory();
    let path = parent.0.join("native-text-query");
    let config = crate::Config::new(4_000_000, 2_000_000).unwrap();
    let db = Database::create_catalog_with_effects(&path, config, &mut Effects::default()).unwrap();
    let table = TableId::new(17).unwrap();
    let cancel = CancellationToken::new();
    db.catalog_writer()
        .unwrap()
        .create_table(
            "facts",
            table,
            &declarations(),
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let full = "é".repeat(native_unit::MAX_TEXT_BYTES / 2);
    let strings = [full.as_str(), full.as_str(), full.as_str(), "ignored"];
    let columns = [
        InputColumn {
            id: catalog_schema::ColumnId::new(29).unwrap(),
            values: InputValues::String(&strings),
            validity: &[7],
        },
        InputColumn {
            id: catalog_schema::ColumnId::new(3).unwrap(),
            values: InputValues::Double(&[10., 20., 30., 40.]),
            validity: &[15],
        },
    ];
    db.catalog_writer()
        .unwrap()
        .append(table, &columns, &cancel, &mut Effects::default())
        .unwrap();
    let query = crate::frontend::prepare_catalog(&db, "FROM facts |> SELECT amount,note").unwrap();
    let before = db.reserved_memory_bytes();
    let held = db
        .reserve_memory(
            config.memory_limit_bytes() - before - 2048,
            "force query refusal",
        )
        .unwrap();
    assert!(matches!(
        db.execute(&query, &cancel),
        Err(Error::Resource { .. })
    ));
    assert_eq!(db.reserved_memory_bytes(), before + held.bytes());
    drop(held);
    // The whole native query admission and drain must fit the observed stack.
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(48 * 1024)
            .spawn_scoped(scope, || {
                let reported = pipesql_filesystem::test_current_thread_stack_bytes();
                assert!(
                    reported > 0 && reported <= 65_536,
                    "native scan stack: {reported}"
                );
                let mut running = db.execute(&query, &cancel).unwrap();
                let charged = db.reserved_memory_bytes();
                let mut rows = 0;
                let mut batches = 0;
                let mut finished = false;
                for _ in 0..64 {
                    match running.step() {
                        QueryStep::Progress => {}
                        QueryStep::Rows(batch) => {
                            batches += 1;
                            let mut bytes = 0;
                            for row in 0..batch.len() {
                                assert_eq!(
                                    batch.value(row, 0),
                                    Some(Value::Double(10. * (rows + 1) as f64))
                                );
                                if rows < 3 {
                                    let Some(Value::String(value)) = batch.value(row, 1) else {
                                        panic!("text value");
                                    };
                                    assert_eq!(value.as_str(), full);
                                    bytes += value.as_str().len();
                                } else {
                                    assert_eq!(batch.value(row, 1), Some(Value::Null));
                                }
                                rows += 1;
                            }
                            assert!(bytes <= crate::batch::MAX_TEXT_BYTES);
                        }
                        QueryStep::Finished => {
                            finished = true;
                            break;
                        }
                        QueryStep::Failed(error) => panic!("text query failed: {error:?}"),
                    }
                    assert_eq!(db.reserved_memory_bytes(), charged);
                }
                assert!(finished);
                assert_eq!(rows, 4);
                assert_eq!(batches, 3);
            })
            .unwrap()
            .join()
            .unwrap();
    });
    assert_eq!(db.reserved_memory_bytes(), before);
    let cancellation = CancellationToken::new();
    let mut running = db.execute(&query, &cancellation).unwrap();
    assert!(matches!(running.step(), QueryStep::Progress));
    cancellation.cancel();
    assert!(matches!(
        running.step(),
        QueryStep::Failed(Error::Cancelled)
    ));
    assert!(matches!(
        running.step(),
        QueryStep::Failed(Error::Cancelled)
    ));
    drop(running);
    assert_eq!(db.reserved_memory_bytes(), before);
    assert!(matches!(
        db.execute(&query, &cancellation),
        Err(Error::Cancelled)
    ));
}

#[test]
fn catalog_query_reads_only_demanded_payloads() {
    use crate::QueryStep;
    let parent = Fixture::directory();
    let path = parent.0.join("native-demand");
    let config = crate::Config::new(4_000_000, 2_000_000).unwrap();
    let db = Database::create_catalog_with_effects(&path, config, &mut Effects::default()).unwrap();
    let table = TableId::new(17).unwrap();
    let cancel = CancellationToken::new();
    db.catalog_writer()
        .unwrap()
        .create_table(
            "facts",
            table,
            &declarations(),
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let receipt = db
        .catalog_writer()
        .unwrap()
        .append(table, &append_columns(), &cancel, &mut Effects::default())
        .unwrap();
    let filtered =
        crate::frontend::prepare_catalog(&db, "FROM facts |> WHERE amount < 0 |> SELECT note")
            .unwrap();
    let demanded = crate::frontend::prepare_catalog(&db, "FROM facts |> SELECT note").unwrap();
    // Independent format-6 corruption: retain unit metadata and corrupt only
    // the payload described by stable STRING identity 29.
    let name = object(receipt.transaction().sequence(), 1).name();
    let file = path
        .join(UNITS_NAME)
        .join(std::str::from_utf8(&name).unwrap());
    let mut bytes = fs::read(&file).unwrap();
    let descriptor = bytes[64..128]
        .as_chunks::<32>()
        .0
        .iter()
        .find(|entry| u32::from_le_bytes(entry[..4].try_into().unwrap()) == 29)
        .unwrap();
    let offset = u64::from_le_bytes(descriptor[8..16].try_into().unwrap()) as usize;
    bytes[offset] ^= 1;
    fs::write(&file, bytes).unwrap();
    let mut running = db.execute(&filtered, &cancel).unwrap();
    let mut finished = false;
    for _ in 0..32 {
        match running.step() {
            QueryStep::Progress => {}
            QueryStep::Finished => {
                finished = true;
                break;
            }
            _ => panic!("filtered query read undemanded text"),
        }
    }
    assert!(finished);
    drop(running);
    let before = db.reserved_memory_bytes();
    let mut running = db.execute(&demanded, &cancel).unwrap();
    let mut failed = false;
    for _ in 0..32 {
        match running.step() {
            QueryStep::Progress => {}
            QueryStep::Failed(Error::Corrupt(_)) => {
                failed = true;
                break;
            }
            _ => panic!("corrupt demanded text escaped"),
        }
    }
    assert!(failed);
    assert!(matches!(
        running.step(),
        QueryStep::Failed(Error::Corrupt(_))
    ));
    drop(running);
    assert_eq!(db.reserved_memory_bytes(), before);
}

#[test]
fn catalog_query_typed_nulls_cross_row_quanta() {
    use crate::frontend::DataType;
    use crate::{QueryStep, Value};
    let parent = Fixture::directory();
    let path = parent.0.join("native-typed-query");
    let db = Database::create_catalog_with_effects(
        &path,
        crate::Config::new(4_000_000, 2_000_000).unwrap(),
        &mut Effects::default(),
    )
    .unwrap();
    let table = TableId::new(31).unwrap();
    let cancel = CancellationToken::new();
    let id = |n| catalog_schema::ColumnId::new(n).unwrap();
    let specs = [
        catalog_schema::ColumnSpec::new(id(17), "n", DataType::Int64, true).unwrap(),
        catalog_schema::ColumnSpec::new(id(5), "day", DataType::Date, true).unwrap(),
        catalog_schema::ColumnSpec::new(id(90), "amount", DataType::Double, true).unwrap(),
        catalog_schema::ColumnSpec::new(id(7), "note", DataType::String, true).unwrap(),
    ];
    db.catalog_writer()
        .unwrap()
        .create_table("typed", table, &specs, &cancel, &mut Effects::default())
        .unwrap();
    let rows: usize = 4101;
    let threshold = 9_007_199_254_740_993_i64;
    let mut integers: Vec<_> = (0..rows).map(|row| threshold + row as i64).collect();
    integers[rows - 1] = i64::MAX;
    let dates: Vec<_> = (0..rows)
        .map(|row| crate::DateValue::from_days(if row % 2 == 0 { 10_957 } else { 0 }).unwrap())
        .collect();
    let numbers: Vec<_> = (0..rows)
        .map(|row| {
            [
                f64::from_bits(0x7ff8000000000042),
                -0.0,
                f64::INFINITY,
                f64::NEG_INFINITY,
                1.25,
            ][row % 5]
        })
        .collect();
    let strings = vec!["雪"; rows];
    let validity = |period| {
        let mut bits = vec![0; rows.div_ceil(8)];
        for row in 0..rows {
            if row % period != 0 {
                bits[row / 8] |= 1 << (row % 8);
            }
        }
        bits
    };
    let int_bits = validity(7);
    let date_bits = validity(11);
    let number_bits = validity(13);
    let text_bits = validity(17);
    let columns = [
        InputColumn {
            id: id(7),
            values: InputValues::String(&strings),
            validity: &text_bits,
        },
        InputColumn {
            id: id(90),
            values: InputValues::Double(&numbers),
            validity: &number_bits,
        },
        InputColumn {
            id: id(5),
            values: InputValues::Date(&dates),
            validity: &date_bits,
        },
        InputColumn {
            id: id(17),
            values: InputValues::Int64(&integers),
            validity: &int_bits,
        },
    ];
    db.catalog_writer()
        .unwrap()
        .append(table, &columns, &cancel, &mut Effects::default())
        .unwrap();
    let query = crate::frontend::prepare_catalog(&db, "FROM typed |> WHERE n > 9007199254740993 |> WHERE day >= DATE '2000-01-01' |> SELECT amount,note,n,day").unwrap();
    let mut result = db.execute(&query, &cancel).unwrap();
    let mut observed = Vec::new();
    let mut finished = false;
    for _ in 0..256 {
        match result.step() {
            QueryStep::Progress => {}
            QueryStep::Rows(batch) => {
                assert!(batch.len() <= 256);
                for row in 0..batch.len() {
                    let numeric = match batch.value(row, 0).unwrap() {
                        Value::Null => None,
                        Value::Double(value) => Some(value.to_bits()),
                        _ => panic!("DOUBLE"),
                    };
                    let text = match batch.value(row, 1).unwrap() {
                        Value::Null => None,
                        Value::String(value) => Some(value.as_str().to_owned()),
                        _ => panic!("STRING"),
                    };
                    let Some(Value::Int64(n)) = batch.value(row, 2) else {
                        panic!("INT64");
                    };
                    let Some(Value::Date(day)) = batch.value(row, 3) else {
                        panic!("DATE");
                    };
                    observed.push((n, numeric, text, day));
                }
            }
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("typed query failed: {error:?}"),
        }
    }
    assert!(finished);
    let expected: Vec<_> = (0..rows)
        .filter(|&row| row % 7 != 0 && row % 11 != 0 && row % 2 == 0 && integers[row] > threshold)
        .map(|row| {
            (
                integers[row],
                (row % 13 != 0).then_some(numbers[row].to_bits()),
                (row % 17 != 0).then(|| "雪".to_owned()),
                dates[row],
            )
        })
        .collect();
    observed.sort_by_key(|row| row.0);
    assert_eq!(observed, expected);
}
