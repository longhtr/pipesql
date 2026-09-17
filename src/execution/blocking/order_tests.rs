//! Exercise ordering, DISTINCT, analytic counts and running SUM through the runtime.
//!
//! Expected rows come from literal values and arithmetic sequences, not another
//! sort implementation. Exact-budget runs compare actual allocation capacities
//! with memory charges; one byte less must fail before I/O. A 200,000-row case
//! requires multiple disk runs and checks every output key, value and text field.
//!
//! Cancellation covers each controller phase. Separate faults target reading,
//! writing and temporary-space refusal. Corrupting a record after its run header
//! was read checks that merge consumption still validates the payload. Restoring
//! those bytes must not revive the failed query; a fresh query must still work.

use super::*;
use crate::effects::Effect;
use crate::effects::Faults;
use crate::execution::blocking::MergePhase;
use crate::execution::blocking::test_support::{
    Directory, expected_buffer_charge, two_column_database,
};
use crate::execution::blocking::{Files, RECORD_HEADER};
use crate::execution::{QueryResult, QueryStep, State};
use crate::value::DataType;
use crate::{AppendLimits, ColumnDeclaration, ColumnInput, ColumnValues, Config};

const QUERY: &str = "FROM facts |> ORDER BY k DESC, v ASC |> SELECT v";
const DISTINCT_QUERY: &str = "FROM facts |> SELECT k, k+0 AS copy |> DISTINCT |> SELECT k";
const UNION_DISTINCT_QUERY: &str = "FROM facts |> SELECT k, k+0 AS copy |> UNION DISTINCT (FROM facts |> SELECT k, k+0 AS copy) |> SELECT k";
const WINDOW_QUERY: &str =
    "FROM facts |> SELECT v, k, COUNT(*) OVER () AS n |> WHERE n=180 AND k>=0 |> SELECT v";
const PARTITION_QUERY: &str = "FROM facts |> SELECT v, k, COUNT(*) OVER (PARTITION BY k) AS n |> WHERE n=2 AND k>=0 |> SELECT v";
const SUM_QUERY: &str = "FROM facts |> SELECT SUM(v) OVER (ORDER BY k) AS n";
const STEPS: usize = 100_000;

fn order<'a, 'db>(result: &'a mut QueryResult<'db, '_>) -> &'a mut Order<'db> {
    let State::Running(runtime) = &mut result.state else {
        panic!("live order");
    };
    runtime.first_order_mut()
}

// Count actual capacities independently of the owner's reported charge. Scratch
// creation has a separate reservation until the files have been created.
fn check_physical_account(result: &mut QueryResult<'_, '_>) {
    let State::Running(runtime) = &result.state else {
        panic!("live order");
    };
    let inline = runtime.controller_inline_bytes(&result.plan);
    assert_eq!(inline, size_of::<Order<'_>>() as u64);
    let order = order(result);
    let input = &order.input;
    let expected = size_of::<Order<'_>>()
        + expected_buffer_charge(input.record.bytes.capacity())
        + input
            .sort
            .allocation_capacities()
            .into_iter()
            .map(expected_buffer_charge)
            .sum::<usize>();
    let creation = if matches!(input.files, Files::Pending(_)) {
        crate::storage::scratch::Creation::memory_requirement_bytes()
    } else {
        0
    };
    assert_eq!(
        expected as u64 + creation,
        order.memory_bytes() + inline,
        "inline owner and actual capacities and allocator allowances are charged once"
    );
}

fn phase_index(phase: Phase) -> usize {
    match phase {
        Phase::Create => 0,
        Phase::Read => 1,
        Phase::Await => 2,
        Phase::Capture(_) => 3,
        Phase::Push(_) => 4,
        Phase::Spill(_) => 5,
        Phase::Sort => 6,
        Phase::Load => 7,
        Phase::Emit => 8,
        Phase::Done => 9,
        Phase::CountLoad => 10,
        Phase::Count => 11,
        Phase::Failed => panic!("healthy phase"),
    }
}

#[test]
fn order_exact_admission_precedes_io_and_reconciles_each_transition() {
    let directory = Directory::new();
    let db = two_column_database(&directory);
    let cancel = CancellationToken::new();
    const DIVISION_QUERY: &str =
        "FROM facts |> EXTEND v/2 AS ratio |> WHERE ratio>=0 |> ORDER BY k DESC, v ASC |> SELECT v";
    const SAFE_QUERY: &str = "FROM facts |> EXTEND SAFE_DIVIDE(v, v-v) AS ratio |> WHERE ratio IS NULL |> ORDER BY k DESC, v ASC |> SELECT v";
    const ABS_QUERY: &str = "FROM facts |> EXTEND ABS(-v) AS magnitude |> WHERE magnitude>=0 |> ORDER BY k DESC, v ASC |> SELECT v";
    const MOD_QUERY: &str = "FROM facts |> EXTEND MOD(v, 3) AS remainder |> WHERE remainder>=0 |> ORDER BY k DESC, v ASC |> SELECT v";
    const QUOTIENT_QUERY: &str = "FROM facts |> EXTEND DIV(v, 3) AS quotient |> WHERE quotient>=0 |> ORDER BY k DESC, v ASC |> SELECT v";
    for sql in [
        QUERY,
        DISTINCT_QUERY,
        UNION_DISTINCT_QUERY,
        WINDOW_QUERY,
        PARTITION_QUERY,
        SUM_QUERY,
        DIVISION_QUERY,
        SAFE_QUERY,
        ABS_QUERY,
        MOD_QUERY,
        QUOTIENT_QUERY,
    ] {
        let query = db.prepare(sql).unwrap();
        let baseline = db.reserved_memory_bytes();
        let result = db.execute(&query, &cancel).unwrap();
        // Opening the source also needs a transient catalog buffer. Leave room
        // for that peak in addition to the workspace retained by QueryResult.
        let peak = result.accounted_memory_bytes() + crate::storage::catalog::MAX_BYTES as u64;
        drop(result);
        for shortfall in [0, 1] {
            let pressure = db
                .reserve_memory(
                    db.config().memory_limit_bytes() - baseline - peak + shortfall,
                    "order minimum test",
                )
                .unwrap();
            let mut effects = Effects::default();
            let admitted = db.execute_with_effects(&query, &cancel, &mut effects);
            if shortfall == 1 {
                assert!(matches!(admitted, Err(Error::Resource { .. })));
                assert_eq!(effects.count(), 0);
            } else {
                let mut result = admitted.unwrap();
                let expected: Vec<_> = if sql == QUERY
                    || sql == DIVISION_QUERY
                    || sql == SAFE_QUERY
                    || sql == ABS_QUERY
                    || sql == MOD_QUERY
                    || sql == QUOTIENT_QUERY
                {
                    (0..90)
                        .rev()
                        .flat_map(|key| [key * 2, key * 2 + 1])
                        .collect()
                } else if sql == SUM_QUERY {
                    (0..90)
                        .flat_map(|key| [(key + 1) * (key * 2 + 1); 2])
                        .collect()
                } else if sql == PARTITION_QUERY {
                    (0..90).flat_map(|key| [key * 2 + 1, key * 2]).collect()
                } else if sql == WINDOW_QUERY {
                    (0..90).rev().chain((90..180).rev()).collect()
                } else {
                    (0..90).collect()
                };
                let mut observed = vec![];
                let mut done = false;
                for _ in 0..STEPS {
                    check_physical_account(&mut result);
                    assert_eq!(
                        db.reserved_memory_bytes(),
                        baseline + pressure.bytes() + result.accounted_memory_bytes()
                    );
                    match result.step_with_effects(&mut effects) {
                        QueryStep::Rows(batch) => {
                            for row in 0..batch.len() {
                                let Some(Value::Int64(v)) = batch.value(row, 0) else {
                                    panic!("integer");
                                };
                                observed.push(v);
                            }
                        }
                        QueryStep::Finished => {
                            done = true;
                            break;
                        }
                        QueryStep::Progress => (),
                        QueryStep::Failed(error) => panic!("order exact minimum: {error}"),
                    }
                }
                assert!(done);
                assert_eq!(observed, expected);
            }
            drop(pressure);
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
    }
}

#[test]
fn analytic_replay_resets_partial_groups_and_emission() {
    let directory = Directory::new();
    let db = two_column_database(&directory);
    for sql in [PARTITION_QUERY, SUM_QUERY] {
        let query = db.prepare(sql).unwrap();
        let baseline = db.reserved_memory_bytes();
        let expected: Vec<_> = if sql == SUM_QUERY {
            (0..90)
                .flat_map(|key| [(key + 1) * (key * 2 + 1); 2])
                .collect()
        } else {
            (0..90).flat_map(|key| [key * 2 + 1, key * 2]).collect()
        };
        for during_count in [true, false] {
            let cancel = CancellationToken::new();
            let mut result = db.execute(&query, &cancel).unwrap();
            let mut reached = false;
            for _ in 0..STEPS {
                let owner = order(&mut result);
                if during_count && matches!(owner.phase, Phase::CountLoad) && owner.group.rows == 1
                {
                    reached = true;
                    break;
                }
                match result.step() {
                    QueryStep::Progress => (),
                    QueryStep::Rows(_) if !during_count => {
                        reached = true;
                        break;
                    }
                    _ => panic!("replay point must precede completion"),
                }
            }
            assert!(reached);
            order(&mut result).replay(&cancel).unwrap();
            assert!(matches!(
                order(&mut result).replay(&cancel),
                Err(Error::Corrupt(_))
            ));
            let mut observed = Vec::new();
            let mut done = false;
            for _ in 0..STEPS {
                check_physical_account(&mut result);
                match result.step() {
                    QueryStep::Progress => (),
                    QueryStep::Rows(batch) => {
                        for row in 0..batch.len() {
                            let Some(Value::Int64(value)) = batch.value(row, 0) else {
                                panic!("replayed value");
                            };
                            observed.push(value);
                        }
                    }
                    QueryStep::Finished => {
                        done = true;
                        break;
                    }
                    QueryStep::Failed(error) => panic!("{error}"),
                }
            }
            assert!(done);
            assert_eq!(observed, expected);
            drop(result);
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
    }
}

#[test]
fn cancellation_covers_every_order_phase_and_completion() {
    let directory = Directory::new();
    let db = two_column_database(&directory);
    for sql in [
        QUERY,
        DISTINCT_QUERY,
        UNION_DISTINCT_QUERY,
        WINDOW_QUERY,
        PARTITION_QUERY,
        SUM_QUERY,
    ] {
        let query = db.prepare(sql).unwrap();
        let baseline = db.reserved_memory_bytes();
        for target in 0..if sql == PARTITION_QUERY || sql == SUM_QUERY {
            12
        } else {
            10
        } {
            let cancel = CancellationToken::new();
            let mut result = db.execute(&query, &cancel).unwrap();
            let mut effects = Effects::default();
            let mut reached = false;
            for _ in 0..STEPS {
                if phase_index(order(&mut result).phase) == target {
                    cancel.cancel();
                    let before = effects.count();
                    for _ in 0..2 {
                        let step = result.step_with_effects(&mut effects);
                        if target == 9 {
                            assert!(
                                matches!(step, QueryStep::Finished),
                                "completed order is terminal"
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
fn sorted_output_faults_and_temp_refusal_are_terminal_and_release_owners() {
    let directory = Directory::new();
    let db = two_column_database(&directory);
    for sql in [
        QUERY,
        DISTINCT_QUERY,
        UNION_DISTINCT_QUERY,
        WINDOW_QUERY,
        PARTITION_QUERY,
        SUM_QUERY,
    ] {
        let query = db.prepare(sql).unwrap();
        let baseline = db.reserved_memory_bytes();
        let cancel = CancellationToken::new();
        // Damage the first output record, truncate its file, fail or shorten a
        // read or write, then refuse temporary space before collection starts.
        for fault in 0..7 {
            let mut result = db.execute(&query, &cancel).unwrap();
            let mut effects = Effects::default();
            if fault < 6 {
                let mut reached = false;
                for _ in 0..STEPS {
                    let order = order(&mut result);
                    let target = if fault < 4 {
                        matches!(order.phase, Phase::CountLoad)
                            || (sql != PARTITION_QUERY && matches!(order.phase, Phase::Load))
                    } else {
                        matches!(order.phase, Phase::Sort | Phase::Spill(_))
                            && order.input.sort.phase == SortPhase::Flush
                    };
                    if target {
                        reached = true;
                        break;
                    }
                    assert!(matches!(result.step(), QueryStep::Progress));
                }
                assert!(reached, "fault {fault}");
                if fault < 2 {
                    let input = &mut order(&mut result).input;
                    let slot = input.sort.merge.input.slot;
                    let offset = input.sort.merge.result.unwrap().start;
                    let Files::Open(scratch) = &mut input.files else {
                        panic!("sorted files");
                    };
                    if fault == 0 {
                        scratch
                            .write(
                                slot,
                                offset + RECORD_HEADER as u64 + 10,
                                &[255],
                                &cancel,
                                &mut effects,
                            )
                            .unwrap();
                    } else {
                        scratch.reset(slot, &cancel, &mut effects).unwrap();
                    }
                } else {
                    effects = Effects::with_faults(Faults {
                        fail_at: if fault % 2 == 0 { Some(0) } else { None },
                        short_at: if fault % 2 == 1 { Some(0) } else { None },
                        action: Some(Box::new(move |index, effect| {
                            if index == 0 {
                                assert_eq!(
                                    effect,
                                    Effect::Load(if fault < 4 {
                                        crate::effects::LoadEffect::ReadStaging
                                    } else {
                                        crate::effects::LoadEffect::WriteStaging
                                    })
                                );
                            }
                        })),
                        ..Faults::default()
                    });
                }
            } else {
                db.temporary
                    .reserve(db.config().temp_limit_bytes())
                    .unwrap();
            }
            let mut failed = false;
            for _ in 0..STEPS {
                match result.step_with_effects(&mut effects) {
                    QueryStep::Progress => (),
                    QueryStep::Failed(error) => {
                        match fault {
                            0 | 1 => assert!(matches!(error, Error::Corrupt(_)), "{error}"),
                            6 => assert!(matches!(error, Error::Resource { .. }), "{error}"),
                            _ => assert!(matches!(error, Error::Io { .. }), "{error}"),
                        }
                        failed = true;
                        break;
                    }
                    _ => panic!("fault {fault} published or completed"),
                }
            }
            assert!(failed, "fault {fault}");
            let before = effects.count();
            assert!(matches!(
                result.step_with_effects(&mut effects),
                QueryStep::Failed(_)
            ));
            assert_eq!(effects.count(), before);
            drop(result);
            if fault == 6 {
                db.temporary.release(db.config().temp_limit_bytes());
            }
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
            assert!(!db.needs_reopen());
        }
    }
}

#[test]
fn sort_layout_context_binds_comparison_policy_and_retained_field_mapping() {
    let columns = [
        SemanticColumn::new(1, DataType::Int64, true),
        SemanticColumn::new(2, DataType::Double, false),
    ];
    let keys = [planning::OrderColumn {
        column: 0,
        direction: Direction::Ascending,
        nulls: NullPlacement::First,
    }];
    let baseline = RowLayout::for_order(columns.into_iter(), &keys).unwrap();
    for mutation in 0..5 {
        let mut columns = columns;
        let mut keys = keys;
        match mutation {
            0 => keys[0].column = 1,
            1 => keys[0].direction = Direction::Descending,
            2 => keys[0].nulls = NullPlacement::Last,
            3 => columns[0] = SemanticColumn::new(1, DataType::Int64, false),
            4 => columns[0] = SemanticColumn::new(1, DataType::Date, true),
            _ => unreachable!(),
        }
        let layout = RowLayout::for_order(columns.into_iter(), &keys).unwrap();
        assert_ne!(layout.layout, baseline.layout, "mutation {mutation}");
    }
    // Once the first comparison ties, repeating that column cannot order the
    // tied rows. A different second policy must not add another stored key.
    let mut repeated = [keys[0]; 2];
    repeated[1].direction = Direction::Descending;
    repeated[1].nulls = NullPlacement::Last;
    let deduplicated = RowLayout::for_order(columns.into_iter(), &repeated).unwrap();
    assert_eq!(deduplicated.layout, baseline.layout);
    assert_eq!(deduplicated.key_count, 1);
    assert_eq!(deduplicated.max_bytes, baseline.max_bytes);
}

#[test]
fn large_nonempty_sort_preserves_duplicate_keys_and_utf8_payloads() {
    const ROWS: usize = 200_000;
    const CHUNK: usize = 16_384;
    const MEMORY: u64 = 4_000_000;
    // Even the shortest row values exceed the engine's memory limit in total.
    // Build the fixture one append chunk at a time; caller buffers are separate.
    assert!(ROWS as u64 * (16 + 7) > MEMORY);
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("db"),
        Config::new(MEMORY, 64_000_000).unwrap(),
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
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "tag",
                data_type: DataType::String,
                nullable: false,
            },
        ],
        &cancel,
    )
    .unwrap();
    let mut append = db
        .begin_append(
            "facts",
            AppendLimits {
                batches: ROWS.div_ceil(CHUNK) as u32,
                encoded_bytes: 16_000_000,
            },
            &cancel,
        )
        .unwrap();
    for start in (0..ROWS).step_by(CHUNK) {
        let end = (start + CHUNK).min(ROWS);
        // Multiplication by 7,919 permutes 0..200,000 because the numbers are
        // coprime. Dividing values by two puts each repeated key far apart in
        // input order, so merging must reunite copies from different runs.
        let values: Vec<_> = (start..end)
            .map(|position| ((position * 7919) % ROWS) as i64)
            .collect();
        let keys: Vec<_> = values.iter().map(|value| value / 2).collect();
        let tags: Vec<_> = values
            .iter()
            .map(|value| {
                if value % 2 == 0 {
                    "PROMOµ"
                } else {
                    "STANDARD"
                }
            })
            .collect();
        let valid = vec![255; (end - start).div_ceil(8)];
        assert_eq!((end - start) % 8, 0);
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
                    ColumnInput {
                        values: ColumnValues::String(&tags),
                        validity: &valid,
                    },
                ],
                &cancel,
            )
            .unwrap();
    }
    append.commit(&cancel).unwrap();
    let resident = db.reserved_memory_bytes();
    let query = db
        .prepare("FROM facts |> ORDER BY k, v |> SELECT k, tag, v")
        .unwrap();
    let baseline = db.reserved_memory_bytes();
    let mut result = db.execute(&query, &cancel).unwrap();
    let mut runs = 0;
    let mut seen = 0;
    let mut finished = false;
    let mut steps = 0;
    for _ in 0..ROWS * 128 {
        steps += 1;
        if runs == 0 {
            let order = order(&mut result);
            if matches!(order.phase, Phase::Load) {
                runs = order.input.sort.runs;
                assert!(runs >= 7);
                assert_eq!(order.input.sort.merge.result.unwrap().rows, ROWS as u64);
            }
        }
        assert_eq!(
            db.reserved_memory_bytes(),
            baseline + result.accounted_memory_bytes()
        );
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    assert!(seen < ROWS);
                    assert!(
                        matches!(batch.value(row, 0), Some(Value::Int64(key)) if key == (seen / 2) as i64)
                    );
                    assert!(
                        matches!(batch.value(row, 2), Some(Value::Int64(value)) if value == seen as i64)
                    );
                    let tag = if seen % 2 == 0 { "PROMOµ" } else { "STANDARD" };
                    assert!(
                        matches!(batch.value(row, 1), Some(Value::String(value)) if value.as_str() == tag)
                    );
                    seen += 1;
                }
            }
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("large public sort: {error}"),
        }
    }
    assert!(finished && runs >= 7);
    assert_eq!(seen, ROWS);
    drop(result);
    drop(query);
    assert_eq!(db.reserved_memory_bytes(), resident);
    assert_eq!(db.reserved_temp_bytes(), 0);
    type ExpectedTotal = fn(i64) -> i64;
    let windows: [(&str, ExpectedTotal); 4] = [
        ("COUNT(*) OVER (PARTITION BY k)", |_| 2),
        ("COUNT(*) OVER (PARTITION BY whole)", |_| 200_000),
        ("SUM(v) OVER (ORDER BY k)", |value| {
            let groups = value / 2 + 1;
            groups * (2 * groups - 1)
        }),
        ("SUM(v) OVER (ORDER BY whole)", |_| 19_999_900_000),
    ];
    for (window, expected) in windows {
        let query = db
            .prepare(&format!(
                "FROM facts |> EXTEND 0 AS whole |> SELECT k, tag, v, {window} AS n"
            ))
            .unwrap();
        let baseline = db.reserved_memory_bytes();
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut present = vec![false; ROWS];
        let mut seen = 0;
        let mut runs = 0;
        let mut finished = false;
        for _ in 0..ROWS * 160 {
            if runs == 0 && matches!(order(&mut result).phase, Phase::CountLoad) {
                runs = order(&mut result).input.sort.runs;
            }
            assert_eq!(
                db.reserved_memory_bytes(),
                baseline + result.accounted_memory_bytes()
            );
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Rows(batch) => {
                    for row in 0..batch.len() {
                        let Some(Value::Int64(value)) = batch.value(row, 2) else {
                            panic!("retained integer");
                        };
                        let index = usize::try_from(value).unwrap();
                        assert!(index < ROWS && !present[index]);
                        present[index] = true;
                        assert_eq!(batch.value(row, 0), Some(Value::Int64(value / 2)));
                        let tag = if value % 2 == 0 {
                            "PROMOµ"
                        } else {
                            "STANDARD"
                        };
                        assert!(
                            matches!(batch.value(row, 1), Some(Value::String(text)) if text.as_str() == tag)
                        );
                        assert_eq!(batch.value(row, 3), Some(Value::Int64(expected(value))));
                        seen += 1;
                    }
                }
                QueryStep::Finished => {
                    finished = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("window {window}: {error}"),
            }
        }
        assert!(finished && runs >= 7);
        assert_eq!(seen, ROWS);
        assert!(present.into_iter().all(|seen| seen));
        drop(result);
        drop(query);
        assert_eq!(db.reserved_memory_bytes(), resident);
        assert_eq!(db.reserved_temp_bytes(), 0);
        eprintln!("window {window}: {seen} rows, {runs} runs");
    }
    db.close().unwrap();
    eprintln!("large public sort: {seen} rows, {runs} runs, {steps} steps");
}

#[test]
fn merge_consumption_rejects_late_corruption_and_stays_failed_after_restore() {
    use std::os::unix::fs::FileExt;

    let directory = Directory::new();
    let db = two_column_database(&directory);
    let query = db.prepare(QUERY).unwrap();
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let mut result = db.execute(&query, &cancel).unwrap();
    let mut reached = false;
    for _ in 0..STEPS {
        let input = &order(&mut result).input;
        if input.sort.phase == SortPhase::Merge && input.sort.merge.phase == MergePhase::Pair {
            reached = true;
            break;
        }
        assert!(matches!(result.step(), QueryStep::Progress));
    }
    assert!(reached);
    let input = &order(&mut result).input;
    assert!(input.sort.runs > 1);
    let merge = &input.sort.merge;
    // The merge accepted both run headers but has not read the left payload.
    // Change a payload byte through another handle before its first read.
    assert_eq!(merge.input_index, 2);
    assert!(!merge.pair.left.loaded);
    assert_eq!(merge.pair.left.reader.buffered_bytes(), 0);
    let offset = merge.pair.left.run.unwrap().start + RECORD_HEADER as u64 + 3;
    let Files::Open(scratch) = &input.files else {
        panic!("merge owns scratch");
    };
    let observer = scratch.test_file(merge.input.slot).try_clone().unwrap();
    let mut original = [0];
    observer.read_exact_at(&mut original, offset).unwrap();
    observer.write_all_at(&[original[0] ^ 1], offset).unwrap();
    let mut effects = Effects::default();
    assert!(matches!(
        result.step_with_effects(&mut effects),
        QueryStep::Failed(Error::Corrupt("group argument checksum"))
    ));
    // Restore the byte while retaining the failed result. A second step must
    // return the same failure without attempting another read.
    observer.write_all_at(&original, offset).unwrap();
    drop(observer);
    let failed_at = effects.count();
    assert!(matches!(
        result.step_with_effects(&mut effects),
        QueryStep::Failed(Error::Corrupt("group argument checksum"))
    ));
    assert_eq!(effects.count(), failed_at);
    drop(result);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert!(!db.needs_reopen());

    // Only the query's scratch file was damaged. The published source must
    // still support a fresh query with every expected row.
    let mut healed = db.execute(&query, &cancel).unwrap();
    let mut seen = 0;
    let mut done = false;
    for _ in 0..STEPS {
        match healed.step() {
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    assert!(seen < 180);
                    let expected = 178 - (seen / 2) * 2 + seen % 2;
                    assert!(
                        matches!(batch.value(row, 0), Some(Value::Int64(value)) if value == expected)
                    );
                    seen += 1;
                }
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                done = true;
                break;
            }
            QueryStep::Failed(error) => panic!("healed sort: {error}"),
        }
    }
    assert!(done);
    assert_eq!(seen, 180);
    drop(healed);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
}
