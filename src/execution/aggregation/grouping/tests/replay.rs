//! Check grouping replay through computed, sorted, joined and aggregated producers.
//!
//! Memory pressure forces hash fallback after work has begun. Literal expected rows
//! and controller observations establish both the answer and the replayed producer.
//! A producer may have emitted a prefix or finished before replay is requested.
//! Retained disk bytes are damaged after consumption to require fresh validation;
//! cancellation and text-capacity cases check that replay preserves ownership.

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
    #[derive(Debug)]
    enum Producer {
        Join,
        SortedSet,
        Order,
        Scan,
    }
    use Producer::*;

    // Keep the producer to observe, input program and literal answer together.
    for (producer, sql, expected) in [
        (
            Join,
            "FROM facts AS l |> JOIN facts AS r ON l.k = r.k |> AGGREGATE SUM(l.n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY l.k",
            [[1, 14, 4], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> AGGREGATE SUM(n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 7, 2], [2, 7, 1]],
        ),
        (
            Join,
            "FROM facts AS l |> JOIN facts AS r ON l.k = r.k |> LIMIT 5 |> LIMIT 5 |> AGGREGATE SUM(l.n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY l.k",
            [[1, 14, 4], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> LIMIT 3 |> LIMIT 2 |> AGGREGATE SUM(n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 4, 1], [2, 7, 1]],
        ),
        (
            Scan,
            "FROM facts |> LIMIT 3 |> LIMIT 3 |> AGGREGATE SUM(n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 7, 2], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> SELECT k, n+1 AS shifted |> SELECT k, shifted-1 AS n |> ORDER BY n DESC |> AGGREGATE SUM(n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k |> SELECT k, total+0 AS total, nrows",
            [[1, 7, 2], [2, 7, 1]],
        ),
        (
            Join,
            "FROM facts |> SELECT k, n+0 AS n |> AS l |> JOIN facts AS r ON l.k=r.k |> SELECT l.k AS k, l.n+0 AS n |> AGGREGATE SUM(n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k |> SELECT k, total+0 AS total, nrows",
            [[1, 14, 4], [2, 7, 1]],
        ),
        (
            Scan,
            "FROM facts |> SELECT k, n+1 AS n |> LIMIT 3 |> SELECT k, n-1 AS n |> LIMIT 3 |> AGGREGATE SUM(n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k |> SELECT k, total+0 AS total, nrows",
            [[1, 7, 2], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> DISTINCT |> LIMIT 3 |> AGGREGATE SUM(n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 7, 2], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> SELECT k |> DISTINCT |> LIMIT 2 |> AGGREGATE SUM(k) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 1, 1], [2, 2, 1]],
        ),
        (
            Order,
            "FROM facts |> UNION DISTINCT (FROM facts) |> AGGREGATE SUM(n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 7, 2], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> EXTEND COUNT(*) OVER () AS partition_rows |> WHERE partition_rows=3 |> AGGREGATE SUM(n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 7, 2], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> AGGREGATE SUM(n) AS total, COUNT(SAFE_DIVIDE(n, k-1)) AS nrows GROUP AND ORDER BY k",
            [[1, 7, 0], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> AGGREGATE SUM(ABS(n-5)) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 3, 2], [2, 2, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> AGGREGATE SUM(MOD(n, 3)) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 1, 2], [2, 1, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> AGGREGATE SUM(DIV(n, 3)) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 2, 2], [2, 2, 1]],
        ),
        (
            Join,
            "FROM facts AS l |> LEFT JOIN (FROM facts |> WHERE k=2) AS r ON l.k=r.k |> AGGREGATE SUM(l.n) AS total, COUNT(r.n) AS nrows GROUP AND ORDER BY l.k",
            [[1, 7, 0], [2, 7, 1]],
        ),
        (
            Join,
            "FROM facts AS l |> LEFT JOIN (FROM facts |> WHERE k=2) AS r ON l.k=r.k |> AGGREGATE SUM(COALESCE(r.n, 5)) AS total, COUNT(COALESCE(r.n, 0)) AS nrows GROUP AND ORDER BY l.k",
            [[1, 10, 2], [2, 7, 1]],
        ),
        (
            SortedSet,
            "FROM facts |> EXCEPT DISTINCT (FROM facts |> WHERE n=3) |> AGGREGATE SUM(n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 4, 1], [2, 7, 1]],
        ),
        (
            SortedSet,
            "FROM facts |> INTERSECT DISTINCT (FROM facts |> WHERE n>3) |> AGGREGATE SUM(n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 4, 1], [2, 7, 1]],
        ),
        (
            SortedSet,
            "FROM facts |> UNION ALL (FROM facts) |> EXCEPT ALL (FROM facts |> WHERE n=3) |> AGGREGATE SUM(n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 11, 3], [2, 14, 2]],
        ),
        (
            SortedSet,
            "FROM facts |> UNION ALL (FROM facts) |> INTERSECT ALL (FROM facts |> UNION ALL (FROM facts)) |> AGGREGATE SUM(n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 14, 4], [2, 14, 2]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> AGGREGATE SUM(NULLIF(n, 3)) AS total, COUNT(NULLIF(n, 3)) AS nrows GROUP AND ORDER BY k",
            [[1, 4, 1], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> EXTEND NULLIF(n, 3) AS normalized |> WHERE normalized IS DISTINCT FROM 3 |> AGGREGATE SUM(n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 7, 2], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> WHERE n NOT IN (3) AND n NOT BETWEEN 0 AND 2 |> AGGREGATE SUM(n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 4, 1], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> AGGREGATE SUM(SIGN(n-5)) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, -2, 2], [2, 1, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> AGGREGATE SUM(n) AS total, COUNT(NULLIF(FLOOR(n/3), 1)) AS nrows GROUP AND ORDER BY k",
            [[1, 7, 0], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> AGGREGATE SUM(n) AS total, COUNT(NULLIF(CEILING(n/3), 1)) AS nrows GROUP AND ORDER BY k",
            [[1, 7, 1], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> AGGREGATE SUM(n) AS total, COUNT(NULLIF(ROUND(n/2), 2)) AS nrows GROUP AND ORDER BY k",
            [[1, 7, 0], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> AGGREGATE SUM(n) AS total, COUNT(NULLIF(SQRT(n-3), 1)) AS nrows GROUP AND ORDER BY k",
            [[1, 7, 1], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> AGGREGATE SUM(n) AS total, COUNT(NULLIF(LN(n-2), 0)) AS nrows GROUP AND ORDER BY k",
            [[1, 7, 1], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> AGGREGATE SUM(n) AS total, COUNT(NULLIF(EXP(n-3), 1)) AS nrows GROUP AND ORDER BY k",
            [[1, 7, 1], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> AGGREGATE SUM(n) AS total, COUNT(NULLIF(LOG10(n-2), 0)) AS nrows GROUP AND ORDER BY k",
            [[1, 7, 1], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> AGGREGATE SUM(n) AS total, COUNT(NULLIF(POWER(n-2, 2), 1)) AS nrows GROUP AND ORDER BY k",
            [[1, 7, 1], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> EXTEND '雪' AS text |> ORDER BY n DESC |> EXTEND BYTE_LENGTH(text) AS width |> AGGREGATE SUM(width+n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 13, 2], [2, 10, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> EXTEND '雪' AS text |> EXTEND BYTE_LENGTH(text) AS width |> AGGREGATE SUM(width+n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 13, 2], [2, 10, 1]],
        ),
        (
            Order,
            "FROM facts |> EXTEND '雪' AS text |> ORDER BY n DESC |> EXTEND CHAR_LENGTH(text) AS width |> AGGREGATE SUM(width+n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 9, 2], [2, 8, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> EXTEND '雪' AS text |> EXTEND CHAR_LENGTH(text) AS width |> AGGREGATE SUM(width+n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 9, 2], [2, 8, 1]],
        ),
        (
            Order,
            "FROM facts |> EXTEND CAST(n AS DOUBLE) AS converted |> ORDER BY n DESC |> AGGREGATE SUM(n) AS total, COUNT(NULLIF(converted, 3)) AS nrows GROUP AND ORDER BY k",
            [[1, 7, 1], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> EXTEND CAST(n AS FLOAT64) AS converted |> AGGREGATE SUM(n) AS total, COUNT(NULLIF(converted, 3)) AS nrows GROUP AND ORDER BY k",
            [[1, 7, 1], [2, 7, 1]],
        ),
        (
            Order,
            "FROM facts |> EXTEND DATE '2000-02-29' AS calendar_date |> ORDER BY n DESC |> EXTEND EXTRACT(YEAR FROM calendar_date) AS y |> AGGREGATE SUM(y+n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 4007, 2], [2, 2007, 1]],
        ),
        (
            Order,
            "FROM facts |> ORDER BY n DESC |> EXTEND DATE '2000-02-29' AS calendar_date |> EXTEND EXTRACT(YEAR FROM calendar_date) AS y |> AGGREGATE SUM(y+n) AS total, COUNT(*) AS nrows GROUP AND ORDER BY k",
            [[1, 4007, 2], [2, 2007, 1]],
        ),
    ] {
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
                replay |= match producer {
                    Join => runtime.first_join_mut().was_replayed(),
                    SortedSet => runtime.first_sorted_set_mut().was_replayed(),
                    Order => runtime.first_order_mut().was_replayed(),
                    Scan => disk,
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
                QueryStep::Failed(error) => panic!("{producer:?}: {sql}: {error}"),
            }
        }
        assert!(done && disk && replay, "{producer:?}: {sql}");
        assert_eq!(rows, expected, "{producer:?}: {sql}");
        drop(result);
        drop(pressure);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
}

#[test]
fn runtime_replays_retained_output_after_prefix_or_completion() {
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
    for mode in 0..5 {
        let sql = if mode == 0 {
            "FROM facts |> AGGREGATE SUM(n) AS total"
        } else if mode == 4 {
            "FROM facts |> AGGREGATE COUNT(SAFE_DIVIDE(n, k-1)) AS present GROUP AND ORDER BY k"
        } else if mode == 3 {
            "FROM facts |> SELECT COUNT(*) OVER () AS n"
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
            } else if mode == 4 {
                vec![
                    vec![None, Some(0)],
                    vec![Some(1), Some(0)],
                    vec![Some(2), Some(2)],
                ]
            } else if mode == 3 {
                vec![vec![Some(4)]; 4]
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

#[test]
fn full_length_text_extrema_survive_hash_fallback_and_cancelled_reduction() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(16_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    database
        .declare_table(
            "words",
            &[
                crate::ColumnDeclaration {
                    name: "k",
                    data_type: DataType::Int64,
                    nullable: false,
                },
                crate::ColumnDeclaration {
                    name: "word",
                    data_type: DataType::String,
                    nullable: true,
                },
            ],
            &cancel,
        )
        .unwrap();
    let long_m = "m".repeat(crate::batch::MAX_TEXT_BYTES);
    let long_n = "n".repeat(crate::batch::MAX_TEXT_BYTES);
    // Separate source batches let sorted replay encounter more than one full
    // text arena's worth of values within a single group.
    for (key, word) in [
        (1, Some(long_m.as_str())),
        (2, Some("é")),
        (1, Some("")),
        (2, Some("z")),
        (0, None),
        (0, None),
        (1, Some(long_n.as_str())),
    ] {
        let mut append = database
            .begin_append(
                "words",
                crate::AppendLimits {
                    batches: 1,
                    encoded_bytes: 200_000,
                },
                &cancel,
            )
            .unwrap();
        append
            .write(
                &[
                    crate::ColumnInput {
                        values: crate::ColumnValues::Int64(&[key]),
                        validity: &[1],
                    },
                    crate::ColumnInput {
                        values: crate::ColumnValues::String(&[word.unwrap_or("")]),
                        validity: &[u8::from(word.is_some())],
                    },
                ],
                &cancel,
            )
            .unwrap();
        append.commit(&cancel).unwrap();
    }
    let query = database.prepare(
        "FROM words |> AGGREGATE MIN(word) AS lo, MAX(word) AS hi, COUNT(word) AS n GROUP AND ORDER BY k"
    ).unwrap();
    let baseline = database.reserved_memory_bytes();
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Scenario {
        Memory,
        Spill,
        CancelReduction,
        RefuseTemporary,
    }
    for scenario in [
        Scenario::Memory,
        Scenario::Spill,
        Scenario::CancelReduction,
        Scenario::RefuseTemporary,
    ] {
        let hash_groups = if scenario == Scenario::Memory { 3 } else { 1 };
        let cancel_reduction = scenario == Scenario::CancelReduction;
        let occupied = if scenario == Scenario::RefuseTemporary {
            database.config().temp_limit_bytes()
        } else {
            0
        };
        let cancel = CancellationToken::new();
        let mut result = database.execute(&query, &cancel).unwrap();
        let State::Running(runtime) = &mut result.state else {
            unreachable!()
        };
        let Aggregation::General(owner) = &mut runtime.aggregates[0] else {
            unreachable!()
        };
        let general = &mut owner[0];
        // Change only optional capacity; the public plan, admitted fallback,
        // source replay, scheduler, and output ownership remain intact.
        general.memory.clear();
        let groups = MemoryGroups::new(
            &database,
            &general.aggregate,
            &general.keys,
            hash_groups,
            hash_groups * general.keys.max_bytes,
        )
        .unwrap();
        assert!(general.memory.capacity() >= 1);
        general.memory.push(groups);
        database.temporary.reserve(occupied).unwrap();
        let mut refused = false;
        let mut observed = Vec::new();
        let mut saw_spill = false;
        let mut saw_reduce = false;
        let mut finished = false;
        let mut cancelled = false;
        for _ in 0..50_000 {
            let Aggregation::General(owner) = &runtime.aggregates[0] else {
                unreachable!()
            };
            saw_spill |=
                matches!(owner[0].files, Files::Open(_)) && database.reserved_temp_bytes() != 0;
            if owner[0].phase == Phase::Reduce {
                saw_reduce = true;
                if cancel_reduction {
                    cancel.cancel();
                }
            }
            match runtime.step(&result.plan, &cancel, &mut Effects::default()) {
                Ok(Advance::Rows) => {
                    let output = runtime.output();
                    for row in 0..output.len() {
                        let integer = |column| match output.value(row, column).unwrap() {
                            Value::Int64(value) => value,
                            _ => panic!("integer output"),
                        };
                        let text = |column| match output.value(row, column).unwrap() {
                            Value::Null => None,
                            Value::String(value) => Some(value.as_str().to_owned()),
                            _ => panic!("text output"),
                        };
                        observed.push((integer(0), text(1), text(2), integer(3)));
                    }
                }
                Ok(Advance::Progress) => (),
                Ok(Advance::Finished) => {
                    finished = true;
                    break;
                }
                Err(Error::Cancelled) if cancel_reduction => {
                    cancelled = true;
                    break;
                }
                Err(Error::Resource {
                    owner: "database temporary storage",
                    ..
                }) if scenario == Scenario::RefuseTemporary => {
                    refused = true;
                    break;
                }
                Err(error) => panic!("{error:?}"),
            }
        }
        if scenario == Scenario::RefuseTemporary {
            assert!(refused && !finished);
            assert!(observed.is_empty(), "temporary refusal publishes no groups");
        } else if cancel_reduction {
            assert!(saw_reduce && cancelled && !finished);
            assert!(
                observed.is_empty(),
                "unfinished reduction publishes no groups"
            );
        } else {
            assert!(finished);
            assert_eq!(
                observed,
                vec![
                    (0, None, None, 0),
                    (1, Some(String::new()), Some(long_n.clone()), 3),
                    (2, Some("z".into()), Some("é".into()), 2),
                ]
            );
        }
        if scenario != Scenario::RefuseTemporary {
            assert_eq!(saw_spill, hash_groups == 1);
            assert_eq!(saw_reduce, hash_groups == 1);
        }
        drop(result);
        assert_eq!(database.reserved_temp_bytes(), occupied);
        database.temporary.release(occupied);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
    drop(query);
    database.close().unwrap();
}

#[test]
fn event_report_replays_nullable_joined_dates_and_labels() {
    use crate::{AppendLimits, ColumnDeclaration, ColumnInput, ColumnValues, DateValue};

    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("report"),
        crate::Config::new(16_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    database
        .declare_table(
            "events",
            &[
                ColumnDeclaration {
                    name: "dimension_id",
                    data_type: DataType::Int64,
                    nullable: true,
                },
                ColumnDeclaration {
                    name: "happened",
                    data_type: DataType::Date,
                    nullable: true,
                },
                ColumnDeclaration {
                    name: "amount",
                    data_type: DataType::Int64,
                    nullable: true,
                },
            ],
            &cancel,
        )
        .unwrap();
    database
        .declare_table(
            "dimensions",
            &[
                ColumnDeclaration {
                    name: "id",
                    data_type: DataType::Int64,
                    nullable: false,
                },
                ColumnDeclaration {
                    name: "label",
                    data_type: DataType::String,
                    nullable: false,
                },
            ],
            &cancel,
        )
        .unwrap();
    let dates =
        [10_956, 11_016, 0, 11_016].map(|day| DateValue::from_days_since_unix_epoch(day).unwrap());
    let mut append = database
        .begin_append(
            "events",
            AppendLimits {
                batches: 1,
                encoded_bytes: 2048,
            },
            &cancel,
        )
        .unwrap();
    append
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&[1, 2, 99, 0]),
                    validity: &[0b0111],
                },
                ColumnInput {
                    values: ColumnValues::Date(&dates),
                    validity: &[0b1011],
                },
                ColumnInput {
                    values: ColumnValues::Int64(&[10, 20, 0, 3]),
                    validity: &[0b1011],
                },
            ],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    let mut append = database
        .begin_append(
            "dimensions",
            AppendLimits {
                batches: 1,
                encoded_bytes: 2048,
            },
            &cancel,
        )
        .unwrap();
    append
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&[1, 2, 2]),
                    validity: &[0b111],
                },
                ColumnInput {
                    values: ColumnValues::String(&["north", "south", "南"]),
                    validity: &[0b111],
                },
            ],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();

    let resident = database.reserved_memory_bytes();
    let query = database
        .prepare(include_str!("../../../../../examples/event_report.sql"))
        .unwrap();
    let baseline = database.reserved_memory_bytes();
    let result = database.execute(&query, &cancel).unwrap();
    let Some(Aggregation::General(owner)) = result.first_aggregate() else {
        panic!("general grouping");
    };
    let general = &owner[0];
    // Admit hash grouping but leave room for only one key. Its second distinct
    // year/label pair must trigger disk fallback and replay the retained join.
    let retry_peak = result.accounted_memory_bytes() + crate::catalog::MAX_BYTES as u64
        - general.memory.first().map_or(0, MemoryGroups::memory_bytes)
        + MemoryGroups::requirement(&general.aggregate, &general.keys, 1, 9)
            .unwrap()
            .1;
    drop(result);
    let pressure = database
        .reserve_memory(
            database.config().memory_limit_bytes() - baseline - retry_peak,
            "event report replay pressure",
        )
        .unwrap();
    let mut result = database.execute(&query, &cancel).unwrap();
    let Some(Aggregation::General(owner)) = result.first_aggregate() else {
        panic!("general grouping");
    };
    assert_eq!(owner[0].memory.len(), 1);
    let expected = [
        (None, None, 1, 0, None),
        (Some(1999), Some("north"), 1, 1, Some(10)),
        (Some(2000), None, 1, 1, Some(3)),
        (Some(2000), Some("south"), 1, 1, Some(20)),
        (Some(2000), Some("南"), 1, 1, Some(20)),
    ];
    let (mut seen, mut disk, mut replay, mut finished) = (0, false, false, false);
    for _ in 0..10_000 {
        assert_eq!(
            database.reserved_memory_bytes(),
            baseline + pressure.bytes() + result.accounted_memory_bytes()
        );
        if let Some(Aggregation::General(owner)) = result.first_aggregate() {
            disk |= matches!(owner[0].files, Files::Open(_));
        }
        if let State::Running(runtime) = &mut result.state {
            replay |= runtime.first_join_mut().was_replayed();
        }
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let &(year, label, entries, present, total) =
                        expected.get(seen).expect("extra group");
                    assert_eq!(
                        batch.value(row, 0),
                        Some(year.map_or(Value::Null, Value::Int64))
                    );
                    match (batch.value(row, 1), label) {
                        (Some(Value::Null), None) => (),
                        (Some(Value::String(actual)), Some(label)) => {
                            assert_eq!(actual.as_str(), label)
                        }
                        _ => panic!("unexpected label"),
                    }
                    assert_eq!(batch.value(row, 2), Some(Value::Int64(entries)));
                    assert_eq!(batch.value(row, 3), Some(Value::Int64(present)));
                    assert_eq!(
                        batch.value(row, 4),
                        Some(total.map_or(Value::Null, Value::Int64))
                    );
                    seen += 1;
                }
            }
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("event report replay: {error}"),
        }
    }
    assert!(finished && disk && replay);
    assert_eq!(seen, expected.len());
    drop(result);
    drop(pressure);
    drop(query);
    assert_eq!(database.reserved_memory_bytes(), resident);
    assert_eq!(database.reserved_temp_bytes(), 0);
    database.close().unwrap();
}
