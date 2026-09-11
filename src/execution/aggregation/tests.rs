use super::*;
use crate::Config;
use crate::effects::Faults;
use crate::execution::planning::{lower, validate_physical};
use crate::execution::scan::legacy::{BLOCK_ROWS, Layout};
use crate::execution::test_support::*;
use crate::execution::*;
use crate::frontend::MAX_STAGES;
use crate::frontend::{FilterLiteral, Predicate};
use crate::namespace::inspect_namespace;
use crate::scalar::{Expression, Op};
use crate::storage_format::RootState;
use std::sync::atomic::Ordering;

#[test]
fn concurrent_aggregate_readers_reconcile_one_memory_account() {
    let (_fixture, database) = loaded(65_537);
    let path = database.path().to_owned();
    database.close().unwrap();
    for (source, limit, workers, expected) in [
        (
            include_str!("../../../tests/fixtures/q6.pipe.sql"),
            6_000_000,
            4,
            4,
        ),
        (
            include_str!("../../../tests/fixtures/upstream/q1-upstream.pipe.sql"),
            4_000_000,
            2,
            2,
        ),
    ] {
        let database = Database::open(&path, Config::new(limit, 1_000_000).unwrap()).unwrap();
        let query = database.prepare(source).unwrap();
        let barrier = std::sync::Barrier::new(workers + 1);
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..workers)
                .map(|_| {
                    let barrier = &barrier;
                    let database = &database;
                    let query = &query;
                    scope.spawn(move || {
                        let token = CancellationToken::new();
                        let admitted = std::panic::catch_unwind(|| database.execute(query, &token));
                        barrier.wait();
                        barrier.wait();
                        match admitted.expect("execution admission panic") {
                            Ok(result) => {
                                assert_eq!(
                                    finish_query(result, &mut Effects::default()).unwrap(),
                                    1
                                );
                                true
                            }
                            Err(Error::Resource { .. }) => false,
                            Err(error) => panic!("unexpected admission error: {error}"),
                        }
                    })
                })
                .collect();
            barrier.wait();
            let charged = database.reserved_memory_bytes();
            barrier.wait();
            assert!(charged <= limit);
            let admitted = handles
                .into_iter()
                .map(|handle| usize::from(handle.join().unwrap()))
                .sum::<usize>();
            assert_eq!(admitted, expected);
        });
        assert_eq!(
            database.reserved_memory_bytes(),
            query.accounted_memory_bytes() + database.path_memory_bytes()
        );
        drop(query);
        database.close().unwrap();
    }
}

#[test]
fn scalar_width_shrinks_to_one_before_typed_admission_failure() {
    let (_fixture, database) = loaded(513);
    let query = database
        .prepare(include_str!(
            "../../../tests/fixtures/upstream/q1-upstream.pipe.sql"
        ))
        .unwrap();
    let semantic = query.plan.aggregates.first().unwrap();
    let groups = Groups::new(
        &database,
        semantic,
        query.plan.aggregate_demand(0),
        query.plan.input_columns(),
    )
    .unwrap();
    let lane_bytes = groups.aggregate.scratch.len() / groups.aggregate.lanes * size_of::<u64>();
    let minimum =
        groups.aggregate.reservation.bytes() - ((groups.aggregate.lanes - 1) * lane_bytes) as u64;
    drop(groups);
    let physical = lower(&database, &query, RootState::Empty, 0).unwrap();
    let query_bytes = Layout::new(&query, &physical).workspace_bytes().unwrap()
        + RESULT_BYTES
        + PhysicalPlan::required_bytes(&query)
        + Runtime::required_bytes(&physical);
    drop(physical);
    let pressure_bytes = database.config().memory_limit_bytes()
        - database.reserved_memory_bytes()
        - query_bytes
        - minimum;
    let cancellation = CancellationToken::new();
    for extra in [0, 1] {
        let pressure = database
            .reserve_memory(pressure_bytes + extra, "test pressure")
            .unwrap();
        let mut effects = Effects::default();
        let outcome = database.execute_with_effects(&query, &cancellation, &mut effects);
        if extra == 1 {
            assert!(matches!(outcome, Err(Error::Resource { .. })));
            assert_eq!(effects.count(), 0);
        } else {
            let mut result = outcome.unwrap();
            assert_eq!(result.first_aggregate().unwrap().dense().aggregate.lanes, 1);
            let mut finished = false;
            let mut rows = 0;
            for _ in 0..4096 {
                match result.step() {
                    QueryStep::Rows(batch) => {
                        rows += batch.len();
                        let Some(Value::Double(sum)) = batch.value(0, 2) else {
                            panic!("SUM quantity");
                        };
                        assert_eq!(sum, (0..513).map(|row| row % 31).sum::<u32>() as f64);
                        assert!(matches!(batch.value(0, 9), Some(Value::Int64(513))));
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
        }
        drop(pressure);
        assert_eq!(
            database.reserved_memory_bytes(),
            query.accounted_memory_bytes() + database.path_memory_bytes()
        );
    }
}

#[test]
fn source_occurrences_do_not_split_shared_aggregate_state() {
    let (_fixture, database) = loaded(1);
    let sql = "FROM lineitem |> AGGREGATE SUM(l_quantity*2) AS total,AVG(l_quantity*2) AS mean";
    let query = database.prepare(sql).unwrap();
    let semantic = query.plan.aggregates.first().unwrap();
    assert_ne!(semantic.entries[0].span, semantic.entries[1].span);
    let groups = Groups::new(
        &database,
        semantic,
        query.plan.aggregate_demand(0),
        query.plan.input_columns(),
    )
    .unwrap();
    assert_eq!(groups.aggregate.states, 1);
    assert_eq!(
        groups.aggregate.entry_states[0],
        groups.aggregate.entry_states[1]
    );
    eprintln!(
        "diagnostic bytes: error={} cause={} expression={} entry={} aggregate_state={}",
        size_of::<Error>(),
        size_of::<crate::ErrorCause>(),
        size_of::<Expression>(),
        size_of::<crate::frontend::AggregateEntry>(),
        size_of::<AggregateState<'_>>()
    );
}

#[test]
fn integer_aggregate_range_covers_the_last_admitted_row_and_refuses_the_next() {
    let (_fixture, database) = loaded(1);
    let query = database.prepare("FROM lineitem |> AGGREGATE SUM(-9223372036854775808) AS total, AVG(-9223372036854775808) AS mean").unwrap();
    let mut groups = Groups::new(
        &database,
        query.plan.aggregates.first().unwrap(),
        query.plan.aggregate_demand(0),
        query.plan.input_columns(),
    )
    .unwrap();
    groups.aggregate.cells.counts[0] = u32::try_from(MAX_AGGREGATE_ROWS - 1).unwrap();
    groups.aggregate.cells.integers[0] = i128::from(i64::MIN) * i128::from(MAX_AGGREGATE_ROWS - 1);
    let mut batch = Batch::empty();
    batch.publish_rows(1);
    groups.consume(&batch).unwrap();
    let expected = i128::from(i64::MIN) * i128::from(MAX_AGGREGATE_ROWS);
    assert_eq!(groups.aggregate.cells.integers[0], expected);
    assert!(matches!(
        groups.value(0, 0),
        Err(Error::ArithmeticOverflow {
            operation: "SUM",
            ..
        })
    ));
    assert_eq!(groups.value(0, 1).unwrap(), Value::Double(i64::MIN as f64));
    assert!(matches!(
        groups.consume(&batch),
        Err(Error::Corrupt("aggregate row count overflow"))
    ));
    assert_eq!(
        u64::from(groups.aggregate.cells.counts[0]),
        MAX_AGGREGATE_ROWS
    );
    assert_eq!(groups.aggregate.cells.integers[0], expected);
}

#[test]
fn aggregate_positions_preserve_group_isolation_across_batch_and_lane_widths() {
    let path = std::env::temp_dir().join(format!(
        "pipesql-aggregate-positions-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path).unwrap();
    let fixture = Fixture(path);
    let database = Database::create_empty(
        &fixture.0.join("db"),
        Config::new(2_000_000, 1_000_000).unwrap(),
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
    let query = database.prepare("FROM facts |> AGGREGATE COUNT(*) AS nrows, SUM(n) AS ns, AVG(n) AS na, SUM(d) AS ds, AVG(d) AS da, SUM(2) AS twos").unwrap();
    let semantic = query.plan.aggregates.first().unwrap();
    let demand = query.plan.aggregate_demand(0);
    let columns: Vec<_> = query.plan.columns().map(SourceColumn::semantic).collect();
    let before = database.reserved_memory_bytes();
    let integers = [
        Value::Int64(i64::MAX),
        Value::Null,
        Value::Int64(i64::MAX),
        Value::Int64(10),
        Value::Null,
        Value::Int64(-i64::MAX),
        Value::Int64(14),
    ];
    let doubles = [f64::MAX, -0.0, f64::MAX, 10.0, -0.0, -f64::MAX, 14.0];
    // Slot identities are supplied by a physical group lookup, not decoded
    // from a finite key domain. Permuting them cannot change aggregate values.
    for slots in [[0, 1, 2, 3], [3, 0, 1, 2]] {
        for batch_rows in [1, 3, BATCH_ROWS] {
            for lanes in [1, 2, BATCH_ROWS] {
                let mut aggregate = AggregateState::new(
                    &database.memory,
                    semantic,
                    demand,
                    4,
                    query.plan.input_columns(),
                )
                .unwrap();
                let depth = aggregate.scratch.len() / aggregate.lanes;
                aggregate.lanes = lanes;
                aggregate.scratch.truncate(depth * lanes);
                aggregate
                    .validate_plan(semantic, demand, 4, columns.iter().copied())
                    .unwrap();
                assert!(
                    aggregate
                        .validate_plan(semantic, demand, 3, columns.iter().copied())
                        .is_err()
                );
                let mut batch =
                    Batch::new(&[DataType::Int64, DataType::Double], 1_000_000).unwrap();
                let assignments = [2, 0, 2, 1, 0, 2, 1];
                for first in (0..assignments.len()).step_by(batch_rows) {
                    let end = (first + batch_rows).min(assignments.len());
                    batch.clear();
                    let mut positions = [(0, 0); BATCH_ROWS];
                    for (row, input) in (first..end).enumerate() {
                        batch.set(row, 0, integers[input]).unwrap();
                        batch.set(row, 1, Value::Double(doubles[input])).unwrap();
                        let group = slots[assignments[input]];
                        positions[row] = (group, aggregate.cells.count_row(group).unwrap());
                    }
                    batch.publish_rows(end - first);
                    aggregate
                        .consume(&batch, &positions[..end - first])
                        .unwrap();
                }
                for (logical, count) in [2, 2, 3, 0].into_iter().enumerate() {
                    assert_eq!(
                        aggregate.value(slots[logical], 0).unwrap(),
                        Value::Int64(count)
                    );
                    assert_eq!(
                        aggregate.value(slots[logical], 5).unwrap(),
                        if count == 0 {
                            Value::Null
                        } else {
                            Value::Int64(count * 2)
                        }
                    );
                }
                for column in [1, 2] {
                    assert_eq!(aggregate.value(slots[0], column).unwrap(), Value::Null);
                }
                for column in 1..6 {
                    assert_eq!(aggregate.value(slots[3], column).unwrap(), Value::Null);
                }
                assert_eq!(aggregate.value(slots[1], 1).unwrap(), Value::Int64(24));
                assert_eq!(aggregate.value(slots[1], 2).unwrap(), Value::Double(12.0));
                assert_eq!(
                    aggregate.value(slots[2], 1).unwrap(),
                    Value::Int64(i64::MAX)
                );
                assert_eq!(
                    aggregate.value(slots[2], 2).unwrap(),
                    Value::Double(i64::MAX as f64 / 3.0)
                );
                for (logical, sum, mean) in [
                    (0, -0.0, -0.0),
                    (1, 24.0, 12.0),
                    (2, f64::MAX, f64::MAX / 3.0),
                ] {
                    for (column, expected) in [(3, sum), (4, mean)] {
                        let Value::Double(actual) =
                            aggregate.value(slots[logical], column).unwrap()
                        else {
                            panic!("DOUBLE result");
                        };
                        assert_eq!(actual.to_bits(), expected.to_bits());
                    }
                }
                drop(aggregate);
                assert_eq!(database.reserved_memory_bytes(), before);
            }
        }
    }
}

#[test]
fn aggregate_consumes_derived_values_without_storage_metadata() {
    let (_fixture, database) = loaded(3);
    let producer = database
        .prepare("FROM lineitem |> AGGREGATE SUM(l_quantity) AS total")
        .unwrap();
    let cancellation = CancellationToken::new();
    let mut result = database.execute(&producer, &cancellation).unwrap();
    let mut produced = None;
    let mut finished = false;
    for _ in 0..64 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                assert!(produced.is_none());
                assert_eq!(batch.len(), 1);
                let Some(Value::Double(value)) = batch.value(0, 0) else {
                    panic!("expected derived DOUBLE");
                };
                produced = Some(value);
            }
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("{error:?}"),
        }
    }
    assert!(finished);
    assert_eq!(produced, Some(3.0));
    drop(result);

    // Narrow trusted test boundary: bind the consumer program to a derived
    // relation identity. The public parser does not yet admit this pipeline.
    let derived = SemanticColumn::new(1001, DataType::Double, true);
    let mut consumer = database
        .prepare("FROM lineitem |> AGGREGATE SUM(l_quantity) AS total")
        .unwrap();
    let semantic = consumer.plan.aggregates.first_mut().unwrap();
    let expression = semantic.entries[0].expression.as_mut().unwrap();
    expression.ops[0] = Op::Column(derived);
    expression.validate(&[derived]).unwrap();
    let resident = database.reserved_memory_bytes();
    let mut state =
        AggregateState::new(&database.memory, semantic, 1, 1, [derived].into_iter()).unwrap();
    state
        .validate_plan(semantic, 1, 1, [derived].into_iter())
        .unwrap();
    for wrong in [
        SemanticColumn::new(1002, DataType::Double, true),
        SemanticColumn::new(1001, DataType::Int64, true),
        SemanticColumn::new(1001, DataType::Double, false),
    ] {
        assert!(
            state
                .validate_plan(semantic, 1, 1, [wrong].into_iter())
                .is_err()
        );
        assert!(
            state
                .validate_plan(semantic, 1, 1, [wrong, derived].into_iter())
                .is_err()
        );
    }
    let charge = database
        .reserve_memory(
            Batch::required_bytes(&[DataType::Double]).unwrap(),
            "derived input test batch",
        )
        .unwrap();
    let mut batch = Batch::new(&[DataType::Double], charge.bytes()).unwrap();
    let mut positions = [(0, 0); 3];
    for (row, value) in [
        Value::Double(produced.unwrap()),
        Value::Null,
        Value::Double(7.0),
    ]
    .into_iter()
    .enumerate()
    {
        batch.set(row, 0, value).unwrap();
        positions[row] = (0, state.cells.count_row(0).unwrap());
    }
    batch.publish_rows(3);
    state.consume(&batch, &positions).unwrap();
    assert_eq!(state.value(0, 0).unwrap(), Value::Double(10.0));
    assert_eq!(state.cells.nonnull_counts, [2]);
    drop(batch);
    drop(charge);
    drop(state);
    assert_eq!(database.reserved_memory_bytes(), resident);
}

#[test]
fn aggregate_workspace_and_mappings_are_independently_checked() {
    let (_fixture, database) = loaded(1);
    let query = database
        .prepare(include_str!(
            "../../../tests/fixtures/upstream/q1-upstream.pipe.sql"
        ))
        .unwrap();
    let semantic = query.plan.aggregates.first().unwrap();
    let columns: Vec<_> = query.plan.columns().map(SourceColumn::semantic).collect();
    for mutation in 0..11 {
        let mut groups = Groups::new(
            &database,
            semantic,
            query.plan.aggregate_demand(0),
            query.plan.input_columns(),
        )
        .unwrap();
        groups
            .validate_plan(
                semantic,
                query.plan.aggregate_demand(0),
                columns.iter().copied(),
            )
            .unwrap();
        match mutation {
            0 => {
                groups.aggregate.scratch.pop();
            }
            1 => {
                groups.aggregate.lanes = 0;
            }
            2 => groups.aggregate.inputs[0] = None,
            3 => groups.aggregate.entry_states[0] = MAX_COLUMNS,
            4 => groups.aggregate.input_columns[0] = None,
            5 => groups.aggregate.cells.sum_states ^= 1,
            6 => groups.keys[0] = 0,
            7 => groups.aggregate.states = MAX_COLUMNS + 1,
            8 => groups.aggregate.lanes = 0,
            9 => groups.aggregate.lanes = BATCH_ROWS + 1,
            10 => {
                groups.aggregate.input_columns[MAX_COLUMNS - 1] =
                    Some(SourceColumn::QUANTITY.semantic())
            }
            _ => unreachable!(),
        }
        assert!(
            groups
                .validate_plan(
                    semantic,
                    query.plan.aggregate_demand(0),
                    columns.iter().copied()
                )
                .is_err(),
            "mutation {mutation}"
        );
    }
    assert_eq!(
        database.reserved_memory_bytes(),
        query.accounted_memory_bytes() + database.path_memory_bytes()
    );
}

#[test]
fn post_aggregate_mapping_and_demand_are_independently_checked() {
    let (_fixture, database) = loaded(3);
    let query = database.prepare("FROM lineitem |> AGGREGATE SUM(l_quantity*2) AS s,AVG(l_quantity) AS a,COUNT(*) AS n GROUP BY l_returnflag |> WHERE n > 0 |> SELECT a AS n,a AS again").unwrap();
    let namespace =
        inspect_namespace(database.path(), &database.memory, &mut Effects::default()).unwrap();
    for mutation in 0..10 {
        let mut changed_predicate;
        let mut physical = lower(
            &database,
            &query,
            namespace.state,
            namespace.projected_crc32c,
        )
        .unwrap();
        validate_physical(
            &physical,
            &query,
            &database,
            namespace.state,
            namespace.projected_crc32c,
        )
        .unwrap();
        changed_predicate = *physical.output().filters[0].predicate;
        match mutation {
            0 => physical.output_mut().columns[0] = 1,
            1 => physical.output_mut().columns[2] = 2,
            2 => physical.output_mut().filter_count = 0,
            3 => physical.output_mut().filters[0].column = 1,
            4 => {
                let Predicate::Compare { literal, .. } = &mut changed_predicate else {
                    unreachable!()
                };
                *literal = FilterLiteral::Int64(9);
                physical.output_mut().filters[0].predicate = &changed_predicate;
            }
            5 => physical.output_mut().filters[1].column = 1,
            6 => physical.output_mut().column_count = 1,
            7 => physical.output_mut().filter_count = MAX_STAGES + 1,
            8 => physical.output_mut().filters[0].control.negated = true,
            9 => physical.output_mut().filters[0].control.other = 1,
            _ => unreachable!(),
        }
        assert!(
            validate_physical(
                &physical,
                &query,
                &database,
                namespace.state,
                namespace.projected_crc32c
            )
            .is_err(),
            "post aggregate physical mutation {mutation}"
        );
    }
    let semantic = query.plan.aggregates.first().unwrap();
    let columns: Vec<_> = query.plan.columns().map(SourceColumn::semantic).collect();
    for mutation in 0..3 {
        let mut groups = Groups::new(
            &database,
            semantic,
            query.plan.aggregate_demand(0),
            query.plan.input_columns(),
        )
        .unwrap();
        assert_eq!(groups.aggregate.states, 1);
        assert!(
            groups.value(0, 1).is_err(),
            "unused SUM is not represented by a NULL value"
        );
        match mutation {
            0 => groups.aggregate.demand ^= 1,
            1 => groups.aggregate.entry_states[0] = 0,
            2 => groups.aggregate.cells.sum_states = 1,
            _ => unreachable!(),
        }
        assert!(
            groups
                .validate_plan(
                    semantic,
                    query.plan.aggregate_demand(0),
                    columns.iter().copied()
                )
                .is_err(),
            "demand mutation {mutation}"
        );
    }
}

#[test]
fn aggregate_cancellation_at_every_public_step_releases_owners() {
    let (_fixture, database) = loaded(17);
    for sql in [
        "FROM lineitem |> AGGREGATE SUM(l_quantity) AS total,COUNT(*) AS n",
        "FROM lineitem |> AGGREGATE COUNT(*) AS n GROUP BY l_returnflag,l_linestatus",
        "FROM lineitem |> WHERE l_quantity < -1 |> AGGREGATE COUNT(*) AS n",
    ] {
        let query = database.prepare(sql).unwrap();
        let resident = database.reserved_memory_bytes();
        let cancel = CancellationToken::new();
        let mut control = database.execute(&query, &cancel).unwrap();
        let mut steps = None;
        let mut rows = 0;
        for step in 0..256 {
            match control.step() {
                QueryStep::Progress => (),
                QueryStep::Rows(batch) => rows += batch.len(),
                QueryStep::Finished => {
                    steps = Some(step);
                    break;
                }
                QueryStep::Failed(error) => panic!("{error:?}"),
            }
        }
        assert_eq!(rows, 1);
        drop(control);
        for cut in 0..=steps.expect("bounded fixture completes") {
            let cancel = CancellationToken::new();
            let mut result = database.execute(&query, &cancel).unwrap();
            for _ in 0..cut {
                assert!(matches!(
                    result.step(),
                    QueryStep::Progress | QueryStep::Rows(_)
                ));
            }
            cancel.cancel();
            assert!(matches!(result.step(), QueryStep::Failed(Error::Cancelled)));
            assert_eq!(result.accounted_memory_bytes(), RESULT_BYTES);
            assert!(matches!(result.step(), QueryStep::Failed(Error::Cancelled)));
            drop(result);
            assert_eq!(database.reserved_memory_bytes(), resident);
        }
    }
}

#[test]
fn aggregate_effect_cuts_release_all_owners_before_output() {
    let (_fixture, database) = loaded(BLOCK_ROWS + 1);
    let query = database.prepare("FROM lineitem |> WHERE l_shipdate BETWEEN DATE '1994-01-01' AND DATE '1994-12-31' AND l_quantity < 31 |> AGGREGATE SUM(l_quantity) AS total, AVG(l_quantity) AS mean, COUNT(*) AS n GROUP AND ORDER BY l_returnflag,l_linestatus |> WHERE n > 0 |> SELECT mean,total,n").unwrap();
    let cancellation = CancellationToken::new();
    let mut control = Effects::default();
    let mut result = database
        .execute_with_effects(&query, &cancellation, &mut control)
        .unwrap();
    let mut finished = false;
    let mut rows = 0;
    for _ in 0..4096 {
        match result.step_with_effects(&mut control) {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => rows += batch.len(),
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("{error:?}"),
        }
    }
    assert!(finished);
    assert_eq!(rows, 1);
    assert_eq!(result.accounted_memory_bytes(), RESULT_BYTES);
    drop(result);
    for cut in 0..control.count() {
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(cut),
            ..Faults::default()
        });
        match database.execute_with_effects(&query, &cancellation, &mut effects) {
            Err(Error::Io { .. }) => (),
            Err(error) => panic!("{error:?}"),
            Ok(mut result) => {
                let mut failed = false;
                for _ in 0..4096 {
                    match result.step_with_effects(&mut effects) {
                        QueryStep::Progress => (),
                        QueryStep::Rows(_) | QueryStep::Finished => {
                            panic!("refused aggregate input cannot publish a result")
                        }
                        QueryStep::Failed(error) => {
                            assert!(matches!(error, Error::Io { .. }));
                            failed = true;
                            break;
                        }
                    }
                }
                assert!(failed);
                assert_eq!(result.accounted_memory_bytes(), RESULT_BYTES);
                let effects_after_failure = effects.count();
                assert!(matches!(
                    result.step_with_effects(&mut effects),
                    QueryStep::Failed(_)
                ));
                assert_eq!(effects.count(), effects_after_failure);
                drop(result);
            }
        }
        assert_eq!(
            database.reserved_memory_bytes(),
            query.accounted_memory_bytes() + database.path_memory_bytes()
        );
    }
}
