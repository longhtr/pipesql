//! Prefix consumption owns counters and an output batch, never a child cursor.
use crate::batch::Batch;
use crate::execution::computed::RowValues;
use crate::execution::planning::Pipeline;
use crate::execution::{BATCH_ROWS, ConsumerInput, ConsumerStep};
use crate::frontend;
use crate::{CancellationToken, Error};

#[derive(Clone, Copy)]
enum Phase {
    Read,
    Await,
    Done,
    Failed,
}

pub(super) struct Limit {
    bounds: frontend::LimitBounds,
    remaining: u64,
    skip: u64,
    phase: Phase,
}

impl Limit {
    pub(super) fn new(bounds: frontend::LimitBounds) -> Self {
        Self {
            bounds,
            remaining: bounds.count(),
            skip: bounds.offset(),
            phase: Phase::Read,
        }
    }

    pub(super) fn replay(&mut self) -> Result<(), Error> {
        if matches!(self.phase, Phase::Failed) {
            return Err(Error::Corrupt("failed limit cannot replay"));
        }
        // The scheduler also rewinds the child; resetting these counters alone
        // would select a different prefix from an advanced input.
        *self = Self::new(self.bounds);
        Ok(())
    }

    pub(super) fn step(
        &mut self,
        input: ConsumerInput<'_>,
        output: &mut Batch,
        plan: &Pipeline,
        cancel: &CancellationToken,
    ) -> Result<ConsumerStep, Error> {
        output.clear();
        let phase = std::mem::replace(&mut self.phase, Phase::Failed);
        cancel.check()?;
        match phase {
            Phase::Failed => Err(Error::Corrupt("limit has failed")),
            Phase::Done => {
                self.phase = Phase::Done;
                Ok(ConsumerStep::Finished)
            }
            Phase::Read => {
                // Offset is consumed even when count is zero. Neither value is
                // added to the other, including at the INT64 boundary.
                if self.skip == 0 && self.remaining == 0 {
                    self.phase = Phase::Done;
                    Ok(ConsumerStep::Finished)
                } else {
                    self.phase = Phase::Await;
                    Ok(ConsumerStep::Input)
                }
            }
            Phase::Await => {
                if input.finished {
                    if !input.batch.is_empty() {
                        return Err(Error::Corrupt("finished limit input contains rows"));
                    }
                    self.phase = Phase::Done;
                    return Ok(ConsumerStep::Finished);
                }
                let rows = input.batch.len();
                if rows == 0 || rows > BATCH_ROWS {
                    return Err(Error::Corrupt("limit input batch outside capacity"));
                }
                let skipped = self.skip.min(rows as u64) as usize;
                self.skip -= skipped as u64;
                let taken = self.remaining.min((rows - skipped) as u64) as usize;
                // Count candidate rows before post-LIMIT filters. A rejected
                // row cannot extend the quota. One step touches at most one
                // admitted batch, including its bounded per-column text bytes.
                self.remaining -= taken as u64;
                let mut retained = 0;
                for row in skipped..skipped + taken {
                    let value = |column: u8| {
                        input
                            .batch
                            .value(row, usize::from(column))
                            .ok_or(Error::Corrupt("limit input position"))
                    };
                    let mut values = RowValues::new(plan, value);
                    let keep = values.retains()?;
                    if keep {
                        for (position, column) in
                            plan.columns[..plan.column_count].iter().enumerate()
                        {
                            output.set(retained, position, values.value(*column)?)?;
                        }
                        retained += 1;
                    }
                }
                output.publish_rows(retained);
                self.phase = Phase::Read;
                Ok(if retained == 0 {
                    ConsumerStep::Progress
                } else {
                    ConsumerStep::Rows
                })
            }
        }
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;
    use crate::effects::Effects;
    use crate::effects::Faults;
    use crate::execution::planning::lower;
    use crate::execution::{QueryStep, planning};
    use crate::storage_format::RootState;
    use crate::{DataType, Database, Value};

    struct Directory(std::path::PathBuf);

    impl Drop for Directory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn database(name: &str, declared: bool) -> (Directory, Database) {
        let directory = Directory(
            std::env::temp_dir().join(format!("pipesql-limit-{name}-{}", std::process::id())),
        );
        let config = crate::Config::new(2_000_000, 1_000_000).unwrap();
        let db = if declared {
            Database::create_empty(&directory.0, config)
        } else {
            Database::create(&directory.0, config)
        }
        .unwrap();
        (directory, db)
    }

    #[test]
    fn every_limit_phase_cancels_and_failed_state_cannot_replay() {
        let (_directory, db) = database("phases", false);
        let query = db
            .prepare("FROM lineitem |> SELECT l_quantity |> LIMIT 2 OFFSET 1")
            .unwrap();
        let plan = lower(&db, &query, RootState::Empty, 0).unwrap();
        let pipeline = &plan.pipelines()[1];
        let planning::Producer::Limit { bounds, .. } = pipeline.producer else {
            unreachable!();
        };
        let mut input = Batch::new(&[DataType::Double], crate::batch::MAX_BYTES).unwrap();
        let mut output = Batch::new(&[DataType::Double], crate::batch::MAX_BYTES).unwrap();
        for row in 0..4 {
            input.set(row, 0, Value::Double(row as f64)).unwrap();
        }
        input.publish_rows(4);
        for phase in [Phase::Read, Phase::Await, Phase::Done] {
            let mut limit = Limit::new(bounds);
            limit.phase = phase;
            let cancel = CancellationToken::new();
            cancel.cancel();
            assert!(matches!(
                limit.step(
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
            assert!(limit.replay().is_err());
            assert!(matches!(
                limit.step(
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
        let mut limit = Limit::new(bounds);
        limit.phase = Phase::Await;
        assert!(
            limit
                .step(
                    ConsumerInput {
                        batch: &input,
                        finished: true
                    },
                    &mut output,
                    pipeline,
                    &CancellationToken::new()
                )
                .is_err()
        );
        assert!(limit.replay().is_err());
    }

    #[test]
    fn limit_stops_unrequested_reads_but_propagates_admission_and_input_failures() {
        let (_directory, db) = database("io", true);
        let cancel = CancellationToken::new();
        db.declare_table(
            "facts",
            &[crate::ColumnDeclaration {
                name: "n",
                data_type: DataType::Int64,
                nullable: false,
            }],
            &cancel,
        )
        .unwrap();
        let mut writer = db
            .begin_append(
                "facts",
                crate::AppendLimits {
                    batches: 1,
                    encoded_bytes: 1024,
                },
                &cancel,
            )
            .unwrap();
        writer
            .write(
                &[crate::ColumnInput {
                    values: crate::ColumnValues::Int64(&[1, 2]),
                    validity: &[3],
                }],
                &cancel,
            )
            .unwrap();
        writer.commit(&cancel).unwrap();
        for (count, offset, reads) in [(0, 0, false), (1, 0, true), (0, 1, true)] {
            let query = db
                .prepare(&format!("FROM facts |> LIMIT {count} OFFSET {offset}"))
                .unwrap();
            let baseline = db.reserved_memory_bytes();
            let mut effects = Effects::with_faults(Faults {
                fail_at: Some(0),
                ..Faults::default()
            });
            assert!(matches!(
                db.execute_with_effects(&query, &cancel, &mut effects),
                Err(Error::Io { .. })
            ));
            assert_eq!(db.reserved_memory_bytes(), baseline);
            let mut effects = Effects::default();
            let mut result = db
                .execute_with_effects(&query, &cancel, &mut effects)
                .unwrap();
            let admitted = effects.count();
            effects.fail_at(admitted);
            let mut terminal = false;
            for _ in 0..100 {
                match result.step_with_effects(&mut effects) {
                    QueryStep::Progress => (),
                    QueryStep::Finished => {
                        assert!(!reads);
                        terminal = true;
                        break;
                    }
                    QueryStep::Failed(Error::Io { .. }) => {
                        assert!(reads);
                        terminal = true;
                        break;
                    }
                    _ => panic!("LIMIT input failure must precede output"),
                }
            }
            assert!(terminal);
            assert_eq!(effects.count(), admitted + u64::from(reads));
            drop(result);
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
    }

    #[test]
    fn limit_minimum_is_admitted_before_io_and_every_owner_reconciles() {
        let (_directory, db) = database("minimum", true);
        let cancel = CancellationToken::new();
        db.declare_table(
            "facts",
            &[crate::ColumnDeclaration {
                name: "n",
                data_type: DataType::Int64,
                nullable: false,
            }],
            &cancel,
        )
        .unwrap();
        let query = db
            .prepare("FROM facts |> LIMIT 1 |> LIMIT 0 OFFSET 1")
            .unwrap();
        let baseline = db.reserved_memory_bytes();
        let result = db.execute(&query, &cancel).unwrap();
        let peak = result.accounted_memory_bytes() + crate::catalog::MAX_BYTES as u64;
        drop(result);
        for shortfall in [0, 1] {
            let pressure = db
                .reserve_memory(
                    db.config().memory_limit_bytes() - baseline - peak + shortfall,
                    "limit minimum test",
                )
                .unwrap();
            let mut effects = Effects::default();
            let result = db.execute_with_effects(&query, &cancel, &mut effects);
            if shortfall != 0 {
                assert!(matches!(result, Err(Error::Resource { .. })));
                assert_eq!(effects.count(), 0);
            } else {
                let mut result = result.unwrap();
                let mut done = false;
                for _ in 0..100 {
                    assert_eq!(
                        db.reserved_memory_bytes(),
                        baseline + pressure.bytes() + result.accounted_memory_bytes()
                    );
                    match result.step() {
                        QueryStep::Progress => (),
                        QueryStep::Finished => {
                            done = true;
                            break;
                        }
                        _ => panic!("empty admitted limit must finish"),
                    }
                }
                assert!(done);
                assert!(!cancel.is_cancelled());
            }
            drop(pressure);
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
    }
}
