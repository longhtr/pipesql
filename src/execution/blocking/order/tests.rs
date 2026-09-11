use super::*;
use crate::effects::Effect;
use crate::effects::Faults;
use crate::execution::blocking::MergePhase;
use crate::execution::blocking::test_support::Directory;
use crate::execution::blocking::{Files, RECORD_HEADER};
use crate::execution::{QueryResult, QueryStep, State};
use crate::frontend::DataType;
use crate::{AppendLimits, ColumnDeclaration, ColumnInput, ColumnValues, Config};

const QUERY: &str = "FROM facts |> ORDER BY k DESC,v ASC |> SELECT v";
const DISTINCT_QUERY: &str = "FROM facts |> SELECT k,k+0 AS copy |> DISTINCT |> SELECT k";
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
    // 180 rows exceed the sorter's byte-limited first run. Adjacent equal keys
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

fn order<'a, 'db>(result: &'a mut QueryResult<'db, '_>) -> &'a mut Order<'db> {
    let State::Running(runtime) = &mut result.state else {
        panic!("live order");
    };
    runtime.first_order_mut()
}

fn check_physical_account(order: &Order<'_>) {
    fn bytes<T>(v: &Vec<T>) -> usize {
        v.capacity() * size_of::<T>()
    }
    let input = &order.input;
    let run = &input.sort.buffer;
    let merge = &input.sort.merge;
    let physical = size_of::<Order<'_>>()
        + bytes(&input.record.bytes)
        + bytes(&run.bytes)
        + bytes(&run.spans)
        + bytes(&run.work)
        + merge.writer.allocated_bytes()
        + bytes(&merge.pair.previous_key)
        + bytes(&merge.pair.left.record.bytes)
        + bytes(&merge.pair.right.record.bytes)
        + merge.pair.left.reader.allocated_bytes()
        + merge.pair.right.reader.allocated_bytes();
    let creation = if matches!(input.files, Files::Pending(_)) {
        crate::scratch::Creation::memory_requirement_bytes()
    } else {
        0
    };
    assert_eq!(physical as u64 + creation, order.memory_bytes());
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
        Phase::Failed => panic!("healthy phase"),
    }
}

#[test]
fn order_exact_admission_precedes_io_and_reconciles_each_transition() {
    let directory = Directory::new();
    let db = database(&directory);
    let cancel = CancellationToken::new();
    for sql in [QUERY, DISTINCT_QUERY] {
        let query = db.prepare(sql).unwrap();
        let baseline = db.reserved_memory_bytes();
        let result = db.execute(&query, &cancel).unwrap();
        let peak = result.accounted_memory_bytes() + crate::catalog::MAX_BYTES as u64;
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
                let expected: Vec<_> = if sql == QUERY {
                    (0..90)
                        .rev()
                        .flat_map(|key| [key * 2, key * 2 + 1])
                        .collect()
                } else {
                    (0..90).collect()
                };
                let mut observed = vec![];
                let mut done = false;
                for _ in 0..STEPS {
                    check_physical_account(order(&mut result));
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
fn cancellation_covers_every_order_phase_and_completion() {
    let directory = Directory::new();
    let db = database(&directory);
    for sql in [QUERY, DISTINCT_QUERY] {
        let query = db.prepare(sql).unwrap();
        let baseline = db.reserved_memory_bytes();
        for target in 0..10 {
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
    let db = database(&directory);
    for sql in [QUERY, DISTINCT_QUERY] {
        let query = db.prepare(sql).unwrap();
        let baseline = db.reserved_memory_bytes();
        let cancel = CancellationToken::new();
        // Corruption, truncation, failed/short final reads, failed/short run
        // writes and exhausted temporary storage cross distinct controller edges.
        for fault in 0..7 {
            let mut result = db.execute(&query, &cancel).unwrap();
            let mut effects = Effects::default();
            if fault < 6 {
                let mut reached = false;
                for _ in 0..STEPS {
                    let order = order(&mut result);
                    let target = if fault < 4 {
                        matches!(order.phase, Phase::Load)
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
    // The fixed-width payloads plus the shorter text value already exceed the
    // database memory cap. Caller setup owns only one bounded append chunk.
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
        // 7,919 is coprime to 200,000: every value occurs once, in a shuffled
        // order. Equal keys are far apart in the source and cross sort runs.
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
        .prepare("FROM facts |> ORDER BY k,v |> SELECT k,tag,v")
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
    db.close().unwrap();
    eprintln!("large public sort: {seen} rows, {runs} runs, {steps} steps");
}

#[test]
fn merge_consumption_rejects_late_corruption_and_stays_failed_after_restore() {
    use std::os::unix::fs::FileExt;

    let directory = Directory::new();
    let db = database(&directory);
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
    // Both headers have been consumed, but payload reads must still validate
    // their own bytes. The existing test boundary retains one observer handle.
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
    // A later clean read must not certify the earlier changed payload. Restore
    // the bytes, then verify that the public result remains terminal without I/O.
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

    // The failed query's private files do not poison the published source or
    // retain writer/cleanup debt. A fresh query returns every original value.
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
