use super::*;

#[test]
fn grouping_consumes_computed_batches_and_requests_replay_without_a_scan() {
    let directory = Directory::new();
    let database = database(&directory, &[]);
    let cancel = CancellationToken::new();
    let query = database.prepare(QUERY).unwrap();
    let baseline = database.reserved_memory_bytes();
    let rows = [
        (Some(1), Some(10), 10.0),
        (Some(2), Some(8), 8.0),
        (Some(1), Some(14), 14.0),
        (None, None, 0.0),
    ];
    for hash_groups in [1, 3] {
        // Reuse the narrow component-construction boundary, then destroy the
        // entire source. Only the consumer and independently owned output survive.
        let mut running = database.execute(&query, &cancel).unwrap();
        let mut general = connect(&database, &mut running, hash_groups, 1, true);
        let map = running.plan.output().clone();
        let mut output = {
            let State::Running(source) = std::mem::replace(&mut running.state, State::Finished)
            else {
                panic!("source owner")
            };
            source.into_output()
        };
        drop(running);
        let types = [DataType::Int64, DataType::Int64, DataType::Double];
        let mut charge = database
            .reserve_memory(Batch::required_bytes(&types).unwrap(), "computed input")
            .unwrap();
        let mut input = OwnedBatch::new(&types, &mut charge).unwrap();
        drop(charge);
        let mut next = 0;
        let mut replays = 0;
        let mut input_finished = false;
        let mut done = false;
        let mut observed = Vec::new();
        for _ in 0..4096 {
            match general
                .step(
                    ConsumerInput {
                        batch: &input,
                        finished: input_finished,
                    },
                    &mut output,
                    &map,
                    &cancel,
                    &mut Effects::default(),
                )
                .unwrap()
            {
                ConsumerStep::Input => {
                    assert!(!input_finished);
                    input.clear();
                    let end = (next + 2).min(rows.len());
                    for (index, &(key, value, double)) in rows[next..end].iter().enumerate() {
                        input
                            .set(index, 0, key.map_or(Value::Null, Value::Int64))
                            .unwrap();
                        input
                            .set(index, 1, value.map_or(Value::Null, Value::Int64))
                            .unwrap();
                        input.set(index, 2, Value::Double(double)).unwrap();
                    }
                    input.publish_rows(end - next);
                    input_finished = end == next;
                    next = end;
                }
                ConsumerStep::Replay => {
                    replays += 1;
                    assert_eq!(replays, 1, "fallback requests one complete input replay");
                    next = 0;
                    input_finished = false;
                    input.clear();
                }
                ConsumerStep::Rows => {
                    let integer = |column| match output.value(0, column).unwrap() {
                        Value::Int64(value) => Some(value),
                        Value::Null => None,
                        _ => panic!("integer result"),
                    };
                    let Some(Value::Double(average)) = output.value(0, 2) else {
                        panic!("average result")
                    };
                    observed.push((integer(0), integer(1), average, integer(3)));
                }
                ConsumerStep::Progress => (),
                ConsumerStep::Finished => {
                    done = true;
                    break;
                }
            }
        }
        assert!(done);
        assert_eq!(replays, usize::from(hash_groups == 1));
        assert_eq!(
            observed,
            [
                (None, None, 0.0, Some(1)),
                (Some(1), Some(24), 12.0, Some(2)),
                (Some(2), Some(8), 8.0, Some(1)),
            ]
        );
        drop(general);
        drop(input);
        drop(output);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
}

#[test]
fn grouping_fallback_replays_sorted_producers_without_reopening_sources() {
    let directory = Directory::new();
    let database = database(
        &directory,
        &[
            (Some(1), Some(3), 3.0),
            (Some(1), Some(4), 4.0),
            (Some(2), Some(7), 7.0),
        ],
    );
    let cancel = CancellationToken::new();
    for variant in 0..10 {
        let joined = matches!(variant, 0 | 2 | 6);
        let sql = if joined {
            "FROM facts AS l |> JOIN facts AS r ON l.k = r.k |> AGGREGATE SUM(l.n) AS total,COUNT(*) AS nrows GROUP AND ORDER BY l.k"
        } else {
            "FROM facts |> ORDER BY n DESC |> AGGREGATE SUM(n) AS total,COUNT(*) AS nrows GROUP AND ORDER BY k"
        };
        let sql = match variant {
            2 => {
                "FROM facts AS l |> JOIN facts AS r ON l.k = r.k |> LIMIT 5 |> LIMIT 5 |> AGGREGATE SUM(l.n) AS total,COUNT(*) AS nrows GROUP AND ORDER BY l.k"
            }
            3 => {
                "FROM facts |> ORDER BY n DESC |> LIMIT 3 |> LIMIT 2 |> AGGREGATE SUM(n) AS total,COUNT(*) AS nrows GROUP AND ORDER BY k"
            }
            4 => {
                "FROM facts |> LIMIT 3 |> LIMIT 3 |> AGGREGATE SUM(n) AS total,COUNT(*) AS nrows GROUP AND ORDER BY k"
            }
            5 => {
                "FROM facts |> SELECT k,n+1 AS shifted |> SELECT k,shifted-1 AS n |> ORDER BY n DESC |> AGGREGATE SUM(n) AS total,COUNT(*) AS nrows GROUP AND ORDER BY k |> SELECT k,total+0 AS total,nrows"
            }
            6 => {
                "FROM facts |> SELECT k,n+0 AS n |> AS l |> JOIN facts AS r ON l.k=r.k |> SELECT l.k AS k,l.n+0 AS n |> AGGREGATE SUM(n) AS total,COUNT(*) AS nrows GROUP AND ORDER BY k |> SELECT k,total+0 AS total,nrows"
            }
            7 => {
                "FROM facts |> SELECT k,n+1 AS n |> LIMIT 3 |> SELECT k,n-1 AS n |> LIMIT 3 |> AGGREGATE SUM(n) AS total,COUNT(*) AS nrows GROUP AND ORDER BY k |> SELECT k,total+0 AS total,nrows"
            }
            8 => {
                "FROM facts |> DISTINCT |> LIMIT 3 |> AGGREGATE SUM(n) AS total,COUNT(*) AS nrows GROUP AND ORDER BY k"
            }
            9 => {
                "FROM facts |> SELECT k |> DISTINCT |> LIMIT 2 |> AGGREGATE SUM(k) AS total,COUNT(*) AS nrows GROUP AND ORDER BY k"
            }
            _ => sql,
        };
        let query = database.prepare(sql).unwrap();
        let baseline = database.reserved_memory_bytes();
        let result = database.execute(&query, &cancel).unwrap();
        let Some(Aggregation::General(owner)) = result.first_aggregate() else {
            panic!("general grouping");
        };
        let general = &owner[0];
        let retry_peak = result.accounted_memory_bytes() + crate::catalog::MAX_BYTES as u64
            - general.memory.first().map_or(0, MemoryGroups::memory_bytes)
            + MemoryGroups::requirement(&general.aggregate, &general.keys, 1, 9)
                .unwrap()
                .1;
        drop(result);
        let pressure = database
            .reserve_memory(
                database.config().memory_limit_bytes() - baseline - retry_peak,
                "join grouping replay",
            )
            .unwrap();
        let mut result = database.execute(&query, &cancel).unwrap();
        let Some(Aggregation::General(owner)) = result.first_aggregate() else {
            unreachable!();
        };
        assert_eq!(owner[0].memory.len(), 1, "start in hash grouping");
        let mut rows = vec![];
        let mut disk = false;
        let mut replay = false;
        let mut done = false;
        for _ in 0..4000 {
            assert_eq!(
                database.reserved_memory_bytes(),
                baseline + pressure.bytes() + result.accounted_memory_bytes()
            );
            if let Some(Aggregation::General(owner)) = result.first_aggregate() {
                disk |= matches!(owner[0].files, Files::Open(_));
            }
            if let State::Running(runtime) = &mut result.state {
                replay |= if joined {
                    runtime.first_join_mut().was_replayed()
                } else if !matches!(variant, 4 | 7) {
                    runtime.first_order_mut().was_replayed()
                } else {
                    disk
                };
            }
            match result.step() {
                QueryStep::Rows(batch) => {
                    for row in 0..batch.len() {
                        let values = std::array::from_fn::<_, 3, _>(|column| {
                            match batch.value(row, column) {
                                Some(Value::Int64(value)) => value,
                                _ => panic!("integer aggregate"),
                            }
                        });
                        rows.push(values);
                    }
                }
                QueryStep::Progress => (),
                QueryStep::Finished => {
                    done = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("join replay: {error}"),
            }
        }
        assert!(done && disk && replay);
        assert_eq!(
            rows,
            match variant {
                0 | 2 | 6 => [[1, 14, 4], [2, 7, 1]],
                1 | 4 | 5 | 7 | 8 => [[1, 7, 2], [2, 7, 1]],
                3 => [[1, 4, 1], [2, 7, 1]],
                9 => [[1, 1, 1], [2, 2, 1]],
                _ => unreachable!(),
            }
        );
        drop(result);
        drop(pressure);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
}

#[test]
fn runtime_replays_retained_aggregate_output_after_prefix_or_completion() {
    let directory = Directory::new();
    let database = database(
        &directory,
        &[
            (Some(2), Some(4), 4.0),
            (Some(1), Some(3), 3.0),
            (Some(2), Some(5), 5.0),
            (None, None, 0.0),
        ],
    );
    let cancel = CancellationToken::new();
    for mode in 0..3 {
        let sql = if mode == 0 {
            "FROM facts |> AGGREGATE SUM(n) AS total"
        } else {
            "FROM facts |> AGGREGATE SUM(n) AS total GROUP AND ORDER BY k"
        };
        let query = database.prepare(sql).unwrap();
        let baseline = database.reserved_memory_bytes();
        let disk_peak = if mode == 2 {
            let initial = database.execute(&query, &cancel).unwrap();
            let Some(Aggregation::General(owner)) = initial.first_aggregate() else {
                unreachable!()
            };
            initial.accounted_memory_bytes() + crate::catalog::MAX_BYTES as u64
                - owner[0].memory[0].memory_bytes()
        } else {
            0
        };
        let pressure = (mode == 2).then(|| {
            database
                .reserve_memory(
                    database.config().memory_limit_bytes() - baseline - disk_peak,
                    "retained-output replay pressure",
                )
                .unwrap()
        });
        let pressured = database.reserved_memory_bytes();
        for complete in [false, true] {
            let mut result = database.execute(&query, &cancel).unwrap();
            if let Some(Aggregation::General(owner)) = result.first_aggregate() {
                assert_eq!(owner[0].memory.is_empty(), mode == 2);
            }
            let mut effects = Effects::default();
            let State::Running(runtime) = &mut result.state else {
                unreachable!()
            };
            let mut first = Vec::new();
            let mut reached = false;
            for _ in 0..8192 {
                match runtime.step(&result.plan, &cancel, &mut effects).unwrap() {
                    Advance::Rows => {
                        for row in 0..runtime.output().len() {
                            first.push(
                                (0..result.plan.output().column_count)
                                    .map(|column| {
                                        match runtime.output().value(row, column).unwrap() {
                                            Value::Int64(value) => Some(value),
                                            Value::Null => None,
                                            _ => panic!("integer aggregate output"),
                                        }
                                    })
                                    .collect::<Vec<_>>(),
                            );
                        }
                        if !complete {
                            reached = true;
                            break;
                        }
                    }
                    Advance::Finished => {
                        reached = true;
                        break;
                    }
                    Advance::Progress => (),
                }
            }
            assert!(reached);
            runtime.replay_output_for_test();
            let effects_before = effects.count();
            let mut replayed = Vec::new();
            let mut finished = false;
            for _ in 0..8192 {
                match runtime.step(&result.plan, &cancel, &mut effects).unwrap() {
                    Advance::Rows => {
                        for row in 0..runtime.output().len() {
                            replayed.push(
                                (0..result.plan.output().column_count)
                                    .map(|column| {
                                        match runtime.output().value(row, column).unwrap() {
                                            Value::Int64(value) => Some(value),
                                            Value::Null => None,
                                            _ => panic!("integer aggregate output"),
                                        }
                                    })
                                    .collect::<Vec<_>>(),
                            );
                        }
                    }
                    Advance::Finished => {
                        finished = true;
                        break;
                    }
                    Advance::Progress => (),
                }
            }
            assert!(finished);
            let expected = if mode == 0 {
                vec![vec![Some(12)]]
            } else {
                vec![
                    vec![None, None],
                    vec![Some(1), Some(3)],
                    vec![Some(2), Some(9)],
                ]
            };
            assert_eq!(replayed, expected, "mode={mode} complete={complete}");
            assert!(replayed.starts_with(&first));
            if complete {
                assert_eq!(first, replayed);
            }
            if mode != 2 {
                assert_eq!(
                    effects.count(),
                    effects_before,
                    "retained memory replay performs no I/O"
                );
            }
            runtime.replay_output_for_test();
            assert!(runtime.step(&result.plan, &cancel, &mut effects).is_err());
            assert!(runtime.step(&result.plan, &cancel, &mut effects).is_err());
            drop(result);
            assert_eq!(database.reserved_memory_bytes(), pressured);
            assert_eq!(database.reserved_temp_bytes(), 0);
        }
        drop(pressure);
        assert_eq!(database.reserved_memory_bytes(), baseline);
    }
}

#[test]
fn aggregate_replay_rechecks_consumed_disk_bytes_and_cancellation() {
    let directory = Directory::new();
    let database = database(
        &directory,
        &[(Some(1), Some(3), 3.0), (Some(2), Some(9), 9.0)],
    );
    let cancel = CancellationToken::new();
    let query = database
        .prepare("FROM facts |> AGGREGATE SUM(n) AS total GROUP AND ORDER BY k")
        .unwrap();
    let baseline = database.reserved_memory_bytes();
    let initial = database.execute(&query, &cancel).unwrap();
    let Some(Aggregation::General(owner)) = initial.first_aggregate() else {
        unreachable!()
    };
    let disk_peak = initial.accounted_memory_bytes() + crate::catalog::MAX_BYTES as u64
        - owner[0].memory[0].memory_bytes();
    drop(initial);
    let pressure = database
        .reserve_memory(
            database.config().memory_limit_bytes() - baseline - disk_peak,
            "disk replay fault pressure",
        )
        .unwrap();
    let pressured = database.reserved_memory_bytes();
    for cancelled in [false, true] {
        let token = CancellationToken::new();
        let mut result = database.execute(&query, &token).unwrap();
        let mut emitted = false;
        for _ in 0..8192 {
            match result.step() {
                QueryStep::Rows(_) => {
                    emitted = true;
                    break;
                }
                QueryStep::Progress => (),
                _ => panic!("expected first disk result"),
            }
        }
        assert!(emitted);
        let State::Running(runtime) = &mut result.state else {
            unreachable!()
        };
        let Aggregation::General(owner) = &mut runtime.aggregates[0] else {
            unreachable!()
        };
        let general = &mut owner[0];
        assert!(general.memory.is_empty());
        assert!(general.result_cursor > 0);
        if cancelled {
            token.cancel();
        } else {
            let slot = general.sort.spool_slot();
            let Files::Open(scratch) = &mut general.files else {
                unreachable!()
            };
            scratch
                .write(slot, 24, &[1], &cancel, &mut Effects::default())
                .unwrap();
        }
        runtime.replay_output_for_test();
        let mut failed = false;
        for _ in 0..16 {
            match result.step() {
                QueryStep::Failed(error) => {
                    assert!(if cancelled {
                        matches!(error, Error::Cancelled)
                    } else {
                        matches!(error, Error::Corrupt(_))
                    });
                    failed = true;
                    break;
                }
                QueryStep::Progress => (),
                _ => panic!("replay must not publish corrupted or cancelled output"),
            }
        }
        assert!(failed);
        assert!(matches!(result.step(), QueryStep::Failed(_)));
        drop(result);
        assert_eq!(database.reserved_memory_bytes(), pressured);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
    drop(pressure);
}

#[test]
fn downstream_hash_fallback_replays_retained_aggregate_input() {
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
    let query = database.prepare(
        "FROM facts |> AGGREGATE SUM(n) AS total GROUP AND ORDER BY k |> AGGREGATE SUM(total) AS subtotal GROUP AND ORDER BY total"
    ).unwrap();
    let baseline = database.reserved_memory_bytes();
    for disk in [false, true] {
        let mut result = database.execute(&query, &cancel).unwrap();
        let State::Running(runtime) = &mut result.state else {
            unreachable!()
        };
        // Keep public preparation and scheduler transitions; set only optional
        // hash capacities before execution to make fallback deterministic.
        for (index, aggregation) in runtime.aggregates.iter_mut().enumerate() {
            let Aggregation::General(owner) = aggregation else {
                unreachable!()
            };
            let general = &mut owner[0];
            if index == 0 {
                assert!(!general.memory.is_empty());
                if disk {
                    general.memory.clear();
                    general.phase = Phase::Create;
                }
            } else {
                general.memory.clear();
                let groups = MemoryGroups::new(
                    &database,
                    &general.aggregate,
                    &general.keys,
                    1,
                    general.keys.max_bytes,
                )
                .unwrap();
                if general.memory.capacity() == 0 {
                    general.memory =
                        allocate(1, 1, "hash group owner", groups.memory_bytes()).unwrap();
                }
                general.memory.push(groups);
                general.phase = Phase::Read;
            }
        }
        let mut effects = Effects::default();
        let mut rows = Vec::new();
        let mut finished = false;
        for _ in 0..16384 {
            match runtime.step(&result.plan, &cancel, &mut effects).unwrap() {
                Advance::Rows => {
                    for row in 0..runtime.output().len() {
                        rows.push(
                            (0..2)
                                .map(
                                    |column| match runtime.output().value(row, column).unwrap() {
                                        Value::Int64(value) => Some(value),
                                        Value::Null => None,
                                        _ => panic!("integer grouping result"),
                                    },
                                )
                                .collect::<Vec<_>>(),
                        );
                    }
                }
                Advance::Finished => {
                    finished = true;
                    break;
                }
                Advance::Progress => (),
            }
        }
        assert!(finished, "disk={disk}");
        assert_eq!(
            rows,
            [
                vec![None, None],
                vec![Some(3), Some(3)],
                vec![Some(9), Some(9)]
            ]
        );
        let Aggregation::General(upstream) = &runtime.aggregates[0] else {
            unreachable!()
        };
        assert!(upstream[0].replayed, "downstream must request input replay");
        assert_eq!(upstream[0].memory.is_empty(), disk);
        let Aggregation::General(downstream) = &runtime.aggregates[1] else {
            unreachable!()
        };
        assert!(
            downstream[0].memory.is_empty(),
            "hash capacity must force fallback"
        );
        drop(result);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
}
