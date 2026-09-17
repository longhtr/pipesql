//! Check typed CSV decoding with literal inputs and independent expected values.
//!
//! Small read chunks split Unicode, quotes and record endings at every boundary.
//! The tests distinguish NULL from empty text, verify declaration-order header
//! mapping and require original byte offsets for malformed input. Exact limits and
//! one-step excesses exercise bounded records, fields, rows and retained text.
//! Reader failures, cancellation and interrupted reads check progress and terminal
//! behavior; admission refusal must occur before the source is read.

use super::*;
use crate::{DataType, DateValue, StringValue, Value};

fn limits() -> CsvLimits {
    CsvLimits {
        input_bytes: 100_000,
        rows: 1_000,
        record_bytes: 4_096,
        field_bytes: 1_024,
        batch_rows: 2,
        batch_text_bytes: 1_024,
    }
}

#[test]
fn header_maps_types_and_quoted_text() {
    let schema = [
        ColumnDeclaration {
            name: "id",
            data_type: DataType::Int64,
            nullable: false,
        },
        ColumnDeclaration {
            name: "text",
            data_type: DataType::String,
            nullable: true,
        },
        ColumnDeclaration {
            name: "date",
            data_type: DataType::Date,
            nullable: false,
        },
        ColumnDeclaration {
            name: "number",
            data_type: DataType::Double,
            nullable: false,
        },
    ];
    let input =
        b"TEXT,number,ID,date\r\n\"a,\"\"b\"\"\n\",-0,42,1970-01-02\r\n\\N,1.25e2,-7,1969-12-31";
    let cancel = CancellationToken::new();
    let mut decoder = CsvDecoder::new(&input[..], &schema, limits(), 1_000_000, &cancel).unwrap();
    let batch = decoder.next_batch(&cancel).unwrap().unwrap();
    assert_eq!(batch.row_count(), 2);
    assert_eq!(batch.column_count(), 4);
    assert_eq!(batch.value(0, 0), Some(Value::Int64(42)));
    assert_eq!(
        batch.value(0, 1),
        Some(Value::String(StringValue::new("a,\"b\"\n")))
    );
    assert_eq!(
        batch.value(0, 2),
        Some(Value::Date(
            DateValue::from_days_since_unix_epoch(1).unwrap()
        ))
    );
    let Some(Value::Double(number)) = batch.value(0, 3) else {
        panic!("DOUBLE")
    };
    assert_eq!(number.to_bits(), (-0.0f64).to_bits());
    assert_eq!(batch.value(1, 0), Some(Value::Int64(-7)));
    assert_eq!(batch.value(1, 1), Some(Value::Null));
    assert_eq!(
        batch.value(1, 2),
        Some(Value::Date(
            DateValue::from_days_since_unix_epoch(-1).unwrap()
        ))
    );
    assert_eq!(batch.value(1, 3), Some(Value::Double(125.0)));
    assert_eq!(batch.value(2, 0), None);
    assert!(decoder.next_batch(&cancel).unwrap().is_none());
    assert!(decoder.next_batch(&cancel).unwrap().is_none());
}

const STRING: [ColumnDeclaration<'static>; 1] = [ColumnDeclaration {
    name: "s",
    data_type: DataType::String,
    nullable: true,
}];

struct Chunks<'a> {
    remaining: &'a [u8],
    size: usize,
    split: usize,
    consumed: usize,
}

impl Read for Chunks<'_> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let to_split = if self.consumed < self.split {
            self.split - self.consumed
        } else {
            usize::MAX
        };
        let length = output
            .len()
            .min(self.size)
            .min(self.remaining.len())
            .min(to_split);
        output[..length].copy_from_slice(&self.remaining[..length]);
        self.remaining = &self.remaining[length..];
        self.consumed += length;
        Ok(length)
    }
}

fn strings<R: Read>(reader: R, limits: CsvLimits) -> Result<Vec<Vec<Option<String>>>, Error> {
    let cancel = CancellationToken::new();
    let mut decoder = CsvDecoder::new(reader, &STRING, limits, 10_000_000, &cancel)?;
    let mut batches = Vec::new();
    while let Some(batch) = decoder.next_batch(&cancel)? {
        let mut values = Vec::new();
        for row in 0..batch.row_count() {
            values.push(match batch.value(row, 0).unwrap() {
                Value::Null => None,
                Value::String(value) => Some(value.as_str().to_owned()),
                _ => panic!("expected STRING"),
            });
        }
        batches.push(values);
    }
    Ok(batches)
}

#[test]
fn every_split_preserves_unicode_quotes_endings_and_nulls() {
    let input = "s\r\n\"雪,\"\"x\"\"\r\ny\"\n\\N\n\"\\N\"\n\"\"\n\nlast".as_bytes();
    let expected = vec![
        vec![Some("雪,\"x\"\r\ny".to_owned()), None],
        vec![Some("\\N".to_owned()), Some(String::new())],
        vec![Some(String::new()), Some("last".to_owned())],
    ];
    for split in 0..=input.len() {
        for size in [1, 2, 3, 7, READ_BYTES] {
            let reader = Chunks {
                remaining: input,
                size,
                split,
                consumed: 0,
            };
            assert_eq!(
                strings(reader, limits()).unwrap(),
                expected,
                "split {split}, chunk {size}"
            );
        }
    }
}

#[test]
fn text_limit_defers_one_complete_row_without_losing_it() {
    let mut limit = limits();
    limit.batch_rows = 256;
    limit.batch_text_bytes = 4;
    assert_eq!(
        strings(&b"s\naa\nbbb\n\n\\N\nc\ndd\n"[..], limit).unwrap(),
        vec![
            vec![Some("aa".into())],
            vec![Some("bbb".into()), Some("".into()), None, Some("c".into())],
            vec![Some("dd".into())],
        ]
    );
}

fn failure(
    input: &[u8],
    schema: &[ColumnDeclaration<'_>],
    limits: CsvLimits,
    expected: &str,
    offset: u64,
) {
    for size in [1, 2, 7, READ_BYTES] {
        let cancel = CancellationToken::new();
        let reader = Chunks {
            remaining: input,
            size,
            split: 0,
            consumed: 0,
        };
        let mut decoder = CsvDecoder::new(reader, schema, limits, 20_000_000, &cancel).unwrap();
        let error = loop {
            match decoder.next_batch(&cancel) {
                Ok(Some(_)) => {}
                Ok(None) => panic!("missing {expected}"),
                Err(error) => break error,
            }
        };
        assert!(
            matches!(error, Error::Input { message, byte_offset } if message == expected && byte_offset == offset),
            "{error:?}, input={input:?}, chunk={size}"
        );
        assert!(matches!(
            decoder.next_batch(&cancel),
            Err(Error::Unsupported("CSV decoder already failed"))
        ));
    }
}

#[test]
fn malformed_records_have_original_byte_offsets() {
    for (input, message, offset) in [
        (&b""[..], "missing CSV header", 0),
        (&b"s\n\"abc"[..], "incomplete CSV record", 6),
        (&b"s\nx\r"[..], "incomplete CSV record", 4),
        (&b"s\nx\ry"[..], "expected LF after CR", 4),
        (&b"s\na\"b\n"[..], "invalid CSV quote", 3),
        (&b"s\n\"a\" \n"[..], "invalid CSV quote", 5),
        (&b"s\na,b\n"[..], "too many CSV fields", 4),
        (&b"s\n\"a\"\"\xff\"\n"[..], "invalid CSV UTF-8", 6),
        (&b"s\n\xe9\x9b\n"[..], "invalid CSV UTF-8", 2),
        (&b"\xef\xbb\xbfs\n"[..], "unknown CSV header column", 0),
    ] {
        failure(input, &STRING, limits(), message, offset);
    }
    let schema = [
        STRING[0],
        ColumnDeclaration {
            name: "t",
            ..STRING[0]
        },
    ];
    failure(b"s\n", &schema, limits(), "too few CSV fields", 1);
    failure(
        b"s,S\n",
        &schema,
        limits(),
        "duplicate CSV header column",
        2,
    );
    failure(b"s,t\nvalue\n", &schema, limits(), "too few CSV fields", 9);
    failure(b"s,z\n", &schema, limits(), "unknown CSV header column", 2);
}

#[test]
fn limits_accept_exact_boundaries_and_locate_first_excess() {
    let mut limit = limits();
    let input = b"s\n\"a\"\"b\"\r\n";
    limit.record_bytes = 8;
    limit.field_bytes = 3;
    limit.input_bytes = input.len() as u64;
    limit.rows = 1;
    limit.batch_text_bytes = 3;
    assert_eq!(
        strings(&input[..], limit).unwrap(),
        vec![vec![Some("a\"b".into())]]
    );
    let mut short = limit;
    short.input_bytes -= 1;
    failure(input, &STRING, short, "CSV input byte limit", 9);
    short = limit;
    short.record_bytes -= 1;
    failure(input, &STRING, short, "CSV record byte limit", 9);
    short = limit;
    short.field_bytes -= 1;
    failure(input, &STRING, short, "CSV field byte limit", 6);
    short = limit;
    short.batch_text_bytes -= 1;
    failure(input, &STRING, short, "CSV row exceeds batch text limit", 2);
    short = limits();
    short.rows = 1;
    failure(b"s\na\nb\n", &STRING, short, "CSV row limit", 4);
}

#[test]
fn numeric_and_date_syntax_rejects_noncanonical_or_out_of_range_input() {
    for (kind, message, rejected) in [
        (
            DataType::Int64,
            "invalid CSV INT64",
            vec![
                "",
                " 1",
                "1 ",
                "+",
                "--1",
                "1.0",
                "1e2",
                "9223372036854775808",
                "-9223372036854775809",
                "١",
            ],
        ),
        (
            DataType::Double,
            "invalid CSV DOUBLE",
            vec![
                "",
                " ",
                ".",
                "1e",
                "+e2",
                "1e+",
                "1e9999",
                "inf",
                "nan",
                "+Infinity",
                "NaN ",
                "0x1",
                "1_0",
            ],
        ),
        (
            DataType::Date,
            "invalid CSV DATE",
            vec![
                "2023-02-29",
                "1900-02-29",
                "0000-01-01",
                "10000-01-01",
                "2000-13-01",
                "2000-01-00",
                "2000-1-01",
                " 2000-01-01",
            ],
        ),
    ] {
        let schema = [ColumnDeclaration {
            name: "s",
            data_type: kind,
            nullable: true,
        }];
        for text in rejected {
            failure(
                format!("s\n\"{text}\"\n").as_bytes(),
                &schema,
                limits(),
                message,
                2,
            );
        }
    }
    let schema = [ColumnDeclaration {
        nullable: false,
        ..STRING[0]
    }];
    failure(
        b"s\n\\N\n",
        &schema,
        limits(),
        "NULL in nonnullable CSV column",
        2,
    );
}

#[test]
fn numeric_extremes_special_values_and_calendar_boundaries() {
    let cancel = CancellationToken::new();
    for (kind, input, expected) in [
        (
            DataType::Int64,
            "s\n-9223372036854775808\n+9223372036854775807",
            vec![Value::Int64(i64::MIN), Value::Int64(i64::MAX)],
        ),
        (
            DataType::Double,
            "s\n.5\n1.\n+2E-1\n-1e-9999",
            vec![
                Value::Double(0.5),
                Value::Double(1.0),
                Value::Double(0.2),
                Value::Double(-0.0),
            ],
        ),
        (
            DataType::Date,
            "s\n0001-01-01\n9999-12-31\n2000-02-29",
            vec![
                Value::Date(DateValue::from_days_since_unix_epoch(-719_162).unwrap()),
                Value::Date(DateValue::from_days_since_unix_epoch(2_932_896).unwrap()),
                Value::Date(DateValue::from_days_since_unix_epoch(11_016).unwrap()),
            ],
        ),
    ] {
        let schema = [ColumnDeclaration {
            name: "s",
            data_type: kind,
            nullable: true,
        }];
        let mut decoder =
            CsvDecoder::new(input.as_bytes(), &schema, limits(), 1_000_000, &cancel).unwrap();
        let mut index = 0;
        while let Some(batch) = decoder.next_batch(&cancel).unwrap() {
            for row in 0..batch.row_count() {
                assert_eq!(batch.value(row, 0), Some(expected[index]));
                index += 1;
            }
        }
        assert_eq!(index, expected.len());
    }
    let schema = [ColumnDeclaration {
        data_type: DataType::Double,
        ..STRING[0]
    }];
    let mut decoder = CsvDecoder::new(
        &b"s\nNaN\nInfinity\n-Infinity"[..],
        &schema,
        limits(),
        1_000_000,
        &cancel,
    )
    .unwrap();
    let batch = decoder.next_batch(&cancel).unwrap().unwrap();
    assert!(matches!(batch.value(0, 0), Some(Value::Double(value)) if value.is_nan()));
    assert_eq!(batch.value(1, 0), Some(Value::Double(f64::INFINITY)));
    assert_eq!(
        decoder.next_batch(&cancel).unwrap().unwrap().value(0, 0),
        Some(Value::Double(f64::NEG_INFINITY))
    );
    assert!(decoder.next_batch(&cancel).unwrap().is_none());
}

struct FailingReader<'a> {
    data: &'a [u8],
    fail_at: usize,
    consumed: usize,
    cancel: Option<&'a CancellationToken>,
}

impl Read for FailingReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if self.consumed == self.fail_at {
            if let Some(cancel) = self.cancel {
                cancel.cancel();
                output[0] = b'x';
                return Ok(1);
            }
            return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        }
        let length = output
            .len()
            .min(self.data.len())
            .min(self.fail_at - self.consumed);
        output[..length].copy_from_slice(&self.data[..length]);
        self.data = &self.data[length..];
        self.consumed += length;
        Ok(length)
    }
}

#[test]
fn io_and_cancellation_failures_stop_at_every_input_prefix() {
    let input = b"s\n\"one\"\n\"two\"\n\"three\"\n";
    for fail_at in 0..=input.len() {
        for cancelled in [false, true] {
            let cancel = CancellationToken::new();
            let reader = FailingReader {
                data: input,
                fail_at,
                consumed: 0,
                cancel: cancelled.then_some(&cancel),
            };
            let mut decoder =
                CsvDecoder::new(reader, &STRING, limits(), 1_000_000, &cancel).unwrap();
            let mut rows = 0;
            let error = loop {
                match decoder.next_batch(&cancel) {
                    Ok(Some(batch)) => rows += batch.row_count(),
                    Ok(None) => panic!("failure omitted at {fail_at}"),
                    Err(error) => break error,
                }
            };
            assert!(rows <= 2, "last incomplete batch must not escape");
            if cancelled {
                assert!(matches!(error, Error::Cancelled));
            } else {
                assert!(
                    matches!(error, Error::Io { operation: "read CSV", source } if source.kind() == std::io::ErrorKind::PermissionDenied)
                );
            }
            assert!(matches!(
                decoder.next_batch(&cancel),
                Err(Error::Unsupported(_))
            ));
        }
    }
}

#[test]
fn admission_and_entry_cancellation_do_not_read() {
    struct NoRead;
    impl Read for NoRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("must not read")
        }
    }
    let cancel = CancellationToken::new();
    let required = CsvDecoder::<NoRead>::required_memory(&STRING, limits()).unwrap();
    assert!(
        matches!(CsvDecoder::new(NoRead, &STRING, limits(), required - 1, &cancel), Err(Error::Resource { owner: "CSV decoder", required: actual, limit }) if actual == required && limit == required - 1)
    );
    let decoder = CsvDecoder::new(NoRead, &STRING, limits(), required, &cancel).unwrap();
    assert_eq!(decoder.memory_bytes(), required);
    drop(decoder);
    let mut decoder = CsvDecoder::new(NoRead, &STRING, limits(), required, &cancel).unwrap();
    cancel.cancel();
    assert!(matches!(decoder.next_batch(&cancel), Err(Error::Cancelled)));
    assert!(matches!(
        CsvDecoder::new(NoRead, &STRING, limits(), required, &cancel),
        Err(Error::Cancelled)
    ));
    let cancel = CancellationToken::new();
    for field in 0..7 {
        let mut invalid = limits();
        match field {
            0 => invalid.input_bytes = 0,
            1 => invalid.rows = 0,
            2 => invalid.record_bytes = 0,
            3 => invalid.field_bytes = 0,
            4 => invalid.field_bytes = MAX_FIELD_BYTES + 1,
            5 => invalid.batch_rows = 257,
            _ => invalid.batch_text_bytes = 0,
        }
        assert!(matches!(
            CsvDecoder::new(NoRead, &STRING, invalid, u64::MAX, &cancel),
            Err(Error::InvalidConfig(_))
        ));
    }
    for schema in [
        &[][..],
        &[STRING[0], STRING[0]][..],
        &[ColumnDeclaration {
            name: "bad name",
            ..STRING[0]
        }][..],
    ] {
        assert!(matches!(
            CsvDecoder::new(NoRead, schema, limits(), u64::MAX, &cancel),
            Err(Error::InvalidConfig(_))
        ));
    }
}

#[test]
fn largest_fields_cross_read_buffers_without_growth() {
    let text = "雪".repeat(21_845) + "\0";
    assert_eq!(text.len(), 65_536);
    let input = format!("s\n\"{text}\"\r\n");
    let limit = CsvLimits {
        field_bytes: 65_536,
        record_bytes: 131_074,
        batch_text_bytes: 65_536,
        ..limits()
    };
    let cancel = CancellationToken::new();
    let mut decoder =
        CsvDecoder::new(input.as_bytes(), &STRING, limit, 1_000_000, &cancel).unwrap();
    let capacities = (
        decoder.raw.capacity(),
        decoder.decoded.capacity(),
        decoder.cells.capacity(),
        decoder.text.capacity(),
    );
    let batch = decoder.next_batch(&cancel).unwrap().unwrap();
    assert_eq!(batch.row_count(), 1);
    assert_eq!(
        batch.value(0, 0),
        Some(Value::String(StringValue::new(&text)))
    );
    assert!(decoder.next_batch(&cancel).unwrap().is_none());
    assert_eq!(
        (
            decoder.raw.capacity(),
            decoder.decoded.capacity(),
            decoder.cells.capacity(),
            decoder.text.capacity()
        ),
        capacities
    );
    let too_long = format!("s\n\"{text}x\"\n");
    failure(
        too_long.as_bytes(),
        &STRING,
        limit,
        "CSV field byte limit",
        65_539,
    );
}

#[test]
fn interrupted_reads_retry_with_a_finite_limit_and_eof_stays_finished() {
    struct Interruptions {
        left: usize,
        reads: usize,
    }
    impl Read for Interruptions {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            if self.left > 0 {
                self.left -= 1;
                Err(std::io::ErrorKind::Interrupted.into())
            } else {
                output[..2].copy_from_slice(b"s\n");
                Ok(2)
            }
        }
    }
    let cancel = CancellationToken::new();
    let mut decoder = CsvDecoder::new(
        Interruptions { left: 17, reads: 0 },
        &STRING,
        limits(),
        1_000_000,
        &cancel,
    )
    .unwrap();
    assert!(
        matches!(decoder.next_batch(&cancel), Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::Interrupted)
    );
    assert_eq!(decoder.reader.reads, 17);
    struct Once {
        state: u8,
    }
    impl Read for Once {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            self.state += 1;
            match self.state {
                1 => {
                    output[..3].copy_from_slice(b"s\nx");
                    Ok(3)
                }
                2 => Ok(0),
                _ => panic!("must not read after EOF"),
            }
        }
    }
    assert_eq!(
        strings(Once { state: 0 }, limits()).unwrap(),
        vec![vec![Some("x".into())]]
    );
}

#[test]
fn maximum_column_and_batch_counts_preserve_declaration_order() {
    let names: Vec<_> = (0..64).map(|column| format!("c{column}")).collect();
    let schema: Vec<_> = names
        .iter()
        .map(|name| ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: false,
        })
        .collect();
    let mut input = names
        .iter()
        .rev()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(",")
        + "\n";
    for row in 0..257 {
        for column in (0..64).rev() {
            use std::fmt::Write;
            write!(
                &mut input,
                "{}{}",
                row * 1000 + column,
                if column == 0 { '\n' } else { ',' }
            )
            .unwrap();
        }
    }
    let limit = CsvLimits {
        input_bytes: 1_000_000,
        batch_rows: 256,
        batch_text_bytes: 1,
        ..limits()
    };
    let cancel = CancellationToken::new();
    let mut decoder =
        CsvDecoder::new(input.as_bytes(), &schema, limit, 1_000_000, &cancel).unwrap();
    let mut count = 0;
    for expected_size in [256, 1] {
        let batch = decoder.next_batch(&cancel).unwrap().unwrap();
        assert_eq!(batch.row_count(), expected_size);
        assert_eq!(batch.column_count(), 64);
        for row in 0..expected_size {
            for column in 0..64 {
                assert_eq!(
                    batch.value(row, column),
                    Some(Value::Int64(((count + row) * 1000 + column) as i64))
                );
            }
        }
        count += expected_size;
    }
    assert_eq!(count, 257);
    assert!(decoder.next_batch(&cancel).unwrap().is_none());
}

#[test]
fn quoted_delimiter_combinations_keep_exact_text() {
    let alphabet = ["a", ",", "\"", "\r", "\n", "雪", "\0"];
    for first in alphabet {
        for second in alphabet {
            for third in alphabet {
                let text = format!("{first}{second}{third}");
                let input = format!("s\r\n\"{}\"\r\n", text.replace('"', "\"\""));
                assert_eq!(
                    strings(input.as_bytes(), limits()).unwrap(),
                    vec![vec![Some(text)]]
                );
            }
        }
    }
}

#[test]
fn field_refusal_does_not_wait_for_a_record_ending() {
    let cancel = CancellationToken::new();
    for (data, field_bytes, offset) in [(&b"s\nabc"[..], 2, 4), (&b"s\n\"a\"\""[..], 1, 4)] {
        let reader = FailingReader {
            data,
            fail_at: data.len(),
            consumed: 0,
            cancel: None,
        };
        let mut decoder = CsvDecoder::new(
            reader,
            &STRING,
            CsvLimits {
                field_bytes,
                ..limits()
            },
            1_000_000,
            &cancel,
        )
        .unwrap();
        assert!(
            matches!(decoder.next_batch(&cancel), Err(Error::Input { message: "CSV field byte limit", byte_offset }) if byte_offset == offset)
        );
    }
}
