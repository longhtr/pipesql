use super::{
    ARENA_BYTES, BLOCK_ROWS, DESCRIPTOR_CEILING, Layout, MAX_WORKSPACE_BYTES, SELECTION_CEILING,
    buffer_range, read_date, read_f64,
};
use crate::Config;
use crate::StringValue;
use crate::effects::Faults;
use crate::effects::{Effect, Effects, QueryEffect};
use crate::execution::planning::{PhysicalPlan, lower, validate_physical};
use crate::execution::runtime::Runtime;
use crate::execution::scan::Source;
use crate::execution::{BATCH_ROWS, QueryStep, RESULT_BYTES, State};
use crate::frontend::Comparison;
use crate::frontend::{DataType, FilterLiteral, MAX_COLUMNS, Predicate};
use crate::namespace::inspect_namespace;
use crate::storage_format::{BlockDescriptor, RootState};
use crate::{CancellationToken, Database, DateValue, Error, Value};
use std::mem::size_of;
use std::sync::atomic::Ordering;

use crate::execution::test_support::{Fixture, NEXT, SQL, drain, finish_query, loaded};

#[test]
fn emitted_batch_outlives_source_and_keeps_its_charge() {
    let (_fixture, database) = loaded(3);
    for (sql, expected) in [
        ("FROM lineitem |> SELECT l_quantity", Value::Double(0.0)),
        (
            "FROM lineitem |> SELECT l_quantity+1 AS q",
            Value::Double(1.0),
        ),
        ("FROM lineitem |> AGGREGATE COUNT(*) AS n", Value::Int64(3)),
    ] {
        let query = database.prepare(sql).unwrap();
        let baseline = database.reserved_memory_bytes();
        let cancel = CancellationToken::new();
        let mut result = database.execute(&query, &cancel).unwrap();
        let mut emitted = false;
        for _ in 0..1024 {
            match result.step() {
                QueryStep::Rows(_) => {
                    emitted = true;
                    break;
                }
                QueryStep::Progress => (),
                _ => panic!("expected result batch"),
            }
        }
        assert!(emitted);
        let batch = {
            let State::Running(workspace) = std::mem::replace(&mut result.state, State::Finished)
            else {
                panic!("running producer")
            };
            workspace.into_output()
        };
        drop(result);
        cancel.cancel();
        assert!(batch.memory_bytes() > 0);
        assert_eq!(
            database.reserved_memory_bytes(),
            baseline + batch.memory_bytes()
        );
        assert_eq!(batch.value(0, 0), Some(expected));
        drop(batch);
        assert_eq!(database.reserved_memory_bytes(), baseline);
    }
}

#[test]
fn shared_readers_refuse_repair_without_mutating_namespace() {
    let (_fixture, database) = loaded(1);
    // Preserve the original campaign's allowance for two concurrent queries.
    let path = database.path().to_owned();
    database.close().unwrap();
    let database = Database::open(&path, Config::new(4_000_000, 1_000_000).unwrap()).unwrap();
    let missing = database.path().join(crate::namespace::ROOT_B_NAME);
    std::fs::remove_file(&missing).unwrap();
    let transaction = crate::TransactionId::for_attempt(database.database_identity(), 1).unwrap();
    std::thread::scope(|scope| {
        let readers: Vec<_> = [
            include_str!("../../../../tests/fixtures/q6.pipe.sql"),
            include_str!("../../../../tests/fixtures/upstream/q1-upstream.pipe.sql"),
        ]
        .into_iter()
        .map(|source| {
            let database = &database;
            scope.spawn(move || {
                let query = database.prepare(source).unwrap();
                let cancellation = CancellationToken::new();
                assert!(matches!(
                    database.execute(&query, &cancellation),
                    Err(Error::RecoveryRequired { .. })
                ));
                assert!(matches!(
                    database.resolve_commit(transaction),
                    Err(Error::RecoveryRequired { .. })
                ));
            })
        })
        .collect();
        for reader in readers {
            reader.join().unwrap();
        }
    });
    assert!(!missing.exists());
    assert!(!database.path().join("ROOT.B.next").exists());
    assert_eq!(
        database.reserved_memory_bytes(),
        database.path_memory_bytes()
    );
    let path = database.path().to_owned();
    database.close().unwrap();
    let reopened = Database::open(&path, Config::new(2_000_000, 1_000_000).unwrap()).unwrap();
    assert!(missing.is_file());
    let query = reopened
        .prepare(include_str!("../../../../tests/fixtures/q6.pipe.sql"))
        .unwrap();
    let cancellation = CancellationToken::new();
    assert_eq!(
        finish_query(
            reopened.execute(&query, &cancellation).unwrap(),
            &mut Effects::default()
        )
        .unwrap(),
        1
    );
}

#[test]
fn cancellation_at_every_effect_and_from_real_thread() {
    let (_fixture, database) = loaded(BLOCK_ROWS + 1);
    for source in [
        include_str!("../../../../tests/fixtures/q6.pipe.sql"),
        include_str!("../../../../tests/fixtures/upstream/q1-upstream.pipe.sql"),
    ] {
        let query = database.prepare(source).unwrap();
        let token = CancellationToken::new();
        let mut control = Effects::default();
        let result = database
            .execute_with_effects(&query, &token, &mut control)
            .unwrap();
        assert_eq!(finish_query(result, &mut control).unwrap(), 1);
        for cut in 0..control.count() {
            let token = std::sync::Arc::new(CancellationToken::new());
            let trigger = std::sync::Arc::clone(&token);
            let mut effects = Effects::with_faults(Faults {
                action: Some(Box::new(move |index, _| {
                    if index == cut {
                        trigger.cancel();
                    }
                })),
                ..Faults::default()
            });
            let result = database
                .execute_with_effects(&query, &token, &mut effects)
                .and_then(|result| finish_query(result, &mut effects));
            assert!(
                matches!(result, Err(Error::Cancelled)),
                "cancel effect {cut}"
            );
            assert_eq!(
                database.reserved_memory_bytes(),
                query.accounted_memory_bytes() + database.path_memory_bytes()
            );
        }
        let token = CancellationToken::new();
        let (entered_send, entered_receive) = std::sync::mpsc::sync_channel(0);
        let (resume_send, resume_receive) = std::sync::mpsc::sync_channel(0);
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let mut notified = false;
                let mut effects = Effects::with_faults(Faults {
                    action: Some(Box::new(move |_, effect| {
                        if !notified && effect == Effect::Query(QueryEffect::ReadPayload) {
                            notified = true;
                            entered_send.send(()).unwrap();
                            resume_receive.recv().unwrap();
                        }
                    })),
                    ..Faults::default()
                });
                database
                    .execute_with_effects(&query, &token, &mut effects)
                    .and_then(|result| finish_query(result, &mut effects))
            });
            entered_receive
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            token.cancel();
            resume_send.send(()).unwrap();
            assert!(matches!(worker.join().unwrap(), Err(Error::Cancelled)));
        });
        assert_eq!(
            database.reserved_memory_bytes(),
            query.accounted_memory_bytes() + database.path_memory_bytes()
        );
    }
}

#[test]
fn foreign_plans_and_value_extent_neighbors_refuse() {
    let (_left_fixture, left) = loaded(1);
    let (_right_fixture, right) = loaded(1);
    for source in [
        include_str!("../../../../tests/fixtures/q6.pipe.sql"),
        include_str!("../../../../tests/fixtures/upstream/q1-upstream.pipe.sql"),
    ] {
        let query = left.prepare(source).unwrap();
        let token = CancellationToken::new();
        let mut effects = Effects::default();
        assert!(
            right
                .execute_with_effects(&query, &token, &mut effects)
                .is_err()
        );
        assert_eq!(effects.count(), 0);
        assert_eq!(right.reserved_memory_bytes(), right.path_memory_bytes());
    }
    for raw in [
        0,
        1,
        1_u64 << 63,
        0x7ff0000000000000,
        0x7ff0000000000001,
        0x7ff8000000000001,
    ] {
        assert_eq!(read_f64(&raw.to_le_bytes(), 0).unwrap().to_bits(), raw);
    }
    assert!(read_date(&[0; 3], 0).is_err());
    assert!(read_date(&[0; 4], usize::MAX).is_err());
    assert!(read_date(&i32::MAX.to_le_bytes(), 0).is_err());
    for index in [1, usize::MAX, usize::MAX / size_of::<f64>()] {
        assert!(read_f64(&[0; 8], index).is_err());
    }
    assert!(read_f64(&[0; 7], 0).is_err());
}

#[test]
fn all_stored_values_across_date_and_numeric_boundaries() {
    let (_fixture, database) = loaded(2 * BLOCK_ROWS + 1);
    let query = database.prepare("FROM lineitem").unwrap();
    assert_eq!(query.result_column_count(), 7);
    let cancellation = CancellationToken::new();
    let mut effects = Effects::default();
    let mut result = database
        .execute_with_effects(&query, &cancellation, &mut effects)
        .unwrap();
    let mut rows = 0;
    let mut finished = false;
    for _ in 0..4096 {
        let before = effects.count();
        match result.step_with_effects(&mut effects) {
            QueryStep::Progress => (),
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("{error:?}"),
            QueryStep::Rows(batch) => {
                assert!(batch.len() <= BATCH_ROWS);
                for row in 0..batch.len() {
                    assert_eq!(batch.value(row, 0), Some(Value::Double((rows % 31) as f64)));
                    assert_eq!(
                        batch.value(row, 1),
                        Some(Value::Double((rows % 97 + 1) as f64))
                    );
                    let Some(Value::String(flag)) = batch.value(row, 4) else {
                        panic!("STRING column");
                    };
                    assert_eq!(flag.as_str(), "A");
                    let Some(Value::Date(date)) = batch.value(row, 6) else {
                        panic!("DATE column");
                    };
                    assert_eq!(date.days_since_unix_epoch(), 8766);
                    assert_eq!(date.to_string(), "1994-01-01");
                    assert_eq!(batch.value(row, 7), None);
                    rows += 1;
                }
                assert_eq!(batch.value(batch.len(), 0), None);
            }
        }
        assert!(effects.count() - before <= 1, "one block read per step");
    }
    assert!(finished);
    assert_eq!(rows, 2 * BLOCK_ROWS + 1);
    assert_eq!(result.accounted_memory_bytes(), RESULT_BYTES);
    drop(result);
    drop(query);
    assert_eq!(
        database.reserved_memory_bytes(),
        database.path_memory_bytes()
    );
}

#[test]
fn stored_blocks_ownership_and_effect_cuts() {
    let (_fixture, database) = loaded(BLOCK_ROWS + 1);
    let query = database.prepare(SQL).unwrap();
    let cancellation = CancellationToken::new();
    let mut control = Effects::default();
    let mut result = database
        .execute_with_effects(&query, &cancellation, &mut control)
        .unwrap();
    let expected: Vec<_> = (0..BLOCK_ROWS + 1).filter(|row| row % 31 < 25).collect();
    assert_eq!(
        drain(&mut result, &mut control).unwrap(),
        (
            expected.len(),
            expected.iter().map(|row| (row % 97 + 1) as u64).sum()
        )
    );
    assert_eq!(result.accounted_memory_bytes(), RESULT_BYTES);
    drop(result);
    let effects_count = control.count();
    let mut prefix_failures = 0;
    for short in [false, true] {
        for cut in 0..effects_count {
            let mut effects = Effects::with_faults(Faults {
                fail_at: (!short).then_some(cut),
                short_at: short.then_some(cut),
                ..Faults::default()
            });
            let mut prefix = 0;
            match database.execute_with_effects(&query, &cancellation, &mut effects) {
                Err(_) => (),
                Ok(mut result) => {
                    let mut failed = false;
                    let mut terminated = false;
                    for _ in 0..100_000 {
                        match result.step_with_effects(&mut effects) {
                            QueryStep::Rows(batch) => prefix += batch.len(),
                            QueryStep::Progress => (),
                            QueryStep::Finished => {
                                terminated = true;
                                break;
                            } // Short non-byte effects are no-ops.
                            QueryStep::Failed(error) => {
                                assert!(matches!(error, Error::Io { .. }));
                                failed = true;
                                terminated = true;
                                break;
                            }
                        }
                    }
                    assert!(
                        terminated,
                        "fault path must terminate within the step allowance"
                    );
                    if failed {
                        assert_eq!(result.accounted_memory_bytes(), RESULT_BYTES);
                        let count = effects.count();
                        assert!(matches!(
                            result.step_with_effects(&mut effects),
                            QueryStep::Failed(_)
                        ));
                        assert_eq!(effects.count(), count, "terminal error must do no more I/O");
                        if prefix > 0 {
                            prefix_failures += 1;
                        }
                    } else {
                        assert!(short, "every refused effect must terminate");
                    }
                    drop(result);
                }
            }
            assert_eq!(
                database.reserved_memory_bytes(),
                query.accounted_memory_bytes() + database.path_memory_bytes()
            );
            assert_eq!(database.reserved_temp_bytes(), 0);
        }
    }
    assert!(prefix_failures >= 2);
    println!(
        "stream effects={effects_count} refusal/short cells={} prefix failures={prefix_failures}",
        effects_count * 2
    );
}

#[test]
fn cancellation_drop_and_empty_progress() {
    let (_fixture, database) = loaded(BLOCK_ROWS + 1);
    for source in [
        SQL,
        "FROM lineitem |> WHERE l_quantity < -1.0 |> SELECT l_extendedprice",
    ] {
        for after_rows in [false, true] {
            let query = database.prepare(source).unwrap();
            let cancellation = CancellationToken::new();
            let mut result = database.execute(&query, &cancellation).unwrap();
            let mut outputs = 0;
            for step in 0..100_000 {
                match result.step() {
                    QueryStep::Rows(batch) => outputs += batch.len(),
                    QueryStep::Progress => (),
                    QueryStep::Finished => break,
                    QueryStep::Failed(_) => panic!("healthy scan failed"),
                }
                if !after_rows || outputs > 0 || step == 32 {
                    cancellation.cancel();
                    assert!(matches!(result.step(), QueryStep::Failed(Error::Cancelled)));
                    assert_eq!(result.accounted_memory_bytes(), RESULT_BYTES);
                    assert!(matches!(result.step(), QueryStep::Failed(Error::Cancelled)));
                    break;
                }
            }
            drop(result);
            assert_eq!(
                database.reserved_memory_bytes(),
                query.accounted_memory_bytes() + database.path_memory_bytes()
            );
            drop(query);
            assert_eq!(
                database.reserved_memory_bytes(),
                database.path_memory_bytes()
            );
        }
    }
}

#[test]
fn every_projection_mask_has_exact_buffers_and_values() {
    let (_fixture, database) = loaded(65_537);
    let names = [
        "l_quantity",
        "l_extendedprice",
        "l_discount",
        "l_tax",
        "l_returnflag",
        "l_linestatus",
        "l_shipdate",
    ];
    let capacities = [262_144, 262_144, 262_144, 262_144, 32_768, 32_768, 262_144];
    let cancellation = CancellationToken::new();
    for mask in 1_u8..128 {
        // Reverse storage order and repeat one column: physical positions
        // and repeated outputs must not change source buffer ownership.
        let mut columns: Vec<usize> = (0..7)
            .rev()
            .filter(|column| mask & (1 << column) != 0)
            .collect();
        columns.push(columns[0]);
        let source = format!(
            "FROM lineitem |> SELECT {}",
            columns
                .iter()
                .enumerate()
                .map(|(index, column)| format!("{} AS c{index}", names[*column]))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let query = database.prepare(&source).unwrap();
        let expected_arena = (0..7)
            .filter(|column| mask & (1 << column) != 0)
            .map(|column| capacities[column])
            .sum::<usize>()
            .max(28_672);
        let widths = [8, 8, 8, 8, 1, 1, 4];
        let batch_bytes = columns
            .iter()
            .map(|column| widths[*column] * 256 + 64)
            .sum::<usize>();
        let physical = lower(&database, &query, RootState::Empty, 0).unwrap();
        let runtime_bytes = Runtime::required_bytes(&physical);
        drop(physical);
        let required = 106_496
            + expected_arena as u64
            + batch_bytes as u64
            + RESULT_BYTES
            + PhysicalPlan::required_bytes(&query)
            + runtime_bytes;
        let available = database.config().memory_limit_bytes() - database.reserved_memory_bytes();
        let pressure = database
            .reserve_memory(available - required + 1, "one byte short")
            .unwrap();
        let mut effects = Effects::default();
        assert!(matches!(
            database.execute_with_effects(&query, &cancellation, &mut effects),
            Err(Error::Resource { .. })
        ));
        assert_eq!(effects.count(), 0);
        drop(pressure);
        let pressure = database
            .reserve_memory(available - required, "exact projection budget")
            .unwrap();
        let mut result = database.execute(&query, &cancellation).unwrap();
        assert_eq!(result.accounted_memory_bytes(), required);
        let State::Running(workspace) = &result.state else {
            panic!("loaded workspace")
        };
        let Source::Legacy(scan) = &workspace.scan().source else {
            panic!("stock projection");
        };
        assert_eq!(scan.arena.len(), expected_arena);
        assert_eq!(scan.arena.capacity(), expected_arena);
        assert_eq!(scan.buffers, Layout::new(&query, &result.plan).offsets);
        let mut invalid = Layout::new(&query, &result.plan);
        invalid.input_types[0] = DataType::Int64;
        assert!(invalid.validate(&result.plan, &query).is_err());
        invalid = Layout::new(&query, &result.plan);
        invalid.input_count += 1;
        assert!(invalid.validate(&result.plan, &query).is_err());
        for offset in 0..8 {
            let mut invalid = Layout::new(&query, &result.plan);
            invalid.offsets[offset] += 1;
            assert!(invalid.validate(&result.plan, &query).is_err());
        }
        for column in 0..7 {
            if mask & (1 << column) == 0 {
                assert!(buffer_range(&scan.buffers, column).is_err());
            }
        }
        let mut rows = 0_usize;
        let mut finished = false;
        for _ in 0..4096 {
            match result.step() {
                QueryStep::Rows(batch) => {
                    for row in 0..batch.len() {
                        let ordinal = rows + row;
                        let expected = [
                            Value::Double((ordinal % 31) as f64),
                            Value::Double((ordinal % 97 + 1) as f64),
                            Value::Double(0.08),
                            Value::Double(0.1),
                            Value::String(StringValue::from_byte(b'A').unwrap()),
                            Value::String(StringValue::from_byte(b'F').unwrap()),
                            Value::Date(DateValue::from_days(8766).unwrap()),
                        ];
                        for (position, column) in columns.iter().enumerate() {
                            assert_eq!(batch.value(row, position), Some(expected[*column]));
                        }
                    }
                    rows += batch.len();
                }
                QueryStep::Progress => (),
                QueryStep::Finished => {
                    finished = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("projection mask {mask}: {error:?}"),
            }
        }
        assert!(finished);
        assert_eq!(rows, 65_537);
        drop(result);
        drop(pressure);
        assert_eq!(
            database.reserved_memory_bytes(),
            query.accounted_memory_bytes() + database.path_memory_bytes()
        );
        drop(query);
        assert_eq!(
            database.reserved_memory_bytes(),
            database.path_memory_bytes()
        );
    }
}

#[test]
fn count_without_values_retains_metadata_scratch_without_payload_reads() {
    let (_fixture, database) = loaded(65_537);
    let query = database
        .prepare("FROM lineitem |> AGGREGATE COUNT(*) AS n")
        .unwrap();
    let cancellation = CancellationToken::new();
    let mut effects = Effects::with_faults(Faults {
        action: Some(Box::new(|_, effect| {
            assert_ne!(effect, Effect::Query(QueryEffect::ReadPayload));
        })),
        ..Faults::default()
    });
    let mut result = database
        .execute_with_effects(&query, &cancellation, &mut effects)
        .unwrap();
    let State::Running(workspace) = &result.state else {
        panic!("count workspace")
    };
    let Source::Legacy(scan) = &workspace.scan().source else {
        panic!("stock count");
    };
    assert_eq!(scan.arena.len(), 28_672);
    assert_eq!(
        workspace.memory_bytes(),
        // The runtime now also owns COUNT's one u32 row counter.
        106_496
            + 28_672
            + 64
            + 256 * 8
            + size_of::<u32>() as u64
            + Runtime::required_bytes(&result.plan)
    );
    assert_eq!(scan.buffers, [0; 8]);
    let mut rows = 0;
    let mut finished = false;
    for _ in 0..4096 {
        match result.step_with_effects(&mut effects) {
            QueryStep::Rows(batch) => {
                assert_eq!(batch.len(), 1);
                assert_eq!(batch.value(0, 0), Some(Value::Int64(65_537)));
                rows += 1;
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("{error:?}"),
        }
    }
    assert!(finished);
    assert_eq!(rows, 1);
    drop(result);
    assert_eq!(
        database.reserved_memory_bytes(),
        query.accounted_memory_bytes() + database.path_memory_bytes()
    );
}

#[test]
fn admission_before_io_and_physical_validation() {
    let (_fixture, database) = loaded(1);
    let query = database.prepare(SQL).unwrap();
    let cancellation = CancellationToken::new();
    let physical = lower(&database, &query, RootState::Empty, 0).unwrap();
    let required = Layout::new(&query, &physical).workspace_bytes().unwrap()
        + RESULT_BYTES
        + PhysicalPlan::required_bytes(&query)
        + Runtime::required_bytes(&physical);
    drop(physical);
    let remaining = database.config().memory_limit_bytes() - database.reserved_memory_bytes();
    let occupied = database
        .reserve_memory(remaining - required + 1, "test pressure")
        .unwrap();
    let mut effects = Effects::default();
    assert!(matches!(
        database.execute_with_effects(&query, &cancellation, &mut effects),
        Err(Error::Resource { .. })
    ));
    assert_eq!(effects.count(), 0);
    drop(occupied);
    let occupied = database
        .reserve_memory(remaining - required, "exact test pressure")
        .unwrap();
    let mut result = database.execute(&query, &cancellation).unwrap();
    assert_eq!(drain(&mut result, &mut Effects::default()).unwrap(), (1, 1));
    drop(result);
    drop(occupied);
    let namespace =
        inspect_namespace(database.path(), &database.memory, &mut Effects::default()).unwrap();
    for mutation in 0..7 {
        let mut changed_predicate;
        let mut plan = lower(
            &database,
            &query,
            namespace.state,
            namespace.projected_crc32c,
        )
        .unwrap();
        changed_predicate = *plan.scan().filters[0].predicate;
        match mutation {
            0 => plan.scan_mut().column_count = MAX_COLUMNS + 1,
            1 => plan.scan_mut().columns[0] = 3,
            2 => plan.scan_mut().filter_count = 0,
            3 => {
                let Predicate::Compare { literal, .. } = &mut changed_predicate else {
                    unreachable!()
                };
                *literal = FilterLiteral::Double(f64::NAN.to_bits());
                plan.scan_mut().filters[0].predicate = &changed_predicate;
            }
            4 => {
                let Predicate::Compare { comparison, .. } = &mut changed_predicate else {
                    unreachable!()
                };
                *comparison = Comparison::Greater;
                plan.scan_mut().filters[0].predicate = &changed_predicate;
            }
            5 => plan.generation = 0,
            6 => plan.projected_crc32c ^= 1,
            _ => unreachable!(),
        }
        assert!(
            validate_physical(
                &plan,
                &query,
                &database,
                namespace.state,
                namespace.projected_crc32c
            )
            .is_err()
        );
    }
    let heap_ceiling = ARENA_BYTES
        + 2 * crate::batch::MAX_BYTES as usize
        + DESCRIPTOR_CEILING * size_of::<BlockDescriptor>()
        + SELECTION_CEILING * size_of::<u32>();
    assert!(heap_ceiling + 65536 <= MAX_WORKSPACE_BYTES as usize);
}

#[test]
fn empty_computed_result_releases_unused_scratch_before_return() {
    let directory = std::env::temp_dir().join(format!(
        "pipesql-empty-computed-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&directory).unwrap();
    let fixture = Fixture(directory);
    let database = Database::create(
        &fixture.0.join("database"),
        Config::new(2_000_000, 1_000_000).unwrap(),
    )
    .unwrap();
    let query = database
        .prepare("FROM lineitem |> LIMIT 1 |> SELECT 1 AS x")
        .unwrap();
    let baseline = database.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let mut result = database.execute(&query, &cancel).unwrap();
    assert_eq!(result.accounted_memory_bytes(), RESULT_BYTES);
    assert!(matches!(result.step(), QueryStep::Finished));
    assert_eq!(database.reserved_memory_bytes(), baseline + RESULT_BYTES);
    drop(result);
    assert_eq!(database.reserved_memory_bytes(), baseline);
}

#[test]
fn computed_scan_reserves_before_io_at_exact_and_next_byte() {
    let (_fixture, database) = loaded(1);
    let query = database
        .prepare("FROM lineitem |> SELECT l_quantity*2 AS x |> WHERE x >= 0")
        .unwrap();
    let cancel = CancellationToken::new();
    let physical = lower(&database, &query, RootState::Empty, 0).unwrap();
    let required = Layout::new(&query, &physical).workspace_bytes().unwrap()
        + RESULT_BYTES
        + PhysicalPlan::required_bytes(&query)
        + Runtime::required_bytes(&physical);
    drop(physical);
    let remaining = database.config().memory_limit_bytes() - database.reserved_memory_bytes();
    let pressure = database
        .reserve_memory(remaining - required + 1, "computed next-byte pressure")
        .unwrap();
    let mut effects = Effects::default();
    assert!(matches!(
        database.execute_with_effects(&query, &cancel, &mut effects),
        Err(Error::Resource { .. })
    ));
    assert_eq!(effects.count(), 0);
    drop(pressure);
    let baseline = database.reserved_memory_bytes();
    let pressure = database
        .reserve_memory(remaining - required, "computed exact pressure")
        .unwrap();
    let mut result = database.execute(&query, &cancel).unwrap();
    assert_eq!(drain(&mut result, &mut Effects::default()).unwrap(), (1, 0));
    drop(result);
    drop(pressure);
    assert_eq!(database.reserved_memory_bytes(), baseline);
}

#[test]
#[cfg_attr(
    all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"),
    ignore = "GNU aarch64 pthread minimum exceeds the 64-KiB reported-stack ceiling"
)]
fn shared_threads_and_small_reported_stack() {
    let (_fixture, database) = loaded(4097);
    let path = database.path().to_owned();
    database.close().unwrap();
    std::thread::Builder::new()
        .stack_size(48 * 1024)
        .spawn(move || {
            let reported = pipesql_filesystem::test_current_thread_stack_bytes();
            assert!(
                reported <= 65536,
                "actual stack exceeds test ceiling: {reported}"
            );
            let database =
                Database::open(&path, Config::new(4_000_000, 1_000_000).unwrap()).unwrap();
            std::thread::scope(|scope| {
                let left = scope.spawn(|| {
                    let query = database.prepare(SQL).unwrap();
                    let cancellation = CancellationToken::new();
                    let mut result = database.execute(&query, &cancellation).unwrap();
                    drain(&mut result, &mut Effects::default()).unwrap()
                });
                let right = scope.spawn(|| {
                    let query = database.prepare(SQL).unwrap();
                    let cancellation = CancellationToken::new();
                    let mut result = database.execute(&query, &cancellation).unwrap();
                    drain(&mut result, &mut Effects::default()).unwrap()
                });
                assert_eq!(left.join().unwrap(), right.join().unwrap());
            });
            // This call, including metadata admission and decoding, uses the measured small stack.
            let query = database.prepare(SQL).unwrap();
            let cancellation = CancellationToken::new();
            let mut result = database.execute(&query, &cancellation).unwrap();
            drain(&mut result, &mut Effects::default()).unwrap();
            drop(result);
            drop(query);
            assert_eq!(database.reserved_memory_bytes(), database.path_memory_bytes());
            let query = database
                .prepare(include_str!("../../../../tests/fixtures/upstream/q1-upstream.pipe.sql"))
                .unwrap();
            let mut result = database.execute(&query, &cancellation).unwrap();
            let mut rows = 0;
            let mut finished = false;
            for _ in 0..4096 {
                match result.step() {
                    QueryStep::Rows(batch) => rows += batch.len(),
                    QueryStep::Progress => (),
                    QueryStep::Finished => {
                        finished = true;
                        break;
                    }
                    QueryStep::Failed(error) => panic!("{error:?}"),
                }
            }
            assert!(finished);
            assert_eq!(rows, 1);
            drop(result);
            drop(query);
            assert_eq!(database.reserved_memory_bytes(), database.path_memory_bytes());
            let query = database.prepare("FROM lineitem |> AGGREGATE SUM(l_quantity) AS s,AVG(l_quantity) AS a,COUNT(*) AS n GROUP AND ORDER BY l_returnflag,l_linestatus |> SELECT n AS count,a AS mean |> WHERE count > 0 |> SELECT mean").unwrap();
            let mut result = database.execute(&query, &cancellation).unwrap();
            let mut finished = false;
            for _ in 0..4096 {
                match result.step() {
                    QueryStep::Finished => { finished = true; break; }
                    QueryStep::Failed(error) => panic!("{error:?}"),
                    QueryStep::Rows(batch) => assert_eq!(batch.column_count(), 1),
                    QueryStep::Progress => (),
                }
            }
            assert!(finished);
            drop(result);
            drop(query);
            assert_eq!(database.reserved_memory_bytes(), database.path_memory_bytes());
            for sql in [
                "FROM lineitem |> SELECT l_quantity*2 AS q,l_returnflag AS k |> AGGREGATE SUM(q) AS s,COUNT(*) AS n GROUP BY k |> SELECT s+1 AS x,n |> WHERE n > 0 |> SELECT x",
                "FROM lineitem |> SELECT l_quantity AS q |> SELECT q+q AS q |> SELECT q+q AS q |> SELECT q+q AS q |> SELECT q+q AS q |> SELECT q+q AS q |> SELECT q+q AS q |> AGGREGATE SUM(q) AS s |> SELECT s+1 AS x",
            ] {
                let query = database.prepare(sql).unwrap();
                let mut result = database.execute(&query, &cancellation).unwrap();
                let mut finished = false;
                for _ in 0..4096 {
                    match result.step() {
                        QueryStep::Finished => { finished = true; break; }
                        QueryStep::Failed(error) => panic!("{sql}: {error:?}"),
                        QueryStep::Rows(batch) => assert_eq!(batch.column_count(), 1),
                        QueryStep::Progress => (),
                    }
                }
                assert!(finished);
                drop(result);
                drop(query);
                assert_eq!(database.reserved_memory_bytes(), database.path_memory_bytes());
            }
            println!("stream, aggregate and computed projection reported stack={reported}");
        })
        .unwrap()
        .join()
        .unwrap();
}
