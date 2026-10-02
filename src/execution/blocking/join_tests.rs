//! Exercise join matching and cleanup through the query runtime.
//!
//! Integer ranges define every expected pair, including duplicate matches and
//! unmatched LEFT JOIN rows. Tests compare live allocation capacities with their
//! charges, refuse execution one byte below admission, and cancel each controller
//! phase. Disjoint inputs require seven sort runs on each side in both orientations.
//!
//! File faults cover initial reads, rereading a bookmarked group and writing the
//! second input while the first remains retained. A failed result must stop I/O
//! and release its buffers and temporary-space charges.
//!
//! Scratch creation has a different failure boundary: failed pathname changes
//! block later scratch creation until reopen. Tests require ordinary scans to
//! remain usable, then reopen and check cleanup and every expected join pair.

use super::*;
use crate::effects::Effect;
use crate::effects::Faults;
use crate::execution::blocking::test_support::{
    Directory, expected_buffer_charge, limit_initial_run_rows, two_column_database,
};
use crate::execution::blocking::{Files, RECORD_HEADER};
use crate::execution::{QueryResult, QueryStep, State};
use crate::value::DataType;
use crate::{AppendLimits, ColumnDeclaration, ColumnInput, ColumnValues, Config};

const QUERY: &str = "FROM facts AS l |> JOIN facts AS r ON l.k = r.k |> SELECT l.v, r.v";
const LEFT_QUERY: &str = "FROM facts AS l |> LEFT JOIN \
    (FROM facts |> WHERE k >= 1 |> WHERE k <= 88) AS r ON l.k = r.k |> SELECT l.v, r.v";
const DEFAULT_QUERY: &str = "FROM facts AS l |> LEFT JOIN \
    (FROM facts |> WHERE k >= 1 |> WHERE k <= 88) AS r ON l.k = r.k |> SELECT l.v, COALESCE(r.v, -1)";
const STEPS: usize = 100_000;

fn join<'a, 'db>(result: &'a mut QueryResult<'db, '_>) -> &'a mut Join<'db> {
    let State::Running(runtime) = &mut result.state else {
        panic!("live join");
    };
    runtime.first_join_mut()
}

// Count both inputs' actual allocation capacities. Pending scratch creation
// also holds memory that disappears once its files have been created.
fn check_physical_account(result: &mut QueryResult<'_, '_>) {
    let State::Running(runtime) = &result.state else {
        panic!("live join");
    };
    let inline = runtime.controller_inline_bytes(&result.plan);
    assert_eq!(inline, size_of::<Join<'_>>() as u64);
    let join = join(result);
    let mut expected = size_of::<Join<'_>>();
    let mut creation = 0;
    for side in &join.sides {
        expected += expected_buffer_charge(side.record.bytes.capacity())
            + side
                .sort
                .allocation_capacities()
                .into_iter()
                .map(expected_buffer_charge)
                .sum::<usize>();
        if matches!(side.files, Files::Pending(_)) {
            creation += crate::storage::scratch::Creation::memory_requirement_bytes();
        }
    }
    assert_eq!(
        expected as u64 + creation,
        join.memory_bytes() + inline,
        "inline owner and actual capacities and allocator allowances are charged once"
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
        Phase::Unmatched => 19,
        Phase::Failed => panic!("healthy phase"),
    }
}

// Rewind faults occur after some pairs have been emitted. Accept that prefix,
// but require the expected failure before the query can report completion.
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
    let db = two_column_database(&directory);
    let cancel = CancellationToken::new();
    for (sql, left_join, defaults) in [
        (QUERY, false, false),
        (LEFT_QUERY, true, false),
        (DEFAULT_QUERY, true, true),
    ] {
        let query = db.prepare(sql).unwrap();
        let baseline = db.reserved_memory_bytes();
        let result = db.execute(&query, &cancel).unwrap();
        // Leave room for the transient catalog buffer used to open a source,
        // in addition to both inputs' retained workspace.
        let peak = result.accounted_memory_bytes() + crate::storage::catalog::MAX_BYTES as u64;
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
                // Enumerate pairs directly from k=v/2. This expectation does
                // not sort inputs or use the production join comparator.
                let expected: Vec<_> = (0..180)
                    .flat_map(|l| {
                        let matched = !left_join || (2..178).contains(&l);
                        let mut rows = Vec::new();
                        if matched {
                            for r in 0..180 {
                                if l / 2 == r / 2 {
                                    rows.push((l, Some(r)));
                                }
                            }
                        } else {
                            rows.push((l, if defaults { Some(-1) } else { None }));
                        }
                        rows
                    })
                    .collect();
                let mut observed = vec![];
                let mut done = false;
                let mut retired = false;
                for _ in 0..STEPS {
                    check_physical_account(&mut result);
                    if matches!(join(&mut result).phase, Phase::Create(1)) {
                        let join = join(&mut result);
                        let first = join.sides[0].sort.allocation_capacities();
                        let second = join.sides[1].sort.allocation_capacities();
                        assert_eq!(join.sides[0].record.bytes.capacity(), 0);
                        assert!(join.sides[1].record.bytes.capacity() > 0);
                        assert_eq!((first[6], first[8]), (0, 0));
                        assert!(second[6] > 0 && second[8] > 0);
                        retired = true;
                    }
                    assert_eq!(
                        db.reserved_memory_bytes(),
                        baseline + pressure.bytes() + result.accounted_memory_bytes()
                    );
                    match result.step_with_effects(&mut effects) {
                        QueryStep::Rows(batch) => {
                            for row in 0..batch.len() {
                                let Some(Value::Int64(l)) = batch.value(row, 0) else {
                                    panic!("left value")
                                };
                                let r = match batch.value(row, 1) {
                                    Some(Value::Int64(r)) => Some(r),
                                    Some(Value::Null) => None,
                                    other => panic!("right value: {other:?}"),
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
                assert!(done && retired);
                observed.sort_unstable();
                assert_eq!(observed, expected);
            }
            drop(pressure);
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
    }
}

#[test]
fn cancellation_covers_both_inputs_sort_matching_and_duplicate_rewind() {
    let directory = Directory::new();
    let db = two_column_database(&directory);
    for (sql, phases) in [(QUERY, 19), (LEFT_QUERY, 20), (DEFAULT_QUERY, 20)] {
        let query = db.prepare(sql).unwrap();
        let baseline = db.reserved_memory_bytes();
        for target in 0..phases {
            let cancel = CancellationToken::new();
            let mut result = db.execute(&query, &cancel).unwrap();
            let mut effects = Effects::default();
            let mut reached = false;
            for _ in 0..STEPS {
                let owner = join(&mut result);
                if let Phase::Read(side) = owner.phase
                    && owner.sides[side].ordinal == 0
                {
                    limit_initial_run_rows(&mut owner.sides[side], 37);
                }
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
}

#[test]
fn sorted_input_corruption_truncation_read_failure_and_temp_refusal_are_terminal() {
    let directory = Directory::new();
    let db = two_column_database(&directory);
    let query = db.prepare(QUERY).unwrap();
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    for fault in 0..5 {
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut effects = Effects::default();
        // Fault 3 changes the saved right group after its first traversal.
        // The next equal left row must reread it rather than trust cached bytes.
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
    let db = two_column_database(&directory);
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
    // Hold another creator inside scratch bootstrap before the second input
    // starts. Buffered sends and timed receives let the worker exit if the
    // parent fails; contention must release the already sorted first input.
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
            let scratch = crate::storage::scratch::Scratch::new(
                database,
                &CancellationToken::new(),
                &mut effects,
            )
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
        assert!(matches!(
            result.step_with_effects(&mut effects),
            QueryStep::Failed(Error::Contention("scratch bootstrap"))
        ));
        assert_eq!(effects.count(), 0, "terminal contention cannot retry I/O");
        resume_tx.send(()).unwrap();
        creator.join().unwrap();
    });
    drop(result);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    // Contention made no namespace changes. Once the creator has finished, a
    // fresh result must use scratch without reopening and emit every pair.
    let mut retry = db.execute(&query, &cancel).unwrap();
    let mut seen = [false; 360];
    let mut finished = false;
    for _ in 0..STEPS {
        match retry.step() {
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 2);
                for row in 0..batch.len() {
                    let (Some(Value::Int64(left)), Some(Value::Int64(right))) =
                        (batch.value(row, 0), batch.value(row, 1))
                    else {
                        panic!("integer join pair");
                    };
                    assert!((0..180).contains(&left) && (0..180).contains(&right));
                    assert_eq!(left / 2, right / 2);
                    let index = (left * 2 + right % 2) as usize;
                    assert!(!seen[index], "duplicate pair");
                    seen[index] = true;
                }
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("contention left recovery debt: {error}"),
        }
    }
    assert!(finished && seen.into_iter().all(|present| present));
    drop(retry);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
}

#[test]
fn short_matching_reads_and_second_input_writes_are_terminal() {
    let directory = Directory::new();
    let db = two_column_database(&directory);
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
    let db = two_column_database(&directory);
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
    // Discover the second input's pathname operations from a healthy creation.
    // Payload I/O failures are covered separately and need no reopen.
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
        let db = two_column_database(&directory);
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
        // Memory admission may succeed, but scratch bootstrap must refuse
        // without attempting live repair of the uncertain directory state.
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
        // The uncertainty concerns scratch names, not published table data.
        // A query that needs no scratch files must still read the source.
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
        let private = directory
            .0
            .join("db")
            .join(crate::storage::recovery::PRIVATE_NAME);
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
    // Seven initial runs leave an unpaired run during merging. Disjoint keys
    // exercise exhaustion of either input without emitting a pair.
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
                "FROM {left} AS l |> JOIN {right} AS r ON l.k = r.k |> SELECT l.v, r.v"
            ))
            .unwrap();
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut sorted = false;
        let mut done = false;
        for _ in 0..STEPS {
            let join = join(&mut result);
            if let Phase::Read(side) = join.phase
                && join.sides[side].ordinal == 0
            {
                limit_initial_run_rows(&mut join.sides[side], 174);
            }
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

#[test]
fn groups_and_records_larger_than_read_ahead_keep_every_pair() {
    // Exercise transitions after replay, groups spanning transfer buffers and
    // single large records. Unequal rows make the first row a poor size estimate.
    for (width, unequal) in [
        (2048, false),
        (2048, true),
        (16_384, false),
        (16_384, true),
        (65_536, false),
        (65_536, true),
    ] {
        let directory = Directory::new();
        let db = Database::create_empty(
            &directory.0.join("db"),
            Config::new(12_000_000, 16_000_000).unwrap(),
        )
        .unwrap();
        let cancel = CancellationToken::new();
        db.declare_table(
            "facts",
            &[
                ColumnDeclaration {
                    name: "k",
                    data_type: DataType::Int64,
                    nullable: false,
                },
                ColumnDeclaration {
                    name: "v",
                    data_type: DataType::String,
                    nullable: false,
                },
            ],
            &cancel,
        )
        .unwrap();
        let texts: Vec<_> = (0..24)
            .map(|id| {
                let size = if unequal && id % 7 == 0 {
                    width / 4
                } else {
                    width
                };
                format!("{id:08x}{}", "x".repeat(size - 8))
            })
            .collect();
        let mut append = db
            .begin_append(
                "facts",
                AppendLimits {
                    batches: 24,
                    encoded_bytes: 2_000_000,
                },
                &cancel,
            )
            .unwrap();
        for id in (0..24).rev() {
            append
                .write(
                    &[
                        ColumnInput {
                            values: ColumnValues::Int64(&[id as i64 / 8]),
                            validity: &[1],
                        },
                        ColumnInput {
                            values: ColumnValues::String(&[texts[id].as_str()]),
                            validity: &[1],
                        },
                    ],
                    &cancel,
                )
                .unwrap();
        }
        append.commit(&cancel).unwrap();
        let query = db.prepare(QUERY).unwrap();
        let baseline = db.reserved_memory_bytes();
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut seen = [[false; 24]; 24];
        let mut spilled = false;
        let mut finished = false;
        for _ in 0..STEPS {
            spilled |= db.reserved_temp_bytes() > 0;
            match result.step() {
                QueryStep::Rows(batch) => {
                    for row in 0..batch.len() {
                        let ids: [usize; 2] = std::array::from_fn(|column| {
                            let Some(Value::String(value)) = batch.value(row, column) else {
                                panic!("text pair");
                            };
                            let text = value.as_str();
                            let id = usize::from_str_radix(&text[..8], 16).unwrap();
                            assert!(id < 24);
                            assert_eq!(text, texts[id]);
                            id
                        });
                        let [left, right] = ids;
                        assert_eq!(left / 8, right / 8);
                        assert!(!std::mem::replace(&mut seen[left][right], true));
                    }
                }
                QueryStep::Progress => (),
                QueryStep::Finished => {
                    finished = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("width {width}: {error}"),
            }
        }
        assert!(finished && spilled);
        for (left, pairs) in seen.iter().enumerate() {
            for (right, present) in pairs.iter().enumerate() {
                assert_eq!(*present, left / 8 == right / 8);
            }
        }
        drop(result);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}

#[test]
fn numeric_join_limits_preserve_late_expression_demand() {
    let directory = Directory::new();
    let db = two_column_database(&directory);
    let token = CancellationToken::new();
    // Stable source ordinals put right value 1 before 0 in the first key.
    // The first pair is safe; demanding the second pair overflows.
    let expression = "9223372036854775807 + (1 - r.v)";
    for (operation, expected) in [
        (format!("SELECT {expression} AS n"), i64::MAX),
        (
            format!("EXTEND {expression} AS n |> WHERE n > 0 |> SELECT l.v"),
            1,
        ),
    ] {
        for count in [1, 2] {
            let sql = format!(
                "FROM facts AS l |> JOIN facts AS r ON l.k = r.k |> {operation} |> LIMIT {count}"
            );
            let query = db.prepare(&sql).unwrap();
            let baseline = db.reserved_memory_bytes();
            let mut result = db.execute(&query, &token).unwrap();
            let mut rows = Vec::new();
            let mut terminal = false;
            for _ in 0..STEPS {
                match result.step() {
                    QueryStep::Progress => (),
                    QueryStep::Rows(batch) => {
                        for row in 0..batch.len() {
                            assert_eq!(batch.value(row, 0), Some(Value::Int64(expected)));
                            rows.push(expected);
                        }
                    }
                    QueryStep::Finished if count == 1 => {
                        terminal = true;
                        break;
                    }
                    QueryStep::Failed(Error::ArithmeticOverflow { .. }) if count == 2 => {
                        terminal = true;
                        assert!(matches!(
                            result.step(),
                            QueryStep::Failed(Error::ArithmeticOverflow { .. })
                        ));
                        break;
                    }
                    QueryStep::Failed(error) => panic!("unexpected limit {count} failure: {error}"),
                    _ => panic!("unexpected limit {count} completion"),
                }
            }
            assert!(terminal);
            assert_eq!(rows, [expected]);
            drop(result);
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
    }
}

#[test]
fn large_fixed_width_match_groups_replay_and_cancel() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("db"),
        Config::new(8_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let token = CancellationToken::new();
    db.declare_table(
        "facts",
        &[
            ("k", DataType::Int64),
            ("id", DataType::Int64),
            ("n", DataType::Int64),
            ("d", DataType::Double),
            ("day", DataType::Date),
        ]
        .map(|(name, data_type)| ColumnDeclaration {
            name,
            data_type,
            nullable: name == "n",
        }),
        &token,
    )
    .unwrap();
    let ids: Vec<i64> = (0..513).collect();
    let keys = vec![0; 513];
    let double_bits = [
        0,
        0x8000000000000000,
        0x7ff8000000000042,
        0x7ff0000000000000,
        0xfff0000000000000,
        1,
    ];
    let doubles: Vec<_> = (0..513)
        .map(|row| f64::from_bits(double_bits[row % double_bits.len()]))
        .collect();
    let date_days = [-719_162, 0, 10_957, 2_932_896];
    let dates: Vec<_> = (0..513)
        .map(|row| crate::DateValue::from_days(date_days[row % date_days.len()]).unwrap())
        .collect();
    let mut valid = vec![0; 65];
    let mut nullable = vec![0; 65];
    for row in 0..513 {
        valid[row / 8] |= 1 << (row % 8);
        if row % 3 != 0 {
            nullable[row / 8] |= 1 << (row % 8);
        }
    }
    let mut append = db
        .begin_append(
            "facts",
            AppendLimits {
                batches: 1,
                encoded_bytes: 100_000,
            },
            &token,
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
                    values: ColumnValues::Int64(&ids),
                    validity: &valid,
                },
                ColumnInput {
                    values: ColumnValues::Int64(&ids),
                    validity: &nullable,
                },
                ColumnInput {
                    values: ColumnValues::Double(&doubles),
                    validity: &valid,
                },
                ColumnInput {
                    values: ColumnValues::Date(&dates),
                    validity: &valid,
                },
            ],
            &token,
        )
        .unwrap();
    append.commit(&token).unwrap();
    let joined = "FROM (FROM facts |> WHERE id < 2) AS l |> JOIN facts AS r ON l.k=r.k";
    let query = db
        .prepare(&format!(
            "{joined} |> SELECT l.id, r.id, l.n, r.n, r.d, r.day"
        ))
        .unwrap();
    let baseline = db.reserved_memory_bytes();
    // Healthy execution, replay after a prefix, cancellation after a batch,
    // then a fresh complete retry all use the same immutable prepared query.
    for mode in 0..4 {
        let token = CancellationToken::new();
        let mut result = db.execute(&query, &token).unwrap();
        let mut seen = [[false; 513]; 2];
        let mut count = 0;
        let mut replayed = false;
        let mut first_rows = 0;
        let mut largest_batch = 0;
        let mut terminal = false;
        for _ in 0..STEPS {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Rows(batch) => {
                    assert_eq!(batch.column_count(), 6);
                    assert!(batch.len() <= crate::batch::ROWS);
                    largest_batch = largest_batch.max(batch.len());
                    if first_rows == 0 {
                        first_rows = batch.len();
                    }
                    for row in 0..batch.len() {
                        let (Some(Value::Int64(left)), Some(Value::Int64(right))) =
                            (batch.value(row, 0), batch.value(row, 1))
                        else {
                            panic!("pair identities");
                        };
                        assert!((0..2).contains(&left) && (0..513).contains(&right));
                        assert_eq!(
                            batch.value(row, 2),
                            Some(if left == 0 {
                                Value::Null
                            } else {
                                Value::Int64(1)
                            })
                        );
                        assert_eq!(
                            batch.value(row, 3),
                            Some(if right % 3 == 0 {
                                Value::Null
                            } else {
                                Value::Int64(right)
                            })
                        );
                        let Some(Value::Double(number)) = batch.value(row, 4) else {
                            panic!("DOUBLE payload");
                        };
                        assert_eq!(
                            number.to_bits(),
                            double_bits[right as usize % double_bits.len()]
                        );
                        let Some(Value::Date(day)) = batch.value(row, 5) else {
                            panic!("DATE payload");
                        };
                        assert_eq!(
                            day.days_since_unix_epoch(),
                            date_days[right as usize % date_days.len()]
                        );
                        assert!(!std::mem::replace(
                            &mut seen[left as usize][right as usize],
                            true
                        ));
                        count += 1;
                    }
                    if mode == 1 && !replayed {
                        let State::Running(runtime) = &mut result.state else {
                            panic!("live joined prefix");
                        };
                        runtime.replay_output_for_test();
                        replayed = true;
                        seen = [[false; 513]; 2];
                        count = 0;
                    } else if mode == 2 {
                        token.cancel();
                    }
                }
                QueryStep::Finished if mode != 2 => {
                    terminal = true;
                    break;
                }
                QueryStep::Failed(Error::Cancelled) if mode == 2 => {
                    terminal = true;
                    assert!(matches!(result.step(), QueryStep::Failed(Error::Cancelled)));
                    break;
                }
                QueryStep::Failed(error) => panic!("numeric batch mode {mode}: {error}"),
                _ => panic!("cancelled work completed"),
            }
        }
        assert!(terminal);
        assert_eq!(largest_batch, crate::batch::ROWS);
        if mode != 2 {
            assert_eq!(count, 1026);
            assert!(seen.iter().flatten().all(|present| *present));
        } else {
            assert_eq!(count, first_rows);
        }
        drop(result);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    // These consumers now receive multi-row join batches. LIMIT crosses a
    // duplicate rewind, and RANGE SUM must include both equal right-key peers.
    for (suffix, mut expected) in [
        (
            "SELECT l.id AS left_id, r.id AS right_id |> LIMIT 3 OFFSET 511 |> EXTEND 0 AS n",
            vec![(0, 511, 0), (0, 512, 0), (1, 0, 0)],
        ),
        (
            "SELECT l.id AS left_id, r.id AS right_id |> LIMIT 3 OFFSET 511 |> WHERE right_id > 0 |> EXTEND 0 AS n",
            vec![(0, 511, 0), (0, 512, 0)],
        ),
        (
            "AGGREGATE COUNT(*) AS n, SUM(r.id) AS total GROUP AND ORDER BY l.id",
            vec![(0, 513, 131_328), (1, 513, 131_328)],
        ),
        (
            "SELECT l.id, r.id, COUNT(*) OVER () AS n",
            (0..2)
                .flat_map(|left| (0..513).map(move |right| (left, right, 1026)))
                .collect(),
        ),
        (
            "SELECT l.id, r.id, SUM(r.id) OVER (ORDER BY r.id) AS n",
            (0..2)
                .flat_map(|left| (0..513).map(move |right| (left, right, right * (right + 1))))
                .collect(),
        ),
    ] {
        let query = db.prepare(&format!("{joined} |> {suffix}")).unwrap();
        let baseline = db.reserved_memory_bytes();
        let token = CancellationToken::new();
        let mut result = db.execute(&query, &token).unwrap();
        let mut actual = Vec::new();
        let mut finished = false;
        for _ in 0..STEPS {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Rows(batch) => {
                    assert_eq!(batch.column_count(), 3);
                    for row in 0..batch.len() {
                        let integer = |column| match batch.value(row, column) {
                            Some(Value::Int64(value)) => value,
                            _ => panic!("integer composed answer: {suffix}"),
                        };
                        actual.push((integer(0), integer(1), integer(2)));
                    }
                }
                QueryStep::Finished => {
                    finished = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("{suffix}: {error}"),
            }
        }
        assert!(finished, "{suffix}");
        actual.sort_unstable();
        expected.sort_unstable();
        assert_eq!(actual, expected, "{suffix}");
        drop(result);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}

#[test]
fn fixed_width_double_and_date_keys_preserve_join_equality() {
    // This model uses primitive equality, independently of sorted-key ordering.
    // Keep original bits when checking output: equal signed zeros can differ.
    #[derive(Clone, Copy, PartialEq)]
    enum Key {
        Double(f64),
        Day(i32),
    }
    fn check_key(actual: Option<Value<'_>>, expected: Option<Key>) {
        match (actual, expected) {
            (Some(Value::Null), None) => (),
            (Some(Value::Double(actual)), Some(Key::Double(expected))) => {
                assert_eq!(actual.to_bits(), expected.to_bits());
            }
            (Some(Value::Date(actual)), Some(Key::Day(expected))) => {
                assert_eq!(actual.days_since_unix_epoch(), expected);
            }
            _ => panic!("wrong typed join key"),
        }
    }
    for kind in [DataType::Double, DataType::Date] {
        let (left, right) = if kind == DataType::Double {
            let nan_a = f64::from_bits(0x7ff8_0000_0000_0042);
            let nan_b = f64::from_bits(0x7ff8_0000_0000_0099);
            let left = [
                Some(-0.0),
                Some(0.0),
                Some(nan_a),
                Some(nan_b),
                Some(f64::INFINITY),
                Some(f64::NEG_INFINITY),
                None,
                Some(1.0),
                Some(2.0),
            ]
            .map(|v| v.map(Key::Double))
            .to_vec();
            let mut right: Vec<_> = (0..513)
                .map(|i| Some(Key::Double(if i % 2 == 0 { 0.0 } else { -0.0 })))
                .collect();
            right.extend(
                [
                    Some(nan_b),
                    None,
                    Some(f64::NEG_INFINITY),
                    Some(1.0),
                    Some(nan_a),
                    Some(f64::INFINITY),
                    Some(3.0),
                ]
                .map(|v| v.map(Key::Double)),
            );
            (left, right)
        } else {
            let left = [
                Some(0),
                Some(0),
                Some(-719_162),
                Some(2_932_896),
                None,
                Some(-1),
                Some(2),
            ]
            .map(|v| v.map(Key::Day))
            .to_vec();
            let mut right = vec![Some(Key::Day(0)); 513];
            right.extend(
                [Some(2_932_896), None, Some(-719_162), Some(-1), Some(3)].map(|v| v.map(Key::Day)),
            );
            (left, right)
        };
        let directory = Directory::new();
        let db = Database::create_empty(
            &directory.0.join("db"),
            Config::new(8_000_000, 8_000_000).unwrap(),
        )
        .unwrap();
        let token = CancellationToken::new();
        for (name, keys) in [("lhs", &left), ("rhs", &right)] {
            db.declare_table(
                name,
                &[
                    ColumnDeclaration {
                        name: "id",
                        data_type: DataType::Int64,
                        nullable: false,
                    },
                    ColumnDeclaration {
                        name: "k",
                        data_type: kind,
                        nullable: true,
                    },
                ],
                &token,
            )
            .unwrap();
            let ids: Vec<_> = (0..keys.len() as i64).collect();
            let mut valid = vec![0; keys.len().div_ceil(8)];
            let mut present = valid.clone();
            for (i, key) in keys.iter().enumerate() {
                valid[i / 8] |= 1 << (i % 8);
                if key.is_some() {
                    present[i / 8] |= 1 << (i % 8);
                }
            }
            let doubles: Vec<_> = keys
                .iter()
                .map(|key| match key {
                    Some(Key::Double(value)) => *value,
                    _ => 0.0,
                })
                .collect();
            let days: Vec<_> = keys
                .iter()
                .map(|key| {
                    crate::DateValue::from_days(match key {
                        Some(Key::Day(value)) => *value,
                        _ => 0,
                    })
                    .unwrap()
                })
                .collect();
            let values = if kind == DataType::Double {
                ColumnValues::Double(&doubles)
            } else {
                ColumnValues::Date(&days)
            };
            let mut append = db
                .begin_append(
                    name,
                    AppendLimits {
                        batches: 1,
                        encoded_bytes: 100_000,
                    },
                    &token,
                )
                .unwrap();
            append
                .write(
                    &[
                        ColumnInput {
                            values: ColumnValues::Int64(&ids),
                            validity: &valid,
                        },
                        ColumnInput {
                            values,
                            validity: &present,
                        },
                    ],
                    &token,
                )
                .unwrap();
            append.commit(&token).unwrap();
        }
        for outer in [false, true] {
            let mut expected = vec![];
            for (l, a) in left.iter().enumerate() {
                let before = expected.len();
                for (r, b) in right.iter().enumerate() {
                    if a.zip(*b).is_some_and(|(a, b)| a == b) {
                        expected.push((l, Some(r)));
                    }
                }
                if outer && expected.len() == before {
                    expected.push((l, None));
                }
            }
            let query = db
                .prepare(&format!(
                    "FROM lhs AS l |> {}JOIN rhs AS r ON l.k=r.k |> SELECT l.id, r.id, l.k, r.k",
                    if outer { "LEFT " } else { "" }
                ))
                .unwrap();
            let baseline = db.reserved_memory_bytes();
            for cancel_after_batch in [false, true, false] {
                let token = CancellationToken::new();
                let mut result = db.execute(&query, &token).unwrap();
                assert!(join(&mut result).batch_matches);
                let mut effects = Effects::default();
                let mut actual = vec![];
                let mut sorted = false;
                let mut finished = false;
                let mut largest = 0;
                for _ in 0..STEPS {
                    let owner = join(&mut result);
                    if let Phase::Read(side) = owner.phase
                        && owner.sides[side].ordinal == 0
                    {
                        limit_initial_run_rows(&mut owner.sides[side], 37);
                    }
                    if !sorted && matches!(owner.phase, Phase::Seek) {
                        assert_eq!(owner.sides[1].sort.runs as usize, right.len().div_ceil(37));
                        assert!(owner.sides[1].sort.runs > 1);
                        sorted = true;
                    }
                    match result.step_with_effects(&mut effects) {
                        QueryStep::Rows(batch) => {
                            assert_eq!(batch.column_count(), 4);
                            largest = largest.max(batch.len());
                            for row in 0..batch.len() {
                                let Some(Value::Int64(l)) = batch.value(row, 0) else {
                                    panic!("left identity")
                                };
                                let l = usize::try_from(l).unwrap();
                                let r = match batch.value(row, 1) {
                                    Some(Value::Null) => None,
                                    Some(Value::Int64(r)) => Some(usize::try_from(r).unwrap()),
                                    _ => panic!("right identity"),
                                };
                                assert!(
                                    expected.binary_search(&(l, r)).is_ok(),
                                    "unexpected pair {l}, {r:?}"
                                );
                                check_key(batch.value(row, 2), left[l]);
                                check_key(batch.value(row, 3), r.and_then(|r| right[r]));
                                actual.push((l, r));
                            }
                            if cancel_after_batch && batch.len() == crate::batch::ROWS {
                                token.cancel();
                                let before = effects.count();
                                for _ in 0..2 {
                                    assert!(matches!(
                                        result.step_with_effects(&mut effects),
                                        QueryStep::Failed(Error::Cancelled)
                                    ));
                                }
                                assert_eq!(effects.count(), before);
                                finished = true;
                                break;
                            }
                        }
                        QueryStep::Progress => (),
                        QueryStep::Finished => {
                            assert!(!cancel_after_batch);
                            finished = true;
                            break;
                        }
                        QueryStep::Failed(error) => panic!("typed join: {error}"),
                    }
                }
                assert!(sorted && finished);
                assert_eq!(largest, crate::batch::ROWS);
                actual.sort_unstable();
                assert!(actual.windows(2).all(|pair| pair[0] != pair[1]));
                if !cancel_after_batch {
                    assert_eq!(actual, expected);
                }
                drop(result);
                assert_eq!(db.reserved_memory_bytes(), baseline);
                assert_eq!(db.reserved_temp_bytes(), 0);
            }
        }
    }
}

#[test]
fn fixed_width_match_groups_fit_the_reported_stack() {
    std::thread::Builder::new()
        .stack_size(pipesql_filesystem::TEST_SMALL_STACK_REQUEST_BYTES)
        .spawn(|| {
            pipesql_filesystem::test_assert_small_stack();
            large_fixed_width_match_groups_replay_and_cancel();
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn numeric_pair_read_failure_after_a_prefix_stays_terminal() {
    let directory = Directory::new();
    let db = two_column_database(&directory);
    let token = CancellationToken::new();
    // A computed projection retains one-row expression demand.
    let query = db
        .prepare("FROM facts AS l |> JOIN facts AS r ON l.k=r.k |> SELECT l.v+0, r.v")
        .unwrap();
    let baseline = db.reserved_memory_bytes();
    let mut result = db.execute(&query, &token).unwrap();
    let mut reached = false;
    for _ in 0..STEPS {
        let owner = join(&mut result);
        if matches!(owner.phase, Phase::Emit) {
            // The first right record is loaded. Force the next record to need
            // I/O after the first pair has been returned.
            owner.sides[1].sort.sorted_rows().cursor.reader.clear();
            reached = true;
            break;
        }
        assert!(matches!(result.step(), QueryStep::Progress));
    }
    assert!(reached && db.reserved_temp_bytes() > 0);
    let QueryStep::Rows(batch) = result.step() else {
        panic!("loaded first pair must precede the next read");
    };
    assert_eq!(batch.len(), 1);
    assert_eq!(batch.value(0, 0), Some(Value::Int64(1)));
    assert_eq!(batch.value(0, 1), Some(Value::Int64(1)));
    let mut effects = Effects::with_faults(Faults {
        short_at: Some(0),
        ..Faults::default()
    });
    assert!(matches!(
        result.step_with_effects(&mut effects),
        QueryStep::Failed(Error::Io { .. })
    ));
    let count = effects.count();
    assert!(count > 0);
    assert!(matches!(
        result.step_with_effects(&mut effects),
        QueryStep::Failed(Error::Io { .. })
    ));
    assert_eq!(effects.count(), count);
    drop(result);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
}

#[test]
fn later_numeric_pair_failure_keeps_the_batch_private() {
    let directory = Directory::new();
    let db = two_column_database(&directory);
    for cancel_during_read in [false, true] {
        let token = std::sync::Arc::new(CancellationToken::new());
        let query = db.prepare(QUERY).unwrap();
        let baseline = db.reserved_memory_bytes();
        let mut result = db.execute(&query, &token).unwrap();
        let mut reached = false;
        for _ in 0..STEPS {
            let owner = join(&mut result);
            if matches!(owner.phase, Phase::Emit) {
                // The first right record is loaded. Force the next record to need
                // I/O after the first pair's cells have been filled privately.
                owner.sides[1].sort.sorted_rows().cursor.reader.clear();
                reached = true;
                break;
            }
            assert!(matches!(result.step(), QueryStep::Progress));
        }
        assert!(reached && db.reserved_temp_bytes() > 0);
        let request = std::sync::Arc::clone(&token);
        let mut effects = Effects::with_faults(Faults {
            short_at: (!cancel_during_read).then_some(0),
            action: Some(Box::new(move |index, _| {
                if cancel_during_read && index == 0 {
                    request.cancel();
                }
            })),
            ..Faults::default()
        });
        let expected_failure = |step: QueryStep<'_>| match step {
            QueryStep::Failed(Error::Cancelled) if cancel_during_read => (),
            QueryStep::Failed(Error::Io { .. }) if !cancel_during_read => (),
            _ => panic!("a later pair must fail before publishing the batch"),
        };
        expected_failure(result.step_with_effects(&mut effects));
        let count = effects.count();
        assert!(count > 0);
        expected_failure(result.step_with_effects(&mut effects));
        assert_eq!(effects.count(), count);
        drop(result);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}
