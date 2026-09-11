use super::*;
use crate::execution::aggregation::*;
use crate::execution::blocking::test_support::Directory;

#[test]
fn argument_capture_rejects_mask_tails_before_reserving_memory() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let baseline = database.reserved_memory_bytes();
    let valid = ArgumentShape {
        count: 1,
        nonnull: 0,
        integers: 0,
        presence: 1,
        dates: 0,
        text: 0,
    };
    for shape in [
        ArgumentShape {
            nonnull: 2,
            ..valid
        },
        ArgumentShape {
            integers: 2,
            ..valid
        },
        ArgumentShape {
            presence: 2,
            dates: 0,
            text: 0,
            ..valid
        },
        ArgumentShape { count: 0, ..valid },
        ArgumentShape { dates: 2, ..valid },
        ArgumentShape { text: 2, ..valid },
        ArgumentShape { text: 1, ..valid },
        ArgumentShape {
            text: 1,
            presence: 0,
            integers: 1,
            ..valid
        },
        // Stored dates require integer values and cannot be presence-only.
        ArgumentShape { dates: 1, ..valid },
        ArgumentShape {
            integers: 1,
            dates: 1,
            text: 0,
            ..valid
        },
        ArgumentShape {
            presence: 0,
            dates: 1,
            text: 0,
            ..valid
        },
    ] {
        assert!(matches!(
            ArgumentBatch::new(&database, shape, 1),
            Err(Error::Corrupt("argument batch shape"))
        ));
        assert_eq!(database.reserved_memory_bytes(), baseline);
    }
    let batch = ArgumentBatch::new(&database, valid, 1).unwrap();
    drop(batch);
    assert_eq!(database.reserved_memory_bytes(), baseline);
    database.close().unwrap();
}

#[test]
fn count_only_arguments_own_counters_and_share_numeric_work_when_needed() {
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
            &[crate::ColumnDeclaration {
                name: "n",
                data_type: DataType::Int64,
                nullable: true,
            }],
            &cancel,
        )
        .unwrap();
    let charge = database
        .reserve_memory(crate::batch::MAX_BYTES, "count test input")
        .unwrap();
    let mut batch = Batch::new(&[DataType::Int64], charge.bytes()).unwrap();
    for (row, value) in [Value::Null, Value::Int64(2), Value::Int64(4)]
        .into_iter()
        .enumerate()
    {
        batch.set(row, 0, value).unwrap();
    }
    batch.publish_rows(3);
    for (sql, shared) in [
        ("FROM facts |> AGGREGATE COUNT(n) AS c", false),
        (
            "FROM facts |> AGGREGATE COUNT(n) AS c,SUM(n) AS s,AVG(n) AS a",
            true,
        ),
    ] {
        let query = database.prepare(sql).unwrap();
        let semantic = query.plan.aggregates.first().unwrap();
        let demand = query.plan.aggregate_demand(0);
        let baseline = database.reserved_memory_bytes();
        let mut state = AggregateState::new(
            &database.memory,
            semantic,
            demand,
            1,
            query.plan.input_columns(),
        )
        .unwrap();
        state
            .validate_plan(semantic, demand, 1, query.plan.input_columns())
            .unwrap();
        assert_eq!(
            state.states, 1,
            "identical numeric arguments share evaluation"
        );
        assert_eq!(state.cells.nonnull_counts.len(), 1);
        assert!(state.cells.values.is_empty() && state.cells.flags.is_empty());
        assert_eq!(state.cells.integers.len(), usize::from(shared));
        if !shared {
            state.cells.value_slots[0] = 0;
            assert!(
                state
                    .validate_plan(semantic, demand, 1, query.plan.input_columns())
                    .is_err(),
                "validator rejects a count-only value slot"
            );
            state.cells.value_slots[0] = u8::MAX;
        }
        let shape = ArgumentShape::from_aggregate(&state);
        assert_eq!(shape.presence, u16::from(!shared));
        let mut arguments = ArgumentBatch::new(&database, shape, 3).unwrap();
        arguments
            .evaluate(&mut state, &batch, 0..3, &cancel)
            .unwrap();
        assert_eq!(arguments.value(0, 0), None);
        assert_eq!(arguments.value(0, 1), Some(if shared { 2 } else { 0 }));
        arguments.fold_group(&mut state, 0).unwrap();
        assert_eq!(state.value(0, 0).unwrap(), Value::Int64(2));
        if shared {
            assert_eq!(state.value(0, 1).unwrap(), Value::Int64(6));
            assert_eq!(state.value(0, 2).unwrap(), Value::Double(3.0));
        }
        drop(arguments);
        drop(state);
        assert_eq!(database.reserved_memory_bytes(), baseline);
    }
}

#[test]
fn extrema_capture_keeps_values_and_validation_rejects_misdirected_slots() {
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
            &[crate::ColumnDeclaration {
                name: "n",
                data_type: DataType::Int64,
                nullable: true,
            }],
            &cancel,
        )
        .unwrap();
    let query = database
        .prepare("FROM facts |> AGGREGATE MIN(n) AS lo,MAX(n) AS hi,COUNT(n) AS c")
        .unwrap();
    let semantic = query.plan.aggregates.first().unwrap();
    let demand = query.plan.aggregate_demand(0);
    let baseline = database.reserved_memory_bytes();
    let mut state = AggregateState::new(
        &database.memory,
        semantic,
        demand,
        1,
        query.plan.input_columns(),
    )
    .unwrap();
    state
        .validate_plan(semantic, demand, 1, query.plan.input_columns())
        .unwrap();
    assert_eq!(state.states, 1, "MIN/MAX/COUNT share one argument");
    assert!(state.cells.integers.is_empty());
    assert!(state.cells.values.is_empty());
    assert!(state.cells.flags.is_empty());
    assert_eq!(state.cells.extrema.len(), 2);
    assert_eq!(state.cells.extrema_slots[0][0], 0);
    assert_eq!(state.cells.extrema_slots[1][0], 1);

    // Challenge the independently derived direction, demand, and array extents.
    for (direction, argument, replacement) in [(0, 0, 1), (1, 0, u8::MAX), (0, 1, 0)] {
        let original = state.cells.extrema_slots[direction][argument];
        state.cells.extrema_slots[direction][argument] = replacement;
        assert!(
            state
                .validate_plan(semantic, demand, 1, query.plan.input_columns())
                .is_err()
        );
        state.cells.extrema_slots[direction][argument] = original;
    }
    let removed = state.cells.extrema.pop().unwrap();
    assert!(
        state
            .validate_plan(semantic, demand, 1, query.plan.input_columns())
            .is_err()
    );
    state.cells.extrema.push(removed);
    state.cells.value_slots[0] = 0;
    assert!(matches!(
        state.validate_plan(semantic, demand, 1, query.plan.input_columns()),
        Err(Error::Corrupt("non-summing state owns a sum cell"))
    ));
    state.cells.value_slots[0] = u8::MAX;
    state
        .validate_plan(semantic, demand, 1, query.plan.input_columns())
        .unwrap();

    let shape = ArgumentShape::from_aggregate(&state);
    assert_eq!(shape.presence, 0, "extrema must capture argument values");
    let mut arguments = ArgumentBatch::new(&database, shape, 3).unwrap();
    {
        let charge = database
            .reserve_memory(crate::batch::MAX_BYTES, "extrema test input")
            .unwrap();
        let mut batch = Batch::new(&[DataType::Int64], charge.bytes()).unwrap();
        for (row, value) in [Value::Null, Value::Int64(7), Value::Int64(-4)]
            .into_iter()
            .enumerate()
        {
            batch.set(row, 0, value).unwrap();
        }
        batch.publish_rows(3);
        arguments
            .evaluate(&mut state, &batch, 0..3, &cancel)
            .unwrap();
    }
    // The producer is gone; capture still owns the values used by reduction.
    assert_eq!(arguments.value(0, 0), None);
    assert_eq!(arguments.value(0, 1), Some(7));
    assert_eq!(arguments.value(0, 2), Some(-4_i64 as u64));
    arguments.fold_group(&mut state, 0).unwrap();
    assert_eq!(state.value(0, 0).unwrap(), Value::Int64(-4));
    assert_eq!(state.value(0, 1).unwrap(), Value::Int64(7));
    assert_eq!(state.value(0, 2).unwrap(), Value::Int64(2));
    state.cells.clear_group(0);
    assert_eq!(state.value(0, 0).unwrap(), Value::Null);
    assert_eq!(state.value(0, 1).unwrap(), Value::Null);
    arguments.fold_group(&mut state, 0).unwrap();
    assert_eq!(state.value(0, 0).unwrap(), Value::Int64(-4));
    assert_eq!(state.value(0, 1).unwrap(), Value::Int64(7));
    drop(arguments);
    drop(state);
    assert_eq!(database.reserved_memory_bytes(), baseline);
}

#[test]
fn key_mapping_uses_the_bound_semantic_identity() {
    let directory = Directory::new();
    let database = Database::create(
        &directory.0.join("db"),
        crate::Config::new(2_000_000, 1).unwrap(),
    )
    .unwrap();
    let query = database.prepare("FROM lineitem |> SELECT l_linestatus AS status, l_returnflag AS flag |> AGGREGATE COUNT(*) AS n,SUM(1) AS s,AVG(2.0) AS a GROUP BY flag,status").unwrap();
    let inputs: Vec<_> = query.plan.input_columns().collect();
    let keys = grouping::key_layout(query.plan.aggregates.first().unwrap(), &inputs).unwrap();
    let aggregate = AggregateState::new(
        &database.memory,
        query.plan.aggregates.first().unwrap(),
        query.plan.aggregate_demand(0),
        1,
        inputs.iter().copied(),
    )
    .unwrap();
    let arguments = ArgumentShape::from_aggregate(&aggregate);
    assert_eq!(arguments.count, 2);
    assert_eq!(arguments.nonnull, 3);

    let mut batch = Batch::new(&[DataType::String, DataType::String], 1_000_000).unwrap();
    for (column, source) in inputs.iter().enumerate() {
        let text = if *source == SourceColumn::RETURN_FLAG.semantic() {
            "A"
        } else {
            "F"
        };
        batch
            .set(0, column, Value::String(StringValue::new(text)))
            .unwrap();
    }
    batch.publish_rows(1);
    let mut key = Vec::with_capacity(keys.max_bytes);
    keys.encode(&batch, 0, &mut key).unwrap();
    assert_eq!(
        keys.value(&key, 0).unwrap(),
        Value::String(StringValue::new("A"))
    );
    assert_eq!(
        keys.value(&key, 1).unwrap(),
        Value::String(StringValue::new("F"))
    );
    assert!(grouping::key_layout(query.plan.aggregates.first().unwrap(), &[]).is_err());
}

#[test]
fn argument_batches_preserve_demand_and_refuse_before_publication() {
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
            &cancel,
        )
        .unwrap();
    let query = database
        .prepare("FROM facts |> AGGREGATE SUM(n*2) AS bad,COUNT(*) AS nrows |> SELECT nrows")
        .unwrap();
    let mut aggregate = AggregateState::new(
        &database.memory,
        query.plan.aggregates.first().unwrap(),
        query.plan.aggregate_demand(0),
        1,
        query.plan.input_columns(),
    )
    .unwrap();
    let mut arguments =
        ArgumentBatch::new(&database, ArgumentShape::from_aggregate(&aggregate), 3).unwrap();
    let charge = database
        .reserve_memory(crate::batch::MAX_BYTES, "test input batch")
        .unwrap();
    let mut batch = Batch::new(&[DataType::Int64, DataType::Double], charge.bytes()).unwrap();
    for row in 0..3 {
        batch
            .set(
                row,
                0,
                if row == 0 {
                    Value::Null
                } else {
                    Value::Int64(i64::MAX)
                },
            )
            .unwrap();
        batch.set(row, 1, Value::Double(f64::NAN)).unwrap();
    }
    batch.publish_rows(3);
    arguments
        .evaluate(&mut aggregate, &batch, 0..3, &cancel)
        .unwrap();
    assert_eq!(
        arguments.shape.count, 0,
        "hidden overflow expression is not evaluated"
    );
    assert_eq!(arguments.values.capacity(), 0);
    arguments.fold_group(&mut aggregate, 0).unwrap();
    assert_eq!(aggregate.value(0, 1).unwrap(), Value::Int64(3));
    aggregate.cells.clear_group(0);
    assert_eq!(aggregate.value(0, 1).unwrap(), Value::Int64(0));
    let query = database
        .prepare("FROM facts |> AGGREGATE SUM(n*2) AS bad,SUM(d) AS ds")
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
    let mut arguments = ArgumentBatch::new(&database, shape, 3).unwrap();
    assert!(matches!(
        arguments.evaluate(&mut aggregate, &batch, 0..3, &cancel),
        Err(Error::ArithmeticOverflow {
            operation: "multiplication",
            ..
        })
    ));
    assert_eq!(arguments.rows, 0);
    assert!(arguments.source_start.is_none());
    assert_eq!(
        aggregate.cells.counts[0], 0,
        "failed capture cannot accumulate partial input"
    );
    arguments
        .evaluate(&mut aggregate, &batch, 0..1, &cancel)
        .unwrap();
    assert_eq!(arguments.rows, 1);
    assert_eq!(arguments.value(0, 0), None);
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        arguments.evaluate(&mut aggregate, &batch, 0..1, &cancelled),
        Err(Error::Cancelled)
    ));
    assert_eq!(arguments.rows, 0);
    assert!(
        arguments
            .evaluate(&mut aggregate, &batch, 1..4, &cancel)
            .is_err()
    );
    let required = arguments.reservation.bytes();
    drop(arguments);
    let before = database.reserved_memory_bytes();
    for extra in [0, 1] {
        let pressure = database
            .reserve_memory(
                database.config().memory_limit_bytes() - before - required + extra,
                "argument admission test",
            )
            .unwrap();
        let admitted = ArgumentBatch::new(&database, shape, 3);
        if extra == 0 {
            drop(admitted.unwrap());
        } else {
            assert!(matches!(admitted, Err(Error::Resource { .. })));
        }
        drop(pressure);
        assert_eq!(database.reserved_memory_bytes(), before);
    }
    assert!(
        crate::scalar::NumericOutput::from_bits(&[0; BATCH_ROWS + 1], [u64::MAX; BATCH_ROWS / 64])
            .is_err()
    );
}

#[test]
fn text_extrema_reuse_owned_slots_for_growing_and_shrinking_values() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    database
        .declare_table(
            "words",
            &[crate::ColumnDeclaration {
                name: "word",
                data_type: DataType::String,
                nullable: true,
            }],
            &cancel,
        )
        .unwrap();
    let query = database
        .prepare("FROM words |> AGGREGATE MIN(word) AS lo,MAX(word) AS hi")
        .unwrap();
    let columns: Vec<_> = query.plan.input_columns().collect();
    let semantic = query.plan.aggregates.first().unwrap();
    let baseline = database.reserved_memory_bytes();
    let mut aggregate =
        AggregateState::new(&database.memory, semantic, 3, 1, columns.iter().copied()).unwrap();
    aggregate
        .validate_plan(semantic, 3, 1, columns.iter().copied())
        .unwrap();
    assert_eq!(
        aggregate.cells.text.capacity(),
        2 * crate::batch::MAX_TEXT_BYTES
    );
    let charge = aggregate.reservation.bytes();
    assert_eq!(
        charge as usize,
        aggregate.cells.text.capacity()
            + aggregate.cells.extrema.capacity() * size_of::<u64>()
            + aggregate.cells.nonnull_counts.capacity() * size_of::<u32>()
            + aggregate.cells.counts.capacity() * size_of::<u32>()
    );
    let long = "m".repeat(crate::batch::MAX_TEXT_BYTES);
    for text in [long.as_str(), "z", "a", "é", "", "a longer middle value"] {
        let types = [DataType::String];
        let capacities = [Some(crate::batch::MAX_TEXT_BYTES)];
        let input_charge = database
            .reserve_memory(
                Batch::required_bytes_with_text(&types, &capacities).unwrap(),
                "text extrema test batch",
            )
            .unwrap();
        let mut batch = Batch::new_with_text(&types, &capacities, input_charge.bytes()).unwrap();
        batch
            .set(0, 0, Value::String(StringValue::new(text)))
            .unwrap();
        batch.publish_rows(1);
        let count = aggregate.cells.count_row(0).unwrap();
        aggregate.consume(&batch, &[(0, count)]).unwrap();
        drop(batch);
        drop(input_charge);
        assert_eq!(database.reserved_memory_bytes() - baseline, charge);
    }
    assert_eq!(
        aggregate.value(0, 0).unwrap(),
        Value::String(StringValue::new(""))
    );
    assert_eq!(
        aggregate.value(0, 1).unwrap(),
        Value::String(StringValue::new("é"))
    );
    aggregate.cells.clear_group(0);
    assert_eq!(aggregate.value(0, 0).unwrap(), Value::Null);
    assert_eq!(aggregate.value(0, 1).unwrap(), Value::Null);
    let count = aggregate.cells.count_row(0).unwrap();
    aggregate.cells.fold_text(0, 0, count, "new").unwrap();
    for index in 0..2 {
        assert_eq!(
            aggregate.value(0, index).unwrap(),
            Value::String(StringValue::new("new"))
        );
    }
    aggregate.cells.text_offsets[1] -= 1;
    assert!(
        aggregate
            .validate_plan(semantic, 3, 1, columns.iter().copied())
            .is_err()
    );
    aggregate.cells.text_offsets[1] += 1;
    aggregate
        .validate_plan(semantic, 3, 1, columns.iter().copied())
        .unwrap();
    drop(aggregate);
    assert_eq!(database.reserved_memory_bytes(), baseline);
}

#[test]
fn text_capture_and_replay_stop_at_byte_capacity_before_row_capacity() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    database
        .declare_table(
            "words",
            &[crate::ColumnDeclaration {
                name: "word",
                data_type: DataType::String,
                nullable: true,
            }],
            &cancel,
        )
        .unwrap();
    let query = database
        .prepare("FROM words |> AGGREGATE MIN(word) AS lo,MAX(word) AS hi,COUNT(word) AS n")
        .unwrap();
    let semantic = query.plan.aggregates.first().unwrap();
    let baseline = database.reserved_memory_bytes();
    let mut aggregate =
        AggregateState::new(&database.memory, semantic, 7, 1, query.plan.input_columns()).unwrap();
    let shape = ArgumentShape::from_aggregate(&aggregate);
    assert_eq!(shape.count, 1);
    assert_eq!(shape.text, 1);
    assert_eq!(shape.presence, 0);
    let mut arguments = ArgumentBatch::new(&database, shape, 3).unwrap();
    let keys = crate::execution::blocking::test_support::schema(&[]);
    let record_bytes = RECORD_HEADER + shape.max_payload_bytes();
    let record_charge = database
        .reserve_memory(record_bytes as u64, "text capture test frame")
        .unwrap();
    let mut record = SortRecord::new(record_bytes, record_charge.bytes()).unwrap();
    let text = "m".repeat(crate::batch::MAX_TEXT_BYTES);
    {
        let types = [DataType::String];
        let capacities = [Some(crate::batch::MAX_TEXT_BYTES)];
        let input_charge = database
            .reserve_memory(
                Batch::required_bytes_with_text(&types, &capacities).unwrap(),
                "text capture test source",
            )
            .unwrap();
        let mut input = Batch::new_with_text(&types, &capacities, input_charge.bytes()).unwrap();
        input
            .set(0, 0, Value::String(StringValue::new(&text)))
            .unwrap();
        input.publish_rows(1);
        arguments
            .evaluate(&mut aggregate, &input, 0..1, &cancel)
            .unwrap();
        arguments
            .encode_record(&mut record, &keys, &input, 0, 0)
            .unwrap();
    }
    assert_eq!(arguments.text_value(0, 0).unwrap(), Some(text.as_str()));
    arguments.fold_group(&mut aggregate, 0).unwrap();
    assert_eq!(
        aggregate.value(0, 0).unwrap(),
        Value::String(StringValue::new(&text))
    );
    arguments.clear();
    aggregate.cells.clear_group(0);
    assert!(arguments.can_append(&record).unwrap());
    arguments.append(&record).unwrap();
    assert_eq!(arguments.rows, 1);
    assert_eq!(arguments.capacity, 3);
    assert!(!arguments.can_append(&record).unwrap());
    assert!(arguments.append(&record).is_err());
    assert_eq!(arguments.rows, 1, "failed append does not publish a row");
    arguments.fold_group(&mut aggregate, 0).unwrap();
    arguments.clear();
    assert!(arguments.can_append(&record).unwrap());
    arguments.append(&record).unwrap();
    arguments.fold_group(&mut aggregate, 0).unwrap();
    assert_eq!(
        aggregate.value(0, 0).unwrap(),
        Value::String(StringValue::new(&text))
    );
    assert_eq!(
        aggregate.value(0, 1).unwrap(),
        Value::String(StringValue::new(&text))
    );
    assert_eq!(aggregate.value(0, 2).unwrap(), Value::Int64(2));
    drop(record);
    drop(record_charge);
    drop(arguments);
    drop(aggregate);
    assert_eq!(database.reserved_memory_bytes(), baseline);
    assert_eq!(database.reserved_temp_bytes(), 0);
}
