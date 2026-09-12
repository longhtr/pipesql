//! Full-partition count when input demand contains no values to retain.
use crate::batch::Batch;
use crate::execution::computed::RowValues;
use crate::execution::planning::Pipeline;
use crate::execution::{BATCH_ROWS, ConsumerInput, ConsumerStep, MAX_AGGREGATE_ROWS};
use crate::{CancellationToken, Error};

#[derive(Clone, Copy)]
enum Phase {
    Read,
    Await,
    Emit,
    Done,
    Failed,
}

// Stored inline in the admitted runtime node. The completed count is sufficient
// to replay every zero-field input row; no row records or scratch files survive.
pub(super) struct Count {
    rows: u64,
    remaining: u64,
    phase: Phase,
    replayed: bool,
}

impl Count {
    pub(super) fn new(input_columns: usize) -> Result<Self, Error> {
        if input_columns != 0 {
            return Err(Error::Corrupt("count input still has demanded values"));
        }
        Ok(Self {
            rows: 0,
            remaining: 0,
            phase: Phase::Read,
            replayed: false,
        })
    }

    pub(super) fn replay(&mut self, cancel: &CancellationToken) -> Result<(), Error> {
        cancel.check()?;
        if self.replayed || !matches!(self.phase, Phase::Emit | Phase::Done) {
            return Err(Error::Corrupt("count replay requires completed input"));
        }
        self.remaining = self.rows;
        self.phase = Phase::Emit;
        self.replayed = true;
        Ok(())
    }

    pub(super) fn step(
        &mut self,
        input: ConsumerInput<'_>,
        output: &mut Batch,
        plan: &Pipeline<'_>,
        cancel: &CancellationToken,
    ) -> Result<ConsumerStep, Error> {
        output.clear();
        if matches!(self.phase, Phase::Done) {
            return Ok(ConsumerStep::Finished);
        }
        if matches!(self.phase, Phase::Failed) {
            return Err(Error::Corrupt("count has failed"));
        }
        let phase = std::mem::replace(&mut self.phase, Phase::Failed);
        cancel.check()?;
        let mut step = ConsumerStep::Progress;
        self.phase = match phase {
            Phase::Read => {
                step = ConsumerStep::Input;
                Phase::Await
            }
            Phase::Await => {
                if input.batch.column_count() != 0 {
                    return Err(Error::Corrupt("count received demanded values"));
                }
                if input.finished {
                    if !input.batch.is_empty() {
                        return Err(Error::Corrupt("finished count input contains rows"));
                    }
                    self.remaining = self.rows;
                    Phase::Emit
                } else {
                    let rows = input.batch.len();
                    if rows == 0 || rows > BATCH_ROWS {
                        return Err(Error::Corrupt("count input batch outside capacity"));
                    }
                    if rows as u64 > MAX_AGGREGATE_ROWS - self.rows {
                        return Err(Error::Resource {
                            owner: "aggregate input rows",
                            required: MAX_AGGREGATE_ROWS + 1,
                            limit: MAX_AGGREGATE_ROWS,
                        });
                    }
                    self.rows += rows as u64;
                    Phase::Read
                }
            }
            Phase::Emit => {
                if self.remaining == 0 {
                    Phase::Done
                } else {
                    // Evaluate one candidate per step. LIMIT can stop later
                    // ordinary expressions, even though counting consumed all input.
                    let mut values = RowValues::new(plan, |_| {
                        Err(Error::Corrupt("count expression demands an input value"))
                    })
                    .with_partition_count(self.rows);
                    if values.retains()? {
                        for (position, column) in
                            plan.columns[..plan.column_count].iter().enumerate()
                        {
                            output.set(0, position, values.value(*column)?)?;
                        }
                        output.publish_rows(1);
                        step = ConsumerStep::Rows;
                    }
                    self.remaining -= 1;
                    Phase::Emit
                }
            }
            Phase::Done | Phase::Failed => unreachable!("terminal count handled before work"),
        };
        Ok(step)
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;
    use crate::execution::planning::lower;
    use crate::execution::test_support::loaded;
    use crate::storage_format::RootState;
    use crate::{DataType, Value};

    #[test]
    fn counter_preserves_cardinality_and_replays_without_reading_input() {
        let (_fixture, db) = loaded(1);
        let query = db
            .prepare("FROM lineitem |> SELECT COUNT(*) OVER () AS n")
            .unwrap();
        let plan = lower(&db, &query, RootState::Empty, 0).unwrap();
        let pipeline = &plan.pipelines()[1];
        let mut input = Batch::new(&[], 0).unwrap();
        let mut output = Batch::new(&[DataType::Int64], crate::batch::MAX_BYTES).unwrap();
        let cancel = CancellationToken::new();
        let mut count = Count::new(0).unwrap();
        assert!(count.replay(&cancel).is_err());
        assert!(matches!(
            count.step(
                ConsumerInput {
                    batch: &input,
                    finished: false
                },
                &mut output,
                pipeline,
                &cancel
            ),
            Ok(ConsumerStep::Input)
        ));
        input.publish_rows(3);
        assert!(matches!(
            count.step(
                ConsumerInput {
                    batch: &input,
                    finished: false
                },
                &mut output,
                pipeline,
                &cancel
            ),
            Ok(ConsumerStep::Progress)
        ));
        assert!(output.is_empty());
        assert!(matches!(
            count.step(
                ConsumerInput {
                    batch: &input,
                    finished: false
                },
                &mut output,
                pipeline,
                &cancel
            ),
            Ok(ConsumerStep::Input)
        ));
        input.clear();
        count
            .step(
                ConsumerInput {
                    batch: &input,
                    finished: true,
                },
                &mut output,
                pipeline,
                &cancel,
            )
            .unwrap();
        for replay in [false, true] {
            if replay {
                count.replay(&cancel).unwrap();
            }
            // A completed input remains empty throughout both emissions.
            for _ in 0..3 {
                assert!(matches!(
                    count.step(
                        ConsumerInput {
                            batch: &input,
                            finished: true
                        },
                        &mut output,
                        pipeline,
                        &cancel
                    ),
                    Ok(ConsumerStep::Rows)
                ));
                assert_eq!(output.len(), 1);
                assert_eq!(output.value(0, 0), Some(Value::Int64(3)));
            }
            count
                .step(
                    ConsumerInput {
                        batch: &input,
                        finished: true,
                    },
                    &mut output,
                    pipeline,
                    &cancel,
                )
                .unwrap();
            assert!(matches!(
                count.step(
                    ConsumerInput {
                        batch: &input,
                        finished: true
                    },
                    &mut output,
                    pipeline,
                    &cancel
                ),
                Ok(ConsumerStep::Finished)
            ));
            assert!(output.is_empty());
        }
        assert!(count.replay(&cancel).is_err());
    }

    #[test]
    fn counter_rejects_values_invalid_batches_and_the_first_row_over_its_bound() {
        let (_fixture, db) = loaded(1);
        let query = db
            .prepare("FROM lineitem |> SELECT COUNT(*) OVER () AS n")
            .unwrap();
        let plan = lower(&db, &query, RootState::Empty, 0).unwrap();
        let pipeline = &plan.pipelines()[1];
        let mut output = Batch::new(&[DataType::Int64], crate::batch::MAX_BYTES).unwrap();
        let cancel = CancellationToken::new();
        assert!(Count::new(1).is_err());
        for (columns, rows, finished) in [(0, 0, false), (1, 1, false), (0, 1, true)] {
            let mut input =
                Batch::new(&[DataType::Int64][..columns], crate::batch::MAX_BYTES).unwrap();
            input.publish_rows(rows);
            let mut count = Count::new(0).unwrap();
            count.phase = Phase::Await;
            assert!(matches!(
                count.step(
                    ConsumerInput {
                        batch: &input,
                        finished
                    },
                    &mut output,
                    pipeline,
                    &cancel
                ),
                Err(Error::Corrupt(_))
            ));
            assert!(count.replay(&cancel).is_err());
        }
        let mut input = Batch::new(&[], 0).unwrap();
        input.publish_rows(1);
        let mut count = Count::new(0).unwrap();
        count.phase = Phase::Await;
        count.rows = MAX_AGGREGATE_ROWS - 1;
        count
            .step(
                ConsumerInput {
                    batch: &input,
                    finished: false,
                },
                &mut output,
                pipeline,
                &cancel,
            )
            .unwrap();
        assert_eq!(count.rows, MAX_AGGREGATE_ROWS);
        count.phase = Phase::Await;
        assert!(
            matches!(count.step(ConsumerInput { batch: &input, finished: false }, &mut output, pipeline, &cancel), Err(Error::Resource { required, limit, .. }) if required == 134_217_729 && limit == 134_217_728)
        );
        assert!(count.replay(&cancel).is_err());
    }

    #[test]
    fn every_live_counter_phase_cancels_without_publishing_or_replaying() {
        let (_fixture, db) = loaded(1);
        let query = db
            .prepare("FROM lineitem |> SELECT COUNT(*) OVER () AS n")
            .unwrap();
        let plan = lower(&db, &query, RootState::Empty, 0).unwrap();
        let pipeline = &plan.pipelines()[1];
        let input = Batch::new(&[], 0).unwrap();
        let mut output = Batch::new(&[DataType::Int64], crate::batch::MAX_BYTES).unwrap();
        for phase in [Phase::Read, Phase::Await, Phase::Emit] {
            let mut count = Count::new(0).unwrap();
            count.phase = phase;
            count.rows = 3;
            count.remaining = 3;
            let cancel = CancellationToken::new();
            cancel.cancel();
            assert!(matches!(
                count.step(
                    ConsumerInput {
                        batch: &input,
                        finished: false
                    },
                    &mut output,
                    pipeline,
                    &cancel
                ),
                Err(Error::Cancelled)
            ));
            assert!(output.is_empty());
            assert!(count.replay(&CancellationToken::new()).is_err());
            assert!(matches!(
                count.step(
                    ConsumerInput {
                        batch: &input,
                        finished: false
                    },
                    &mut output,
                    pipeline,
                    &CancellationToken::new()
                ),
                Err(Error::Corrupt(_))
            ));
        }
    }
    #[test]
    fn counter_minimum_is_admitted_before_io_and_released_on_completion() {
        use crate::effects::Effects;
        use crate::execution::QueryStep;
        use crate::{ColumnDeclaration, Config, Database};
        let (fixture, legacy) = loaded(1);
        legacy.close().unwrap();
        let db = Database::create_empty(
            &fixture.0.join("native"),
            Config::new(4_000_000, 2_000_000).unwrap(),
        )
        .unwrap();
        let cancel = CancellationToken::new();
        db.declare_table(
            "facts",
            &[ColumnDeclaration {
                name: "v",
                data_type: DataType::Int64,
                nullable: false,
            }],
            &cancel,
        )
        .unwrap();
        db.close().unwrap();
        let db = Database::open(
            &fixture.0.join("native"),
            Config::new(4_000_000, 1).unwrap(),
        )
        .unwrap();
        let query = db
            .prepare("FROM facts |> SELECT COUNT(*) OVER () AS n")
            .unwrap();
        let baseline = db.reserved_memory_bytes();
        let result = db.execute(&query, &cancel).unwrap();
        let minimum = result.accounted_memory_bytes() + crate::catalog::MAX_BYTES as u64;
        drop(result);
        for shortfall in [0, 1] {
            let pressure = db
                .reserve_memory(
                    db.config().memory_limit_bytes() - baseline - minimum + shortfall,
                    "counter minimum test",
                )
                .unwrap();
            let mut effects = Effects::default();
            let attempt = db.execute_with_effects(&query, &cancel, &mut effects);
            if shortfall == 1 {
                assert!(matches!(attempt, Err(Error::Resource { .. })));
                assert_eq!(effects.count(), 0);
            } else {
                let mut result = attempt.unwrap();
                let mut finished = false;
                for _ in 0..100 {
                    assert_eq!(
                        db.reserved_memory_bytes(),
                        baseline + pressure.bytes() + result.accounted_memory_bytes()
                    );
                    assert_eq!(db.reserved_temp_bytes(), 0);
                    match result.step() {
                        QueryStep::Progress => (),
                        QueryStep::Finished => {
                            finished = true;
                            break;
                        }
                        _ => panic!("empty counter must finish without rows"),
                    }
                }
                assert!(finished);
            }
            drop(pressure);
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
    }
}
