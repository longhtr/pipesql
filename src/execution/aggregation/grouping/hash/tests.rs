use super::*;
use crate::effects::Faults;
use crate::execution::aggregation::grouping::reduction::Reduction;
use crate::execution::blocking::test_support::{Directory, schema};
use crate::execution::blocking::{
    Io, MAX_KEYS, RECORD_HEADER, RowSort, SortPhase, SortRecord, append_value,
};
use crate::execution::scan::{Source, declared};
use crate::execution::*;
use crate::frontend::DataType;

fn database(directory: &Directory) -> Database {
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(16_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    database
        .declare_table(
            "facts",
            &[
                crate::ColumnDeclaration {
                    name: "n",
                    data_type: DataType::Int64,
                    nullable: true,
                },
                crate::ColumnDeclaration {
                    name: "d",
                    data_type: DataType::Double,
                    nullable: false,
                },
            ],
            &CancellationToken::new(),
        )
        .unwrap();
    database
}

fn append(database: &Database, values: &[i64]) {
    assert!(!values.is_empty() && values.len() <= 2);
    let cancel = CancellationToken::new();
    let doubles: Vec<_> = values.iter().map(|value| *value as f64).collect();
    let valid = [(1 << values.len()) - 1];
    let mut writer = database
        .begin_append(
            "facts",
            crate::AppendLimits {
                batches: 1,
                encoded_bytes: 10_000,
            },
            &cancel,
        )
        .unwrap();
    writer
        .write(
            &[
                crate::ColumnInput {
                    values: crate::ColumnValues::Int64(values),
                    validity: &valid,
                },
                crate::ColumnInput {
                    values: crate::ColumnValues::Double(&doubles),
                    validity: &valid,
                },
            ],
            &cancel,
        )
        .unwrap();
    writer.commit(&cancel).unwrap();
}

fn scan<'a>(running: &'a mut QueryResult<'_, '_>) -> &'a mut declared::Scan {
    let State::Running(workspace) = &mut running.state else {
        panic!("live query workspace");
    };
    let Source::Declared(owner) = workspace.scan_mut().source_mut() else {
        panic!("declared-table source");
    };
    &mut owner[0]
}

#[test]
fn native_restart_keeps_the_pinned_input_and_cannot_revive_failed_work() {
    let directory = Directory::new();
    let database = database(&directory);
    append(&database, &[1, 2]);
    append(&database, &[3, 4]);
    let cancel = CancellationToken::new();
    let query = database.prepare("FROM facts |> SELECT n").unwrap();
    let mut running = database.execute(&query, &cancel).unwrap();
    let mut effects = Effects::default();
    assert!(scan(&mut running).next_unit(&cancel, &mut effects).unwrap());
    scan(&mut running).load(0, &cancel, &mut effects).unwrap();
    assert_eq!(scan(&mut running).value(0, 0).unwrap(), Value::Int64(1));
    append(&database, &[5, 6]);
    database.reclaim(&cancel).unwrap();
    let at = effects.count();
    scan(&mut running).restart_once(&cancel).unwrap();
    assert_eq!(
        effects.count(),
        at,
        "restart moves cursors without reopening the catalog"
    );
    assert!(!scan(&mut running).has_unit());
    assert_eq!(scan(&mut running).rows(), 0);
    assert!(scan(&mut running).value(0, 0).is_err());
    let mut observed = Vec::new();
    for _ in 0..3 {
        let scan = scan(&mut running);
        if !scan.next_unit(&cancel, &mut effects).unwrap() {
            break;
        }
        scan.load(0, &cancel, &mut effects).unwrap();
        for row in 0..scan.rows() {
            let Value::Int64(value) = scan.value(0, row).unwrap() else {
                panic!("typed fixture");
            };
            observed.push(value);
        }
        scan.finish_unit();
    }
    assert_eq!(
        observed,
        [1, 2, 3, 4],
        "replay must exclude the later committed unit"
    );
    assert!(matches!(
        scan(&mut running).restart_once(&cancel),
        Err(Error::Resource {
            required: 2,
            limit: 1,
            ..
        })
    ));
    let at = effects.count();
    assert!(scan(&mut running).next_unit(&cancel, &mut effects).is_err());
    assert_eq!(effects.count(), at);
    drop(running);
    let current = database.prepare("FROM facts |> SELECT n").unwrap();
    let mut running = database.execute(&current, &cancel).unwrap();
    let mut observed = Vec::new();
    for _ in 0..100 {
        match running.step() {
            QueryStep::Rows(rows) => {
                for row in 0..rows.len() {
                    let Some(Value::Int64(value)) = rows.value(row, 0) else {
                        panic!("typed fixture");
                    };
                    observed.push(value);
                }
            }
            QueryStep::Finished => break,
            QueryStep::Failed(error) => panic!("public query failed: {error:?}"),
            QueryStep::Progress => {}
        }
    }
    assert_eq!(observed, [1, 2, 3, 4, 5, 6]);
    drop(running);
    // Index-page failure and unit-open failure leave different inner cursor
    // states. Neither permits a source restart after the outer read failed.
    for fail_at in [0, 1] {
        let mut running = database.execute(&query, &cancel).unwrap();
        let mut failed = Effects::with_faults(Faults {
            fail_at: Some(fail_at),
            ..Faults::default()
        });
        assert!(scan(&mut running).next_unit(&cancel, &mut failed).is_err());
        assert!(scan(&mut running).restart_once(&cancel).is_err());
        let at = effects.count();
        assert!(scan(&mut running).next_unit(&cancel, &mut effects).is_err());
        assert_eq!(effects.count(), at);
    }
    let all = database.prepare("FROM facts").unwrap();
    let mut running = database.execute(&all, &cancel).unwrap();
    scan(&mut running).next_unit(&cancel, &mut effects).unwrap();
    scan(&mut running).load(0, &cancel, &mut effects).unwrap();
    let mut failed = Effects::with_faults(Faults {
        fail_at: Some(0),
        ..Faults::default()
    });
    assert!(scan(&mut running).load(1, &cancel, &mut failed).is_err());
    assert!(
        scan(&mut running).value(0, 0).is_err(),
        "failure closes access to prior payloads"
    );
    assert!(scan(&mut running).restart_once(&cancel).is_err());
}

#[test]
fn memory_groups_match_sorted_reduction_without_owning_its_reservation() {
    let directory = Directory::new();
    let database = database(&directory);
    let cancel = CancellationToken::new();
    let query = database.prepare("FROM facts |> AGGREGATE COUNT(*) AS nrows,SUM(n) AS ns,AVG(n) AS na,SUM(d) AS ds,AVG(d) AS da,SUM(2) AS twos").unwrap();
    let semantic = query.plan.aggregates.first().unwrap();
    let mut keys = schema(&[(DataType::Double, true)]);
    keys.columns[0].input = 2;
    let nan = f64::from_bits(0x7ff8_0000_0000_0123);
    let rows = [
        (Value::Double(-0.0), Some(i64::MAX), f64::MAX),
        (Value::Double(nan), None, -0.0),
        (Value::Double(0.0), Some(i64::MAX), f64::MAX),
        (Value::Null, Some(10), 10.0),
        (
            Value::Double(f64::from_bits(0xfff8_0000_0000_0456)),
            None,
            -0.0,
        ),
        (Value::Double(0.0), Some(-i64::MAX), -f64::MAX),
        (Value::Null, Some(14), 14.0),
        (Value::Double(f64::INFINITY), Some(1), nan),
        (Value::Double(-1.0), Some(i64::MAX), f64::MAX),
        (Value::Double(-1.0), Some(i64::MAX), f64::MAX),
        (Value::Double(f64::INFINITY), Some(2), f64::INFINITY),
    ];
    let input_charge = database
        .reserve_memory(crate::batch::MAX_BYTES, "test input batch")
        .unwrap();
    let mut input = Batch::new(
        &[DataType::Int64, DataType::Double, DataType::Double],
        input_charge.bytes(),
    )
    .unwrap();
    for (row, (key, integer, double)) in rows.iter().copied().enumerate() {
        input
            .set(row, 0, integer.map_or(Value::Null, Value::Int64))
            .unwrap();
        input.set(row, 1, Value::Double(double)).unwrap();
        input.set(row, 2, key).unwrap();
    }
    input.publish_rows(rows.len());
    let mut direct = AggregateState::new(
        &database.memory,
        semantic,
        query.plan.aggregate_demand(0),
        5,
        query.plan.input_columns(),
    )
    .unwrap();
    let mut positions = [(0, 0); BATCH_ROWS];
    for (row, (key, _, _)) in rows.iter().enumerate() {
        let group = class(*key);
        positions[row] = (group, direct.cells.count_row(group).unwrap());
    }
    direct.consume(&input, &positions[..rows.len()]).unwrap();
    let before = database.reserved_memory_bytes();
    for capacity in [1, 3, BATCH_ROWS] {
        for (group_limit, key_limit, expected_limit) in [
            (5, 45, None),
            (4, 45, Some(HashLimit::Groups)),
            (5, 0, Some(HashLimit::KeyBytes)),
        ] {
            let mut aggregate = AggregateState::new(
                &database.memory,
                semantic,
                query.plan.aggregate_demand(0),
                1,
                query.plan.input_columns(),
            )
            .unwrap();
            let shape = ArgumentShape::from_aggregate(&aggregate);
            let mut arguments = ArgumentBatch::new(&database, shape, capacity).unwrap();
            let record_bytes = RECORD_HEADER + keys.max_bytes + 8 * shape.count;
            let charge = database
                .reserve_memory(record_bytes as u64, "test record")
                .unwrap();
            let mut record = SortRecord::new(record_bytes, charge.bytes()).unwrap();
            // Reserve fallback buffers and creation memory before optional state.
            // File creation remains lazy and acquires no new memory reservation.
            let mut sort = RowSort::new(&database, &keys, shape, 2 * record_bytes, 2).unwrap();
            let mut effects = Effects::default();
            let creation = crate::scratch::Creation::reserve(&database).unwrap();
            let fallback_bytes = database.reserved_memory_bytes();
            let mut memory =
                MemoryGroups::new(&database, &aggregate, &keys, group_limit, key_limit).unwrap();
            assert_eq!(
                database.reserved_memory_bytes(),
                fallback_bytes + memory.reservation.bytes()
            );
            let mut limit = None;
            'input: for start in (0..rows.len()).step_by(capacity) {
                let end = (start + capacity).min(rows.len());
                arguments
                    .evaluate(&mut aggregate, &input, start..end, &cancel)
                    .unwrap();
                memory.begin(&arguments).unwrap();
                for _ in 0..=end - start {
                    let cursor = memory.cursor;
                    match memory.step(&input, &arguments, &keys, &cancel).unwrap() {
                        HashStep::Progress => {}
                        HashStep::Complete => break,
                        HashStep::Fallback(reason) => {
                            assert_eq!(memory.cursor, cursor, "refused row remains unconsumed");
                            assert_eq!(memory.input_rows, (start + cursor) as u64);
                            assert_eq!(
                                memory.step(&input, &arguments, &keys, &cancel).unwrap(),
                                HashStep::Fallback(reason)
                            );
                            limit = Some(reason);
                            break 'input;
                        }
                    }
                }
            }
            assert_eq!(limit, expected_limit);
            assert_eq!(
                aggregate.cells.counts[0], 0,
                "optional grouping cannot mutate fallback cells"
            );
            if limit.is_none() {
                assert_eq!(memory.len(), 5);
                memory.begin_order().unwrap();
                let mut ordered = false;
                for _ in 0..100 {
                    if memory.order_step(&keys, &cancel).unwrap() {
                        ordered = true;
                        break;
                    }
                }
                assert!(ordered);
                for index in 0..5 {
                    let group = memory.ordered_group(index).unwrap();
                    assert_eq!(class(memory.key_value(group, 0, &keys).unwrap()), index);
                }
                for group in 0..memory.len() {
                    let key = memory.key_value(group, 0, &keys).unwrap();
                    let logical = class(key);
                    if logical == 1 {
                        assert!(
                            matches!(key, Value::Double(value) if value.to_bits() == nan.to_bits())
                        );
                    }
                    if logical == 3 {
                        assert!(
                            matches!(key, Value::Double(value) if value.to_bits() == (-0.0_f64).to_bits())
                        );
                    }
                    memory.load_group(group, &mut aggregate).unwrap();
                    compare_values(&direct, logical, &aggregate, semantic.entries.len());
                }
            }
            drop(memory);
            assert_eq!(database.reserved_memory_bytes(), fallback_bytes);
            // Occupy every free byte: disk sorting and reduction must use their
            // existing owners, even after another query takes the freed budget.
            let pressure = database
                .reserve_memory(
                    database.config().memory_limit_bytes() - fallback_bytes,
                    "fallback competition",
                )
                .unwrap();
            let attempted_at = effects.count();
            assert!(
                matches!(
                    crate::scratch::Scratch::new(&database, &cancel, &mut effects),
                    Err(Error::Resource { .. })
                ),
                "fresh admission cannot consume another owner's reserved minimum"
            );
            assert_eq!(
                effects.count(),
                attempted_at,
                "missing path admission precedes filesystem effects"
            );
            let mut scratch = creation.create(&cancel, &mut effects).unwrap();
            let path_pressure = database
                .reserve_memory(
                    database.config().memory_limit_bytes() - database.reserved_memory_bytes(),
                    "released creation memory",
                )
                .unwrap();
            for start in (0..rows.len()).step_by(capacity) {
                let end = (start + capacity).min(rows.len());
                arguments
                    .evaluate(&mut aggregate, &input, start..end, &cancel)
                    .unwrap();
                for row in 0..end - start {
                    arguments
                        .encode_record(&mut record, &keys, &input, row, (start + row) as u64)
                        .unwrap();
                    if !sort.push(&record).unwrap() {
                        for _ in 0..20 {
                            if sort
                                .step(&keys, &mut Io::new(&mut scratch, &cancel, &mut effects))
                                .unwrap()
                                == SortPhase::Collect
                            {
                                break;
                            }
                        }
                        assert!(sort.push(&record).unwrap());
                    }
                }
            }
            sort.finish().unwrap();
            let mut arena_pressure = None;
            for _ in 0..1000 {
                let phase = sort
                    .step(&keys, &mut Io::new(&mut scratch, &cancel, &mut effects))
                    .unwrap();
                if phase == SortPhase::Merge && arena_pressure.is_none() {
                    arena_pressure = Some(
                        database
                            .reserve_memory(
                                database.config().memory_limit_bytes()
                                    - database.reserved_memory_bytes(),
                                "released run memory",
                            )
                            .unwrap(),
                    );
                }
                if phase == SortPhase::Done {
                    break;
                }
            }
            let mut reduction = Reduction::Start;
            let mut groups = 0;
            for _ in 0..100 {
                match reduction
                    .step(
                        &mut sort,
                        &mut arguments,
                        &mut aggregate,
                        &keys,
                        &mut Io::new(&mut scratch, &cancel, &mut effects),
                    )
                    .unwrap()
                {
                    Reduction::Group => {
                        let key = reduction.value(&sort, &aggregate, &keys, 0).unwrap();
                        assert_eq!(class(key), groups);
                        compare_values(&direct, groups, &aggregate, semantic.entries.len());
                        groups += 1;
                    }
                    Reduction::Done => break,
                    _ => {}
                }
            }
            assert_eq!(reduction, Reduction::Done);
            assert_eq!(groups, 5);
            drop(arena_pressure);
            drop(path_pressure);
            drop(pressure);
            drop(scratch);
            drop(sort);
            drop(record);
            drop(charge);
            drop(arguments);
            drop(aggregate);
            assert_eq!(database.reserved_memory_bytes(), before);
            assert_eq!(database.reserved_temp_bytes(), 0);
        }
    }
}

fn class(value: Value<'_>) -> usize {
    match value {
        Value::Null => 0,
        Value::Double(value) if value.is_nan() => 1,
        Value::Double(-1.0) => 2,
        Value::Double(0.0) => 3,
        Value::Double(value) if value == f64::INFINITY => 4,
        _ => panic!("fixture key class"),
    }
}

fn compare_values(
    direct: &AggregateState<'_>,
    group: usize,
    actual: &AggregateState<'_>,
    entries: usize,
) {
    for entry in 0..entries {
        match (direct.value(group, entry), actual.value(0, entry)) {
            (Ok(Value::Double(left)), Ok(Value::Double(right))) => {
                assert_eq!(left.to_bits(), right.to_bits())
            }
            (Ok(left), Ok(right)) => assert_eq!(left, right),
            (
                Err(Error::ArithmeticOverflow {
                    operation: left,
                    span: left_span,
                }),
                Err(Error::ArithmeticOverflow {
                    operation: right,
                    span: right_span,
                }),
            ) => {
                assert_eq!(left, right);
                assert_eq!(left_span, right_span);
            }
            mismatch => panic!("hash/sorted aggregate mismatch: {mismatch:?}"),
        }
    }
}

#[test]
fn lookup_limits_bound_collisions_and_wide_key_comparisons() {
    let directory = Directory::new();
    let database = database(&directory);
    let query = database
        .prepare("FROM facts |> AGGREGATE COUNT(*) AS nrows")
        .unwrap();
    let aggregate = AggregateState::new(
        &database.memory,
        query.plan.aggregates.first().unwrap(),
        query.plan.aggregate_demand(0),
        1,
        query.plan.input_columns(),
    )
    .unwrap();
    let keys = schema(&[(DataType::Int64, false)]);
    let mut groups = MemoryGroups::new(&database, &aggregate, &keys, 128, 128 * 9).unwrap();
    for value in 0..MAX_HASH_PROBES {
        groups.key.clear();
        append_value(
            &mut groups.key,
            Value::Int64(value as i64),
            DataType::Int64,
            false,
        )
        .unwrap();
        // Force equal full hashes at the private lookup boundary. The production
        // step always computes its hash from the key; no injectable hash exists.
        assert_eq!(groups.lookup(&keys, 0).unwrap(), Ok(value));
    }
    groups.key.clear();
    append_value(
        &mut groups.key,
        Value::Int64(MAX_HASH_PROBES as i64),
        DataType::Int64,
        false,
    )
    .unwrap();
    assert_eq!(groups.lookup(&keys, 0).unwrap(), Err(HashLimit::ProbeWork));
    assert_eq!(groups.len(), MAX_HASH_PROBES);
    let keys = schema(&[(DataType::String, false); MAX_KEYS]);
    let mut groups = MemoryGroups::new(&database, &aggregate, &keys, 4, 3 * MAX_KEY_BYTES).unwrap();
    for (index, byte) in ['a', 'b', 'c'].iter().copied().enumerate() {
        let text: String = std::iter::repeat_n(byte, crate::batch::MAX_TEXT_BYTES).collect();
        groups.key.clear();
        for _ in 0..MAX_KEYS {
            append_value(
                &mut groups.key,
                Value::String(StringValue::new(&text)),
                DataType::String,
                false,
            )
            .unwrap();
        }
        assert_eq!(groups.key.len(), MAX_KEY_BYTES);
        let expected = if index < 2 {
            Ok(index)
        } else {
            Err(HashLimit::ProbeWork)
        };
        assert_eq!(groups.lookup(&keys, 0).unwrap(), expected);
    }
    assert_eq!(
        groups.len(),
        2,
        "byte-work refusal precedes a third insertion"
    );
}

#[test]
fn hash_admission_and_cancellation_preserve_the_base_accumulator() {
    let directory = Directory::new();
    let database = database(&directory);
    let query = database
        .prepare("FROM facts |> AGGREGATE COUNT(*) AS nrows")
        .unwrap();
    let mut aggregate = AggregateState::new(
        &database.memory,
        query.plan.aggregates.first().unwrap(),
        query.plan.aggregate_demand(0),
        1,
        query.plan.input_columns(),
    )
    .unwrap();
    let shape = ArgumentShape::from_aggregate(&aggregate);
    let keys = schema(&[(DataType::Int64, false)]);
    let groups = MemoryGroups::new(&database, &aggregate, &keys, 3, 27).unwrap();
    let required = groups.reservation.bytes();
    drop(groups);
    let before = database.reserved_memory_bytes();
    for extra in [0, 1] {
        let pressure = database
            .reserve_memory(
                database.config().memory_limit_bytes() - before - required + extra,
                "hash admission boundary",
            )
            .unwrap();
        let admitted = MemoryGroups::new(&database, &aggregate, &keys, 3, 27);
        if extra == 0 {
            drop(admitted.unwrap());
        } else {
            assert!(matches!(admitted, Err(Error::Resource { .. })));
        }
        drop(pressure);
        assert_eq!(database.reserved_memory_bytes(), before);
        assert_eq!(aggregate.cells.counts[0], 0);
    }
    let charge = database
        .reserve_memory(crate::batch::MAX_BYTES, "test input")
        .unwrap();
    let mut input = Batch::new(&[DataType::Int64], charge.bytes()).unwrap();
    input.set(0, 0, Value::Int64(7)).unwrap();
    input.publish_rows(1);
    let mut arguments = ArgumentBatch::new(&database, shape, 1).unwrap();
    let cancel = CancellationToken::new();
    arguments
        .evaluate(&mut aggregate, &input, 0..1, &cancel)
        .unwrap();
    let mut groups = MemoryGroups::new(&database, &aggregate, &keys, 3, 27).unwrap();
    groups.begin(&arguments).unwrap();
    cancel.cancel();
    assert!(matches!(
        groups.step(&input, &arguments, &keys, &cancel),
        Err(Error::Cancelled)
    ));
    assert_eq!(groups.phase, Phase::Failed);
    assert_eq!(groups.len(), 0);
    assert!(
        groups
            .step(&input, &arguments, &keys, &CancellationToken::new())
            .is_err()
    );
    assert_eq!(aggregate.cells.counts[0], 0);
}

#[test]
fn reserved_creation_releases_its_memory_on_abandonment_and_failure() {
    let directory = Directory::new();
    let database = database(&directory);
    let before = database.reserved_memory_bytes();
    let creation = crate::scratch::Creation::reserve(&database).unwrap();
    assert_eq!(
        database.reserved_memory_bytes(),
        before + (2 * crate::path::MAX_PATH_BYTES) as u64
    );
    // A pending reservation does not block another scratch constructor.
    drop(
        crate::scratch::Scratch::new(
            &database,
            &CancellationToken::new(),
            &mut Effects::default(),
        )
        .unwrap(),
    );
    drop(creation);
    assert_eq!(database.reserved_memory_bytes(), before);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let creation = crate::scratch::Creation::reserve(&database).unwrap();
    let mut effects = Effects::default();
    assert!(matches!(
        creation.create(&cancel, &mut effects),
        Err(Error::Cancelled)
    ));
    assert_eq!(effects.count(), 0);
    assert_eq!(database.reserved_memory_bytes(), before);
    let creation = crate::scratch::Creation::reserve(&database).unwrap();
    let mut effects = Effects::with_faults(Faults {
        fail_at: Some(0),
        ..Faults::default()
    });
    assert!(matches!(
        creation.create(&CancellationToken::new(), &mut effects),
        Err(Error::Io { .. })
    ));
    assert_eq!(database.reserved_memory_bytes(), before);
    assert_eq!(database.reserved_temp_bytes(), 0);
}
