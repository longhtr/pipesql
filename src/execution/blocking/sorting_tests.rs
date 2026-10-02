//! Test temporary row encoding and external merging directly, without SQL.
//!
//! Small row and byte limits force spills, overlapping runs and odd merge passes.
//! Expected order comes from literal floating-point classes and Rust tuple order;
//! payload checks preserve original bytes, NULLs and argument values. These tests
//! use the production encoder, so they are not independent format-decoder evidence.
//!
//! Malformed frames with valid checksums test length, type and NULL validation.
//! Separate mutations test checksum rejection. Allocation tests distinguish an
//! admitted record or run limit from extra capacity added by allocation rounding.
//!
//! Fault tests interrupt recorded effects, cancel selected sorting phases and
//! exhaust temporary space. Failed sort controllers must stop I/O; dropping their
//! buffers and scratch files must release the charges. PairMerge is also tested
//! alone, where the caller is responsible for stopping after a failure.

use super::test_support::{Directory, schema};
use super::*;
use crate::batch::Batch;
use crate::batch::OwnedBatch;
use crate::effects::Faults;
use crate::query::SemanticColumn;
use crate::value::DataType;
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
fn sorted_payload_fills_mapped_cells_and_rejects_incomplete_rows() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(32_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let baseline = database.reserved_memory_bytes();
    for width in [4, MAX_ROW_VALUES] {
        let mut types = vec![(DataType::Int64, true); width];
        types[0].0 = DataType::String;
        types[1].0 = DataType::Double;
        types[2].0 = DataType::Date;
        let mut layout = schema(&types);
        layout.columns.swap(0, width - 1);
        let mut input = SortedInput::new(&database, layout).unwrap();
        let expected = |column| match column {
            0 => Value::String(StringValue::new("雪\0payload")),
            1 => Value::Double(f64::from_bits(0x7ff8_0000_0000_0123)),
            2 => Value::Date(DateValue::from_days(-719162).unwrap()),
            3 => Value::Null,
            _ => Value::Int64(-(column as i64)),
        };
        {
            let mut values = [Value::Null; MAX_ROW_VALUES];
            assert!(matches!(
                input.read_values(&mut values[..width]),
                Err(Error::Corrupt("sorted payload requires a loaded row"))
            ));
        }
        let mut bytes = Vec::with_capacity(input.layout.max_bytes);
        for field in &input.layout.columns[..width] {
            append_value(
                &mut bytes,
                expected(field.input),
                field.kind,
                field.nullable,
            )
            .unwrap();
        }
        // Install an already loaded record to isolate payload decoding from
        // frame validation. The normal read path checks the frame first.
        input
            .sort
            .merge
            .pair
            .left
            .record
            .encode(&input.layout, ROW_ARGUMENTS, &bytes, 0, &[])
            .unwrap();
        input.sort.merge.pair.left.loaded = true;
        {
            let mut values = [Value::Int64(999); MAX_ROW_VALUES];
            input.read_values(&mut values[..width]).unwrap();
            for (column, actual) in values[..width].iter().copied().enumerate() {
                match (actual, expected(column)) {
                    (Value::Double(actual), Value::Double(expected)) => {
                        assert_eq!(actual.to_bits(), expected.to_bits());
                    }
                    (actual, expected) => assert_eq!(actual, expected),
                }
            }
            assert!(values[width..].iter().all(|v| *v == Value::Int64(999)));
        }
        let original = input.sort.merge.pair.left.record.bytes.clone();
        for trailing in [false, true] {
            let record = &mut input.sort.merge.pair.left.record;
            record.bytes.clone_from(&original);
            let length = bytes.len();
            if trailing {
                record.bytes.insert(RECORD_HEADER + length, 0);
                record.bytes[16..20].copy_from_slice(&((length + 1) as u32).to_le_bytes());
            } else {
                record.bytes[16..20].copy_from_slice(&((length - 1) as u32).to_le_bytes());
            }
            let mut values = [Value::Null; MAX_ROW_VALUES];
            let error = input.read_values(&mut values[..width]).unwrap_err();
            assert!(matches!(error, Error::Corrupt(_)));
            if trailing {
                assert!(matches!(error, Error::Corrupt("sorted row trailing bytes")));
            }
        }
        drop(input);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
    database.close().unwrap();
}

#[test]
fn optional_run_growth_preserves_minimum_buffers_under_pressure() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let baseline = database.reserved_memory_bytes();
    for transferred in [false, true] {
        for available in [0, 1, 65_536, 1_000_000] {
            let mut run = RunBuffer::new(&database, 8_456, 256).unwrap();
            let mut runtime = database.reserve_memory(0, "inline test owner").unwrap();
            if transferred {
                run.reservation
                    .transfer_to(&mut runtime, size_of::<RunBuffer<'_>>() as u64)
                    .unwrap();
            }
            let pointers = (run.bytes.as_ptr(), run.spans.as_ptr(), run.work.as_ptr());
            let initial = database.reserved_memory_bytes();
            let pressure = database
                .reserve_memory(4_000_000 - initial - available, "other admitted owners")
                .unwrap();
            run.grow_empty(33, &CancellationToken::new()).unwrap();
            if available <= 1 {
                assert_eq!((run.byte_limit, run.row_limit), (8_456, 256));
                assert_eq!(
                    pointers,
                    (run.bytes.as_ptr(), run.spans.as_ptr(), run.work.as_ptr())
                );
                assert_eq!(database.reserved_memory_bytes(), initial + pressure.bytes());
            } else if available == 65_536 {
                assert!(run.row_limit > 256 && run.row_limit < 4096);
            } else {
                assert_eq!((run.byte_limit, run.row_limit), (135_176, 4096));
            }
            assert_eq!(run.phase, RunPhase::Collect);
            assert!(run.bytes.is_empty() && run.spans.is_empty() && run.work.is_empty());
            assert!(database.reserved_memory_bytes() <= 4_000_000);
            run.release();
            assert_eq!(
                database.reserved_memory_bytes(),
                baseline + pressure.bytes() + size_of::<RunBuffer<'_>>() as u64
            );
            drop((run, runtime));
            assert_eq!(
                database.reserved_memory_bytes(),
                baseline + pressure.bytes()
            );
            drop(pressure);
            assert_eq!(database.reserved_memory_bytes(), baseline);
        }
    }
    database.close().unwrap();
}

#[test]
fn optional_run_growth_reduces_merges_and_preserves_stable_records() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let baseline = database.reserved_memory_bytes();
    for rows in [1024, 8192, 16384, 32768] {
        for grow in [false, true] {
            let mut input =
                SortedInput::new(&database, schema(&[(DataType::Int64, false)])).unwrap();
            let cancel = CancellationToken::new();
            let mut effects = Effects::default();
            if grow {
                input.start(&cancel, &mut effects).unwrap();
            } else {
                input.files.create(&cancel, &mut effects).unwrap();
            }
            let mut steps = 0;
            for ordinal in 0..rows {
                input
                    .record
                    .encode(
                        &input.layout,
                        ROW_ARGUMENTS,
                        &encoded(&input.layout, &[Value::Int64((ordinal % 32) as i64)]),
                        ordinal,
                        &[],
                    )
                    .unwrap();
                if !input.sort.push(&input.record).unwrap() {
                    while input.sort.phase() != SortPhase::Collect {
                        assert!(steps < 2_000_000);
                        if grow {
                            input.sort_step(&cancel, &mut effects).unwrap();
                        } else {
                            input
                                .sort
                                .step(
                                    &input.layout,
                                    &mut input.files.io(&cancel, &mut effects).unwrap(),
                                )
                                .unwrap();
                        }
                        steps += 1;
                    }
                    assert!(input.sort.push(&input.record).unwrap());
                }
            }
            input.sort.finish().unwrap();
            while input.sort.phase() != SortPhase::Done {
                assert!(steps < 2_000_000);
                if grow {
                    input.sort_step(&cancel, &mut effects).unwrap();
                } else {
                    input
                        .sort
                        .step(
                            &input.layout,
                            &mut input.files.io(&cancel, &mut effects).unwrap(),
                        )
                        .unwrap();
                }
                steps += 1;
            }
            // Each record is 41 bytes. Literal counts exercise single and
            // multiple grown runs; the minimum buffers also force odd merges.
            // Expected rows use key and original ordinal independently.
            assert_eq!(
                (input.sort.runs, input.sort.merge.input.pass),
                match (rows, grow) {
                    (1024, false) => (5, 3),
                    (1024, true) => (1, 0),
                    (8192, false) => (40, 6),
                    (8192, true) => (2, 1),
                    (16384, false) => (80, 7),
                    (16384, true) => (3, 2),
                    (32768, false) => (160, 8),
                    (32768, true) => (4, 2),
                    _ => unreachable!(),
                }
            );
            assert_eq!(
                input.sort.buffer.row_limit,
                if !grow {
                    256
                } else {
                    match rows {
                        1024 => 4096,
                        8192 => 8192,
                        16384 | 32768 => 16_384,
                        _ => unreachable!(),
                    }
                }
            );
            eprintln!(
                "grow={grow} rows={rows} runs={} passes={} steps={steps} effects={}",
                input.sort.runs,
                input.sort.merge.input.pass,
                effects.count()
            );
            input.begin_read();
            for key in 0..32 {
                for ordinal in (key..rows).step_by(32) {
                    assert!(input.load(&cancel, &mut effects).unwrap());
                    let record = input.sort.sorted_cursor().record().unwrap();
                    assert_eq!(record.ordinal(), ordinal);
                    assert_eq!(
                        input.layout.value(record.key(), 0).unwrap(),
                        Value::Int64(key as i64)
                    );
                    input.consume().unwrap();
                }
            }
            assert!(input.finished());
            drop(input);
            assert_eq!(database.reserved_memory_bytes(), baseline);
            assert_eq!(database.reserved_temp_bytes(), 0);
        }
    }
    database.close().unwrap();
}

#[test]
fn comparison_steps_bound_work_and_cancel_without_writing() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let baseline = database.reserved_memory_bytes();
    for (kind, columns, corrupt, wide_payload) in [
        (DataType::Int64, 1, false, false),
        (DataType::Int64, 16, false, false),
        (DataType::String, 1, false, false),
        (DataType::Int64, 1, true, false),
        (DataType::String, 1, true, false),
        (DataType::Int64, 1, false, true),
    ] {
        let keys = if wide_payload {
            RowLayout::for_join(
                [DataType::Int64, DataType::String]
                    .into_iter()
                    .enumerate()
                    .map(|(index, kind)| SemanticColumn::new(index as u32 + 1, kind, false)),
                0,
            )
            .unwrap()
        } else {
            schema(&vec![(kind, false); columns])
        };
        let record_bytes = RECORD_HEADER + keys.max_bytes;
        let charge = database
            .reserve_memory(
                (buffer_memory_bytes(record_bytes).unwrap() + IO_MEMORY_BYTES) as u64,
                "comparison control buffers",
            )
            .unwrap();
        let mut record = SortRecord::new(record_bytes, charge.bytes()).unwrap();
        let mut writer = WriteBuffer::new(charge.bytes()).unwrap();
        let mut run = RunBuffer::new(&database, 1_048_576, 512).unwrap();
        let payload = "p".repeat(1024);
        for ordinal in 0..512 {
            let text = format!("{:04}", 511 - ordinal);
            let value = if kind == DataType::Int64 {
                Value::Int64(511 - ordinal as i64)
            } else {
                Value::String(StringValue::new(&text))
            };
            let mut values = vec![value; columns];
            if wide_payload {
                values.push(Value::String(StringValue::new(&payload)));
            }
            record
                .encode(&keys, ROW_ARGUMENTS, &encoded(&keys, &values), ordinal, &[])
                .unwrap();
            assert!(run.push(&record).unwrap());
        }
        let cancel = CancellationToken::new();
        let mut effects = Effects::default();
        let mut scratch =
            crate::storage::scratch::Scratch::new(&database, &cancel, &mut effects).unwrap();
        let before = effects.count();
        let memory = database.reserved_memory_bytes();
        run.begin(0).unwrap();
        assert!(
            !run.step(
                &keys,
                &mut writer,
                &mut Io::new(&mut scratch, &cancel, &mut effects)
            )
            .unwrap()
        );
        assert_eq!(run.phase, RunPhase::Sort);
        // This first merge pass compares once per pair and moves each tail
        // without comparison. Both narrow integers and short text reach the
        // move cap; sixteen integers use 45,056 comparison bytes for 128 pairs.
        // A wide non-key payload does not consume the comparison budget.
        assert_eq!(run.output, 256);
        assert_eq!(run.work[0].start, run.spans[1].start);
        assert_eq!(effects.count(), before);
        assert_eq!(database.reserved_memory_bytes(), memory);
        if corrupt {
            // Reject a malformed key after earlier comparisons in this call
            // have already moved spans. Partial sorting must stay terminal.
            run.bytes[run.spans[300].start + RECORD_HEADER] = 255;
            assert!(matches!(
                run.step(
                    &keys,
                    &mut writer,
                    &mut Io::new(&mut scratch, &cancel, &mut effects)
                ),
                Err(Error::Corrupt(_))
            ));
            assert!(run.output > 256);
        } else {
            cancel.cancel();
            assert!(matches!(
                run.step(
                    &keys,
                    &mut writer,
                    &mut Io::new(&mut scratch, &cancel, &mut effects)
                ),
                Err(Error::Cancelled)
            ));
        }
        assert_eq!(run.phase, RunPhase::Failed);
        assert!(matches!(
            run.step(
                &keys,
                &mut writer,
                &mut Io::new(&mut scratch, &CancellationToken::new(), &mut effects)
            ),
            Err(Error::Corrupt(_))
        ));
        assert_eq!(effects.count(), before);
        drop((run, writer, record, scratch, charge));
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
    database.close().unwrap();
}

#[test]
fn comparison_budget_counts_both_records_and_preserves_mixed_width_order() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(8_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let baseline = database.reserved_memory_bytes();
    // Each first-pass pair needs one comparison, then one uncharged tail move.
    // Counts below come from literal encoded widths: 32-byte frame headers,
    // one-byte NULLs, or a presence byte, four-byte length and string bytes.
    for (widths, rows, moves) in [
        (&[Some(1024)][..], 512, 60), // 30 * (2 * 1,061) = 63,660 bytes.
        (&[Some(0), Some(16_000)][..], 128, 8), // 4 * 16,074 = 64,296.
        (&[Some(16_000), Some(0)][..], 128, 8),
        (&[Some(0), Some(16_000), Some(1024), Some(7)][..], 128, 12),
        (&[Some(65_536)][..], 8, 2), // One oversized comparison still progresses.
        (&[None, Some(65_536)][..], 8, 2),
        (&[Some(65_536), None][..], 8, 2),
        (&[None, Some(0), Some(7)][..], 512, 256),
    ] {
        let keys = schema(&[(DataType::String, true)]);
        let values: Vec<_> = (0..rows).map(|i| widths[i % widths.len()]).collect();
        let byte_limit = values
            .iter()
            .map(|width| 32 + width.map_or(1, |width| 5 + width))
            .sum();
        let record_bytes = RECORD_HEADER + keys.max_bytes;
        let charge = database
            .reserve_memory(
                (buffer_memory_bytes(record_bytes).unwrap() + IO_MEMORY_BYTES) as u64,
                "comparison width controls",
            )
            .unwrap();
        let mut record = SortRecord::new(record_bytes, charge.bytes()).unwrap();
        let mut writer = WriteBuffer::new(charge.bytes()).unwrap();
        let mut run = RunBuffer::new(&database, byte_limit, rows).unwrap();
        for (ordinal, width) in values.iter().enumerate() {
            let text = width.map(|width| "x".repeat(width));
            let value = text
                .as_deref()
                .map_or(Value::Null, |text| Value::String(StringValue::new(text)));
            record
                .encode(
                    &keys,
                    ROW_ARGUMENTS,
                    &encoded(&keys, &[value]),
                    ordinal as u64,
                    &[],
                )
                .unwrap();
            assert!(run.push(&record).unwrap());
        }
        let cancel = CancellationToken::new();
        let mut effects = Effects::default();
        let mut scratch =
            crate::storage::scratch::Scratch::new(&database, &cancel, &mut effects).unwrap();
        let before = effects.count();
        let memory = database.reserved_memory_bytes();
        run.begin(0).unwrap();
        run.step(
            &keys,
            &mut writer,
            &mut Io::new(&mut scratch, &cancel, &mut effects),
        )
        .unwrap();
        assert_eq!(run.output, moves, "widths={widths:?}");
        for _ in 0..4096 {
            if run.phase == RunPhase::Header {
                break;
            }
            run.step(
                &keys,
                &mut writer,
                &mut Io::new(&mut scratch, &cancel, &mut effects),
            )
            .unwrap();
        }
        assert_eq!(run.phase, RunPhase::Header);
        assert_eq!(run.spans.len(), rows);
        let mut expected: Vec<_> = values.iter().copied().enumerate().collect();
        expected.sort_by_key(|&(ordinal, width)| (width, ordinal));
        for (span, (ordinal, width)) in run.spans.iter().zip(expected) {
            let bytes = &run.bytes[span.start..span.end];
            assert_eq!(
                u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
                ordinal as u64
            );
            let value = keys.value(&bytes[RECORD_HEADER..], 0).unwrap();
            match (value, width) {
                (Value::Null, None) => (),
                (Value::String(text), Some(width)) => {
                    assert_eq!(text.as_str(), "x".repeat(width));
                }
                _ => panic!("sorted record differs"),
            }
        }
        assert_eq!(effects.count(), before);
        assert_eq!(database.reserved_memory_bytes(), memory);
        drop((run, writer, record, scratch, charge));
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
    database.close().unwrap();
}

#[test]
fn allocation_padding_does_not_extend_record_or_run_limits() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let baseline = database.reserved_memory_bytes();
    let keys = schema(&[(DataType::String, false)]);
    let text = "x".repeat(65_500);
    let key = encoded(&keys, &[Value::String(StringValue::new(&text))]);
    // A 32-byte header, five bytes for presence and text length, then 65,500
    // text bytes put this frame one byte beyond the 64 KiB allocation boundary.
    let charge = database
        .reserve_memory(2 * 81_888 + IO_BYTES as u64, "padded frame test")
        .unwrap();
    let mut record = SortRecord::new(65_537, charge.bytes()).unwrap();
    record.encode(&keys, ROW_ARGUMENTS, &key, 0, &[]).unwrap();
    assert_eq!(record.bytes.len(), 65_537);
    assert_eq!(record.bytes.capacity(), 81_888);
    let mut run = RunBuffer::new(&database, 65_537, 2).unwrap();
    assert_eq!(run.bytes.capacity(), 81_888);
    assert!(run.push(&record).unwrap());
    let empty = encoded(&keys, &[Value::String(StringValue::new(""))]);
    record.encode(&keys, ROW_ARGUMENTS, &empty, 1, &[]).unwrap();
    assert!(
        !run.push(&record).unwrap(),
        "padding is not extra run space"
    );
    assert_eq!(run.spans.len(), 1);
    drop(run);

    let mut run = RunBuffer::new(&database, 65_537, 1_025).unwrap();
    assert_eq!(run.spans.capacity(), 2_046);
    for ordinal in 0..1_025 {
        record
            .encode(&keys, ROW_ARGUMENTS, &empty, ordinal, &[])
            .unwrap();
        assert!(run.push(&record).unwrap());
    }
    record
        .encode(&keys, ROW_ARGUMENTS, &empty, 1_025, &[])
        .unwrap();
    assert!(
        !run.push(&record).unwrap(),
        "padding is not extra row slots"
    );
    assert_eq!(run.spans.len(), 1_025);
    drop(run);

    let longer = format!("{text}y");
    let key = encoded(&keys, &[Value::String(StringValue::new(&longer))]);
    assert!(matches!(
        record.encode(&keys, ROW_ARGUMENTS, &key, 0, &[]),
        Err(Error::Resource {
            required: 65_538,
            limit: 65_537,
            ..
        })
    ));
    // The rounded allocation can hold a larger frame, but this reader was
    // admitted for 65,537 bytes. Even a valid checksum must not raise that limit.
    let mut larger = SortRecord::new(65_538, charge.bytes()).unwrap();
    larger.encode(&keys, ROW_ARGUMENTS, &key, 0, &[]).unwrap();
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let mut scratch =
        crate::storage::scratch::Scratch::new(&database, &cancel, &mut effects).unwrap();
    scratch
        .write(0, 0, &larger.bytes, &cancel, &mut effects)
        .unwrap();
    let mut reader = ReadBuffer::new(charge.bytes()).unwrap();
    let result = record.read(
        &mut reader,
        ReadAt {
            slot: 0,
            offset: 0,
            limit: larger.bytes.len() as u64,
        },
        &keys,
        ROW_ARGUMENTS,
        &mut Io::new(&mut scratch, &cancel, &mut effects),
    );
    assert!(matches!(
        result,
        Err(Error::Corrupt("group argument exceeds admitted buffer"))
    ));
    drop((scratch, reader, larger, record));
    drop(charge);
    assert_eq!(database.reserved_memory_bytes(), baseline);
    assert_eq!(database.reserved_temp_bytes(), 0);
    database.close().unwrap();
}

#[test]
fn text_argument_frames_validate_lengths_utf8_and_null_payloads() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let baseline = database.reserved_memory_bytes();
    let keys = schema(&[(DataType::Int64, false)]);
    let key = encoded(&keys, &[Value::Int64(7)]);
    let shape = ArgumentShape {
        count: 3,
        nonnull: 1,
        integers: 1,
        presence: 0,
        dates: 0,
        text: 6,
    };
    let capacity = RECORD_HEADER + keys.max_bytes + shape.max_payload_bytes();
    let charge = database
        .reserve_memory(
            (IO_BYTES + buffer_capacity(capacity).unwrap()) as u64,
            "text codec test",
        )
        .unwrap();
    let mut record = SortRecord::new(capacity, charge.bytes()).unwrap();
    let mut reader = ReadBuffer::new(charge.bytes()).unwrap();
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let mut scratch =
        crate::storage::scratch::Scratch::new(&database, &cancel, &mut effects).unwrap();
    let long = "é".repeat(crate::batch::MAX_TEXT_BYTES / 2);
    for value in [None, Some(""), Some("abc"), Some(long.as_str())] {
        record.bytes.clear();
        append_bytes(&mut record.bytes, &[0; RECORD_HEADER]).unwrap();
        append_bytes(&mut record.bytes, &key).unwrap();
        record
            .finish_with_text(
                shape.layout(&keys),
                key.len(),
                0,
                &[Some(42), value.map(|text| text.len() as u64), Some(1)],
                &[value.unwrap_or(""), "z"],
            )
            .unwrap();
        let limit = record.bytes.len() as u64;
        scratch
            .write(0, 0, &record.bytes, &cancel, &mut effects)
            .unwrap();
        reader.clear();
        record
            .read(
                &mut reader,
                ReadAt {
                    slot: 0,
                    offset: 0,
                    limit,
                },
                &keys,
                shape,
                &mut Io::new(&mut scratch, &cancel, &mut effects),
            )
            .unwrap();
        assert_eq!(record.bits(0), 42);
        assert_eq!(record.valid() & 2 != 0, value.is_some());
        assert_eq!(record.text_value(1, shape).unwrap(), value.unwrap_or(""));
        assert_eq!(record.text_value(2, shape).unwrap(), "z");
    }
    // Recompute the checksum after mutation. The reader must reject the
    // invalid length, UTF-8 or NULL payload, rather than just a stale checksum.
    for mutation in 0..3 {
        record.bytes.clear();
        append_bytes(&mut record.bytes, &[0; RECORD_HEADER]).unwrap();
        append_bytes(&mut record.bytes, &key).unwrap();
        record
            .finish_with_text(
                shape.layout(&keys),
                key.len(),
                0,
                &[Some(42), Some(1), Some(1)],
                &["a", "z"],
            )
            .unwrap();
        match mutation {
            0 => {
                *record.bytes.last_mut().unwrap() = 0xff;
            }
            1 => {
                let at = RECORD_HEADER + key.len() + 8;
                record.bytes[at..at + 8]
                    .copy_from_slice(&(crate::batch::MAX_TEXT_BYTES as u64 + 1).to_le_bytes());
            }
            2 => record.bytes[24] &= !2,
            _ => unreachable!(),
        }
        record.bytes[28..32].fill(0);
        let checksum = crate::storage::format::crc32c(&record.bytes);
        record.bytes[28..32].copy_from_slice(&checksum.to_le_bytes());
        let limit = record.bytes.len() as u64;
        scratch
            .write(0, 0, &record.bytes, &cancel, &mut effects)
            .unwrap();
        reader.clear();
        let outcome = record.read(
            &mut reader,
            ReadAt {
                slot: 0,
                offset: 0,
                limit,
            },
            &keys,
            shape,
            &mut Io::new(&mut scratch, &cancel, &mut effects),
        );
        let expected = match mutation {
            0 => "group text argument UTF-8",
            1 => "group text argument length",
            2 => "NULL group argument payload",
            _ => unreachable!(),
        };
        assert!(matches!(outcome, Err(Error::Corrupt(message)) if message == expected));
    }
    drop(scratch);
    drop(reader);
    drop(record);
    drop(charge);
    assert_eq!(database.reserved_memory_bytes(), baseline);
    assert_eq!(database.reserved_temp_bytes(), 0);
    database.close().unwrap();
}

#[test]
fn date_argument_frames_reject_out_of_range_days_with_valid_checksums() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let baseline = database.reserved_memory_bytes();
    let keys = schema(&[(DataType::Int64, false)]);
    let key = encoded(&keys, &[Value::Int64(7)]);
    let shape = ArgumentShape {
        count: 1,
        nonnull: 0,
        integers: 1,
        presence: 0,
        dates: 1,
        text: 0,
    };
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let charge = database
        .reserve_memory((IO_BYTES + 49) as u64, "date codec test")
        .unwrap();
    let mut record = SortRecord::new(49, charge.bytes()).unwrap();
    let mut reader = ReadBuffer::new(charge.bytes()).unwrap();
    let mut scratch =
        crate::storage::scratch::Scratch::new(&database, &cancel, &mut effects).unwrap();
    for (value, valid) in [
        (None, true),
        (Some(-719162_i64), true),
        (Some(2932896), true),
        (Some(-719163), false),
        (Some(2932897), false),
        (Some(i64::MAX), false),
    ] {
        // The encoder accepts these argument words and checksums them. The
        // reader must separately enforce the DATE range and signed-day encoding.
        record
            .encode(&keys, shape, &key, 0, &[value.map(|day| day as u64)])
            .unwrap();
        let limit = record.bytes.len() as u64;
        scratch
            .write(0, 0, &record.bytes, &cancel, &mut effects)
            .unwrap();
        reader.clear();
        let outcome = record.read(
            &mut reader,
            ReadAt {
                slot: 0,
                offset: 0,
                limit,
            },
            &keys,
            shape,
            &mut Io::new(&mut scratch, &cancel, &mut effects),
        );
        if valid {
            outcome.unwrap();
            assert_eq!(record.valid(), u16::from(value.is_some()));
            assert_eq!(record.bits(0), value.unwrap_or(0) as u64);
        } else {
            assert!(matches!(
                outcome,
                Err(Error::Corrupt("group DATE argument range"))
            ));
        }
        reader.clear();
        assert!(
            record
                .read(
                    &mut reader,
                    ReadAt {
                        slot: 0,
                        offset: 0,
                        limit
                    },
                    &keys,
                    ArgumentShape { dates: 0, ..shape },
                    &mut Io::new(&mut scratch, &cancel, &mut effects)
                )
                .is_err(),
            "DATE layout cannot be interpreted as numeric input"
        );
    }
    drop((scratch, reader, record, charge));
    assert_eq!(database.reserved_memory_bytes(), baseline);
    assert_eq!(database.reserved_temp_bytes(), 0);
}

#[test]
fn count_presence_frames_reject_values_and_changed_interpretation() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let baseline = database.reserved_memory_bytes();
    let keys = schema(&[(DataType::Int64, false)]);
    let key = encoded(&keys, &[Value::Int64(7)]);
    let shape = ArgumentShape {
        count: 1,
        nonnull: 0,
        integers: 1,
        presence: 1,
        dates: 0,
        text: 0,
    };
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let charge = database
        .reserve_memory((IO_BYTES + 49) as u64, "presence codec test")
        .unwrap();
    let mut record = SortRecord::new(49, charge.bytes()).unwrap();
    let mut reader = ReadBuffer::new(charge.bytes()).unwrap();
    let mut scratch =
        crate::storage::scratch::Scratch::new(&database, &cancel, &mut effects).unwrap();
    for value in [None, Some(0), Some(1)] {
        // Count-only arguments store presence, not values. Encode a nonzero
        // word with a valid checksum to reach the reader's payload check.
        record.encode(&keys, shape, &key, 0, &[value]).unwrap();
        let limit = record.bytes.len() as u64;
        scratch
            .write(0, 0, &record.bytes, &cancel, &mut effects)
            .unwrap();
        reader.clear();
        let result = record.read(
            &mut reader,
            ReadAt {
                slot: 0,
                offset: 0,
                limit,
            },
            &keys,
            shape,
            &mut Io::new(&mut scratch, &cancel, &mut effects),
        );
        if value == Some(1) {
            assert!(matches!(
                result,
                Err(Error::Corrupt("count-only group argument payload"))
            ));
        } else {
            result.unwrap();
            assert_eq!(record.valid(), u16::from(value.is_some()));
            assert_eq!(record.bits(0), 0);
        }
        reader.clear();
        assert!(
            record
                .read(
                    &mut reader,
                    ReadAt {
                        slot: 0,
                        offset: 0,
                        limit
                    },
                    &keys,
                    ArgumentShape {
                        presence: 0,
                        dates: 0,
                        text: 0,
                        ..shape
                    },
                    &mut Io::new(&mut scratch, &cancel, &mut effects)
                )
                .is_err(),
            "a valid checksum cannot turn presence into a numeric argument"
        );
    }
    drop((scratch, reader, record, charge));
    assert_eq!(database.reserved_memory_bytes(), baseline);
    assert_eq!(database.reserved_temp_bytes(), 0);
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
    let mut types = [DataType::Int64; MAX_ROW_VALUES];
    types[0] = DataType::String;
    types[1] = DataType::Double;
    types[2] = DataType::Date;
    types[4..23].fill(DataType::String);
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
    assert_eq!(layout.count, MAX_ROW_VALUES);
    assert_eq!(layout.key_count, 1);
    let record_bytes = RECORD_HEADER + layout.max_bytes;
    assert!(record_bytes > MAX_ARGUMENT_RECORD_BYTES);
    let record_charge = database
        .reserve_memory(
            buffer_capacity(record_bytes).unwrap() as u64,
            "wide sorted row",
        )
        .unwrap();
    let mut record = SortRecord::new(record_bytes, record_charge.bytes()).unwrap();
    let shape = ArgumentShape {
        count: 0,
        nonnull: 0,
        integers: 0,
        presence: 0,
        dates: 0,
        text: 0,
    };
    let mut sort = RowSort::new(&database, &layout, shape, record_bytes, 2).unwrap();
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let mut scratch =
        crate::storage::scratch::Scratch::new(&database, &cancel, &mut effects).unwrap();
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
        0 | 4..=22 if row == 1 => Value::Null,
        0 | 4..=22 => Value::String(StringValue::new("雪\0payload")),
        _ if row == 2 && column.is_multiple_of(3) => Value::Null,
        _ => Value::Int64((row * 1000 + column) as i64),
    };
    for row in 0..keys.len() {
        batch.clear();
        for column in 0..MAX_ROW_VALUES {
            batch.set(0, column, value(row, column)).unwrap();
        }
        batch.publish_rows(1);
        record
            .encode_row(&layout, &batch, 0, row as u64, None)
            .unwrap();
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
        for stored in 0..MAX_ROW_VALUES {
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
        presence: 0,
        dates: 0,
        text: 0,
    };
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let before = database.reserved_memory_bytes();
    let mut scratch =
        crate::storage::scratch::Scratch::new(database, &cancel, &mut effects).unwrap();
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
                // A short-transfer request can target an effect that transfers
                // no bytes. Only applicable read/write cuts must fail.
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
        presence: 0,
        dates: 0,
        text: 0,
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
    // Keep every input here to check exact preservation after merging. Expected
    // order uses the literal class number, optional text and input ordinal; it
    // does not call the production comparator. Encoded bytes still come from
    // the production encoder, so this checks preservation, not format correctness.
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
                crate::storage::scratch::Scratch::new(&database, &cancel, &mut effects).unwrap();
            let mut sort =
                RowSort::new(&database, &keys, arguments, byte_limit, row_limit).unwrap();
            let sort_charge = sort.reservation.bytes()
                + sort.buffer.reservation.bytes()
                + sort.merge.reservation.bytes()
                + sort.merge.pair.reservation.bytes();
            assert_eq!(database.reserved_memory_bytes(), before + sort_charge);
            let record_charge = database
                .reserve_memory(
                    buffer_capacity(record_bytes).unwrap() as u64,
                    "test argument record",
                )
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
        check_merge_passes(&database, runs, 19, MergeFault::None);
        assert_eq!(database.reserved_memory_bytes(), before);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
    check_merge_passes(&database, 2, 19, MergeFault::ReversedInput);
    assert_eq!(database.reserved_memory_bytes(), before);
    assert_eq!(database.reserved_temp_bytes(), 0);
    check_merge_passes(&database, 2, 2049, MergeFault::CancelWrite);
    assert_eq!(database.reserved_memory_bytes(), before);
    assert_eq!(database.reserved_temp_bytes(), 0);
    let effects = check_merge_passes(&database, 5, 19, MergeFault::None);
    for fail_at in 0..effects {
        check_merge_passes(&database, 5, 19, MergeFault::Io(fail_at));
        assert_eq!(database.reserved_memory_bytes(), before);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
}

#[derive(Clone, Copy)]
enum MergeFault {
    None,
    Io(u64),
    ReversedInput,
    CancelWrite,
}

fn check_merge_passes(database: &Database, runs: u32, rows: u64, fault: MergeFault) -> u64 {
    let bad_order = matches!(fault, MergeFault::ReversedInput);
    let fail_at = if let MergeFault::Io(index) = fault {
        Some(index)
    } else {
        None
    };
    let keys = schema(&[(DataType::Int64, false)]);
    let arguments = ArgumentShape {
        count: 1,
        nonnull: 1,
        integers: 1,
        presence: 0,
        dates: 0,
        text: 0,
    };
    let cancel = std::sync::Arc::new(CancellationToken::new());
    let mut effects = Effects::default();
    let mut scratch =
        crate::storage::scratch::Scratch::new(database, &cancel, &mut effects).unwrap();
    let mut merge = MergePasses::new(database, &keys, arguments).unwrap();
    assert!(merge.reservation.bytes() >= IO_BYTES as u64);
    let record_charge = database
        .reserve_memory(
            buffer_capacity(RECORD_HEADER + keys.max_bytes + 8).unwrap() as u64,
            "test record",
        )
        .unwrap();
    let mut record =
        SortRecord::new(RECORD_HEADER + keys.max_bytes + 8, record_charge.bytes()).unwrap();
    let mut expected = Vec::new();
    for index in 0..runs {
        // Rust tuple order sorts each input run. Repeated keys and interleaved
        // ordinals require the merge to combine records from different runs.
        let mut input = Vec::new();
        for row in 0..rows {
            let ordinal = row * u64::from(runs) + u64::from(index);
            let key = ((ordinal * 13) % 11) as i64 - 5;
            input.push((key, ordinal));
            expected.push((key, ordinal));
        }
        input.sort();
        if bad_order && index == 0 {
            // Valid record checksums do not establish run ordering. The merge
            // must reject this after emitting an earlier prefix in the same call.
            input.swap(1, 2);
        }
        let bytes = rows * (RECORD_HEADER + 9 + 8) as u64;
        let header = Run::header(RunId { pass: 0, index }, rows, bytes).unwrap();
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
            u64::from(runs) * rows,
            merge.writer.position().unwrap(),
        )
        .unwrap();
    expected.sort();
    // Start counting after fixture creation so cuts exercise merge work only.
    let token = std::sync::Arc::clone(&cancel);
    let mut effects = Effects::with_faults(Faults {
        fail_at,
        action: matches!(fault, MergeFault::CancelWrite).then(|| {
            Box::new(move |_, effect| {
                if effect == crate::effects::Effect::Load(crate::effects::LoadEffect::WriteStaging)
                {
                    token.cancel();
                }
            }) as Box<dyn FnMut(u64, crate::effects::Effect)>
        }),
        ..Faults::default()
    });
    let mut last_pass = 0;
    let mut last_runs = runs;
    let mut completed = false;
    // Allow one step per output row per pass, plus run and pass transitions.
    // A stalled state machine must fail this test instead of looping forever.
    let step_limit = (expected.len() + 4 * runs as usize + 4) * (MAX_MERGE_PASSES as usize + 1);
    for _ in 0..step_limit {
        let remaining = merge.pair.remaining;
        let result = merge.step(&keys, &mut Io::new(&mut scratch, &cancel, &mut effects));
        if let Err(error) = result {
            if bad_order {
                assert!(matches!(
                    error,
                    Error::Corrupt("group run is not strictly ordered")
                ));
                assert!(merge.pair.remaining < remaining);
                assert!(merge.pair.remaining > 0);
            } else if matches!(fault, MergeFault::CancelWrite) {
                assert!(matches!(error, Error::Cancelled));
                assert!(cancel.is_cancelled());
                assert!(
                    merge.pair.remaining < remaining,
                    "cancel after a prefix in this call"
                );
                assert!(merge.pair.remaining > 0);
            } else {
                assert!(fail_at.is_some(), "unexpected merge failure: {error:?}");
            }
            assert_eq!(merge.phase, MergePhase::Failed);
            let stopped_at = effects.count();
            assert!(
                merge
                    .step(
                        &keys,
                        &mut Io::new(&mut scratch, &CancellationToken::new(), &mut effects)
                    )
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
    assert!(!bad_order, "reversed input must be rejected");
    assert!(
        !matches!(fault, MergeFault::CancelWrite),
        "write cancellation must be observed"
    );
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
    check_pair_merge(false, 0);
    check_pair_merge(false, 65);
    check_pair_merge(true, 65);
}

fn check_pair_merge(equal_keys: bool, text_repeats: usize) {
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
        presence: 0,
        dates: 0,
        text: 0,
    };
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let before = database.reserved_memory_bytes();
    let mut merge = PairMerge::new(&database, &keys, arguments).unwrap();
    let merge_bytes = merge.reservation.bytes();
    assert_eq!(database.reserved_memory_bytes(), before + merge_bytes);
    let charge = database
        .reserve_memory(
            (IO_BYTES + buffer_capacity(RECORD_HEADER + keys.max_bytes + 16).unwrap()) as u64,
            "group codec test workspace",
        )
        .unwrap();
    let mut writer = WriteBuffer::new(charge.bytes()).unwrap();
    let mut record = SortRecord::new(RECORD_HEADER + keys.max_bytes + 16, charge.bytes()).unwrap();
    let mut scratch =
        crate::storage::scratch::Scratch::new(&database, &cancel, &mut effects).unwrap();
    let mut runs = [Run {
        start: 0,
        end: 0,
        rows: 0,
    }; 2];
    const ROWS: usize = 1025;
    for (side, run) in runs.iter_mut().enumerate() {
        // The test keeps a complete run body to write its header first. This
        // allocation belongs to the fixture, not the merge's memory account.
        let mut body = Vec::new();
        for index in 0..ROWS {
            let value = index * 2 + side;
            let text = if !equal_keys && value == 7 {
                "x".repeat(crate::batch::MAX_TEXT_BYTES)
            } else {
                "é".repeat(text_repeats)
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
    // PairMerge has no terminal failure state of its own. The cancelled step
    // did no work, so this direct component test resumes with a live token.
    // MergePasses and RowSort tests enforce terminal failure at their owners.
    let merge_start = effects.count();
    let uniform_pair_bytes = 2 * record.bytes.len();
    let mut complete = false;
    let mut largest_step = 0;
    for _ in 0..=2 * ROWS {
        let remaining = merge.remaining;
        complete = merge
            .step(
                &mut writer,
                &keys,
                &mut Io::new(&mut scratch, &cancel, &mut effects),
            )
            .unwrap();
        let moved = remaining - merge.remaining;
        assert!(moved <= 256);
        largest_step = largest_step.max(moved);
        if equal_keys && remaining == (2 * ROWS) as u64 {
            assert_eq!(moved as usize, (65_536 / uniform_pair_bytes).min(256));
            assert!(moved < 256, "wide records must reach the byte limit first");
        }
        assert!(complete || moved > 0, "a yielded merge must make progress");
        if complete {
            break;
        }
    }
    assert!(complete);
    assert!(largest_step > 1, "narrow records share a controller step");
    if text_repeats == 0 {
        assert_eq!(largest_step, 256, "small records reach the row limit");
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
        let text = if !equal_keys && value == 7 {
            "x".repeat(crate::batch::MAX_TEXT_BYTES)
        } else {
            "é".repeat(text_repeats)
        };
        assert_eq!(
            keys.value(row.key(), 1).unwrap(),
            Value::String(StringValue::new(&text))
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
    // The file contains the payload, but this caller allows only the header.
    // The reader must obey the supplied run boundary even on a cache hit.
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
    let crc = format::crc32c(&record.bytes);
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
    // With no right partner, the last run still passes through PairMerge.
    // Check its row count and byte extent after copying.
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
    let mut complete = false;
    for _ in 0..=ROWS {
        if merge
            .step(
                &mut writer,
                &keys,
                &mut Io::new(&mut scratch, &cancel, &mut effects),
            )
            .unwrap()
        {
            complete = true;
            break;
        }
    }
    assert!(complete);
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

#[test]
fn completed_sorted_inputs_release_only_unused_merge_storage() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let baseline = database.reserved_memory_bytes();
    for rows in [0, 1, 17] {
        for sorted_input in [false, true] {
            for transferred in [false, true] {
                let mut input =
                    SortedInput::new(&database, schema(&[(DataType::Int64, false)])).unwrap();
                let mut runtime = database.reserve_memory(0, "inline owner").unwrap();
                if transferred {
                    input.transfer_inline_to(&mut runtime).unwrap();
                }
                let cancel = CancellationToken::new();
                let mut effects = Effects::default();
                input.start(&cancel, &mut effects).unwrap();
                super::test_support::limit_initial_run_rows(&mut input, 2);
                for ordinal in 0..rows {
                    input
                        .record
                        .encode(
                            &input.layout,
                            ROW_ARGUMENTS,
                            &encoded(&input.layout, &[Value::Int64((ordinal % 3) as i64)]),
                            ordinal,
                            &[],
                        )
                        .unwrap();
                    while !input.sort.push(&input.record).unwrap() {
                        while input.sort.phase() != SortPhase::Collect {
                            input.sort_step(&cancel, &mut effects).unwrap();
                        }
                    }
                }
                input.sort.finish().unwrap();
                for _ in 0..4096 {
                    if input.sort.merge.phase == MergePhase::Final {
                        break;
                    }
                    input.sort_step(&cancel, &mut effects).unwrap();
                }
                assert_eq!(input.sort.merge.phase, MergePhase::Final);
                let capacities = input.sort.allocation_capacities();
                assert!(capacities[6] > 0 && capacities[8] > 0);
                let charged = database.reserved_memory_bytes();
                if sorted_input {
                    assert!(input.sort_step(&cancel, &mut effects).unwrap());
                    input.release_merge_right();
                } else {
                    // Grouping owns RowSort directly and must retain its spool
                    // reader and writer. Exercise the same final transition.
                    assert_eq!(
                        input
                            .sort
                            .step(
                                &input.layout,
                                &mut input.files.io(&cancel, &mut effects).unwrap()
                            )
                            .unwrap(),
                        SortPhase::Done
                    );
                }
                let mut expected_capacities = capacities;
                let released = if sorted_input {
                    expected_capacities[6] = 0;
                    expected_capacities[8] = 0;
                    crate::resources::buffer_charge(capacities[6]).unwrap()
                        + crate::resources::buffer_charge(capacities[8]).unwrap()
                } else {
                    0
                };
                assert_eq!(input.sort.allocation_capacities(), expected_capacities);
                assert_eq!(database.reserved_memory_bytes(), charged - released as u64);
                let mut expected: Vec<_> =
                    (0..rows).map(|ordinal| (ordinal % 3, ordinal)).collect();
                expected.sort();
                for _ in 0..2 {
                    input.begin_read();
                    for &(key, ordinal) in &expected {
                        assert!(input.load(&cancel, &mut effects).unwrap());
                        let mut values = [Value::Null];
                        input.read_values(&mut values).unwrap();
                        assert_eq!(values, [Value::Int64(key as i64)]);
                        assert_eq!(
                            input.sort.sorted_cursor().record().unwrap().ordinal(),
                            ordinal
                        );
                        input.consume().unwrap();
                    }
                    assert!(input.finished());
                    assert!(!input.load(&cancel, &mut effects).unwrap());
                }
                if sorted_input {
                    // Repeated completion cannot release the same charge twice.
                    assert!(input.sort_step(&cancel, &mut effects).unwrap());
                    input.release_merge_right();
                    assert_eq!(database.reserved_memory_bytes(), charged - released as u64);
                }
                if sorted_input && rows == 1 && !transferred {
                    let run = input.sort.merge.result.unwrap();
                    let attempts = effects.count();
                    let misuse = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        input.sort.merge.pair.right.begin(Some(run));
                    }));
                    assert!(
                        misuse.is_err(),
                        "retired storage cannot allocate implicitly"
                    );
                    assert_eq!(effects.count(), attempts);
                    assert_eq!(input.sort.allocation_capacities(), expected_capacities);
                }
                if !sorted_input {
                    let spool = input.sort.spool_slot();
                    let payload = "group result 雪\0".as_bytes();
                    let mut io = input.files.io(&cancel, &mut effects).unwrap();
                    input.sort.spool_writer().begin_file();
                    input
                        .sort
                        .spool_writer()
                        .append(spool, payload, &mut io)
                        .unwrap();
                    input.sort.spool_writer().flush(spool, &mut io).unwrap();
                    input.sort.spool_reader().clear();
                    let mut actual = vec![0; payload.len()];
                    input
                        .sort
                        .spool_reader()
                        .read(
                            ReadAt {
                                slot: spool,
                                offset: 0,
                                limit: payload.len() as u64,
                            },
                            &mut actual,
                            &mut io,
                        )
                        .unwrap();
                    assert_eq!(actual, payload);
                }
                if rows > 0 {
                    let run = input.sort.merge.result.unwrap();
                    match &mut input.files {
                        Files::Open(scratch) => scratch,
                        _ => unreachable!(),
                    }
                    .write(
                        input.sort.merge.input.slot,
                        run.start,
                        &[0],
                        &cancel,
                        &mut Effects::default(),
                    )
                    .unwrap();
                    input.begin_read();
                    assert!(matches!(
                        input.load(&cancel, &mut effects),
                        Err(Error::Corrupt(_))
                    ));
                }
                assert_eq!(input.sort.allocation_capacities(), expected_capacities);
                drop((input, runtime));
                assert_eq!(database.reserved_memory_bytes(), baseline);
                assert_eq!(database.reserved_temp_bytes(), 0);
            }
        }
    }
}

#[test]
fn completed_sorted_inputs_release_staging_storage_and_preserve_replay() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(32_000_000, 64_000_000).unwrap(),
    )
    .unwrap();
    let baseline = database.reserved_memory_bytes();
    let text = format!("雪\0{}", "x".repeat(crate::batch::MAX_TEXT_BYTES - 4));
    for (kind, columns, rows) in [
        (DataType::Int64, 1, 0),
        (DataType::Int64, 1, 17),
        (DataType::String, 1, 0),
        (DataType::String, 1, 17),
        (DataType::String, 16, 0),
        (DataType::String, 16, 1),
    ] {
        for transferred in [false, true] {
            let nullable = kind == DataType::String;
            let mut input =
                SortedInput::new(&database, schema(&vec![(kind, nullable); columns])).unwrap();
            let mut runtime = database.reserve_memory(0, "inline owner").unwrap();
            if transferred {
                input.transfer_inline_to(&mut runtime).unwrap();
            }
            let cancel = CancellationToken::new();
            let mut effects = Effects::default();
            input.start(&cancel, &mut effects).unwrap();
            super::test_support::limit_initial_run_rows(&mut input, 2);
            for ordinal in 0..rows {
                let value = if nullable && ordinal % 3 == 2 {
                    Value::Null
                } else if nullable {
                    Value::String(StringValue::new(&text))
                } else {
                    Value::Int64(7)
                };
                input
                    .record
                    .encode(
                        &input.layout,
                        ROW_ARGUMENTS,
                        &encoded(&input.layout, &vec![value; columns]),
                        ordinal,
                        &[],
                    )
                    .unwrap();
                while !input.sort.push(&input.record).unwrap() {
                    while input.sort.phase() != SortPhase::Collect {
                        input.sort_step(&cancel, &mut effects).unwrap();
                    }
                }
                input.ordinal += 1;
            }
            input.sort.finish().unwrap();
            let mut done = false;
            for _ in 0..4096 {
                if input.sort_step(&cancel, &mut effects).unwrap() {
                    done = true;
                    break;
                }
            }
            assert!(done);
            input.begin_read();
            let capacity = input.record.bytes.capacity();
            let charged = database.reserved_memory_bytes();
            let capacities = input.sort.allocation_capacities();
            assert!(capacity > 0);
            input.release_staging_record();
            let released = super::test_support::expected_buffer_charge(capacity) as u64;
            assert_eq!(input.record.bytes.capacity(), 0);
            assert_eq!(input.sort.allocation_capacities(), capacities);
            assert_eq!(database.reserved_memory_bytes(), charged - released);
            assert_eq!(input.ordinal, rows);
            assert_eq!(
                database.reserved_memory_bytes(),
                baseline + input.memory_bytes() + runtime.bytes()
            );
            eprintln!(
                "staging release: kind={kind:?} columns={columns} rows={rows} transferred={transferred} capacity={capacity} charge={released}"
            );
            // NULLs precede present values; equal keys preserve source ordinals.
            let expected: Vec<_> = [true, false]
                .into_iter()
                .flat_map(|null| (0..rows).filter(move |row| (nullable && row % 3 == 2) == null))
                .collect();
            for _ in 0..2 {
                input.begin_read();
                for &ordinal in &expected {
                    assert!(input.load(&cancel, &mut effects).unwrap());
                    let value = if nullable && ordinal % 3 == 2 {
                        Value::Null
                    } else if nullable {
                        Value::String(StringValue::new(&text))
                    } else {
                        Value::Int64(7)
                    };
                    let mut actual = vec![Value::Null; columns];
                    input.read_values(&mut actual).unwrap();
                    assert_eq!(actual, vec![value; columns]);
                    assert_eq!(
                        input.sort.sorted_cursor().record().unwrap().ordinal(),
                        ordinal
                    );
                    input.consume().unwrap();
                }
                assert!(input.finished());
                assert!(!input.load(&cancel, &mut effects).unwrap());
            }
            input.release_staging_record();
            assert_eq!(database.reserved_memory_bytes(), charged - released);
            // A retired staging record must not grow an uncharged allocation.
            assert!(matches!(
                append_bytes(&mut input.record.bytes, &[0; RECORD_HEADER]),
                Err(Error::Resource { limit: 0, .. })
            ));
            assert_eq!(input.record.bytes.capacity(), 0);
            drop((input, runtime));
            assert_eq!(database.reserved_memory_bytes(), baseline);
            assert_eq!(database.reserved_temp_bytes(), 0);
        }
    }
}

#[test]
fn failed_final_sort_validation_keeps_merge_storage_until_drop() {
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(4_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let baseline = database.reserved_memory_bytes();
    for failure in 0..3 {
        let mut input = SortedInput::new(&database, schema(&[(DataType::Int64, false)])).unwrap();
        let cancel = CancellationToken::new();
        let mut effects = Effects::default();
        input.start(&cancel, &mut effects).unwrap();
        input
            .record
            .encode(
                &input.layout,
                ROW_ARGUMENTS,
                &encoded(&input.layout, &[Value::Int64(7)]),
                0,
                &[],
            )
            .unwrap();
        assert!(input.sort.push(&input.record).unwrap());
        input.sort.finish().unwrap();
        for _ in 0..4096 {
            if input.sort.merge.phase == MergePhase::Final {
                break;
            }
            input.sort_step(&cancel, &mut effects).unwrap();
        }
        assert_eq!(input.sort.merge.phase, MergePhase::Final);
        match failure {
            0 => cancel.cancel(),
            1 => {
                effects = Effects::with_faults(Faults {
                    fail_at: Some(0),
                    ..Faults::default()
                })
            }
            2 => {
                match &mut input.files {
                    Files::Open(scratch) => scratch,
                    _ => unreachable!(),
                }
                .write(
                    input.sort.merge.input.slot,
                    0,
                    &[0],
                    &cancel,
                    &mut Effects::default(),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let capacities = input.sort.allocation_capacities();
        let staging_capacity = input.record.bytes.capacity();
        let charged = database.reserved_memory_bytes();
        let error = input.sort_step(&cancel, &mut effects).unwrap_err();
        assert!(match failure {
            0 => matches!(error, Error::Cancelled),
            1 => matches!(error, Error::Io { .. }),
            2 => matches!(error, Error::Corrupt(_)),
            _ => false,
        });
        assert_eq!(input.sort.phase(), SortPhase::Failed);
        assert_eq!(input.sort.allocation_capacities(), capacities);
        assert_eq!(input.record.bytes.capacity(), staging_capacity);
        assert_eq!(database.reserved_memory_bytes(), charged);
        let attempts = effects.count();
        let repeated = input.sort_step(&cancel, &mut effects).unwrap_err();
        assert!(if failure == 0 {
            matches!(repeated, Error::Cancelled)
        } else {
            matches!(repeated, Error::Corrupt(_))
        });
        assert_eq!(effects.count(), attempts);
        drop(input);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
}
