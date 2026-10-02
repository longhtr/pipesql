//! Concatenate two UNION ALL inputs, retaining duplicate rows.
//!
//! Read the left input to completion, then the right. UNION matches columns by
//! position, but each input batch may omit values the query does not need. The
//! column map translates a UNION position to its slot in the chosen input batch.
//! Row evaluation then applies later filters and projections to the mapped values.
//!
//! The runtime supplies batches and handles requests to restart an input. This
//! controller remembers which inputs were visited so replay can leave an unused
//! input at its initial state. Errors leave the controller unable to continue or
//! replay; partial work must not become a successful result.

use super::computed::RowValues;
use super::planning::Pipeline;
use super::{BATCH_ROWS, ConsumerInput, SetStep};
use crate::batch::Batch;
use crate::query::{MAX_COLUMNS, SetPlan};
use crate::{CancellationToken, Error};

#[derive(Clone, Copy)]
enum Phase {
    Read,
    Await,
    Done,
    Failed,
}

pub(super) struct Union {
    // For each side, map UNION positions to input batch slots. u8::MAX means the
    // input omitted that value; requesting it exposes an invalid physical plan.
    columns: [[u8; MAX_COLUMNS]; 2],
    side: usize,
    visited: u8,
    replay_pending: u8,
    phase: Phase,
}

impl Union {
    pub(super) fn new(bound: &SetPlan, inputs: [&Pipeline<'_>; 2]) -> Self {
        let mut columns = [[u8::MAX; MAX_COLUMNS]; 2];
        for (side, (slots, input)) in columns.iter_mut().zip(inputs).enumerate() {
            for (position, slot) in slots.iter_mut().enumerate() {
                let Some(pair) = bound.inputs(position) else {
                    break;
                };
                if let Some(position) = input.position(pair[side].identity()) {
                    *slot = position as u8;
                }
            }
        }
        Self {
            columns,
            side: 0,
            visited: 0,
            replay_pending: 0,
            phase: Phase::Read,
        }
    }

    pub(super) fn replay(&mut self) -> Result<(), Error> {
        if matches!(self.phase, Phase::Failed) {
            return Err(Error::Corrupt("failed union cannot replay"));
        }
        self.side = 0;
        self.replay_pending = self.visited;
        self.phase = Phase::Read;
        Ok(())
    }

    pub(super) fn step(
        &mut self,
        inputs: [ConsumerInput<'_>; 2],
        output: &mut Batch,
        pipeline: &Pipeline,
        cancel: &CancellationToken,
    ) -> Result<SetStep, Error> {
        output.clear();
        let phase = std::mem::replace(&mut self.phase, Phase::Failed);
        cancel.check()?;
        match phase {
            Phase::Failed => Err(Error::Corrupt("union has failed")),
            Phase::Done => {
                self.phase = Phase::Done;
                Ok(SetStep::Finished)
            }
            Phase::Read => {
                let bit = 1 << self.side;
                if self.replay_pending & bit != 0 {
                    // A LIMIT may stop during the left input, before an aggregate
                    // in the right input has run. That aggregate is already fresh
                    // and cannot accept a replay of work it has not completed.
                    self.replay_pending &= !bit;
                    self.phase = Phase::Read;
                    return Ok(SetStep::Replay(self.side));
                }
                self.visited |= bit;
                self.phase = Phase::Await;
                Ok(SetStep::Input(self.side))
            }
            Phase::Await => {
                let input = &inputs[self.side];
                if input.finished {
                    if !input.batch.is_empty() {
                        return Err(Error::Corrupt("finished union input contains rows"));
                    }
                    if self.side == 0 {
                        self.side = 1;
                        self.phase = Phase::Read;
                        return Ok(SetStep::Progress);
                    }
                    self.phase = Phase::Done;
                    return Ok(SetStep::Finished);
                }
                if input.batch.is_empty() || input.batch.len() > BATCH_ROWS {
                    return Err(Error::Corrupt("union input batch outside capacity"));
                }
                let mut retained = 0;
                for row in 0..input.batch.len() {
                    let value = |position: u8| {
                        let slot = *self.columns[self.side]
                            .get(usize::from(position))
                            .ok_or(Error::Corrupt("union raw position"))?;
                        input
                            .batch
                            .value(row, usize::from(slot))
                            .ok_or(Error::Corrupt("union demanded input position"))
                    };
                    let mut values = RowValues::new(pipeline, value);
                    if values.retains()? {
                        for (position, slot) in
                            pipeline.columns[..pipeline.column_count].iter().enumerate()
                        {
                            output.set(retained, position, values.value(*slot)?)?;
                        }
                        retained += 1;
                    }
                }
                output.publish_rows(retained);
                self.phase = Phase::Read;
                Ok(if retained == 0 {
                    SetStep::Progress
                } else {
                    SetStep::Rows
                })
            }
        }
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;
    use crate::effects::Effects;
    use crate::execution::{Advance, State};
    use crate::test_support::Directory;
    use crate::{
        AppendLimits, ColumnDeclaration, ColumnInput, ColumnValues, Config, DataType, Database,
        Value,
    };

    #[test]
    fn union_admits_both_branches_before_io_even_for_a_zero_prefix() {
        let directory = Directory::new();
        let path = directory.0.join("database");
        let db = Database::create_empty(&path, Config::new(4_000_000, 2_000_000).unwrap()).unwrap();
        let cancel = CancellationToken::new();
        db.declare_table(
            "facts",
            &[ColumnDeclaration {
                name: "n",
                data_type: DataType::Int64,
                nullable: false,
            }],
            &cancel,
        )
        .unwrap();
        for suffix in ["", " |> LIMIT 0"] {
            let query = db
                .prepare(&format!("FROM facts |> UNION ALL (FROM facts){suffix}"))
                .unwrap();
            let baseline = db.reserved_memory_bytes();
            let result = db.execute(&query, &cancel).unwrap();
            let peak = result.accounted_memory_bytes() + crate::storage::catalog::MAX_BYTES as u64;
            drop(result);
            for shortfall in [0, 1] {
                let pressure = db
                    .reserve_memory(
                        db.config().memory_limit_bytes() - baseline - peak + shortfall,
                        "union admission test",
                    )
                    .unwrap();
                let mut effects = Effects::default();
                let result = db.execute_with_effects(&query, &cancel, &mut effects);
                if shortfall == 1 {
                    assert!(matches!(result, Err(Error::Resource { .. })));
                    assert_eq!(effects.count(), 0, "refusal precedes all source I/O");
                } else {
                    let mut result = result.unwrap();
                    let mut done = false;
                    for _ in 0..100 {
                        assert_eq!(
                            db.reserved_memory_bytes(),
                            baseline + pressure.bytes() + result.accounted_memory_bytes()
                        );
                        match result.step() {
                            crate::QueryStep::Progress => (),
                            crate::QueryStep::Finished => {
                                done = true;
                                break;
                            }
                            _ => panic!("empty union must complete"),
                        }
                    }
                    assert!(done);
                }
                assert_eq!(db.reserved_memory_bytes(), baseline + pressure.bytes());
                drop(pressure);
                assert_eq!(db.reserved_memory_bytes(), baseline);
                assert_eq!(db.reserved_temp_bytes(), 0);
            }
        }
        db.close().unwrap();
    }

    #[test]
    fn replay_resets_visited_branches_and_leaves_unvisited_aggregates_fresh() {
        let directory = Directory::new();
        let path = directory.0.join("database");
        let db = Database::create_empty(&path, Config::new(4_000_000, 2_000_000).unwrap()).unwrap();
        let cancel = CancellationToken::new();
        db.declare_table(
            "facts",
            &[ColumnDeclaration {
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
                AppendLimits {
                    batches: 1,
                    encoded_bytes: 4096,
                },
                &cancel,
            )
            .unwrap();
        writer
            .write(
                &[ColumnInput {
                    values: ColumnValues::Int64(&[1, 2]),
                    validity: &[3],
                }],
                &cancel,
            )
            .unwrap();
        writer.commit(&cancel).unwrap();
        let baseline = db.reserved_memory_bytes();
        for (mode, suffix, expected) in [
            ("ALL", "", vec![1, 2, 2]),
            ("ALL", " |> LIMIT 1", vec![1]),
            ("ALL", " |> UNION ALL (FROM facts)", vec![1, 2, 2, 1, 2]),
            ("DISTINCT", "", vec![1, 2]),
            ("DISTINCT", " |> LIMIT 1", vec![1]),
        ] {
            let query = db
                .prepare(&format!(
                    "FROM facts |> UNION {mode} (FROM facts |> AGGREGATE COUNT(*) AS n){suffix}"
                ))
                .unwrap();
            for complete in [false, true] {
                let mut result = db.execute(&query, &cancel).unwrap();
                let State::Running(runtime) = &mut result.state else {
                    panic!("running union")
                };
                let mut effects = Effects::default();
                for pass in 0..2 {
                    if pass == 1 {
                        runtime.replay_output_for_test();
                    }
                    let mut rows = Vec::new();
                    let mut reached = false;
                    for _ in 0..1024 {
                        match runtime.step(&result.plan, &cancel, &mut effects).unwrap() {
                            Advance::Progress => (),
                            Advance::Rows => {
                                for row in 0..runtime.output().len() {
                                    let Some(Value::Int64(value)) = runtime.output().value(row, 0)
                                    else {
                                        panic!("integer union output")
                                    };
                                    rows.push(value);
                                }
                                if pass == 0 && !complete {
                                    reached = true;
                                    break;
                                }
                            }
                            Advance::Finished => {
                                reached = true;
                                break;
                            }
                        }
                    }
                    assert!(reached, "bounded union replay");
                    if pass == 1 || complete {
                        assert_eq!(rows, expected, "{suffix}");
                    } else {
                        assert!(expected.starts_with(&rows));
                    }
                }
            }
        }
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
        db.close().unwrap();
    }
}
