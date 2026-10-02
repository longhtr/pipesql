//! Check complete literal exports and failures after an output prefix.
//!
//! Expected bytes are independent of the encoder. Controlled writers exercise
//! short writes, cancellation and terminal flush errors while memory counters
//! check release of execution and export ownership on the same database handle.
//!
//! Cases distinguish exact row and byte limits from one-step excesses, preserve
//! DOUBLE bits and positional columns, and retry after cancellation or a late query
//! error. A panicking writer must also release owners even though no typed result
//! returns to the caller.

use super::*;
use crate::test_support::Directory;
use crate::test_writer::Sink;
use crate::{
    AppendLimits, ColumnDeclaration, ColumnInput, ColumnValues, Config, DateValue, StringValue,
};
use std::io;

const LIMITS: ExportLimits = ExportLimits {
    rows: 10_000,
    bytes: 1_000_000,
};

fn database(directory: &Directory, values: &[i64]) -> Database {
    let db = Database::create_empty(
        &directory.0.join("db"),
        Config::new(8_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "facts",
        &[ColumnDeclaration {
            name: "amount",
            data_type: DataType::Int64,
            nullable: false,
        }],
        &cancel,
    )
    .unwrap();
    if !values.is_empty() {
        let mut append = db
            .begin_append(
                "facts",
                AppendLimits {
                    batches: 32,
                    encoded_bytes: 100_000,
                },
                &cancel,
            )
            .unwrap();
        for chunk in values.chunks(256) {
            let mut validity = [0xff; 32];
            let length = chunk.len().div_ceil(8);
            if chunk.len() % 8 != 0 {
                validity[length - 1] = (1 << (chunk.len() % 8)) - 1;
            }
            append
                .write(
                    &[ColumnInput {
                        values: ColumnValues::Int64(chunk),
                        validity: &validity[..length],
                    }],
                    &cancel,
                )
                .unwrap();
        }
        append.commit(&cancel).unwrap();
    }
    db
}

const HEADER: &str = "{\"format\":\"pipesql-jsonl\",\"version\":1,\"columns\":[{\"name\":\"amount\",\"type\":\"int64\",\"nullable\":false}]}\n";

#[test]
fn cancellation_at_export_completion_preserves_output_and_error() {
    let directory = Directory::new();
    let db = database(&directory, &[7, 11]);
    let query = db.prepare("FROM facts |> ORDER BY amount").unwrap();
    let baseline = db.reserved_memory_bytes();
    let mut expected = Vec::new();
    assert_eq!(
        db.export_jsonl(&query, &mut expected, LIMITS, &CancellationToken::new())
            .unwrap(),
        2
    );
    assert_eq!(
        expected,
        format!(
            "{HEADER}{{\"row\":[\"7\"]}}\n{{\"row\":[\"11\"]}}\n{{\"complete\":true,\"rows\":2}}\n"
        )
        .as_bytes()
    );
    for (during_flush, fail_flush) in [(false, false), (true, false), (true, true)] {
        let cancel = CancellationToken::new();
        let mut sink = Sink {
            short: 17,
            cancel: (!during_flush).then_some(&cancel),
            cancel_after: expected.len(),
            cancel_flush: during_flush.then_some(&cancel),
            fail_flush,
            ..Sink::default()
        };
        let error = db
            .export_jsonl(&query, &mut sink, LIMITS, &cancel)
            .unwrap_err();
        if fail_flush {
            assert!(
                matches!(error, Error::Io { operation: "write result export", source }
                if source.kind() == std::io::ErrorKind::Other)
            );
        } else {
            assert!(matches!(error, Error::Cancelled));
        }
        assert!(cancel.is_cancelled());
        // Complete bytes do not establish API success. A request during the last
        // write must prevent flush; a failed flush keeps its original I/O cause.
        assert_eq!(sink.bytes, expected);
        assert_eq!(sink.flushes, usize::from(during_flush));
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
        let mut retry = Vec::new();
        assert_eq!(
            db.export_jsonl(&query, &mut retry, LIMITS, &CancellationToken::new())
                .unwrap(),
            2
        );
        assert_eq!(retry, expected);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}

#[test]
fn complete_rows_and_exact_limits_release_ownership() {
    let directory = Directory::new();
    let db = database(&directory, &[i64::MIN, 0, i64::MAX]);
    let query = db.prepare("FROM facts |> ORDER BY amount").unwrap();
    let baseline = db.reserved_memory_bytes();
    let expected = format!(
        "{HEADER}{{\"row\":[\"-9223372036854775808\"]}}\n{{\"row\":[\"0\"]}}\n{{\"row\":[\"9223372036854775807\"]}}\n{{\"complete\":true,\"rows\":3}}\n"
    );
    let mut bytes = Vec::new();
    let cancel = CancellationToken::new();
    let exact = ExportLimits {
        rows: 3,
        bytes: expected.len() as u64,
    };
    assert_eq!(
        db.export_jsonl(&query, &mut bytes, exact, &cancel).unwrap(),
        3
    );
    assert_eq!(bytes, expected.as_bytes());
    assert_eq!(db.reserved_memory_bytes(), baseline);
    for (limits, owner) in [
        (
            ExportLimits {
                bytes: exact.bytes - 1,
                ..exact
            },
            "result export bytes",
        ),
        (ExportLimits { rows: 2, ..exact }, "result export rows"),
        (ExportLimits { bytes: 0, ..exact }, "result export bytes"),
    ] {
        bytes.clear();
        let error = db
            .export_jsonl(&query, &mut bytes, limits, &cancel)
            .unwrap_err();
        assert!(matches!(error, Error::Resource { owner: actual, .. } if actual == owner));
        assert!(bytes.len() as u64 <= limits.bytes);
        assert!(!String::from_utf8_lossy(&bytes).contains("\"complete\":true"));
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    bytes.clear();
    assert_eq!(
        db.export_jsonl(&query, &mut bytes, exact, &cancel).unwrap(),
        3
    );
    assert_eq!(bytes, expected.as_bytes());
}

#[test]
fn empty_result_keeps_schema_and_allows_zero_rows() {
    let directory = Directory::new();
    let db = database(&directory, &[]);
    let query = db.prepare("FROM facts").unwrap();
    let mut bytes = Vec::new();
    assert_eq!(
        db.export_jsonl(
            &query,
            &mut bytes,
            ExportLimits { rows: 0, ..LIMITS },
            &CancellationToken::new()
        )
        .unwrap(),
        0
    );
    assert_eq!(
        bytes,
        format!("{HEADER}{{\"complete\":true,\"rows\":0}}\n").as_bytes()
    );
}

#[test]
fn scalar_encoding_preserves_bits_controls_and_null() {
    let cancel = CancellationToken::new();
    let mut bytes = Vec::new();
    let mut encoder = Encoder {
        output: &mut bytes,
        cancel: &cancel,
        buffer: [0; 1024],
        length: 0,
        bytes: 0,
        limit: LIMITS.bytes,
    };
    let values = [
        Value::Null,
        Value::String(StringValue::new("")),
        Value::String(StringValue::new("a\"\\\n\r\t\0é🙂")),
        Value::Date(DateValue::from_days_since_unix_epoch(-719_162).unwrap()),
        Value::Date(DateValue::from_days_since_unix_epoch(2_932_896).unwrap()),
        Value::Double(0.0),
        Value::Double(-0.0),
        Value::Double(f64::INFINITY),
        Value::Double(f64::NEG_INFINITY),
        Value::Double(f64::from_bits(0x7ff8_0000_0000_1234)),
        Value::Double(f64::from_bits(0xfff0_0000_0000_0001)),
        Value::Double(f64::from_bits(1)),
    ];
    for value in values {
        encoder.value(value).unwrap();
        encoder.bytes(b"\n").unwrap();
    }
    encoder.flush_buffer().unwrap();
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        concat!(
            "null\n\"\"\n\"a\\\"\\\\\\u000a\\u000d\\u0009\\u0000é🙂\"\n",
            "\"0001-01-01\"\n\"9999-12-31\"\n",
            "\"0000000000000000\"\n\"8000000000000000\"\n",
            "\"7ff0000000000000\"\n\"fff0000000000000\"\n",
            "\"7ff8000000001234\"\n\"fff0000000000001\"\n\"0000000000000001\"\n"
        )
    );
}

const CONTROL_ESCAPES: &str = concat!(
    "\\u0000\\u0001\\u0002\\u0003\\u0004\\u0005\\u0006\\u0007",
    "\\u0008\\u0009\\u000a\\u000b\\u000c\\u000d\\u000e\\u000f",
    "\\u0010\\u0011\\u0012\\u0013\\u0014\\u0015\\u0016\\u0017",
    "\\u0018\\u0019\\u001a\\u001b\\u001c\\u001d\\u001e\\u001f",
);

#[test]
fn every_control_escape_preserves_bytes_at_every_buffer_alignment() {
    let controls: String = (0..32).map(char::from).collect();
    for padding in 0..1024 {
        let prefix = "x".repeat(padding);
        let input = format!("{prefix}é{controls}\u{7f}🙂\"\\");
        let expected = format!("\"{prefix}é{CONTROL_ESCAPES}\u{7f}🙂\\\"\\\\\"");
        let cancel = CancellationToken::new();
        let mut sink = Sink {
            short: 17,
            ..Sink::default()
        };
        let mut encoder = Encoder {
            output: &mut sink,
            cancel: &cancel,
            buffer: [0; 1024],
            length: 0,
            bytes: 0,
            limit: expected.len() as u64,
        };
        encoder.string(&input).unwrap();
        assert_eq!(encoder.bytes, expected.len() as u64);
        encoder.flush_buffer().unwrap();
        assert_eq!(sink.bytes, expected.as_bytes(), "padding {padding}");
    }
}

#[test]
fn control_escape_limit_admits_all_six_bytes_before_copying() {
    for padding in [1017, 1020, 1023, 1024, 2045] {
        let prefix = "x".repeat(padding);
        for control in 0..32_u8 {
            let input = format!("{prefix}{}", char::from(control));
            let escape = &CONTROL_ESCAPES[usize::from(control) * 6..usize::from(control) * 6 + 6];
            let expected = format!("\"{prefix}{escape}\"");
            for extra in 0..8 {
                let limit = (padding + 1 + extra) as u64;
                let cancel = CancellationToken::new();
                let mut output = Vec::new();
                let written = {
                    let mut encoder = Encoder {
                        output: &mut output,
                        cancel: &cancel,
                        buffer: [0; 1024],
                        length: 0,
                        bytes: 0,
                        limit,
                    };
                    let outcome = encoder.string(&input);
                    if extra == 7 {
                        outcome.unwrap();
                        encoder.flush_buffer().unwrap();
                        expected.len()
                    } else {
                        let required = (padding + if extra < 6 { 7 } else { 8 }) as u64;
                        assert!(matches!(outcome, Err(Error::Resource {
                            owner: "result export bytes", required: actual, limit: actual_limit
                        }) if actual == required && actual_limit == limit));
                        // Refusal cannot publish any part of the denied escape or
                        // flush its accepted tail when the encoder leaves scope.
                        let accepted = padding + 1 + if extra < 6 { 0 } else { 6 };
                        assert_eq!(encoder.bytes, accepted as u64);
                        accepted / 1024 * 1024
                    }
                };
                assert_eq!(output, expected.as_bytes()[..written]);
            }
        }
    }
}

#[test]
fn output_failure_and_cancellation_can_split_an_escape() {
    let prefix = "x".repeat(1021);
    let input = format!("{prefix}\u{1f}");
    let expected = format!("\"{prefix}\\u001f\"");
    for cancellation in [false, true] {
        let cancel = CancellationToken::new();
        let mut sink = Sink {
            short: 1023,
            fail_after: if cancellation { usize::MAX } else { 1023 },
            cancel: cancellation.then_some(&cancel),
            ..Sink::default()
        };
        let error = {
            let mut encoder = Encoder {
                output: &mut sink,
                cancel: &cancel,
                buffer: [0; 1024],
                length: 0,
                bytes: 0,
                limit: LIMITS.bytes,
            };
            encoder.string(&input).unwrap_err()
        };
        if cancellation {
            assert!(matches!(error, Error::Cancelled));
        } else {
            assert!(matches!(error, Error::Io {
                operation: "write result export", source
            } if source.kind() == io::ErrorKind::BrokenPipe));
        }
        assert_eq!(sink.bytes, expected.as_bytes()[..1023]);
        assert_eq!(sink.bytes.last(), Some(&b'\\'));
        assert_eq!(sink.flushes, 0);
    }
}

#[test]
fn short_writes_and_output_failures_preserve_terminal_outcome() {
    let directory = Directory::new();
    let db = database(&directory, &(0..256).collect::<Vec<_>>());
    let query = db.prepare("FROM facts |> ORDER BY amount").unwrap();
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let mut expected = Vec::new();
    db.export_jsonl(&query, &mut expected, LIMITS, &cancel)
        .unwrap();
    let mut short = Sink {
        short: 1,
        ..Sink::default()
    };
    assert_eq!(
        db.export_jsonl(&query, &mut short, LIMITS, &cancel)
            .unwrap(),
        256
    );
    assert_eq!(short.bytes, expected);
    assert_eq!(short.flushes, 1);
    for mut sink in [
        Sink {
            fail_after: 1_111,
            ..Sink::default()
        },
        Sink {
            failure: io::ErrorKind::Interrupted,
            fail_after: 1_111,
            ..Sink::default()
        },
        Sink {
            short: 0,
            ..Sink::default()
        },
        Sink {
            fail_flush: true,
            ..Sink::default()
        },
    ] {
        assert!(matches!(
            db.export_jsonl(&query, &mut sink, LIMITS, &cancel),
            Err(Error::Io { .. })
        ));
        assert_eq!(db.reserved_memory_bytes(), baseline);
        if sink.fail_flush {
            assert_eq!(sink.bytes, expected);
            assert_eq!(sink.flushes, 1);
        } else {
            assert_eq!(sink.flushes, 0);
            assert!(expected.starts_with(&sink.bytes));
        }
    }
}

#[test]
fn cancellation_and_late_query_failure_release_and_allow_retry() {
    let directory = Directory::new();
    let mut values: Vec<i64> = (0..256).collect();
    values.push(i64::MAX);
    let db = database(&directory, &values);
    let query = db
        .prepare("FROM facts\n|> ORDER BY amount\n|> SELECT amount + 1 AS next_amount")
        .unwrap();
    let baseline = db.reserved_memory_bytes();
    let mut bytes = Vec::new();
    let error = db
        .export_jsonl(&query, &mut bytes, LIMITS, &CancellationToken::new())
        .unwrap_err();
    assert!(
        matches!(error, Error::ArithmeticOverflow { operation: "addition", span } if span.start() == 40 && span.end() == 50)
    );
    assert!(bytes.len() >= 1024);
    assert!(!String::from_utf8_lossy(&bytes).contains("\"complete\":true"));
    assert_eq!(db.reserved_memory_bytes(), baseline);
    drop(query);
    let query = db.prepare("FROM facts |> ORDER BY amount").unwrap();
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let mut sink = Sink {
        cancel: Some(&cancel),
        ..Sink::default()
    };
    assert!(matches!(
        db.export_jsonl(&query, &mut sink, LIMITS, &cancel),
        Err(Error::Cancelled)
    ));
    assert_eq!(sink.bytes.len(), 1024);
    assert_eq!(sink.flushes, 0);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    let mut bytes = Vec::new();
    assert!(matches!(
        db.export_jsonl(&query, &mut bytes, LIMITS, &cancel),
        Err(Error::Cancelled)
    ));
    assert!(bytes.is_empty());
    assert_eq!(
        db.export_jsonl(&query, &mut bytes, LIMITS, &CancellationToken::new())
            .unwrap(),
        257
    );
    assert!(bytes.ends_with(b"{\"complete\":true,\"rows\":257}\n"));
    assert_eq!(db.reserved_memory_bytes(), baseline);
}

#[test]
fn buffer_admission_refusal_writes_nothing_and_releases_ownership() {
    let directory = Directory::new();
    let db = database(&directory, &[1]);
    let query = db.prepare("FROM facts").unwrap();
    let baseline = db.reserved_memory_bytes();
    let held = db
        .reserve_memory(
            db.config().memory_limit_bytes() - baseline,
            "test held memory",
        )
        .unwrap();
    let mut bytes = Vec::new();
    assert!(matches!(
        db.export_jsonl(&query, &mut bytes, LIMITS, &CancellationToken::new()),
        Err(Error::Resource {
            owner: "result export buffer",
            ..
        })
    ));
    assert!(bytes.is_empty());
    drop(held);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(
        db.export_jsonl(&query, &mut bytes, LIMITS, &CancellationToken::new())
            .unwrap(),
        1
    );
    assert_eq!(db.reserved_memory_bytes(), baseline);
}

#[test]
fn typed_cursor_exports_nullable_columns_and_reordered_schema() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("db"),
        Config::new(8_000_000, 4_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let columns = [
        ColumnDeclaration {
            name: "id",
            data_type: DataType::Int64,
            nullable: true,
        },
        ColumnDeclaration {
            name: "note",
            data_type: DataType::String,
            nullable: true,
        },
        ColumnDeclaration {
            name: "number",
            data_type: DataType::Double,
            nullable: true,
        },
        ColumnDeclaration {
            name: "day",
            data_type: DataType::Date,
            nullable: true,
        },
    ];
    db.declare_table("typed", &columns, &cancel).unwrap();
    let mut append = db
        .begin_append(
            "typed",
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
                    values: ColumnValues::Int64(&[i64::MIN, i64::MAX, 0]),
                    validity: &[3],
                },
                ColumnInput {
                    values: ColumnValues::String(&["", "x\"\\\n\0é🙂", "ignored"]),
                    validity: &[3],
                },
                ColumnInput {
                    values: ColumnValues::Double(&[
                        -0.0,
                        f64::from_bits(0xfff8_0000_0000_0042),
                        0.0,
                    ]),
                    validity: &[3],
                },
                ColumnInput {
                    values: ColumnValues::Date(&[
                        DateValue::from_days_since_unix_epoch(-719_162).unwrap(),
                        DateValue::from_days_since_unix_epoch(2_932_896).unwrap(),
                        DateValue::from_days_since_unix_epoch(0).unwrap(),
                    ]),
                    validity: &[3],
                },
            ],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    let query = db
        .prepare("FROM typed |> ORDER BY id NULLS LAST |> SELECT day, number, note, id")
        .unwrap();
    let mut bytes = Vec::new();
    assert_eq!(
        db.export_jsonl(&query, &mut bytes, LIMITS, &cancel)
            .unwrap(),
        3
    );
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        concat!(
            "{\"format\":\"pipesql-jsonl\",\"version\":1,\"columns\":[",
            "{\"name\":\"day\",\"type\":\"date\",\"nullable\":true},",
            "{\"name\":\"number\",\"type\":\"double\",\"nullable\":true},",
            "{\"name\":\"note\",\"type\":\"string\",\"nullable\":true},",
            "{\"name\":\"id\",\"type\":\"int64\",\"nullable\":true}]}\n",
            "{\"row\":[\"0001-01-01\",\"8000000000000000\",\"\",\"-9223372036854775808\"]}\n",
            "{\"row\":[\"9999-12-31\",\"fff8000000000042\",\"x\\\"\\\\\\u000a\\u0000é🙂\",\"9223372036854775807\"]}\n",
            "{\"row\":[null,null,null,null]}\n",
            "{\"complete\":true,\"rows\":3}\n"
        )
    );
}

#[test]
fn duplicate_and_unnamed_columns_remain_positional() {
    let directory = Directory::new();
    let db = database(&directory, &[7]);
    let query = db
        .prepare("FROM facts |> SELECT amount, amount, +amount")
        .unwrap();
    let mut bytes = Vec::new();
    db.export_jsonl(&query, &mut bytes, LIMITS, &CancellationToken::new())
        .unwrap();
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        concat!(
            "{\"format\":\"pipesql-jsonl\",\"version\":1,\"columns\":[",
            "{\"name\":\"amount\",\"type\":\"int64\",\"nullable\":false},",
            "{\"name\":\"amount\",\"type\":\"int64\",\"nullable\":false},",
            "{\"name\":null,\"type\":\"int64\",\"nullable\":false}]}\n",
            "{\"row\":[\"7\",\"7\",\"7\"]}\n{\"complete\":true,\"rows\":1}\n"
        )
    );
}

#[test]
fn panicking_writer_drops_execution_and_buffer_reservations() {
    struct Panics;
    impl Write for Panics {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            panic!("injected output panic")
        }
        fn flush(&mut self) -> io::Result<()> {
            panic!("unexpected flush")
        }
    }
    let directory = Directory::new();
    let db = database(&directory, &(0..256).collect::<Vec<_>>());
    let query = db.prepare("FROM facts").unwrap();
    let baseline = db.reserved_memory_bytes();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        db.export_jsonl(&query, &mut Panics, LIMITS, &CancellationToken::new())
    }));
    assert!(outcome.is_err());
    assert_eq!(db.reserved_memory_bytes(), baseline);
    let mut bytes = Vec::new();
    assert_eq!(
        db.export_jsonl(&query, &mut bytes, LIMITS, &CancellationToken::new())
            .unwrap(),
        256
    );
    assert_eq!(db.reserved_memory_bytes(), baseline);
}

#[test]
fn encoded_count_overflow_is_reported_without_writing() {
    let cancel = CancellationToken::new();
    let mut output = Vec::new();
    let mut encoder = Encoder {
        output: &mut output,
        cancel: &cancel,
        buffer: [0; 1024],
        length: 0,
        bytes: u64::MAX,
        limit: u64::MAX,
    };
    assert!(matches!(
        encoder.bytes(b"x"),
        Err(Error::Unsupported(
            "export byte count exceeds unsigned 64-bit range"
        ))
    ));
    assert_eq!(encoder.length, 0);
    assert!(output.is_empty());
}
