use super::*;
use crate::execution::blocking::test_support::{Directory, schema};
use crate::execution::*;
use crate::fixed_text::KEY_DOMAIN;

#[test]
fn fallback_minimum_matches_constructed_owners_and_refusal_releases_them() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(16_000_000, 16_000_000).unwrap(),
    )
    .unwrap();
    let baseline = database.reserved_memory_bytes();
    for kind in [
        DataType::Int64,
        DataType::Double,
        DataType::Date,
        DataType::String,
    ] {
        for width in [1, MAX_AGGREGATE_COLUMNS - 1] {
            for arguments in [0, 1, MAX_AGGREGATE_COLUMNS] {
                for output_width in [1, MAX_ROW_VALUES] {
                    let keys = schema(&[(kind, true); MAX_AGGREGATE_COLUMNS - 1][..width]);
                    let output = OutputLayout::from_columns(
                        [(DataType::String, true); MAX_ROW_VALUES],
                        output_width,
                    )
                    .unwrap();
                    let shape = ArgumentShape {
                        count: arguments,
                        nonnull: 0,
                        integers: 0,
                        presence: 0,
                        dates: 0,
                        text: 0,
                    };
                    let required = Minimum::required(&keys, &output, shape).unwrap();
                    let limits = Limits {
                        arguments: 1,
                        run_rows: 1,
                        run_bytes: RECORD_HEADER + keys.max_bytes + arguments * 8,
                    };
                    for shortfall in [0, 1] {
                        let pressure = database
                            .reserve_memory(
                                database.config().memory_limit_bytes() - baseline - required
                                    + shortfall,
                                "group admission witness pressure",
                            )
                            .unwrap();
                        let before = database.reserved_memory_bytes();
                        let keys = schema(&[(kind, true); MAX_AGGREGATE_COLUMNS - 1][..width]);
                        let output = OutputLayout::from_columns(
                            [(DataType::String, true); MAX_ROW_VALUES],
                            output_width,
                        )
                        .unwrap();
                        let outcome = Minimum::new(&database, keys, output, shape, &limits);
                        if shortfall == 0 {
                            let owner = outcome.unwrap_or_else(|error| panic!("{kind:?}, keys={width}, arguments={arguments}, output={output_width}: {error:?}"));
                            assert_eq!(database.reserved_memory_bytes() - before, required);
                            drop(owner);
                        } else {
                            assert!(matches!(outcome, Err(Error::Resource { .. })));
                        }
                        assert_eq!(database.reserved_memory_bytes(), before);
                        assert_eq!(database.reserved_temp_bytes(), 0);
                        drop(pressure);
                        assert_eq!(database.reserved_memory_bytes(), baseline);
                    }
                }
            }
        }
    }
    database.close().unwrap();
}

#[test]
fn accumulator_requirement_matches_typed_arrays_under_exact_pressure() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(16_000_000, 16_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    for nullable in [false, true] {
        let table = if nullable {
            "nullable_values"
        } else {
            "required_values"
        };
        database
            .declare_table(
                table,
                &[
                    crate::ColumnDeclaration {
                        name: "v",
                        data_type: DataType::Int64,
                        nullable,
                    },
                    crate::ColumnDeclaration {
                        name: "d",
                        data_type: DataType::Double,
                        nullable,
                    },
                ],
                &cancel,
            )
            .unwrap();
        let prepared = database.prepare(&format!(
            "FROM {table} |> AGGREGATE SUM(v) AS a,AVG(v) AS b,SUM(d) AS c,AVG(d) AS e,COUNT(*) AS n,SUM(v+(v*(v+1))) AS f"
        )).unwrap();
        let semantic = prepared.plan.aggregates.first().unwrap();
        let baseline = database.reserved_memory_bytes();
        for demand in [0, 1, 2, 3, (1 << semantic.entries.len()) - 1] {
            for groups in [1, KEY_DOMAIN * KEY_DOMAIN] {
                for lanes in [1, BATCH_ROWS] {
                    let columns = || {
                        prepared
                            .plan
                            .source_columns()
                            .map(|column| column.semantic())
                    };
                    let layout = AggregateLayout::new(semantic, demand, columns());
                    let required = layout.required_bytes(groups, lanes).unwrap();
                    let pressure = database
                        .reserve_memory(
                            database.config().memory_limit_bytes() - baseline - required,
                            "accumulator admission witness pressure",
                        )
                        .unwrap();
                    let before = database.reserved_memory_bytes();
                    let owner =
                        AggregateState::from_layout(&database.memory, layout, groups).unwrap();
                    owner
                        .validate_plan(semantic, demand, groups, columns())
                        .unwrap();
                    // Read the physical Rust owners rather than deriving expected state
                    // counts with the estimator's sharing/nullability calculation.
                    let physical = owner.cells.values.capacity() * size_of::<f64>()
                        + owner.cells.integers.capacity() * size_of::<i128>()
                        + owner.cells.nonnull_counts.capacity() * size_of::<u32>()
                        + owner.cells.counts.capacity() * size_of::<u32>()
                        + owner.cells.flags.capacity() * size_of::<u32>()
                        + owner.scratch.capacity() * size_of::<u64>();
                    assert_eq!(required, physical as u64);
                    assert_eq!(database.reserved_memory_bytes() - before, required);
                    drop(owner);
                    assert_eq!(database.reserved_memory_bytes(), before);
                    drop(pressure);
                    assert_eq!(database.reserved_memory_bytes(), baseline);
                    // Only the true one-lane minimum must refuse one byte below it;
                    // a wider scratch request is allowed to narrow under pressure.
                    if lanes == 1 {
                        let pressure = database
                            .reserve_memory(
                                database.config().memory_limit_bytes() - baseline - required + 1,
                                "accumulator minimum shortfall",
                            )
                            .unwrap();
                        let before = database.reserved_memory_bytes();
                        let layout = AggregateLayout::new(semantic, demand, columns());
                        assert!(matches!(
                            AggregateState::from_layout(&database.memory, layout, groups),
                            Err(Error::Resource { .. })
                        ));
                        assert_eq!(database.reserved_memory_bytes(), before);
                        drop(pressure);
                    }
                    assert_eq!(database.reserved_temp_bytes(), 0);
                }
            }
        }
    }
    database.close().unwrap();
}

#[test]
fn hash_capacity_accounts_for_every_extremum_before_admission() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    database
        .declare_table(
            "facts",
            &[
                crate::ColumnDeclaration {
                    name: "k",
                    data_type: DataType::Int64,
                    nullable: false,
                },
                crate::ColumnDeclaration {
                    name: "n",
                    data_type: DataType::Int64,
                    nullable: true,
                },
            ],
            &cancel,
        )
        .unwrap();
    let query = database.prepare(
        "FROM facts |> AGGREGATE MIN(n) AS a,MAX(n) AS b,MIN(n+1) AS c,MAX(n+1) AS d,MIN(n+2) AS e,MAX(n+2) AS f,MIN(n+3) AS g,MAX(n+3) AS h,MIN(n+4) AS i GROUP BY k"
    ).unwrap();
    let semantic = query.plan.aggregates.first().unwrap();
    let aggregate = AggregateState::new(
        &database.memory,
        semantic,
        query.plan.aggregate_demand(0),
        1,
        query.plan.input_columns(),
    )
    .unwrap();
    let columns: Vec<_> = query.plan.input_columns().collect();
    let keys = key_layout(semantic, &columns).unwrap();
    assert_eq!(aggregate.cells.extrema.len(), 9);
    let baseline = database.reserved_memory_bytes();
    for available in [8_000, 32_000, 128_000, 1_000_000] {
        let pressure = database
            .reserve_memory(
                database.config().memory_limit_bytes() - baseline - available,
                "hash extrema admission pressure",
            )
            .unwrap();
        let (capacity, key_bytes) = MemoryGroups::capacities(&database, &aggregate, &keys).unwrap();
        assert!(capacity > 0);
        let (_, required) =
            MemoryGroups::requirement(&aggregate, &keys, capacity, key_bytes).unwrap();
        assert!(required <= available);
        let before = database.reserved_memory_bytes();
        let groups = MemoryGroups::new(&database, &aggregate, &keys, capacity, key_bytes).unwrap();
        assert_eq!(database.reserved_memory_bytes() - before, required);
        drop(groups);
        assert_eq!(database.reserved_memory_bytes(), before);
        drop(pressure);
        assert_eq!(database.reserved_memory_bytes(), baseline);
    }
}

#[test]
fn text_fallback_minimum_admits_owned_arenas_and_refuses_one_byte_less() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(16_000_000, 16_000_000).unwrap(),
    )
    .unwrap();
    let baseline = database.reserved_memory_bytes();
    for text_arguments in [1, MAX_AGGREGATE_COLUMNS] {
        let shape = ArgumentShape {
            count: text_arguments,
            nonnull: 0,
            integers: 0,
            presence: 0,
            dates: 0,
            text: (1 << text_arguments) - 1,
        };
        let keys = schema(&[(DataType::Int64, false)]);
        let output =
            OutputLayout::from_columns([(DataType::String, true); MAX_ROW_VALUES], text_arguments)
                .unwrap();
        let required = Minimum::required(&keys, &output, shape).unwrap();
        for shortfall in [0, 1] {
            let pressure = database
                .reserve_memory(
                    database.config().memory_limit_bytes() - baseline - required + shortfall,
                    "text minimum pressure",
                )
                .unwrap();
            let keys = schema(&[(DataType::Int64, false)]);
            let output = OutputLayout::from_columns(
                [(DataType::String, true); MAX_ROW_VALUES],
                text_arguments,
            )
            .unwrap();
            let limits = Limits {
                arguments: 1,
                run_rows: 1,
                run_bytes: RECORD_HEADER + keys.max_bytes + shape.max_payload_bytes(),
            };
            let admitted = Minimum::new(&database, keys, output, shape, &limits);
            if shortfall == 0 {
                let owner = admitted.unwrap();
                assert_eq!(
                    database.reserved_memory_bytes() - (baseline + pressure.bytes()),
                    required
                );
                drop(owner);
            } else {
                assert!(matches!(admitted, Err(Error::Resource { .. })));
            }
            assert_eq!(
                database.reserved_memory_bytes(),
                baseline + pressure.bytes()
            );
            assert_eq!(database.reserved_temp_bytes(), 0);
            drop(pressure);
            assert_eq!(database.reserved_memory_bytes(), baseline);
        }
    }
    database.close().unwrap();
}
