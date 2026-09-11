use super::*;

#[test]
fn joined_sources_are_admitted_before_grouping_growth_and_open_without_new_charge() {
    let directory = Directory::new();
    let database = database(
        &directory,
        &[
            (Some(1), Some(10), 10.0),
            (Some(2), Some(20), 20.0),
            (Some(1), Some(14), 14.0),
        ],
    );
    let cancel = CancellationToken::new();
    let query = database
        .prepare(
            "FROM facts |> AGGREGATE SUM(n) AS total GROUP BY k |> AS grouped \
         |> JOIN facts AS right_row ON grouped.k = right_row.k \
         |> SELECT grouped.total, right_row.n",
        )
        .unwrap();
    let baseline = database.reserved_memory_bytes();
    let plan = lower(
        &database,
        &query,
        query.snapshot.as_ref().unwrap().state(),
        0,
    )
    .unwrap();
    validate_physical(
        &plan,
        &query,
        &database,
        query.snapshot.as_ref().unwrap().state(),
        0,
    )
    .unwrap();
    let pipelines = plan.pipelines();
    assert_eq!(pipelines.len(), 4);
    let pressure = database
        .reserve_memory(
            database.config().memory_limit_bytes() - database.reserved_memory_bytes() - 2_500_000,
            "two-source test cap",
        )
        .unwrap();
    let mut scratch = crate::catalog::Scratch::sized(
        &database.memory,
        crate::catalog::MAX_BYTES,
        "shared source catalog admission",
    )
    .unwrap();
    let left =
        declared::admit(&database, &query, &plan, &pipelines[0], Some(&pipelines[1])).unwrap();
    let right = declared::admit(&database, &query, &plan, &pipelines[2], None).unwrap();
    let mut groups = General::open(
        &database,
        query.plan.aggregates.first().unwrap(),
        query.plan.aggregate_demand(0),
        pipelines[0].output_columns(&query.plan),
        pipelines[1].output_columns(&query.plan),
    )
    .unwrap();
    let admitted = database.reserved_memory_bytes();
    let mut effects = Effects::default();
    let mut left = left.open(&mut scratch, &cancel, &mut effects).unwrap();
    assert!(effects.count() > 0);
    let mut right = right.open(&mut scratch, &cancel, &mut effects).unwrap();
    assert_eq!(database.reserved_memory_bytes(), admitted);
    drop(scratch);
    let mut driver = InputDriver::new();
    let mut left_rows = Vec::new();
    let mut right_rows = Vec::new();
    let mut left_done = false;
    let mut right_done = false;
    for _ in 0..4096 {
        if !left_done {
            match drive_consumer(
                &mut groups[0],
                &mut left,
                &mut driver,
                (&pipelines[0], &pipelines[1]),
                &cancel,
                &mut effects,
            )
            .unwrap()
            {
                Advance::Rows => {
                    for row in 0..left.output.len() {
                        let (Some(Value::Int64(key)), Some(Value::Int64(total))) =
                            (left.output.value(row, 0), left.output.value(row, 1))
                        else {
                            panic!("group result")
                        };
                        left_rows.push((key, total));
                    }
                }
                Advance::Finished => left_done = true,
                Advance::Progress => (),
            }
        }
        if !right_done {
            match right.advance(&pipelines[2], &cancel, &mut effects).unwrap() {
                Advance::Rows => {
                    for row in 0..right.input.len() {
                        let (Some(Value::Int64(key)), Some(Value::Int64(value))) =
                            (right.input.value(row, 0), right.input.value(row, 1))
                        else {
                            panic!("right source result")
                        };
                        right_rows.push((key, value));
                    }
                }
                Advance::Finished => right_done = true,
                Advance::Progress => (),
            }
        }
        if left_done && right_done {
            break;
        }
    }
    assert!(left_done && right_done);
    left_rows.sort();
    assert_eq!(left_rows, [(1, 24), (2, 20)]);
    right_rows.sort();
    assert_eq!(right_rows, [(1, 10), (1, 14), (2, 20)]);
    drop((left, right, groups, plan, pressure));
    assert_eq!(database.reserved_memory_bytes(), baseline);
    assert_eq!(database.reserved_temp_bytes(), 0);
}

#[test]
fn public_grouping_admits_the_exact_complete_minimum_before_io() {
    let directory = Directory::new();
    let database = database(
        &directory,
        &[
            (Some(2), Some(4), 4.0),
            (Some(1), Some(3), 3.0),
            (Some(3), Some(5), 5.0),
        ],
    );
    let cancel = CancellationToken::new();
    let query = database.prepare("FROM facts |> AGGREGATE SUM(n) AS total,AVG(d) AS mean,COUNT(*) AS nrows GROUP AND ORDER BY k").unwrap();
    let baseline = database.reserved_memory_bytes();
    let result = database.execute(&query, &cancel).unwrap();
    assert_eq!(
        database.reserved_memory_bytes() - baseline,
        result.accounted_memory_bytes()
    );
    let Some(Aggregation::General(owner)) = result.first_aggregate() else {
        panic!("public general grouping owner");
    };
    let general = &owner[0];
    check_controller_account(general);
    let scalar_extra = (general.aggregate.lanes - 1) * general.aggregate.scratch.len()
        / general.aggregate.lanes
        * size_of::<u64>();
    let argument_extra =
        (general.arguments.capacity - 1) * general.arguments.shape.count * size_of::<u64>();
    let maximum_record = RECORD_HEADER + general.keys.max_bytes + general.arguments.shape.count * 8;
    let run_extra = general.sort.run_limits().0 - maximum_record
        + (general.sort.run_limits().1 - 1) * 2 * size_of::<RecordSpan>();
    let minimum_peak = result.accounted_memory_bytes() + crate::catalog::MAX_BYTES as u64
        - scalar_extra as u64
        - argument_extra as u64
        - run_extra as u64
        - general.memory.first().map_or(0, MemoryGroups::memory_bytes);
    let retry_peak = result.accounted_memory_bytes() + crate::catalog::MAX_BYTES as u64
        - general.memory.first().map_or(0, MemoryGroups::memory_bytes)
        + MemoryGroups::requirement(&general.aggregate, &general.keys, 1, 9)
            .unwrap()
            .1;
    drop(result);
    for shortfall in [0, 1] {
        let pressure = database
            .reserve_memory(
                database.config().memory_limit_bytes() - baseline - minimum_peak + shortfall,
                "public grouping minimum",
            )
            .unwrap();
        let mut effects = Effects::default();
        let admitted = database.execute_with_effects(&query, &cancel, &mut effects);
        if shortfall == 1 {
            assert!(matches!(admitted, Err(Error::Resource { .. })));
            assert_eq!(
                effects.count(),
                0,
                "one byte below the complete minimum refuses before I/O"
            );
        } else {
            let mut result = admitted.unwrap();
            let Some(Aggregation::General(owner)) = result.first_aggregate() else {
                unreachable!()
            };
            let general = &owner[0];
            assert!(general.memory.is_empty());
            assert_eq!(general.aggregate.lanes, 1);
            assert_eq!(general.arguments.capacity, 1);
            assert_eq!(general.sort.run_limits().1, 1);
            check_controller_account(general);
            let mut observed = Vec::new();
            let mut done = false;
            for _ in 0..2000 {
                match result.step_with_effects(&mut effects) {
                    QueryStep::Rows(batch) => {
                        for row in 0..batch.len() {
                            let Some(Value::Int64(key)) = batch.value(row, 0) else {
                                panic!("key");
                            };
                            let Some(Value::Int64(total)) = batch.value(row, 1) else {
                                panic!("total");
                            };
                            observed.push((key, total));
                        }
                    }
                    QueryStep::Finished => {
                        done = true;
                        break;
                    }
                    QueryStep::Failed(error) => panic!("minimum query failed: {error}"),
                    QueryStep::Progress => (),
                }
            }
            assert!(done);
            assert_eq!(observed, [(1, 3), (2, 4), (3, 5)]);
        }
        drop(pressure);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
    let pressure = database
        .reserve_memory(
            database.config().memory_limit_bytes() - baseline - retry_peak,
            "public late grouping fallback",
        )
        .unwrap();
    let mut result = database.execute(&query, &cancel).unwrap();
    let Some(Aggregation::General(owner)) = result.first_aggregate() else {
        unreachable!()
    };
    assert_eq!(
        owner[0].memory.len(),
        1,
        "the query begins with optional hash storage"
    );
    let mut disk = false;
    let mut observed = Vec::new();
    let mut done = false;
    for _ in 0..2000 {
        if let Some(Aggregation::General(owner)) = result.first_aggregate() {
            disk |= matches!(owner[0].files, Files::Open(_));
        }
        match result.step() {
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let Some(Value::Int64(key)) = batch.value(row, 0) else {
                        panic!("key");
                    };
                    observed.push(key);
                }
            }
            QueryStep::Finished => {
                done = true;
                break;
            }
            QueryStep::Failed(error) => panic!("public replay failed: {error}"),
            QueryStep::Progress => (),
        }
    }
    assert!(
        done && disk,
        "public execution must drive the bounded replay"
    );
    assert_eq!(observed, [1, 2, 3]);
    drop(result);
    drop(pressure);
    assert_eq!(database.reserved_memory_bytes(), baseline);
    assert_eq!(database.reserved_temp_bytes(), 0);
}

#[test]
fn controller_retains_the_disk_minimum_under_competing_memory_pressure() {
    let directory = Directory::new();
    let database = database(
        &directory,
        &[
            (Some(2), Some(4), 4.0),
            (Some(1), Some(3), 3.0),
            (Some(3), Some(5), 5.0),
        ],
    );
    let cancel = CancellationToken::new();
    let query = database.prepare(QUERY).unwrap();
    let baseline = database.reserved_memory_bytes();
    let mut running = database.execute(&query, &cancel).unwrap();
    let before = database.reserved_memory_bytes();
    let general = connect(&database, &mut running, 1, 1, true);
    let minimum = database.reserved_memory_bytes() - before - general.memory[0].memory_bytes();
    drop(general);
    drop(running);
    for skip_hash in [false, true] {
        let mut running = database.execute(&query, &cancel).unwrap();
        let mut pressure = Vec::new();
        if skip_hash {
            pressure.push(
                database
                    .reserve_memory(
                        database.config().memory_limit_bytes()
                            - database.reserved_memory_bytes()
                            - minimum,
                        "controller minimum test",
                    )
                    .unwrap(),
            );
        }
        let mut general = connect(&database, &mut running, 1, 1, true);
        assert_eq!(general.memory.is_empty(), skip_hash);
        let mut effects = Effects::default();
        let mut rows = 0;
        let mut done = false;
        for _ in 0..2000 {
            // Occupy newly released hash, path and run-arena memory as soon as
            // another query could claim it. No fallback operation may reacquire it.
            check_controller_account(&general);
            let available =
                database.config().memory_limit_bytes() - database.reserved_memory_bytes();
            if available != 0 {
                pressure.push(
                    database
                        .reserve_memory(available, "competing query")
                        .unwrap(),
                );
            }
            match step(&mut general, &mut running, &cancel, &mut effects).unwrap() {
                Advance::Rows => rows += 1,
                Advance::Finished => {
                    done = true;
                    break;
                }
                Advance::Progress => (),
            }
        }
        assert!(done);
        assert_eq!(rows, 3);
        assert!(
            pressure.len() <= 4,
            "only the named owners release memory during execution"
        );
        drop(general);
        drop(running);
        drop(pressure);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
}

#[test]
fn repeated_grouping_admits_combined_minimum_before_io() {
    check_combined_grouping_minimum(
        "FROM facts |> AGGREGATE SUM(n) AS total GROUP BY k |> AGGREGATE SUM(total) AS subtotal GROUP AND ORDER BY total",
        &[
            vec![None, None],
            vec![Some(3), Some(3)],
            vec![Some(9), Some(9)],
        ],
        true,
    );
}

#[test]
fn derived_join_admits_both_external_aggregates_at_combined_minimum() {
    check_combined_grouping_minimum(
        "FROM (FROM facts |> AGGREGATE SUM(n) AS total GROUP BY k) AS a |> JOIN (FROM facts |> AGGREGATE SUM(n) AS total GROUP BY k) AS b ON a.k = b.k |> SELECT a.total,b.total",
        &[vec![Some(3), Some(3)], vec![Some(9), Some(9)]],
        false,
    );
}

fn check_combined_grouping_minimum(sql: &str, expected: &[Vec<Option<i64>>], ordered: bool) {
    let directory = Directory::new();
    let database = database(
        &directory,
        &[
            (Some(1), Some(3), 3.0),
            (Some(2), Some(9), 9.0),
            (None, None, 0.0),
        ],
    );
    let cancel = CancellationToken::new();
    let query = database.prepare(sql).unwrap();
    let baseline = database.reserved_memory_bytes();
    let initial = database.execute(&query, &cancel).unwrap();
    assert_eq!(
        database.reserved_memory_bytes() - baseline,
        initial.accounted_memory_bytes()
    );
    let State::Running(runtime) = &initial.state else {
        unreachable!()
    };
    // Derive the minimum from admitted physical owners by removing optional
    // capacity, independently of the production combined-minimum calculation.
    let mut minimum_peak = initial.accounted_memory_bytes() + crate::catalog::MAX_BYTES as u64;
    assert_eq!(runtime.aggregates.len(), 2);
    for aggregation in &runtime.aggregates {
        let Aggregation::General(owner) = aggregation else {
            unreachable!()
        };
        let general = &owner[0];
        check_controller_account(general);
        let scalar_extra = (general.aggregate.lanes - 1) * general.aggregate.scratch.len()
            / general.aggregate.lanes
            * size_of::<u64>();
        let argument_extra =
            (general.arguments.capacity - 1) * general.arguments.shape.count * size_of::<u64>();
        let maximum_record =
            RECORD_HEADER + general.keys.max_bytes + general.arguments.shape.count * 8;
        let run_extra = general.sort.run_limits().0 - maximum_record
            + (general.sort.run_limits().1 - 1) * 2 * size_of::<RecordSpan>();
        minimum_peak -= (scalar_extra + argument_extra + run_extra) as u64
            + general.memory.first().map_or(0, MemoryGroups::memory_bytes);
    }
    drop(initial);
    for shortfall in [0, 1] {
        let pressure = database
            .reserve_memory(
                database.config().memory_limit_bytes() - baseline - minimum_peak + shortfall,
                "repeated grouping minimum",
            )
            .unwrap();
        let pressured = database.reserved_memory_bytes();
        let mut effects = Effects::default();
        let admitted = database.execute_with_effects(&query, &cancel, &mut effects);
        if shortfall == 1 {
            assert!(matches!(admitted, Err(Error::Resource { .. })));
            assert_eq!(effects.count(), 0, "refuse before source I/O");
        } else {
            let mut result = admitted.unwrap();
            let State::Running(runtime) = &result.state else {
                unreachable!()
            };
            for aggregation in &runtime.aggregates {
                let Aggregation::General(owner) = aggregation else {
                    unreachable!()
                };
                let general = &owner[0];
                assert!(general.memory.is_empty());
                assert_eq!(general.aggregate.lanes, 1);
                assert_eq!(general.arguments.capacity, 1);
                assert_eq!(general.sort.run_limits().1, 1);
                check_controller_account(general);
            }
            let mut rows = Vec::new();
            let mut finished = false;
            for _ in 0..16384 {
                match result.step_with_effects(&mut effects) {
                    QueryStep::Rows(batch) => {
                        for row in 0..batch.len() {
                            rows.push(
                                (0..2)
                                    .map(|column| match batch.value(row, column).unwrap() {
                                        Value::Int64(value) => Some(value),
                                        Value::Null => None,
                                        _ => panic!("integer grouping result"),
                                    })
                                    .collect::<Vec<_>>(),
                            );
                        }
                    }
                    QueryStep::Finished => {
                        finished = true;
                        break;
                    }
                    QueryStep::Failed(error) => panic!("combined minimum: {error}"),
                    QueryStep::Progress => (),
                }
            }
            assert!(finished);
            if !ordered {
                rows.sort_unstable();
            }
            assert_eq!(rows, expected);
        }
        assert_eq!(database.reserved_memory_bytes(), pressured);
        assert_eq!(database.reserved_temp_bytes(), 0);
        drop(pressure);
        assert_eq!(database.reserved_memory_bytes(), baseline);
    }
}
