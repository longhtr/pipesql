use super::*;
use crate::Value;
use crate::effects::Faults;
use crate::execution::blocking::test_support::{Directory, two_column_database};
use crate::execution::blocking::{Files, RECORD_HEADER};
use crate::execution::{QueryResult, QueryStep, State};

// Repeated keys occupy distinct comparison positions but share a source slot.
// Each 50-byte record fills the 8,720-byte first run after 174 rows.
// The 180 left and 176 right rows both spill; only keys 88 and 89 survive.
const QUERY: &str = "FROM facts |> SELECT k AS a, k AS b \
    |> EXCEPT DISTINCT (FROM facts |> WHERE k<88 |> SELECT k, k)";
const STEPS: usize = 100_000;

fn except<'a, 'db>(result: &'a mut QueryResult<'db, '_>) -> &'a mut Except<'db> {
    let State::Running(runtime) = &mut result.state else {
        panic!("live EXCEPT");
    };
    runtime.first_except_mut()
}

fn phase_index(phase: Phase) -> usize {
    match phase {
        Phase::Create(side) => side * 7,
        Phase::Read(side) => side * 7 + 1,
        Phase::Await(side) => side * 7 + 2,
        Phase::Capture(side, _) => side * 7 + 3,
        Phase::Push(side, _) => side * 7 + 4,
        Phase::Spill(side, _) => side * 7 + 5,
        Phase::Sort(side) => side * 7 + 6,
        Phase::Seek => 14,
        Phase::Emit => 15,
        Phase::Done => 16,
        Phase::Failed => panic!("healthy EXCEPT phase"),
    }
}

// Sum actual capacities rather than repeating constructor admission formulas.
fn check_physical_account(except: &Except<'_>) {
    fn bytes<T>(v: &Vec<T>) -> usize {
        v.capacity() * size_of::<T>()
    }
    let mut physical = size_of::<Except<'_>>();
    let mut creation = 0;
    for side in &except.sides {
        let run = &side.sort.buffer;
        let merge = &side.sort.merge;
        physical += bytes(&side.record.bytes)
            + bytes(&run.bytes)
            + bytes(&run.spans)
            + bytes(&run.work)
            + merge.writer.allocated_bytes()
            + bytes(&merge.pair.previous_key)
            + bytes(&merge.pair.left.record.bytes)
            + bytes(&merge.pair.right.record.bytes)
            + merge.pair.left.reader.allocated_bytes()
            + merge.pair.right.reader.allocated_bytes();
        if matches!(side.files, Files::Pending(_)) {
            creation += crate::scratch::Creation::memory_requirement_bytes();
        }
    }
    assert_eq!(physical as u64 + creation, except.memory_bytes());
}

fn collect(result: &mut QueryResult<'_, '_>, effects: &mut Effects) -> Vec<(i64, i64)> {
    let mut rows = Vec::new();
    for _ in 0..STEPS {
        check_physical_account(except(result));
        match result.step_with_effects(effects) {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let (Some(Value::Int64(a)), Some(Value::Int64(b))) =
                        (batch.value(row, 0), batch.value(row, 1))
                    else {
                        panic!("two required integer positions");
                    };
                    rows.push((a, b));
                }
            }
            QueryStep::Finished => {
                rows.sort_unstable();
                return rows;
            }
            QueryStep::Failed(error) => panic!("EXCEPT failed: {error}"),
        }
    }
    panic!("EXCEPT exceeded fixture work bound");
}

#[test]
fn exact_admission_spill_and_replay_reconcile_owned_capacities() {
    let directory = Directory::new();
    let db = two_column_database(&directory);
    let cancel = CancellationToken::new();
    let query = db.prepare(QUERY).unwrap();
    let baseline = db.reserved_memory_bytes();
    let result = db.execute(&query, &cancel).unwrap();
    let peak = result.accounted_memory_bytes() + crate::catalog::MAX_BYTES as u64;
    drop(result);
    for shortfall in [0, 1] {
        let pressure = db
            .reserve_memory(
                db.config().memory_limit_bytes() - baseline - peak + shortfall,
                "EXCEPT minimum test",
            )
            .unwrap();
        let mut effects = Effects::default();
        let admitted = db.execute_with_effects(&query, &cancel, &mut effects);
        if shortfall == 1 {
            assert!(matches!(admitted, Err(Error::Resource { .. })));
            assert_eq!(effects.count(), 0);
        } else {
            let mut result = admitted.unwrap();
            let expected: Vec<_> = (88..90).map(|key| (key, key)).collect();
            assert_eq!(collect(&mut result, &mut effects), expected);
        }
        drop(pressure);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }

    let mut result = db.execute(&query, &cancel).unwrap();
    let mut effects = Effects::default();
    // Stop after one output row while the source-backed sorted inputs remain.
    for step in 0..STEPS {
        if matches!(result.step_with_effects(&mut effects), QueryStep::Rows(_)) {
            break;
        }
        assert!(step + 1 < STEPS, "EXCEPT did not emit");
    }
    let owner = except(&mut result);
    assert!(owner.sides.iter().all(|side| side.ordinal > 0));
    owner.replay(&cancel).unwrap();
    assert!(matches!(owner.replay(&cancel), Err(Error::Corrupt(_))));
    assert_eq!(
        collect(&mut result, &mut effects),
        (88..90).map(|key| (key, key)).collect::<Vec<_>>()
    );
    drop(result);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
}

#[test]
fn cancellation_reaches_every_input_sort_merge_and_emission_phase() {
    let directory = Directory::new();
    let db = two_column_database(&directory);
    let query = db.prepare(QUERY).unwrap();
    let baseline = db.reserved_memory_bytes();
    for target in 0..17 {
        let cancel = CancellationToken::new();
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut effects = Effects::default();
        let mut reached = false;
        for _ in 0..STEPS {
            if phase_index(except(&mut result).phase) == target {
                cancel.cancel();
                let before = effects.count();
                for _ in 0..2 {
                    let step = result.step_with_effects(&mut effects);
                    if target == 16 {
                        assert!(matches!(step, QueryStep::Finished));
                    } else {
                        assert!(
                            matches!(step, QueryStep::Failed(Error::Cancelled)),
                            "phase {target}"
                        );
                    }
                }
                assert_eq!(effects.count(), before);
                reached = true;
                break;
            }
            let step = result.step_with_effects(&mut effects);
            assert!(
                matches!(step, QueryStep::Progress | QueryStep::Rows(_)),
                "phase {target} was not reached"
            );
        }
        assert!(reached, "phase {target}");
        drop(result);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}

#[test]
fn both_sorted_inputs_reject_corruption_truncation_and_read_failure() {
    let directory = Directory::new();
    let db = two_column_database(&directory);
    let query = db.prepare(QUERY).unwrap();
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    for side in 0..2 {
        for fault in 0..3 {
            let mut result = db.execute(&query, &cancel).unwrap();
            let mut effects = Effects::default();
            let mut reached = false;
            for _ in 0..STEPS {
                if matches!(except(&mut result).phase, Phase::Seek) {
                    reached = true;
                    break;
                }
                assert!(matches!(
                    result.step_with_effects(&mut effects),
                    QueryStep::Progress
                ));
            }
            assert!(reached);
            if side == 1 && fault == 2 {
                // The first Seek quantum loads only the left record. Fail the
                // following I/O to exercise the right reader independently.
                assert!(matches!(
                    result.step_with_effects(&mut effects),
                    QueryStep::Progress
                ));
                let owner = except(&mut result);
                assert!(owner.sides[0].sort.sorted_cursor().record().is_some());
                assert!(owner.sides[1].sort.sorted_cursor().record().is_none());
            }
            let input = &mut except(&mut result).sides[side];
            let slot = input.sort.merge.input.slot;
            let offset = input.sort.merge.result.unwrap().start;
            let Files::Open(scratch) = &mut input.files else {
                panic!("sorted scratch input");
            };
            match fault {
                0 => scratch
                    .write(
                        slot,
                        offset + RECORD_HEADER as u64 + 10,
                        &[255],
                        &cancel,
                        &mut effects,
                    )
                    .unwrap(),
                1 => scratch.reset(slot, &cancel, &mut effects).unwrap(),
                2 => {
                    effects = Effects::with_faults(Faults {
                        fail_at: Some(0),
                        ..Faults::default()
                    })
                }
                _ => unreachable!(),
            }
            let mut failed = false;
            for _ in 0..STEPS {
                match result.step_with_effects(&mut effects) {
                    QueryStep::Progress => (),
                    QueryStep::Failed(error) => {
                        if fault == 2 {
                            assert!(matches!(error, Error::Io { .. }));
                        } else {
                            assert!(matches!(error, Error::Corrupt(_)));
                        }
                        failed = true;
                        break;
                    }
                    _ => panic!("corrupt EXCEPT input emitted or completed"),
                }
            }
            assert!(failed);
            let before = effects.count();
            assert!(matches!(
                result.step_with_effects(&mut effects),
                QueryStep::Failed(_)
            ));
            assert_eq!(effects.count(), before);
            drop(result);
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
    }
    let mut result = db.execute(&query, &cancel).unwrap();
    assert_eq!(collect(&mut result, &mut Effects::default()).len(), 2);
}

#[test]
fn temporary_refusal_is_terminal_and_allows_healthy_reuse() {
    let directory = Directory::new();
    let db = two_column_database(&directory);
    let query = db.prepare(QUERY).unwrap();
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let mut result = db.execute(&query, &cancel).unwrap();
    db.temporary
        .reserve(db.config().temp_limit_bytes())
        .unwrap();
    let mut effects = Effects::default();
    let mut failed = false;
    for _ in 0..STEPS {
        match result.step_with_effects(&mut effects) {
            QueryStep::Progress => (),
            QueryStep::Failed(Error::Resource { .. }) => {
                failed = true;
                break;
            }
            _ => panic!("EXCEPT must refuse exhausted temporary storage"),
        }
    }
    assert!(failed);
    let before = effects.count();
    assert!(matches!(
        result.step_with_effects(&mut effects),
        QueryStep::Failed(Error::Resource { .. })
    ));
    assert_eq!(effects.count(), before);
    drop(result);
    db.temporary.release(db.config().temp_limit_bytes());
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    let mut healthy = db.execute(&query, &cancel).unwrap();
    assert_eq!(
        collect(&mut healthy, &mut Effects::default()),
        vec![(88, 88), (89, 89)]
    );
    drop(healthy);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
}
