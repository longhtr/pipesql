use super::*;
use crate::effects::Faults;
use crate::execution::blocking::Io;
use crate::execution::blocking::test_support::{Directory, schema};
use crate::execution::*;

#[test]
fn evaluated_sorted_arguments_use_the_existing_aggregate_kernel() {
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
    let query = database.prepare("FROM facts |> AGGREGATE COUNT(*) AS nrows,SUM(n) AS ns,AVG(n) AS na,SUM(d) AS ds,AVG(d) AS da,SUM(2) AS twos").unwrap();
    let semantic = query.plan.aggregates.first().unwrap();
    let demand = query.plan.aggregate_demand(0);
    assert!(
        database
            .prepare("FROM facts |> AGGREGATE COUNT(*) AS nrows GROUP BY n")
            .is_ok(),
        "declared grouping uses the public binder"
    );
    // Trusted group metadata is the narrow test boundary. Numeric expressions
    // and demand come from the ordinary parser/binder, without a second program.
    let mut keys = schema(&[(DataType::String, false)]);
    keys.columns[0].input = 2;
    let rows = [
        (2, Some(i64::MAX), f64::MAX),
        (0, None, -0.0),
        (2, Some(i64::MAX), f64::MAX),
        (1, Some(10), 10.0),
        (0, None, -0.0),
        (2, Some(-i64::MAX), -f64::MAX),
        (1, Some(14), 14.0),
        (3, Some(1), f64::from_bits(0x7ff8_0000_0000_0123)),
        (4, Some(i64::MAX), f64::MAX),
        (4, Some(i64::MAX), f64::MAX),
        (3, Some(2), f64::INFINITY),
    ];
    let labels = ["A", "B", "C", "D", "E"];
    let batch_charge = database
        .reserve_memory(crate::batch::MAX_BYTES, "test input batch")
        .unwrap();
    let mut batch = Batch::new(
        &[DataType::Int64, DataType::Double, DataType::String],
        batch_charge.bytes(),
    )
    .unwrap();
    for (row, (group, integer, double)) in rows.iter().copied().enumerate() {
        batch
            .set(row, 0, integer.map_or(Value::Null, Value::Int64))
            .unwrap();
        batch.set(row, 1, Value::Double(double)).unwrap();
        batch
            .set(row, 2, Value::String(StringValue::new(labels[group])))
            .unwrap();
    }
    batch.publish_rows(rows.len());
    let mut direct = AggregateState::new(
        &database.memory,
        semantic,
        demand,
        labels.len(),
        query.plan.input_columns(),
    )
    .unwrap();
    let mut positions = [(0, 0); BATCH_ROWS];
    for (row, (group, _, _)) in rows.iter().enumerate() {
        positions[row] = (*group, direct.cells.count_row(*group).unwrap());
    }
    direct.consume(&batch, &positions[..rows.len()]).unwrap();
    assert_eq!(direct.value(0, 1).unwrap(), Value::Null);
    assert_eq!(direct.value(1, 1).unwrap(), Value::Int64(24));
    assert_eq!(direct.value(2, 1).unwrap(), Value::Int64(i64::MAX));
    assert!(matches!(
        direct.value(4, 1),
        Err(Error::ArithmeticOverflow {
            operation: "SUM",
            ..
        })
    ));
    let before = database.reserved_memory_bytes();
    for capacity in [1, 3, BATCH_ROWS] {
        for lanes in [1, 2, BATCH_ROWS] {
            let mut aggregate = AggregateState::new(
                &database.memory,
                semantic,
                demand,
                1,
                query.plan.input_columns(),
            )
            .unwrap();
            aggregate.lanes = lanes;
            let shape = ArgumentShape::from_aggregate(&aggregate);
            assert_eq!(
                shape.count, 3,
                "SUM/AVG of the same expression share one state"
            );
            assert_eq!(shape.integers, 5);
            let mut arguments = ArgumentBatch::new(&database, shape, capacity).unwrap();
            assert!(arguments.reservation.bytes() >= (shape.count * capacity * 8) as u64);
            let record_bytes = RECORD_HEADER + keys.max_bytes + shape.count * 8;
            let record_charge = database
                .reserve_memory(record_bytes as u64, "test captured record")
                .unwrap();
            let mut record = SortRecord::new(record_bytes, record_charge.bytes()).unwrap();
            let mut sort = RowSort::new(&database, &keys, shape, record_bytes, 2).unwrap();
            let mut effects = Effects::default();
            let mut scratch =
                crate::scratch::Scratch::new(&database, &cancel, &mut effects).unwrap();
            for start in (0..rows.len()).step_by(capacity) {
                let end = (start + capacity).min(rows.len());
                arguments
                    .evaluate(&mut aggregate, &batch, start..end, &cancel)
                    .unwrap();
                for row in 0..end - start {
                    arguments
                        .encode_record(&mut record, &keys, &batch, row, (start + row) as u64)
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
            for _ in 0..1000 {
                if sort
                    .step(&keys, &mut Io::new(&mut scratch, &cancel, &mut effects))
                    .unwrap()
                    == SortPhase::Done
                {
                    break;
                }
            }
            assert_eq!(sort.phase(), SortPhase::Done);
            let mut reduction = Reduction::Start;
            let mut group = 0;
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
                    Reduction::Done => break,
                    Reduction::Group => {}
                    _ => continue,
                }
                assert_eq!(
                    reduction.value(&sort, &aggregate, &keys, 0).unwrap(),
                    Value::String(StringValue::new(labels[group]))
                );
                for entry in 0..semantic.entries.len() {
                    match (
                        direct.value(group, entry),
                        reduction.value(&sort, &aggregate, &keys, entry + 1),
                    ) {
                        (Ok(Value::Double(expected)), Ok(Value::Double(actual))) => {
                            assert_eq!(actual.to_bits(), expected.to_bits())
                        }
                        (Ok(expected), Ok(actual)) => assert_eq!(actual, expected),
                        (
                            Err(Error::ArithmeticOverflow {
                                operation: expected,
                                span: expected_span,
                            }),
                            Err(Error::ArithmeticOverflow {
                                operation: actual,
                                span: actual_span,
                            }),
                        ) => {
                            assert_eq!(actual, expected);
                            assert_eq!(actual_span, expected_span);
                        }
                        mismatch => panic!("replayed group differs: {mismatch:?}"),
                    }
                }
                group += 1;
            }
            assert_eq!(reduction, Reduction::Done);
            assert_eq!(group, labels.len());
            assert_eq!(sort.reduction_progress().0, 0);
            if capacity == BATCH_ROWS && lanes == BATCH_ROWS {
                for stop in [Reduction::Start, Reduction::Input, Reduction::Group] {
                    let cancelled = CancellationToken::new();
                    let mut reduction = Reduction::Start;
                    for _ in 0..100 {
                        if reduction == stop {
                            cancelled.cancel();
                        }
                        if let Err(error) = reduction.step(
                            &mut sort,
                            &mut arguments,
                            &mut aggregate,
                            &keys,
                            &mut Io::new(&mut scratch, &cancelled, &mut effects),
                        ) {
                            assert!(matches!(error, Error::Cancelled));
                            break;
                        }
                    }
                    assert_eq!(reduction, Reduction::Failed);
                    let stopped_at = effects.count();
                    assert!(
                        reduction
                            .step(
                                &mut sort,
                                &mut arguments,
                                &mut aggregate,
                                &keys,
                                &mut Io::new(&mut scratch, &cancel, &mut effects)
                            )
                            .is_err()
                    );
                    assert_eq!(effects.count(), stopped_at);
                }
                for short in [false, true] {
                    let mut injected = Effects::with_faults(Faults {
                        fail_at: if short { None } else { Some(0) },
                        short_at: if short { Some(0) } else { None },
                        ..Faults::default()
                    });
                    let mut reduction = Reduction::Start;
                    reduction
                        .step(
                            &mut sort,
                            &mut arguments,
                            &mut aggregate,
                            &keys,
                            &mut Io::new(&mut scratch, &cancel, &mut injected),
                        )
                        .unwrap();
                    assert!(matches!(
                        reduction.step(
                            &mut sort,
                            &mut arguments,
                            &mut aggregate,
                            &keys,
                            &mut Io::new(&mut scratch, &cancel, &mut injected)
                        ),
                        Err(Error::Io { .. })
                    ));
                    assert_eq!(reduction, Reduction::Failed);
                    assert_eq!(sort.reduction_progress().1, 0);
                }
                // Empty grouped input must finish without inventing a group.
                let mut empty_sort =
                    RowSort::new(&database, &keys, shape, record_bytes, 2).unwrap();
                let mut empty_scratch =
                    crate::scratch::Scratch::new(&database, &cancel, &mut effects).unwrap();
                empty_sort.finish().unwrap();
                for _ in 0..10 {
                    if empty_sort
                        .step(
                            &keys,
                            &mut Io::new(&mut empty_scratch, &cancel, &mut effects),
                        )
                        .unwrap()
                        == SortPhase::Done
                    {
                        break;
                    }
                }
                let mut reduction = Reduction::Start;
                reduction
                    .step(
                        &mut empty_sort,
                        &mut arguments,
                        &mut aggregate,
                        &keys,
                        &mut Io::new(&mut empty_scratch, &cancel, &mut effects),
                    )
                    .unwrap();
                assert_eq!(
                    reduction
                        .step(
                            &mut empty_sort,
                            &mut arguments,
                            &mut aggregate,
                            &keys,
                            &mut Io::new(&mut empty_scratch, &cancel, &mut effects)
                        )
                        .unwrap(),
                    Reduction::Done
                );
            }
            drop(scratch);
            drop(sort);
            drop(record);
            drop(record_charge);
            drop(arguments);
            drop(aggregate);
            assert_eq!(database.reserved_memory_bytes(), before);
            assert_eq!(database.reserved_temp_bytes(), 0);
        }
    }
}
