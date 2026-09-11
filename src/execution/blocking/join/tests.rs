use super::*;
use crate::effects::Effect;
use crate::effects::Faults;
use crate::execution::blocking::test_support::Directory;
use crate::execution::blocking::{Files, RECORD_HEADER};
use crate::execution::{QueryResult, QueryStep, State};
use crate::frontend::DataType;
use crate::{AppendLimits, ColumnDeclaration, ColumnInput, ColumnValues, Config};

const QUERY: &str = "FROM facts AS l |> JOIN facts AS r ON l.k = r.k |> SELECT l.v,r.v";
const STEPS: usize = 100_000;

fn database(directory: &Directory) -> Database {
    let db = Database::create_empty(
        &directory.0.join("db"),
        Config::new(8_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "facts",
        &["k", "v"].map(|name| ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: false,
        }),
        &cancel,
    )
    .unwrap();
    // 180 rows exceed the join's byte-limited first run. Adjacent equal keys
    // cross run boundaries; reverse input order cannot masquerade as a merge.
    for start in [0, 90] {
        let values: Vec<i64> = (start..start + 90).rev().collect();
        let keys: Vec<_> = values.iter().map(|v| v / 2).collect();
        let mut valid = [255; 12];
        valid[11] = 3;
        let mut append = db
            .begin_append(
                "facts",
                AppendLimits {
                    batches: 1,
                    encoded_bytes: 10_000,
                },
                &cancel,
            )
            .unwrap();
        append
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&keys),
                        validity: &valid,
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&values),
                        validity: &valid,
                    },
                ],
                &cancel,
            )
            .unwrap();
        append.commit(&cancel).unwrap();
    }
    db
}

fn join<'a, 'db>(result: &'a mut QueryResult<'db, '_>) -> &'a mut Join<'db> {
    let State::Running(runtime) = &mut result.state else {
        panic!("live join");
    };
    runtime.first_join_mut()
}

fn check_physical_account(join: &Join<'_>) {
    fn bytes<T>(v: &Vec<T>) -> usize {
        v.capacity() * size_of::<T>()
    }
    let mut physical = size_of::<Join<'_>>();
    let mut creation = 0;
    for side in &join.sides {
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
    assert_eq!(
        physical as u64 + creation,
        join.memory_bytes(),
        "inline owner and actual allocation capacities are charged once"
    );
}

fn phase_index(phase: Phase) -> usize {
    match phase {
        Phase::Create(s) => s * 7,
        Phase::Read(s) => s * 7 + 1,
        Phase::Await(s) => s * 7 + 2,
        Phase::Capture(s, _) => s * 7 + 3,
        Phase::Push(s, _) => s * 7 + 4,
        Phase::Spill(s, _) => s * 7 + 5,
        Phase::Sort(s) => s * 7 + 6,
        Phase::Seek => 14,
        Phase::Emit => 15,
        Phase::Right => 16,
        Phase::Left => 17,
        Phase::Done => 18,
        Phase::Failed => panic!("healthy phase"),
    }
}

fn expect_failure(result: &mut QueryResult<'_, '_>, effects: &mut Effects, fault: usize) {
    for _ in 0..STEPS {
        match result.step_with_effects(effects) {
            QueryStep::Rows(_) | QueryStep::Progress => (),
            QueryStep::Finished => panic!("fault {fault} did not fail"),
            QueryStep::Failed(error) => {
                match fault {
                    0 | 1 | 3 => assert!(matches!(error, Error::Corrupt(_)), "{error}"),
                    4 => assert!(matches!(error, Error::Resource { .. }), "{error}"),
                    _ => assert!(matches!(error, Error::Io { .. }), "{error}"),
                }
                return;
            }
        }
    }
    panic!("join did not fail within fixture work bound");
}

#[test]
fn join_exact_admission_precedes_io_and_reconciles_each_transition() {
    let directory = Directory::new();
    let db = database(&directory);
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
                "join minimum test",
            )
            .unwrap();
        let mut effects = Effects::default();
        let admitted = db.execute_with_effects(&query, &cancel, &mut effects);
        if shortfall == 1 {
            assert!(matches!(admitted, Err(Error::Resource { .. })));
            assert_eq!(effects.count(), 0);
        } else {
            let mut result = admitted.unwrap();
            let expected: Vec<_> = (0..180)
                .flat_map(|l| {
                    (0..180)
                        .filter(move |r| l / 2 == r / 2)
                        .map(move |r| (l, r))
                })
                .collect();
            let mut observed = vec![];
            let mut done = false;
            for _ in 0..STEPS {
                check_physical_account(join(&mut result));
                assert_eq!(
                    db.reserved_memory_bytes(),
                    baseline + pressure.bytes() + result.accounted_memory_bytes()
                );
                match result.step_with_effects(&mut effects) {
                    QueryStep::Rows(batch) => {
                        for row in 0..batch.len() {
                            let (Some(Value::Int64(l)), Some(Value::Int64(r))) =
                                (batch.value(row, 0), batch.value(row, 1))
                            else {
                                panic!("pair");
                            };
                            observed.push((l, r));
                        }
                    }
                    QueryStep::Finished => {
                        done = true;
                        break;
                    }
                    QueryStep::Progress => (),
                    QueryStep::Failed(error) => panic!("exact minimum: {error}"),
                }
            }
            assert!(done);
            observed.sort_unstable();
            assert_eq!(observed, expected);
        }
        drop(pressure);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}

#[test]
fn cancellation_covers_both_inputs_sort_matching_and_duplicate_rewind() {
    let directory = Directory::new();
    let db = database(&directory);
    let query = db.prepare(QUERY).unwrap();
    let baseline = db.reserved_memory_bytes();
    for target in 0..19 {
        let cancel = CancellationToken::new();
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut effects = Effects::default();
        let mut reached = false;
        for _ in 0..STEPS {
            if phase_index(join(&mut result).phase) == target {
                cancel.cancel();
                let before = effects.count();
                for _ in 0..2 {
                    let step = result.step_with_effects(&mut effects);
                    if target == 18 {
                        assert!(
                            matches!(step, QueryStep::Finished),
                            "completed join is terminal"
                        );
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
            assert!(
                matches!(
                    result.step_with_effects(&mut effects),
                    QueryStep::Progress | QueryStep::Rows(_)
                ),
                "phase {target}"
            );
        }
        assert!(reached, "phase {target}");
        drop(result);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}

#[test]
fn sorted_input_corruption_truncation_read_failure_and_temp_refusal_are_terminal() {
    let directory = Directory::new();
    let db = database(&directory);
    let query = db.prepare(QUERY).unwrap();
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    for fault in 0..5 {
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut effects = Effects::default();
        let target = if fault == 3 { 17 } else { 14 };
        if fault != 4 {
            let mut reached = false;
            for _ in 0..STEPS {
                if phase_index(join(&mut result).phase) == target {
                    reached = true;
                    break;
                }
                assert!(matches!(
                    result.step_with_effects(&mut effects),
                    QueryStep::Progress | QueryStep::Rows(_)
                ));
            }
            assert!(reached);
            let join = join(&mut result);
            let side = if fault == 3 { 1 } else { 0 };
            let input = &mut join.sides[side];
            let slot = input.sort.merge.input.slot;
            let offset = if fault == 3 {
                join.group.unwrap().offset
            } else {
                input.sort.merge.result.unwrap().start
            };
            let Files::Open(scratch) = &mut input.files else {
                panic!("sorted files");
            };
            match fault {
                0 | 3 => scratch
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
        } else {
            db.temporary
                .reserve(db.config().temp_limit_bytes())
                .unwrap();
        }
        expect_failure(&mut result, &mut effects, fault);
        let before = effects.count();
        assert!(matches!(
            result.step_with_effects(&mut effects),
            QueryStep::Failed(_)
        ));
        assert_eq!(effects.count(), before);
        drop(result);
        if fault == 4 {
            db.temporary.release(db.config().temp_limit_bytes());
        }
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}

#[test]
fn second_input_bootstrap_contention_releases_the_retained_first_input() {
    let directory = Directory::new();
    let db = database(&directory);
    let cancel = CancellationToken::new();
    let query = db.prepare(QUERY).unwrap();
    let baseline = db.reserved_memory_bytes();
    let mut result = db.execute(&query, &cancel).unwrap();
    let mut reached = false;
    for _ in 0..STEPS {
        if matches!(join(&mut result).phase, Phase::Create(1)) {
            reached = true;
            break;
        }
        assert!(matches!(result.step(), QueryStep::Progress));
    }
    assert!(reached && db.reserved_temp_bytes() > 0);
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(1);
    let timeout = std::time::Duration::from_secs(30);
    std::thread::scope(|scope| {
        let database = &db;
        let creator = scope.spawn(move || {
            let mut effects = Effects::with_faults(Faults {
                action: Some(Box::new(move |index, _| {
                    if index == 0 {
                        ready_tx.send(()).unwrap();
                        resume_rx.recv_timeout(timeout).unwrap();
                    }
                })),
                ..Faults::default()
            });
            let scratch =
                crate::scratch::Scratch::new(database, &CancellationToken::new(), &mut effects)
                    .unwrap();
            drop(scratch);
        });
        ready_rx.recv_timeout(timeout).unwrap();
        let mut effects = Effects::default();
        assert!(matches!(
            result.step_with_effects(&mut effects),
            QueryStep::Failed(Error::Contention("scratch bootstrap"))
        ));
        assert_eq!(effects.count(), 0, "contender makes no namespace effects");
        assert_eq!(db.reserved_temp_bytes(), 0, "first input was released");
        resume_tx.send(()).unwrap();
        creator.join().unwrap();
    });
    drop(result);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
}

#[test]
fn short_matching_reads_and_second_input_writes_are_terminal() {
    let directory = Directory::new();
    let db = database(&directory);
    let cancel = CancellationToken::new();
    let query = db.prepare(QUERY).unwrap();
    let baseline = db.reserved_memory_bytes();
    for point in 0..3 {
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut reached = false;
        for _ in 0..STEPS {
            let join = join(&mut result);
            let target = match point {
                0 => matches!(join.phase, Phase::Seek),
                1 => matches!(join.phase, Phase::Left),
                2 => {
                    matches!(join.phase, Phase::Sort(1))
                        && join.sides[1].sort.phase == SortPhase::Flush
                }
                _ => unreachable!(),
            };
            if target {
                if point == 2 {
                    assert!(!join.sides[1].sort.merge.writer.is_empty());
                }
                reached = true;
                break;
            }
            assert!(matches!(
                result.step(),
                QueryStep::Progress | QueryStep::Rows(_)
            ));
        }
        assert!(reached);
        assert!(db.reserved_temp_bytes() > 0);
        let mut effects = Effects::with_faults(Faults {
            short_at: Some(0),
            action: Some(Box::new(move |index, effect| {
                if index == 0 {
                    assert_eq!(
                        effect,
                        Effect::Load(if point == 2 {
                            crate::effects::LoadEffect::WriteStaging
                        } else {
                            crate::effects::LoadEffect::ReadStaging
                        })
                    );
                }
            })),
            ..Faults::default()
        });
        expect_failure(&mut result, &mut effects, 2);
        let before = effects.count();
        assert!(matches!(
            result.step_with_effects(&mut effects),
            QueryStep::Failed(Error::Io { .. })
        ));
        assert_eq!(effects.count(), before);
        drop(result);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert!(
            !db.needs_reopen(),
            "unlinked payload I/O does not create namespace debt"
        );
    }
}

#[test]
fn second_input_namespace_failures_retain_honest_debt_and_heal() {
    use std::{cell::RefCell, rc::Rc};
    let directory = Directory::new();
    let db = database(&directory);
    let cancel = CancellationToken::new();
    let query = db.prepare(QUERY).unwrap();
    let mut result = db.execute(&query, &cancel).unwrap();
    let mut reached = false;
    for _ in 0..STEPS {
        if matches!(join(&mut result).phase, Phase::Create(1)) {
            reached = true;
            break;
        }
        assert!(matches!(result.step(), QueryStep::Progress));
    }
    assert!(reached);
    let trace = Rc::new(RefCell::new(Vec::with_capacity(64)));
    let recorded = Rc::clone(&trace);
    let mut effects = Effects::with_faults(Faults {
        action: Some(Box::new(move |_, effect| {
            let mut events = recorded.borrow_mut();
            assert!(events.len() < 64);
            events.push(effect);
        })),
        ..Faults::default()
    });
    assert!(matches!(
        result.step_with_effects(&mut effects),
        QueryStep::Progress
    ));
    let cuts: Vec<_> = trace
        .borrow()
        .iter()
        .enumerate()
        .filter_map(|(index, effect)| {
            matches!(
                effect,
                Effect::Load(
                    crate::effects::LoadEffect::CreateStaging
                        | crate::effects::LoadEffect::RemoveStaging
                ) | Effect::SyncDirectory(crate::effects::DirectoryKind::Private)
            )
            .then_some(index as u64)
        })
        .collect();
    assert_eq!(cuts.len(), 5, "two creates, two unlinks and their barrier");
    drop(result);
    drop(query);
    db.close().unwrap();
    for cut in cuts {
        let directory = Directory::new();
        let db = database(&directory);
        let query = db.prepare(QUERY).unwrap();
        let baseline = db.reserved_memory_bytes();
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut reached = false;
        for _ in 0..STEPS {
            if matches!(join(&mut result).phase, Phase::Create(1)) {
                reached = true;
                break;
            }
            assert!(matches!(result.step(), QueryStep::Progress));
        }
        assert!(reached && db.reserved_temp_bytes() > 0);
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(cut),
            ..Faults::default()
        });
        assert!(
            matches!(
                result.step_with_effects(&mut effects),
                QueryStep::Failed(Error::RecoveryRequired { .. })
            ),
            "effect {cut}"
        );
        assert!(
            !db.needs_reopen(),
            "scratch debt has no publication authority"
        );
        assert_eq!(db.reserved_temp_bytes(), 0);
        let before = effects.count();
        assert!(matches!(
            result.step_with_effects(&mut effects),
            QueryStep::Failed(Error::RecoveryRequired { .. })
        ));
        assert_eq!(effects.count(), before);
        let mut retry = db.execute(&query, &cancel).unwrap();
        let mut retry_effects = Effects::default();
        assert!(matches!(
            retry.step_with_effects(&mut retry_effects),
            QueryStep::Failed(Error::RecoveryRequired { .. })
        ));
        assert_eq!(
            retry_effects.count(),
            0,
            "bootstrap debt refuses without live repair"
        );
        drop(retry);
        let scan = db.prepare("FROM facts |> AGGREGATE COUNT(*) AS n").unwrap();
        let mut scan_result = db.execute(&scan, &cancel).unwrap();
        let mut rows = 0;
        let mut finished = false;
        for _ in 0..1000 {
            match scan_result.step() {
                QueryStep::Rows(batch) => {
                    assert_eq!(batch.len(), 1);
                    assert_eq!(batch.value(0, 0), Some(Value::Int64(180)));
                    rows += 1;
                }
                QueryStep::Progress => (),
                QueryStep::Finished => {
                    finished = true;
                    break;
                }
                QueryStep::Failed(error) => {
                    panic!("scratch-free query after effect {cut}: {error}")
                }
            }
        }
        assert!(finished && rows == 1);
        drop(scan_result);
        drop(scan);
        drop(result);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        drop(query);
        let private = directory.0.join("db").join(crate::namespace::PRIVATE_NAME);
        let entries: Vec<_> = std::fs::read_dir(&private)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(entries.len() <= 2);
        for entry in entries {
            assert_eq!(entry.metadata().unwrap().len(), 0);
        }
        let config = db.config();
        db.close().unwrap();
        let db = Database::open(&directory.0.join("db"), config).unwrap();
        assert_eq!(std::fs::read_dir(private).unwrap().count(), 0);
        let baseline = db.reserved_memory_bytes();
        let query = db.prepare(QUERY).unwrap();
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut pairs = std::collections::BTreeSet::new();
        let mut done = false;
        for _ in 0..STEPS {
            match result.step() {
                QueryStep::Rows(batch) => {
                    for row in 0..batch.len() {
                        let (Some(Value::Int64(left)), Some(Value::Int64(right))) =
                            (batch.value(row, 0), batch.value(row, 1))
                        else {
                            panic!("pair");
                        };
                        assert!(
                            (0..180).contains(&left)
                                && (0..180).contains(&right)
                                && left / 2 == right / 2
                        );
                        assert!(pairs.insert((left, right)));
                    }
                }
                QueryStep::Progress => (),
                QueryStep::Finished => {
                    done = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("reopen after effect {cut}: {error}"),
            }
        }
        assert!(done);
        assert_eq!(pairs.len(), 360);
        drop(result);
        drop(query);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
        db.close().unwrap();
    }
}

#[test]
fn disjoint_inputs_merge_seven_runs_and_finish_in_both_orientations() {
    const ROWS: usize = 1218;
    let directory = Directory::new();
    let cancel = CancellationToken::new();
    let db = Database::create_empty(
        &directory.0.join("db"),
        Config::new(8_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let mut inputs = vec![];
    for (table, offset) in [("facts", 0), ("peers", ROWS)] {
        db.declare_table(
            table,
            &["k", "v"].map(|name| ColumnDeclaration {
                name,
                data_type: DataType::Int64,
                nullable: false,
            }),
            &cancel,
        )
        .unwrap();
        let keys: Vec<_> = (0..ROWS)
            .map(|i| ((i * 17) % ROWS + offset) as i64)
            .collect();
        let values: Vec<_> = (0..ROWS).map(|i| i as i64).collect();
        let mut validity = vec![255; ROWS.div_ceil(8)];
        *validity.last_mut().unwrap() = 3;
        let mut append = db
            .begin_append(
                table,
                AppendLimits {
                    batches: 1,
                    encoded_bytes: 100_000,
                },
                &cancel,
            )
            .unwrap();
        append
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&keys),
                        validity: &validity,
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&values),
                        validity: &validity,
                    },
                ],
                &cancel,
            )
            .unwrap();
        append.commit(&cancel).unwrap();
        inputs.push(keys);
    }
    assert_eq!(
        inputs[0]
            .iter()
            .flat_map(|left| inputs[1].iter().filter(move |right| left == *right))
            .count(),
        0
    );
    let baseline = db.reserved_memory_bytes();
    for (left, right) in [("facts", "peers"), ("peers", "facts")] {
        let query = db
            .prepare(&format!(
                "FROM {left} AS l |> JOIN {right} AS r ON l.k = r.k |> SELECT l.v,r.v"
            ))
            .unwrap();
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut sorted = false;
        let mut done = false;
        for _ in 0..STEPS {
            let join = join(&mut result);
            if matches!(join.phase, Phase::Seek) {
                for side in &join.sides {
                    assert_eq!(side.sort.runs, 7);
                    assert_eq!(side.sort.merge.result.unwrap().rows, ROWS as u64);
                }
                sorted = true;
            }
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Finished => {
                    done = true;
                    break;
                }
                QueryStep::Rows(_) => panic!("disjoint keys cannot match"),
                QueryStep::Failed(error) => panic!("seven-run join: {error}"),
            }
        }
        assert!(sorted && done);
        drop(result);
        drop(query);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    db.close().unwrap();
}
