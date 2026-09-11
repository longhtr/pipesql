use super::*;
use crate::execution::aggregation::*;
use crate::execution::blocking::test_support::Directory;

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
