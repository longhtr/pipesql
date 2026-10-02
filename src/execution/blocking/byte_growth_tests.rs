//! Check optional run growth, replacement ownership and complete stable rows.
//!
//! Expected rows use independently sorted source values and original ordinals.
//! The production encoder supplies inputs; allocation campaigns separately
//! observe physical allocation and native storage.

use super::test_support::{Directory, schema};
use super::*;
use crate::value::{DataType, StringValue};

#[test]
fn byte_growth_skips_nearly_full_runs_and_waits_for_the_last_write() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(1_000_000, 1_000_000).unwrap(),
    )
    .unwrap();
    let baseline = db.reserved_memory_bytes();
    for (rows, target) in [(1, Some(65_528)), (4, Some(16_384)), (5, None), (8, None)] {
        let mut run = RunBuffer::new(&db, 8192, 8).unwrap();
        // The policy reads counts and lengths before the last write clears
        // them. Payload decoding and stable output have separate checks below.
        run.bytes.resize(8191, 0);
        for row in 0..rows {
            run.spans.push(RecordSpan {
                start: row * 8191 / rows,
                end: (row + 1) * 8191 / rows,
            });
        }
        let memory = db.reserved_memory_bytes();
        run.phase = RunPhase::Write;
        run.output = rows - 1;
        assert_eq!(
            run.growth_after_write().unwrap(),
            target.map(RunGrowth::Bytes)
        );
        if rows > 1 {
            run.output -= 1;
            assert_eq!(run.growth_after_write().unwrap(), None);
        }
        for phase in [
            RunPhase::Collect,
            RunPhase::Sort,
            RunPhase::Header,
            RunPhase::Done,
        ] {
            run.phase = phase;
            run.output = rows - 1;
            assert_eq!(run.growth_after_write().unwrap(), None);
        }
        assert_eq!(db.reserved_memory_bytes(), memory);
        drop(run);
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
}

#[test]
fn row_growth_requires_useful_slots_and_a_complete_replacement() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let baseline = db.reserved_memory_bytes();
    for (limit, used, expected) in [
        (2048, 2048, None),
        (4096, 2048, Some(RunGrowth::Bytes(167_936))),
        (
            4096,
            2049,
            Some(RunGrowth::Rows {
                bytes: 335_872,
                rows: 8192,
            }),
        ),
        (
            8192,
            8192,
            Some(RunGrowth::Rows {
                bytes: 671_744,
                rows: 16_384,
            }),
        ),
        (16_384, 16_384, None),
    ] {
        let mut run = RunBuffer::new(&db, used * 41, limit).unwrap();
        run.bytes.resize(used * 41, 0);
        for index in 0..used {
            run.spans.push(RecordSpan {
                start: index * 41,
                end: (index + 1) * 41,
            });
        }
        run.phase = RunPhase::Write;
        run.output = used - 1;
        assert_eq!(run.growth_after_write().unwrap(), expected);
        run.output -= 1;
        assert_eq!(run.growth_after_write().unwrap(), None);
        run.output += 1;
        run.phase = RunPhase::Sort;
        assert_eq!(run.growth_after_write().unwrap(), None);
        drop(run);
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    // Short observed rows must not discard space reserved for a later maximum
    // record. Row growth preserves that original byte floor.
    {
        let mut run = RunBuffer::new(&db, 1_000_000, 4096).unwrap();
        run.bytes.resize(4096 * 41, 0);
        for index in 0..4096 {
            run.spans.push(RecordSpan {
                start: index * 41,
                end: (index + 1) * 41,
            });
        }
        run.phase = RunPhase::Write;
        run.output = 4095;
        assert_eq!(
            run.growth_after_write().unwrap(),
            Some(RunGrowth::Rows {
                bytes: 1_000_000,
                rows: 8192
            })
        );
    }
    assert_eq!(db.reserved_memory_bytes(), baseline);
    let replacement = run_allocation_bytes(335_872, 8192).unwrap() as u64;
    for transferred in [false, true] {
        for available in [0, 1, replacement - 1, replacement] {
            let mut run = RunBuffer::new(&db, 135_176, 4096).unwrap();
            let mut runtime = db.reserve_memory(0, "inline owner").unwrap();
            if transferred {
                run.reservation
                    .transfer_to(&mut runtime, size_of::<RunBuffer<'_>>() as u64)
                    .unwrap();
            }
            let before = db.reserved_memory_bytes();
            let pressure = db
                .reserve_memory(4_000_000 - before - available, "other owners")
                .unwrap();
            let pointers = (run.bytes.as_ptr(), run.spans.as_ptr(), run.work.as_ptr());
            let cancelled = CancellationToken::new();
            cancelled.cancel();
            assert!(matches!(
                run.replace_empty(335_872, 8192, &cancelled),
                Err(Error::Cancelled)
            ));
            assert_eq!(db.reserved_memory_bytes(), before + pressure.bytes());
            assert_eq!(
                (run.bytes.as_ptr(), run.spans.as_ptr(), run.work.as_ptr()),
                pointers
            );
            let result = run.replace_empty(335_872, 8192, &CancellationToken::new());
            if available < replacement {
                assert!(matches!(result, Err(Error::Resource { .. })));
                assert_eq!((run.byte_limit, run.row_limit), (135_176, 4096));
                assert_eq!(
                    (run.bytes.as_ptr(), run.spans.as_ptr(), run.work.as_ptr()),
                    pointers
                );
                assert_eq!(db.reserved_memory_bytes(), before + pressure.bytes());
            } else {
                result.unwrap();
                assert_eq!((run.byte_limit, run.row_limit), (335_872, 8192));
                assert_eq!(
                    db.reserved_memory_bytes(),
                    before + pressure.bytes() + replacement
                        - run_allocation_bytes(135_176, 4096).unwrap() as u64
                );
            }
            assert_eq!(run.phase, RunPhase::Collect);
            assert!(run.bytes.is_empty() && run.spans.is_empty() && run.work.is_empty());
            drop((run, runtime, pressure));
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
    }
}

#[test]
fn empty_byte_growth_preserves_rows_accounts_and_cancelled_buffers() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let baseline = db.reserved_memory_bytes();
    for transferred in [false, true] {
        for available in [0, 1, 32_768, 262_144, 1_000_000] {
            let mut run = RunBuffer::new(&db, 8_456, 256).unwrap();
            let mut runtime = db.reserve_memory(0, "inline owner").unwrap();
            if transferred {
                run.reservation
                    .transfer_to(&mut runtime, size_of::<RunBuffer<'_>>() as u64)
                    .unwrap();
            }
            let before = db.reserved_memory_bytes();
            let pressure = db
                .reserve_memory(4_000_000 - before - available, "other owners")
                .unwrap();
            let pointers = (run.bytes.as_ptr(), run.spans.as_ptr(), run.work.as_ptr());
            let cancel = CancellationToken::new();
            cancel.cancel();
            assert!(matches!(
                run.grow_bytes_empty(131_072, &cancel),
                Err(Error::Cancelled)
            ));
            assert_eq!(db.reserved_memory_bytes(), before + pressure.bytes());
            assert_eq!(run.bytes.as_ptr(), pointers.0);
            run.grow_bytes_empty(131_072, &CancellationToken::new())
                .unwrap();
            assert_eq!(run.row_limit, 256);
            assert_eq!(
                (run.spans.as_ptr(), run.work.as_ptr()),
                (pointers.1, pointers.2)
            );
            if available <= 1 {
                assert_eq!(run.bytes.as_ptr(), pointers.0);
                assert_eq!(run.byte_limit, 8_456);
            } else if available == 32_768 {
                assert!(run.byte_limit > 8_456 && run.byte_limit < 131_072);
            } else if available == 262_144 && cfg!(target_os = "macos") {
                // The next 16-KiB allocation class would need more than the
                // available bytes under Darwin's large-cache charge.
                assert_eq!(run.byte_limit, 131_040);
            } else {
                assert_eq!(run.byte_limit, 131_072);
            }
            assert_eq!(
                db.reserved_memory_bytes(),
                before + pressure.bytes() + buffer_memory_bytes(run.byte_limit).unwrap() as u64
                    - buffer_memory_bytes(8_456).unwrap() as u64
            );
            assert_eq!(run.phase, RunPhase::Collect);
            assert!(run.bytes.is_empty() && run.spans.is_empty() && run.work.is_empty());
            run.release();
            drop((run, runtime));
            assert_eq!(db.reserved_memory_bytes(), baseline + pressure.bytes());
            drop(pressure);
        }
    }
    assert_eq!(db.reserved_memory_bytes(), baseline);
}

fn step(
    input: &mut SortedInput<'_>,
    grow: bool,
    cancel: &CancellationToken,
    effects: &mut Effects,
) {
    if grow {
        input.sort_step(cancel, effects).unwrap();
    } else {
        input
            .sort
            .step(&input.layout, &mut input.files.io(cancel, effects).unwrap())
            .unwrap();
    }
}

#[test]
fn row_growth_keeps_later_maximum_records_and_complete_nullable_payloads() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(32_000_000, 64_000_000).unwrap(),
    )
    .unwrap();
    let baseline = db.reserved_memory_bytes();
    let wide = "w".repeat(crate::batch::MAX_TEXT_BYTES);
    let text = |ordinal: usize, column: usize| {
        if ordinal == 5000 {
            Some(wide.as_str())
        } else if column == 0 && ordinal.is_multiple_of(17) {
            Some("x")
        } else if column == 1 && ordinal.is_multiple_of(31) {
            Some("雪\0")
        } else {
            None
        }
    };
    // The first two columns determine ordinary-row order; all later columns
    // are NULL. The one wide row has a distinct first key. Rust tuple order
    // gives the complete expected relation independently of the sorter.
    let mut expected: Vec<_> = (0..9000).collect();
    expected.sort_by_key(|&ordinal| (text(ordinal, 0), text(ordinal, 1), ordinal));
    for grow in [false, true] {
        let mut input = SortedInput::new(&db, schema(&[(DataType::String, true); 16])).unwrap();
        let cancel = CancellationToken::new();
        let mut effects = Effects::default();
        input.start(&cancel, &mut effects).unwrap();
        let mut steps = 0;
        for ordinal in 0..9000 {
            if ordinal == 5000 {
                // The maximum record arrives after row growth. Shrinking the
                // arena to observed short widths would make this row fail.
                assert_eq!(input.sort.buffer.row_limit, if grow { 8192 } else { 4096 });
            }
            let mut key = Vec::with_capacity(input.layout.max_bytes);
            for column in 0..16 {
                let value = text(ordinal, column)
                    .map_or(Value::Null, |s| Value::String(StringValue::new(s)));
                append_value(&mut key, value, DataType::String, true).unwrap();
            }
            input
                .record
                .encode(&input.layout, ROW_ARGUMENTS, &key, ordinal as u64, &[])
                .unwrap();
            if !input.sort.push(&input.record).unwrap() {
                while input.sort.phase() != SortPhase::Collect {
                    assert!(steps < 2_000_000);
                    step(&mut input, grow, &cancel, &mut effects);
                    steps += 1;
                }
                assert!(input.sort.push(&input.record).unwrap());
            }
        }
        input.sort.finish().unwrap();
        while input.sort.phase() != SortPhase::Done {
            assert!(steps < 2_000_000);
            step(&mut input, grow, &cancel, &mut effects);
            steps += 1;
        }
        input.begin_read();
        for &ordinal in &expected {
            assert!(input.load(&cancel, &mut effects).unwrap());
            assert_eq!(
                input.sort.sorted_cursor().record().unwrap().ordinal(),
                ordinal as u64
            );
            let mut values = [Value::Null; 16];
            input.read_values(&mut values).unwrap();
            for (column, actual) in values.into_iter().enumerate() {
                let expected = text(ordinal, column)
                    .map_or(Value::Null, |s| Value::String(StringValue::new(s)));
                assert_eq!(actual, expected, "row={ordinal} column={column}");
            }
            input.consume().unwrap();
        }
        assert!(input.finished());
        drop(input);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}

#[test]
fn byte_growth_reduces_wide_runs_and_preserves_complete_stable_rows() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(12_000_000, 128_000_000).unwrap(),
    )
    .unwrap();
    let baseline = db.reserved_memory_bytes();
    for (rows, width, mixed) in [
        (0, 8, false),
        (8192, 8, false),
        (1024, 1024, false),
        (4096, 1024, false),
        (96, crate::batch::MAX_TEXT_BYTES, false),
        (512, 4096, true),
    ] {
        let input_rows: Vec<_> = (0_usize..rows)
            .map(|row| {
                let width = if mixed && row < rows / 4 { 8 } else { width };
                (!row.is_multiple_of(17))
                    .then(|| format!("{}{:08}", "x".repeat(width - 8), row % 13))
            })
            .collect();
        let mut expected: Vec<_> = input_rows.iter().enumerate().collect();
        expected.sort_by(|(a, x), (b, y)| x.cmp(y).then(a.cmp(b)));
        let mut runs = [0; 2];
        for grow in [false, true] {
            let mut input = SortedInput::new(&db, schema(&[(DataType::String, true)])).unwrap();
            let cancel = CancellationToken::new();
            let mut effects = Effects::default();
            input.start(&cancel, &mut effects).unwrap();
            let mut steps = 0;
            for (ordinal, text) in input_rows.iter().enumerate() {
                let value = text
                    .as_deref()
                    .map_or(Value::Null, |text| Value::String(StringValue::new(text)));
                let mut key = Vec::with_capacity(input.layout.max_bytes);
                append_value(&mut key, value, DataType::String, true).unwrap();
                input
                    .record
                    .encode(&input.layout, ROW_ARGUMENTS, &key, ordinal as u64, &[])
                    .unwrap();
                if !input.sort.push(&input.record).unwrap() {
                    while input.sort.phase() != SortPhase::Collect {
                        assert!(steps < 1_000_000);
                        step(&mut input, grow, &cancel, &mut effects);
                        steps += 1;
                    }
                    assert!(input.sort.push(&input.record).unwrap());
                }
            }
            input.sort.finish().unwrap();
            while input.sort.phase() != SortPhase::Done {
                assert!(steps < 1_000_000);
                step(&mut input, grow, &cancel, &mut effects);
                steps += 1;
            }
            runs[usize::from(grow)] = input.sort.runs;
            input.begin_read();
            for (ordinal, text) in &expected {
                assert!(input.load(&cancel, &mut effects).unwrap());
                let record = input.sort.sorted_cursor().record().unwrap();
                assert_eq!(record.ordinal(), *ordinal as u64);
                assert_eq!(
                    input.layout.value(record.key(), 0).unwrap(),
                    text.as_deref()
                        .map_or(Value::Null, |text| Value::String(StringValue::new(text)))
                );
                input.consume().unwrap();
            }
            assert!(input.finished());
            drop(input);
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
        if width > 8 {
            assert!(runs[1] < runs[0], "rows={rows} width={width}: {runs:?}");
        } else {
            assert!(runs[1] <= runs[0]);
        }
    }
}

#[test]
fn competing_byte_replacements_keep_the_losing_arena_and_shared_limit() {
    let directory = Directory::new();
    let limit = 1_000_000;
    let db = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(limit, 4_000_000).unwrap(),
    )
    .unwrap();
    let baseline = db.reserved_memory_bytes();
    let runs = [
        RunBuffer::new(&db, 8_456, 256).unwrap(),
        RunBuffer::new(&db, 8_456, 256).unwrap(),
    ];
    let replacement = buffer_memory_bytes(131_072).unwrap() as u64;
    let pressure = db
        .reserve_memory(
            limit - db.reserved_memory_bytes() - replacement,
            "other owners",
        )
        .unwrap();
    std::thread::scope(|scope| {
        let (finished, done) = std::sync::mpsc::channel();
        let workers = runs.map(|mut run| {
            let (start, begin) = std::sync::mpsc::channel();
            let (release, released) = std::sync::mpsc::channel();
            let finished = finished.clone();
            let worker = scope.spawn(move || {
                let pointer = run.bytes.as_ptr();
                if begin.recv().is_err() {
                    return None;
                }
                let outcome = run.grow_bytes_empty(131_072, &CancellationToken::new());
                let unchanged = run.bytes.as_ptr() == pointer && run.byte_limit == 8_456;
                let grew = run.byte_limit > 8_456;
                let collecting = run.phase == RunPhase::Collect;
                let _ = finished.send(());
                // Parent failure drops release before the scope joins workers.
                let _ = released.recv();
                run.release();
                Some((outcome, unchanged, grew, collecting))
            });
            (worker, start, release)
        });
        drop(finished);
        for (_, start, _) in &workers {
            start.send(()).unwrap();
        }
        for _ in 0..2 {
            done.recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
        }
        assert!(db.reserved_memory_bytes() <= limit);
        for (_, _, release) in &workers {
            release.send(()).unwrap();
        }
        let mut grown = 0;
        for (worker, _, _) in workers {
            let (outcome, unchanged, grew, collecting) = worker.join().unwrap().unwrap();
            match outcome {
                Ok(()) => (),
                Err(Error::Resource { .. } | Error::Contention(_)) => assert!(unchanged),
                Err(error) => panic!("unexpected optional replacement failure: {error}"),
            }
            assert!(collecting);
            grown += usize::from(grew);
        }
        assert!(grown <= 1);
    });
    assert_eq!(db.reserved_memory_bytes(), baseline + pressure.bytes());
    drop(pressure);
    assert_eq!(db.reserved_memory_bytes(), baseline);
}

fn encode_growth_row(input: &mut SortedInput<'_>, row: (Option<i64>, i64), ordinal: usize) {
    let mut key = Vec::with_capacity(input.layout.max_bytes);
    append_value(
        &mut key,
        row.0.map_or(Value::Null, Value::Int64),
        DataType::Int64,
        true,
    )
    .unwrap();
    append_value(&mut key, Value::Int64(row.1), DataType::Int64, false).unwrap();
    input
        .record
        .encode(&input.layout, ROW_ARGUMENTS, &key, ordinal as u64, &[])
        .unwrap();
}

#[test]
fn competing_row_replacements_preserve_answers_cancellation_and_healthy_retry() {
    let directory = Directory::new();
    let limit = 12_000_000;
    let db = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(limit, 16_000_000).unwrap(),
    )
    .unwrap();
    let baseline = db.reserved_memory_bytes();
    for cancelled in [None, Some(0), Some(1)] {
        let sources: [Vec<_>; 2] = std::array::from_fn(|side| {
            (0..16_384)
                .map(|ordinal| {
                    let key = (!(ordinal + side).is_multiple_of(17))
                        .then_some(-(((ordinal * 7 + side) % 31) as i64));
                    (key, side as i64 - 37)
                })
                .collect()
        });
        let mut inputs = Vec::new();
        for rows in &sources {
            let mut input = SortedInput::new(
                &db,
                schema(&[(DataType::Int64, true), (DataType::Int64, false)]),
            )
            .unwrap();
            let cancel = CancellationToken::new();
            let mut effects = Effects::default();
            input.start(&cancel, &mut effects).unwrap();
            assert_eq!(input.sort.buffer.row_limit, 4096);
            let mut pending = None;
            for (ordinal, &row) in rows.iter().enumerate() {
                encode_growth_row(&mut input, row, ordinal);
                if !input.sort.push(&input.record).unwrap() {
                    pending = Some(ordinal);
                    break;
                }
            }
            let pending = pending.expect("the first run must spill");
            let mut steps = 0;
            while input.sort.buffer.phase != RunPhase::Write
                || input.sort.buffer.output + 1 != input.sort.buffer.spans.len()
            {
                assert!(steps < 100_000);
                input.sort_step(&cancel, &mut effects).unwrap();
                steps += 1;
            }
            let Some(RunGrowth::Rows { bytes, rows }) =
                input.sort.buffer.growth_after_write().unwrap()
            else {
                panic!("the nonfinal run must be eligible for row growth");
            };
            assert_eq!(rows, 8192);
            inputs.push((
                input,
                cancel,
                pending,
                run_allocation_bytes(bytes, rows).unwrap() as u64,
            ));
        }
        assert_eq!(inputs[0].3, inputs[1].3);
        let pressure = db
            .reserve_memory(
                limit - db.reserved_memory_bytes() - inputs[0].3,
                "competing run pressure",
            )
            .unwrap();
        std::thread::scope(|scope| {
            let (finished, done) = std::sync::mpsc::channel();
            let workers: Vec<_> = inputs
                .into_iter()
                .zip(sources)
                .enumerate()
                .map(|(side, ((mut input, cancel, pending, _), rows))| {
                    let (advance, resume) = std::sync::mpsc::channel();
                    let finished = finished.clone();
                    let worker = scope.spawn(move || {
                        let mut effects = Effects::default();
                        let pointers = (
                            input.sort.buffer.bytes.as_ptr(),
                            input.sort.buffer.spans.as_ptr(),
                            input.sort.buffer.work.as_ptr(),
                        );
                        let bytes = input.sort.buffer.byte_limit;
                        if resume.recv().is_err() {
                            return;
                        }
                        input.sort_step(&cancel, &mut effects).unwrap();
                        assert_eq!(input.sort.phase(), SortPhase::Collect);
                        let grew = input.sort.buffer.row_limit == 8192;
                        if !grew {
                            assert_eq!(input.sort.buffer.row_limit, 4096);
                            assert_eq!(input.sort.buffer.byte_limit, bytes);
                            assert_eq!(
                                (
                                    input.sort.buffer.bytes.as_ptr(),
                                    input.sort.buffer.spans.as_ptr(),
                                    input.sort.buffer.work.as_ptr(),
                                ),
                                pointers
                            );
                        }
                        // The caller's rejected row is still retained. Admit it
                        // before pressure is released, even when replacement lost.
                        assert!(input.sort.push(&input.record).unwrap());
                        let _ = finished.send(grew);
                        // Parent failure disconnects resume before scope joins.
                        if resume.recv().is_err() {
                            return;
                        }
                        if cancelled == Some(side) {
                            cancel.cancel();
                            let before = effects.count();
                            assert!(matches!(
                                input.sort_step(&cancel, &mut effects),
                                Err(Error::Cancelled)
                            ));
                            assert!(matches!(
                                input.sort_step(&CancellationToken::new(), &mut effects),
                                Err(Error::Corrupt(_))
                            ));
                            assert_eq!(effects.count(), before);
                            return;
                        }
                        let mut steps = 0;
                        for (ordinal, &row) in rows.iter().enumerate().skip(pending + 1) {
                            encode_growth_row(&mut input, row, ordinal);
                            if !input.sort.push(&input.record).unwrap() {
                                while input.sort.phase() != SortPhase::Collect {
                                    assert!(steps < 2_000_000);
                                    input.sort_step(&cancel, &mut effects).unwrap();
                                    steps += 1;
                                }
                                assert!(input.sort.push(&input.record).unwrap());
                            }
                        }
                        input.sort.finish().unwrap();
                        while input.sort.phase() != SortPhase::Done {
                            assert!(steps < 2_000_000);
                            input.sort_step(&cancel, &mut effects).unwrap();
                            steps += 1;
                        }
                        // Optional contention may refuse both racing attempts.
                        // Once pressure is gone, the healthy continuation must
                        // exercise growth rather than silently skipping that path.
                        assert!(input.sort.buffer.row_limit >= 8192);
                        let mut expected: Vec<_> = rows.into_iter().enumerate().collect();
                        expected.sort_by_key(|&(ordinal, (key, payload))| (key, payload, ordinal));
                        input.begin_read();
                        for (ordinal, (key, payload)) in expected {
                            assert!(input.load(&cancel, &mut effects).unwrap());
                            let record = input.sort.sorted_cursor().record().unwrap();
                            assert_eq!(record.ordinal(), ordinal as u64);
                            assert_eq!(
                                input.layout.value(record.key(), 0).unwrap(),
                                key.map_or(Value::Null, Value::Int64)
                            );
                            assert_eq!(
                                input.layout.value(record.key(), 1).unwrap(),
                                Value::Int64(payload)
                            );
                            input.consume().unwrap();
                        }
                        assert!(input.finished());
                    });
                    (worker, advance)
                })
                .collect();
            drop(finished);
            for (_, advance) in &workers {
                advance.send(()).unwrap();
            }
            let mut grown = 0;
            for _ in 0..2 {
                grown += usize::from(
                    done.recv_timeout(std::time::Duration::from_secs(10))
                        .unwrap(),
                );
            }
            assert!(grown <= 1);
            assert!(db.reserved_memory_bytes() <= limit);
            drop(pressure);
            for (_, advance) in &workers {
                advance.send(()).unwrap();
            }
            for (worker, _) in workers {
                worker.join().unwrap();
            }
        });
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    db.close().unwrap();
}
