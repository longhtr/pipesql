use super::test_support::{Directory, schema};
use super::*;
use crate::batch::Batch;
use crate::batch::OwnedBatch;
use crate::effects::Faults;
use crate::frontend::{DataType, SemanticColumn};
use crate::value::{DateValue, StringValue};

fn encoded(keys: &RowLayout, values: &[Value<'_>]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(keys.max_bytes);
    for (index, value) in values.iter().copied().enumerate() {
        append_value(
            &mut bytes,
            value,
            keys.columns[index].kind,
            keys.columns[index].nullable,
        )
        .unwrap();
    }
    keys.validate(&bytes).unwrap();
    bytes
}

#[test]
fn wide_rows_sort_by_key_without_losing_nonkey_payloads() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(16_000_000, 16_000_000).unwrap(),
    )
    .unwrap();
    let baseline = database.reserved_memory_bytes();
    let mut types = [DataType::Int64; MAX_COLUMNS];
    types[0] = DataType::String;
    types[1] = DataType::Double;
    types[2] = DataType::Date;
    types[4..13].fill(DataType::String);
    let text = types.map(|kind| (kind == DataType::String).then_some(crate::batch::MAX_TEXT_BYTES));
    let mut charge = database
        .reserve_memory(
            Batch::required_bytes_with_text(&types, &text).unwrap(),
            "wide sort input",
        )
        .unwrap();
    let mut batch = OwnedBatch::new_with_text(&types, &text, &mut charge).unwrap();
    drop(charge);
    let layout = RowLayout::for_join(
        types
            .into_iter()
            .enumerate()
            .map(|(index, kind)| SemanticColumn::new((index + 1) as u32, kind, true)),
        3,
    )
    .unwrap();
    assert_eq!(layout.count, MAX_COLUMNS);
    assert_eq!(layout.key_count, 1);
    let record_bytes = RECORD_HEADER + layout.max_bytes;
    assert!(record_bytes > MAX_ARGUMENT_RECORD_BYTES);
    let record_charge = database
        .reserve_memory(record_bytes as u64, "wide sorted row")
        .unwrap();
    let mut record = SortRecord::new(record_bytes, record_charge.bytes()).unwrap();
    let shape = ArgumentShape {
        count: 0,
        nonnull: 0,
        integers: 0,
    };
    let mut sort = RowSort::new(&database, &layout, shape, record_bytes, 2).unwrap();
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let mut scratch = crate::scratch::Scratch::new(&database, &cancel, &mut effects).unwrap();
    let long = "é".repeat(crate::batch::MAX_TEXT_BYTES / 2);
    let keys = [Some(2), None, Some(1), Some(2), Some(1), Some(0), Some(0)];
    let doubles = [
        -0.0,
        f64::from_bits(0x7ff8_0000_0000_0123),
        0.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        f64::MIN_POSITIVE,
    ];
    let value = |row: usize, column: usize| match column {
        3 => keys[row].map_or(Value::Null, Value::Int64),
        0 if row == 0 => Value::String(StringValue::new(&long)),
        1 => Value::Double(doubles[row]),
        2 => {
            Value::Date(DateValue::from_days([-719162, 2932896, -1, 0, 1, 100, -100][row]).unwrap())
        }
        0 | 4..=12 if row == 1 => Value::Null,
        0 | 4..=12 => Value::String(StringValue::new("雪\0payload")),
        _ if row == 2 && column.is_multiple_of(3) => Value::Null,
        _ => Value::Int64((row * 1000 + column) as i64),
    };
    for row in 0..keys.len() {
        batch.clear();
        for column in 0..MAX_COLUMNS {
            batch.set(0, column, value(row, column)).unwrap();
        }
        batch.publish_rows(1);
        record.encode_row(&layout, &batch, 0, row as u64).unwrap();
        let mut pushed = false;
        for _ in 0..4096 {
            if sort.phase == SortPhase::Collect && sort.push(&record).unwrap() {
                pushed = true;
                break;
            }
            sort.step(&layout, &mut Io::new(&mut scratch, &cancel, &mut effects))
                .unwrap();
        }
        assert!(pushed);
    }
    sort.finish().unwrap();
    for _ in 0..4096 {
        if sort
            .step(&layout, &mut Io::new(&mut scratch, &cancel, &mut effects))
            .unwrap()
            == SortPhase::Done
        {
            break;
        }
    }
    assert_eq!(sort.phase, SortPhase::Done);
    assert!(sort.runs > 1);
    assert_eq!(sort.merge.pair.previous_key.capacity(), 9);
    let run = sort.merge.result;
    let slot = sort.merge.input.slot;
    let cursor = &mut sort.merge.pair.left;
    cursor.begin(run);
    for row in [1, 5, 6, 2, 4, 0, 3] {
        cursor
            .load(
                slot,
                &layout,
                shape,
                &mut Io::new(&mut scratch, &cancel, &mut effects),
            )
            .unwrap();
        assert_eq!(cursor.record.ordinal(), row as u64);
        for stored in 0..MAX_COLUMNS {
            let original = match stored {
                0 => 3,
                3 => 0,
                other => other,
            };
            let actual = layout.value(cursor.record.key(), stored).unwrap();
            match (actual, value(row, original)) {
                (Value::Double(actual), Value::Double(expected)) => {
                    assert_eq!(actual.to_bits(), expected.to_bits())
                }
                (actual, expected) => assert_eq!(actual, expected),
            }
        }
        cursor.consume().unwrap();
    }
    assert_eq!(cursor.remaining, 0);
    cursor
        .load(
            slot,
            &layout,
            shape,
            &mut Io::new(&mut scratch, &cancel, &mut effects),
        )
        .unwrap();
    cursor.begin(run);
    cursor
        .load(
            slot,
            &layout,
            shape,
            &mut Io::new(&mut scratch, &cancel, &mut effects),
        )
        .unwrap();
    let tail = cursor.record.bytes.len() - 1;
    scratch
        .write(
            slot,
            cursor.offset + tail as u64,
            &[cursor.record.bytes[tail] ^ 1],
            &cancel,
            &mut effects,
        )
        .unwrap();
    cursor.begin(run);
    assert!(matches!(
        cursor.load(
            slot,
            &layout,
            shape,
            &mut Io::new(&mut scratch, &cancel, &mut effects)
        ),
        Err(Error::Corrupt("group argument checksum"))
    ));
    drop((batch, record, sort, scratch, record_charge));
    assert_eq!(database.reserved_memory_bytes(), baseline);
    assert_eq!(database.reserved_temp_bytes(), 0);
}

#[test]
fn argument_sort_cancellation_io_and_temporary_refusal_are_terminal() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let before = database.reserved_memory_bytes();
    let effects = interrupted_sort(&database, SortFault::None);
    for index in 0..effects {
        interrupted_sort(&database, SortFault::Io(index));
        interrupted_sort(&database, SortFault::Short(index));
    }
    for phase in [SortPhase::Collect, SortPhase::Flush, SortPhase::Merge] {
        interrupted_sort(&database, SortFault::Cancel(phase, None));
    }
    for phase in [RunPhase::Sort, RunPhase::Header, RunPhase::Write] {
        interrupted_sort(&database, SortFault::Cancel(SortPhase::Run, Some(phase)));
    }
    interrupted_sort(&database, SortFault::Temporary);
    assert_eq!(database.reserved_memory_bytes(), before);
    assert_eq!(database.reserved_temp_bytes(), 0);
}

#[derive(Clone, Copy)]
enum SortFault {
    None,
    Io(u64),
    Short(u64),
    Cancel(SortPhase, Option<RunPhase>),
    Temporary,
}

fn interrupted_sort(database: &Database, fault: SortFault) -> u64 {
    let keys = schema(&[(DataType::Int64, false)]);
    let arguments = ArgumentShape {
        count: 1,
        nonnull: 1,
        integers: 1,
    };
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let before = database.reserved_memory_bytes();
    let mut scratch = crate::scratch::Scratch::new(database, &cancel, &mut effects).unwrap();
    let mut sort = RowSort::new(database, &keys, arguments, 3 * 49, 3).unwrap();
    let charge = database.reserve_memory(49, "test record").unwrap();
    let mut record = SortRecord::new(49, charge.bytes()).unwrap();
    let pressure = if matches!(fault, SortFault::Temporary) {
        let bytes = database.config().temp_limit_bytes();
        database.temporary.reserve(bytes).unwrap();
        bytes
    } else {
        0
    };
    effects = Effects::with_faults(Faults {
        fail_at: if let SortFault::Io(index) = fault {
            Some(index)
        } else {
            None
        },
        short_at: if let SortFault::Short(index) = fault {
            Some(index)
        } else {
            None
        },
        ..Faults::default()
    });
    let mut input = 0;
    let mut terminated = false;
    for _ in 0..2000 {
        if let SortFault::Cancel(phase, run) = fault
            && sort.phase == phase
            && run.is_none_or(|run| sort.buffer.phase == run)
        {
            cancel.cancel();
        }
        let step = sort.step(&keys, &mut Io::new(&mut scratch, &cancel, &mut effects));
        match step {
            Ok(SortPhase::Collect) => {
                if input == 31 {
                    sort.finish().unwrap();
                } else {
                    record
                        .encode(
                            &keys,
                            arguments,
                            &encoded(&keys, &[Value::Int64((input * 7 % 11) as i64)]),
                            input,
                            &[Some(input)],
                        )
                        .unwrap();
                    if sort.push(&record).unwrap() {
                        input += 1;
                    }
                }
            }
            Ok(SortPhase::Done) => {
                assert!(
                    matches!(fault, SortFault::None | SortFault::Short(_)),
                    "fault must terminate the sort"
                );
                terminated = true;
                break;
            }
            Ok(_) => {}
            Err(error) => {
                match fault {
                    SortFault::None => panic!("healthy sort failed: {error:?}"),
                    SortFault::Cancel(_, _) => assert!(matches!(error, Error::Cancelled)),
                    SortFault::Temporary => assert!(matches!(error, Error::Resource { .. })),
                    SortFault::Io(_) | SortFault::Short(_) => {
                        assert!(matches!(error, Error::Io { .. }))
                    }
                }
                assert_eq!(sort.phase, SortPhase::Failed);
                let stopped_at = effects.count();
                assert!(
                    sort.step(&keys, &mut Io::new(&mut scratch, &cancel, &mut effects))
                        .is_err()
                );
                assert_eq!(effects.count(), stopped_at);
                terminated = true;
                break;
            }
        }
    }
    assert!(
        terminated,
        "sort must terminate within the finite work bound"
    );
    let count = effects.count();
    drop(record);
    drop(charge);
    drop(sort);
    drop(scratch);
    database.temporary.release(pressure);
    assert_eq!(database.reserved_memory_bytes(), before);
    assert_eq!(database.reserved_temp_bytes(), 0);
    count
}

#[test]
fn argument_sort_preserves_unsorted_records_across_row_and_byte_caps() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let keys = schema(&[(DataType::Double, true), (DataType::String, true)]);
    let arguments = ArgumentShape {
        count: 2,
        nonnull: 0,
        integers: 0,
    };
    let record_bytes = RECORD_HEADER + keys.max_bytes + 16;
    let before = database.reserved_memory_bytes();
    let values = [
        (0, Value::Null),
        (1, Value::Double(f64::from_bits(0x7ff8_0000_0000_0123))),
        (1, Value::Double(f64::from_bits(0xfff8_0000_0000_0456))),
        (2, Value::Double(f64::NEG_INFINITY)),
        (3, Value::Double(-1.0)),
        (4, Value::Double(-0.0)),
        (4, Value::Double(0.0)),
        (5, Value::Double(1.0)),
        (6, Value::Double(f64::INFINITY)),
    ];
    // This complete oracle is test-only. Production retains just one bounded
    // run arena and three I/O buffers, regardless of input row count.
    let mut expected = Vec::new();
    for ordinal in 0..123_u64 {
        let (class, number) = values[(ordinal * 7 % values.len() as u64) as usize];
        let text = if ordinal == 7 {
            Some("é".repeat(crate::batch::MAX_TEXT_BYTES / 2))
        } else if ordinal % 3 == 0 {
            None
        } else {
            Some(format!("key-{}", ordinal % 5))
        };
        let key = encoded(
            &keys,
            &[
                number,
                text.as_deref()
                    .map_or(Value::Null, |text| Value::String(StringValue::new(text))),
            ],
        );
        expected.push((class, text, ordinal, key));
    }
    let cancel = CancellationToken::new();
    for row_limit in [1, 2, 3, 17, 64] {
        for byte_limit in [record_bytes, 3 * record_bytes] {
            let mut effects = Effects::default();
            let mut scratch =
                crate::scratch::Scratch::new(&database, &cancel, &mut effects).unwrap();
            let mut sort =
                RowSort::new(&database, &keys, arguments, byte_limit, row_limit).unwrap();
            let sort_charge = sort.reservation.bytes()
                + sort.buffer.reservation.bytes()
                + sort.merge.reservation.bytes()
                + sort.merge.pair.reservation.bytes();
            assert_eq!(database.reserved_memory_bytes(), before + sort_charge);
            let record_charge = database
                .reserve_memory(record_bytes as u64, "test argument record")
                .unwrap();
            let mut record = SortRecord::new(record_bytes, record_charge.bytes()).unwrap();
            for (_, _, ordinal, key) in &expected {
                record
                    .encode(
                        &keys,
                        arguments,
                        key,
                        *ordinal,
                        &[Some(ordinal ^ 0x8000_0000_0000_0000), None],
                    )
                    .unwrap();
                if !sort.push(&record).unwrap() {
                    let retained = sort.rows;
                    for _ in 0..(row_limit * 10 + 2) {
                        if sort
                            .step(&keys, &mut Io::new(&mut scratch, &cancel, &mut effects))
                            .unwrap()
                            == SortPhase::Collect
                        {
                            break;
                        }
                    }
                    assert_eq!(sort.phase, SortPhase::Collect);
                    assert_eq!(
                        sort.rows, retained,
                        "blocked admission cannot consume the row"
                    );
                    assert!(sort.push(&record).unwrap());
                }
            }
            sort.finish().unwrap();
            for _ in 0..10_000 {
                if sort
                    .step(&keys, &mut Io::new(&mut scratch, &cancel, &mut effects))
                    .unwrap()
                    == SortPhase::Done
                {
                    break;
                }
            }
            assert_eq!(sort.phase, SortPhase::Done);
            assert_eq!(sort.buffer.phase, RunPhase::Released);
            assert_eq!(sort.buffer.bytes.capacity(), 0);
            assert_eq!(sort.buffer.spans.capacity(), 0);
            assert_eq!(sort.buffer.work.capacity(), 0);
            assert_eq!(
                database.reserved_memory_bytes(),
                before
                    + sort.reservation.bytes()
                    + size_of::<RunBuffer<'_>>() as u64
                    + sort.merge.reservation.bytes()
                    + sort.merge.pair.reservation.bytes()
                    + record_charge.bytes()
            );
            assert!(sort.runs > 1, "every cap must exercise external merging");
            let mut oracle: Vec<_> = expected.iter().collect();
            oracle.sort_by(|left, right| {
                (&left.0, &left.1, left.2).cmp(&(&right.0, &right.1, right.2))
            });
            let slot = sort.merge.input.slot;
            sort.merge.pair.left.begin(sort.merge.result);
            for (_, _, ordinal, key) in oracle {
                let cursor = &mut sort.merge.pair.left;
                cursor
                    .load(
                        slot,
                        &keys,
                        arguments,
                        &mut Io::new(&mut scratch, &cancel, &mut effects),
                    )
                    .unwrap();
                assert_eq!(cursor.record.ordinal(), *ordinal);
                assert_eq!(cursor.record.key(), key);
                assert_eq!(cursor.record.valid(), 1);
                assert_eq!(cursor.record.bits(0), ordinal ^ 0x8000_0000_0000_0000);
                assert_eq!(cursor.record.bits(1), 0);
                cursor.consume().unwrap();
            }
            sort.merge
                .pair
                .left
                .load(
                    slot,
                    &keys,
                    arguments,
                    &mut Io::new(&mut scratch, &cancel, &mut effects),
                )
                .unwrap();
            assert!(!sort.merge.pair.left.loaded);
            drop(record);
            drop(record_charge);
            drop(sort);
            drop(scratch);
            assert_eq!(database.reserved_memory_bytes(), before);
            assert_eq!(database.reserved_temp_bytes(), 0);
        }
    }
}

#[test]
fn merge_passes_reduce_odd_runs_and_fail_without_retrying_partial_output() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let before = database.reserved_memory_bytes();
    for runs in [0, 1, 2, 3, 5, 17, 65] {
        check_merge_passes(&database, runs, None);
        assert_eq!(database.reserved_memory_bytes(), before);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
    let effects = check_merge_passes(&database, 5, None);
    for fail_at in 0..effects {
        check_merge_passes(&database, 5, Some(fail_at));
        assert_eq!(database.reserved_memory_bytes(), before);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
}

fn check_merge_passes(database: &Database, runs: u32, fail_at: Option<u64>) -> u64 {
    let keys = schema(&[(DataType::Int64, false)]);
    let arguments = ArgumentShape {
        count: 1,
        nonnull: 1,
        integers: 1,
    };
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let mut scratch = crate::scratch::Scratch::new(database, &cancel, &mut effects).unwrap();
    let mut merge = MergePasses::new(database, &keys, arguments).unwrap();
    assert!(merge.reservation.bytes() >= IO_BYTES as u64);
    let record_charge = database
        .reserve_memory((RECORD_HEADER + keys.max_bytes + 8) as u64, "test record")
        .unwrap();
    let mut record =
        SortRecord::new(RECORD_HEADER + keys.max_bytes + 8, record_charge.bytes()).unwrap();
    let mut expected = Vec::new();
    const ROWS: u64 = 19;
    for index in 0..runs {
        // Independent fixture construction: each run is sorted locally, but
        // equal keys overlap every run and input ordinals interleave.
        let mut input = Vec::new();
        for row in 0..ROWS {
            let ordinal = row * u64::from(runs) + u64::from(index);
            let key = ((ordinal * 13) % 11) as i64 - 5;
            input.push((key, ordinal));
            expected.push((key, ordinal));
        }
        input.sort();
        let bytes = ROWS * (RECORD_HEADER + 9 + 8) as u64;
        let header = Run::header(RunId { pass: 0, index }, ROWS, bytes).unwrap();
        merge
            .writer
            .append(
                0,
                &header,
                &mut Io::new(&mut scratch, &cancel, &mut effects),
            )
            .unwrap();
        for (key, ordinal) in input {
            record
                .encode(
                    &keys,
                    arguments,
                    &encoded(&keys, &[Value::Int64(key)]),
                    ordinal,
                    &[Some(ordinal)],
                )
                .unwrap();
            merge
                .writer
                .append(
                    0,
                    &record.bytes,
                    &mut Io::new(&mut scratch, &cancel, &mut effects),
                )
                .unwrap();
        }
    }
    merge
        .writer
        .flush(0, &mut Io::new(&mut scratch, &cancel, &mut effects))
        .unwrap();
    merge
        .begin(
            runs,
            u64::from(runs) * ROWS,
            merge.writer.position().unwrap(),
        )
        .unwrap();
    expected.sort();
    let mut effects = Effects::with_faults(Faults {
        fail_at,
        ..Faults::default()
    });
    let mut last_pass = 0;
    let mut last_runs = runs;
    let mut completed = false;
    // One row per pair step, plus bounded per-pair and per-pass transitions.
    let step_limit = (expected.len() + 4 * runs as usize + 4) * (MAX_MERGE_PASSES as usize + 1);
    for _ in 0..step_limit {
        let result = merge.step(&keys, &mut Io::new(&mut scratch, &cancel, &mut effects));
        if let Err(error) = result {
            assert!(fail_at.is_some(), "unexpected merge failure: {error:?}");
            assert_eq!(merge.phase, MergePhase::Failed);
            let stopped_at = effects.count();
            assert!(
                merge
                    .step(&keys, &mut Io::new(&mut scratch, &cancel, &mut effects))
                    .is_err()
            );
            assert_eq!(
                effects.count(),
                stopped_at,
                "failed merge cannot repeat I/O"
            );
            return stopped_at;
        }
        let physical: u64 = (0..2)
            .map(|slot| scratch.test_file(slot).metadata().unwrap().len())
            .sum();
        assert_eq!(database.reserved_temp_bytes(), physical);
        if merge.input.pass != last_pass {
            assert_eq!(merge.input.pass, last_pass + 1);
            assert_eq!(merge.input.runs, last_runs.div_ceil(2));
            assert!(merge.input.runs < last_runs);
            last_pass = merge.input.pass;
            last_runs = merge.input.runs;
        }
        if result.unwrap() {
            completed = true;
            break;
        }
    }
    assert!(completed, "finite pass/row bound must finish");
    assert!(
        fail_at.is_none(),
        "every recorded effect must be injectable"
    );
    let completed_at = effects.count();
    assert!(
        merge
            .step(&keys, &mut Io::new(&mut scratch, &cancel, &mut effects))
            .unwrap()
    );
    assert_eq!(effects.count(), completed_at, "completed merge is inert");
    assert_eq!(
        scratch
            .test_file(merge.input.slot ^ 1)
            .metadata()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(database.reserved_temp_bytes(), merge.input.end);
    merge.pair.left.begin(merge.result);
    for (key, ordinal) in expected {
        merge
            .pair
            .left
            .load(
                merge.input.slot,
                &keys,
                arguments,
                &mut Io::new(&mut scratch, &cancel, &mut effects),
            )
            .unwrap();
        let record = &merge.pair.left.record;
        assert_eq!(keys.value(record.key(), 0).unwrap(), Value::Int64(key));
        assert_eq!(record.ordinal(), ordinal);
        assert_eq!(record.bits(0), ordinal);
        merge.pair.left.consume().unwrap();
    }
    merge
        .pair
        .left
        .load(
            merge.input.slot,
            &keys,
            arguments,
            &mut Io::new(&mut scratch, &cancel, &mut effects),
        )
        .unwrap();
    assert!(!merge.pair.left.loaded);
    completed_at
}

#[test]
fn buffered_pair_merge_preserves_records_and_checks_run_context() {
    check_pair_merge(false);
    check_pair_merge(true);
}

fn check_pair_merge(equal_keys: bool) {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let keys = schema(&[(DataType::Int64, false), (DataType::String, true)]);
    let arguments = ArgumentShape {
        count: 2,
        nonnull: 1,
        integers: 1,
    };
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let before = database.reserved_memory_bytes();
    let mut merge = PairMerge::new(&database, &keys, arguments).unwrap();
    let merge_bytes = merge.reservation.bytes();
    assert_eq!(database.reserved_memory_bytes(), before + merge_bytes);
    let charge = database
        .reserve_memory(
            (IO_BYTES + RECORD_HEADER + keys.max_bytes + 16) as u64,
            "group codec test workspace",
        )
        .unwrap();
    let mut writer = WriteBuffer::new(charge.bytes()).unwrap();
    let mut record = SortRecord::new(RECORD_HEADER + keys.max_bytes + 16, charge.bytes()).unwrap();
    let mut scratch = crate::scratch::Scratch::new(&database, &cancel, &mut effects).unwrap();
    let mut runs = [Run {
        start: 0,
        end: 0,
        rows: 0,
    }; 2];
    const ROWS: usize = 1025;
    for (side, run) in runs.iter_mut().enumerate() {
        // Input construction and the complete expected sequence are test-only
        // observers; the merge owns only its bounded cursors and I/O buffers.
        let mut body = Vec::new();
        for index in 0..ROWS {
            let value = index * 2 + side;
            let text = if !equal_keys && value == 7 {
                "x".repeat(crate::batch::MAX_TEXT_BYTES)
            } else {
                "é".repeat(65)
            };
            let key = encoded(
                &keys,
                &[
                    Value::Int64(if equal_keys { 0 } else { value as i64 }),
                    Value::String(StringValue::new(&text)),
                ],
            );
            record
                .encode(
                    &keys,
                    arguments,
                    &key,
                    value as u64,
                    &[Some(value as u64), None],
                )
                .unwrap();
            body.extend_from_slice(&record.bytes);
        }
        let header = Run::header(
            RunId {
                pass: 0,
                index: side as u32,
            },
            ROWS as u64,
            body.len() as u64,
        )
        .unwrap();
        writer
            .append(
                0,
                &header,
                &mut Io::new(&mut scratch, &cancel, &mut effects),
            )
            .unwrap();
        let start = writer.position().unwrap();
        for chunk in body.chunks(IO_BYTES) {
            writer
                .append(0, chunk, &mut Io::new(&mut scratch, &cancel, &mut effects))
                .unwrap();
        }
        *run = Run {
            start,
            end: writer.position().unwrap(),
            rows: ROWS as u64,
        };
    }
    writer
        .flush(0, &mut Io::new(&mut scratch, &cancel, &mut effects))
        .unwrap();
    let input_end = writer.position().unwrap();
    let mut read_failure = Effects::with_faults(Faults {
        fail_at: Some(0),
        ..Faults::default()
    });
    assert!(
        Run::read(
            &mut merge.left.reader,
            ReadAt {
                slot: 0,
                offset: 0,
                limit: input_end
            },
            RunId { pass: 0, index: 0 },
            &mut Io::new(&mut scratch, &cancel, &mut read_failure)
        )
        .is_err()
    );
    assert_eq!(
        merge.left.reader.buffered_bytes(),
        0,
        "failed refill cannot publish cached bytes"
    );

    assert_eq!(
        Run::read(
            &mut merge.left.reader,
            ReadAt {
                slot: 0,
                offset: 0,
                limit: input_end
            },
            RunId { pass: 0, index: 0 },
            &mut Io::new(&mut scratch, &cancel, &mut effects)
        )
        .unwrap(),
        runs[0]
    );
    assert!(
        Run::read(
            &mut merge.left.reader,
            ReadAt {
                slot: 0,
                offset: 0,
                limit: input_end
            },
            RunId { pass: 1, index: 0 },
            &mut Io::new(&mut scratch, &cancel, &mut effects)
        )
        .is_err()
    );
    writer.begin_file();
    merge
        .begin(
            runs[0],
            Some(runs[1]),
            RunId { pass: 1, index: 0 },
            &mut writer,
            0,
            &mut Io::new(&mut scratch, &cancel, &mut effects),
        )
        .unwrap();
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let cancelled_at = effects.count();
    assert!(matches!(
        merge.step(
            &mut writer,
            &keys,
            &mut Io::new(&mut scratch, &cancelled, &mut effects)
        ),
        Err(Error::Cancelled)
    ));
    assert_eq!(effects.count(), cancelled_at);
    let merge_start = effects.count();
    for row in 0..=2 * ROWS {
        assert_eq!(
            merge
                .step(
                    &mut writer,
                    &keys,
                    &mut Io::new(&mut scratch, &cancel, &mut effects)
                )
                .unwrap(),
            row == 2 * ROWS
        );
    }
    writer
        .flush(1, &mut Io::new(&mut scratch, &cancel, &mut effects))
        .unwrap();
    assert!(
        effects.count() - merge_start < (ROWS / 8) as u64,
        "small records share physical I/O"
    );
    let output_end = writer.position().unwrap();
    merge.left.reader.clear();
    let output = Run::read(
        &mut merge.left.reader,
        ReadAt {
            slot: 1,
            offset: 0,
            limit: output_end,
        },
        RunId { pass: 1, index: 0 },
        &mut Io::new(&mut scratch, &cancel, &mut effects),
    )
    .unwrap();
    for wrong in [
        ArgumentShape {
            integers: arguments.integers ^ 1,
            ..arguments
        },
        ArgumentShape {
            nonnull: arguments.nonnull ^ 1,
            ..arguments
        },
        ArgumentShape {
            count: MAX_AGGREGATE_COLUMNS + 1,
            ..arguments
        },
    ] {
        assert!(
            matches!(
                record.read(
                    &mut merge.left.reader,
                    ReadAt {
                        slot: 1,
                        offset: output.start,
                        limit: output.end
                    },
                    &keys,
                    wrong,
                    &mut Io::new(&mut scratch, &cancel, &mut effects),
                ),
                Err(Error::Corrupt(_))
            ),
            "a checksum-valid frame cannot change argument types"
        );
    }
    merge.left.begin(Some(output));
    for value in 0..2 * ROWS {
        merge
            .left
            .load(
                1,
                &keys,
                arguments,
                &mut Io::new(&mut scratch, &cancel, &mut effects),
            )
            .unwrap();
        let row = &merge.left.record;
        assert_eq!(row.ordinal(), value as u64);
        assert_eq!(row.valid(), 1);
        assert_eq!(row.bits(0), value as u64);
        assert_eq!(row.bits(1), 0);
        assert_eq!(
            keys.value(row.key(), 0).unwrap(),
            Value::Int64(if equal_keys { 0 } else { value as i64 })
        );
        merge.left.consume().unwrap();
    }
    merge
        .left
        .load(
            1,
            &keys,
            arguments,
            &mut Io::new(&mut scratch, &cancel, &mut effects),
        )
        .unwrap();
    assert!(!merge.left.loaded);
    // A run boundary is checked independently of physical file extent.
    assert!(
        record
            .read(
                &mut merge.left.reader,
                ReadAt {
                    slot: 1,
                    offset: output.start,
                    limit: output.start + RECORD_HEADER as u64
                },
                &keys,
                arguments,
                &mut Io::new(&mut scratch, &cancel, &mut effects)
            )
            .is_err()
    );
    merge.left.reader.clear();
    record
        .read(
            &mut merge.left.reader,
            ReadAt {
                slot: 1,
                offset: output.start,
                limit: output.end,
            },
            &keys,
            arguments,
            &mut Io::new(&mut scratch, &cancel, &mut effects),
        )
        .unwrap();
    record.bytes[24..26].fill(0);
    record.bytes[28..32].fill(0);
    let crc = storage_format::crc32c(&record.bytes);
    record.bytes[28..32].copy_from_slice(&crc.to_le_bytes());
    scratch
        .write(1, output.start, &record.bytes, &cancel, &mut effects)
        .unwrap();
    merge.left.begin(Some(output));
    assert!(matches!(
        merge.left.load(
            1,
            &keys,
            arguments,
            &mut Io::new(&mut scratch, &cancel, &mut effects)
        ),
        Err(Error::Corrupt("required group argument is NULL"))
    ));
    scratch
        .write(
            1,
            output.start + RECORD_HEADER as u64 + 1,
            &[0xff],
            &cancel,
            &mut effects,
        )
        .unwrap();
    merge.left.begin(Some(output));
    assert!(matches!(
        merge.left.load(
            1,
            &keys,
            arguments,
            &mut Io::new(&mut scratch, &cancel, &mut effects)
        ),
        Err(Error::Corrupt("group argument checksum"))
    ));
    // An odd final run is copied through the same merge transition.
    writer.begin_file();
    merge
        .begin(
            runs[0],
            None,
            RunId { pass: 1, index: 0 },
            &mut writer,
            0,
            &mut Io::new(&mut scratch, &cancel, &mut effects),
        )
        .unwrap();
    for row in 0..=ROWS {
        assert_eq!(
            merge
                .step(
                    &mut writer,
                    &keys,
                    &mut Io::new(&mut scratch, &cancel, &mut effects)
                )
                .unwrap(),
            row == ROWS
        );
    }
    writer
        .flush(1, &mut Io::new(&mut scratch, &cancel, &mut effects))
        .unwrap();
    let single = Run::read(
        &mut merge.left.reader,
        ReadAt {
            slot: 1,
            offset: 0,
            limit: writer.position().unwrap(),
        },
        RunId { pass: 1, index: 0 },
        &mut Io::new(&mut scratch, &cancel, &mut effects),
    )
    .unwrap();
    assert_eq!(single.rows, ROWS as u64);
    assert_eq!(single.end - single.start, runs[0].end - runs[0].start);
    drop(scratch);
    assert_eq!(database.reserved_temp_bytes(), 0);
    drop(record);
    drop(writer);
    drop(charge);
    drop(merge);
    assert_eq!(database.reserved_memory_bytes(), before);
    for extra in [0, 1] {
        let pressure = database
            .reserve_memory(
                database.config().memory_limit_bytes() - before - merge_bytes + extra,
                "merge minimum pressure",
            )
            .unwrap();
        let admitted = PairMerge::new(&database, &keys, arguments);
        if extra == 0 {
            drop(admitted.unwrap());
        } else {
            assert!(matches!(admitted, Err(Error::Resource { .. })));
        }
        drop(pressure);
        assert_eq!(database.reserved_memory_bytes(), before);
    }
}
