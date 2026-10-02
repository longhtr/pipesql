//! Verify that complete CSV imports publish once and failed imports publish nothing.
//!
//! Literal rows check conversion independently of the decoder. Late failures occur
//! after an earlier decoder batch has been written, so an empty final table proves
//! that the importer did not commit that prefix. Transaction lookup and a same-handle
//! retry check both the durable outcome and release of writer ownership.

use super::*;
use crate::test_support::Directory;
use crate::{CommitResolution, Config, DataType, DateValue, QueryStep, StringValue, Value};
use std::cell::Cell;

fn database(directory: &Directory) -> Database {
    let db = Database::create_empty(
        &directory.0.join("db"),
        Config::new(8_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    db.declare_table(
        "facts",
        &[
            ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
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
        ],
        &CancellationToken::new(),
    )
    .unwrap();
    db
}

fn limits() -> ImportLimits {
    ImportLimits {
        csv: CsvLimits {
            input_bytes: 100_000,
            rows: 100,
            record_bytes: 4_096,
            field_bytes: 1_024,
            batch_rows: 2,
            batch_text_bytes: 4_096,
        },
        append: AppendLimits {
            batches: 8,
            encoded_bytes: 100_000,
        },
    }
}

fn check_rows(db: &Database, expected: &[[Value<'_>; 4]]) {
    let before = db.reserved_memory_bytes();
    let query = db.prepare("FROM facts |> ORDER BY id").unwrap();
    let cancel = CancellationToken::new();
    let mut result = db.execute(&query, &cancel).unwrap();
    let mut seen = 0;
    let mut finished = false;
    for _ in 0..10_000 {
        match result.step() {
            QueryStep::Progress => {}
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 4);
                for row in 0..batch.len() {
                    assert!(seen < expected.len());
                    for (column, &wanted) in expected[seen].iter().enumerate() {
                        match wanted {
                            Value::Double(wanted) => assert!(
                                matches!(batch.value(row, column), Some(Value::Double(actual)) if actual.to_bits() == wanted.to_bits())
                            ),
                            wanted => assert_eq!(batch.value(row, column), Some(wanted)),
                        }
                    }
                    seen += 1;
                }
            }
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("{error:?}"),
        }
    }
    assert!(finished);
    assert_eq!(seen, expected.len());
    drop(result);
    drop(query);
    assert_eq!(db.reserved_memory_bytes(), before);
}

const VALID: &[u8] = b"note,day,id,number\n\"one,\"\"x\"\"\",1970-01-02,1,1.5\n\\N,\\N,2,\\N\n\"\",1969-12-31,3,-0\n";

#[test]
fn successful_import_reports_issuance_before_reading_and_preserves_old_snapshot() {
    struct Guarded<'a> {
        issued: &'a Cell<bool>,
        input: &'a [u8],
    }
    impl Read for Guarded<'_> {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            assert!(self.issued.get());
            self.input.read(output)
        }
    }
    let directory = Directory::new();
    let db = database(&directory);
    let baseline = db.reserved_memory_bytes();
    let old = db.prepare("FROM facts").unwrap();
    let reported = Cell::new(false);
    let mut transaction = None;
    let cancel = CancellationToken::new();
    let commit = db
        .import_csv(
            "FACTS",
            Guarded {
                issued: &reported,
                input: VALID,
            },
            limits(),
            &cancel,
            |token| {
                assert!(!reported.replace(true));
                transaction = Some(token);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(Some(commit.transaction()), transaction);
    assert_eq!(commit.generation(), 2);
    assert_eq!(
        db.resolve_commit(commit.transaction()).unwrap(),
        CommitResolution::Durable(commit)
    );
    let mut old_result = db.execute(&old, &cancel).unwrap();
    let mut finished = false;
    for _ in 0..100 {
        match old_result.step() {
            QueryStep::Progress => {}
            QueryStep::Finished => {
                finished = true;
                break;
            }
            _ => panic!("old snapshot changed"),
        }
    }
    assert!(finished);
    drop(old_result);
    drop(old);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    check_rows(
        &db,
        &[
            [
                Value::Int64(1),
                Value::String(StringValue::new("one,\"x\"")),
                Value::Double(1.5),
                Value::Date(DateValue::from_days_since_unix_epoch(1).unwrap()),
            ],
            [Value::Int64(2), Value::Null, Value::Null, Value::Null],
            [
                Value::Int64(3),
                Value::String(StringValue::new("")),
                Value::Double(-0.0),
                Value::Date(DateValue::from_days_since_unix_epoch(-1).unwrap()),
            ],
        ],
    );
    db.close().unwrap();
    let db = Database::open(
        &directory.0.join("db"),
        Config::new(8_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    assert_eq!(
        db.resolve_commit(commit.transaction()).unwrap(),
        CommitResolution::Durable(commit)
    );
}

#[test]
fn late_bad_row_aborts_written_prefix_and_allows_retry() {
    let directory = Directory::new();
    let db = database(&directory);
    let baseline = db.reserved_memory_bytes();
    let mut input = VALID.to_vec();
    input.extend_from_slice(b"late,1970-01-01,bad,1\n");
    let mut transaction = None;
    let cancel = CancellationToken::new();
    let error = db
        .import_csv("facts", &input[..], limits(), &cancel, |token| {
            transaction = Some(token);
            Ok(())
        })
        .unwrap_err();
    assert!(
        matches!(error, Error::Input { message: "invalid CSV INT64", byte_offset } if byte_offset == VALID.len() as u64 + 16)
    );
    assert_eq!(
        db.resolve_commit(transaction.unwrap()).unwrap(),
        CommitResolution::Aborted
    );
    assert_eq!(db.generation(), 1);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    check_rows(&db, &[]);
    let commit = db
        .import_csv("facts", VALID, limits(), &cancel, |_| Ok(()))
        .unwrap();
    assert_eq!(commit.generation(), 2);
    assert!(commit.transaction().sequence() > transaction.unwrap().sequence());
}

#[test]
fn receipt_failure_and_empty_input_abort_without_publication() {
    struct NoRead;
    impl Read for NoRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("receipt failed before input")
        }
    }
    let directory = Directory::new();
    let db = database(&directory);
    let cancel = CancellationToken::new();
    let baseline = db.reserved_memory_bytes();
    let mut token = None;
    let error = db
        .import_csv("facts", NoRead, limits(), &cancel, |transaction| {
            token = Some(transaction);
            Err(std::io::ErrorKind::BrokenPipe.into())
        })
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Io {
            operation: "report CSV transaction",
            ..
        }
    ));
    assert_eq!(
        db.resolve_commit(token.unwrap()).unwrap(),
        CommitResolution::Aborted
    );
    for input in [&b""[..], &b"id,note,number,day\n"[..]] {
        let error = db
            .import_csv("facts", input, limits(), &cancel, |_| Ok(()))
            .unwrap_err();
        assert!(matches!(error, Error::Input { byte_offset: 0, .. }));
        assert_eq!(db.generation(), 1);
    }
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    check_rows(&db, &[]);
}

#[test]
fn late_reader_failure_and_cancellation_discard_private_units() {
    struct FailsAtEnd<'a> {
        input: &'a [u8],
        cancel: Option<&'a CancellationToken>,
    }
    impl Read for FailsAtEnd<'_> {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            if self.input.is_empty() {
                if let Some(cancel) = self.cancel {
                    cancel.cancel();
                    return Ok(0);
                }
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            self.input.read(output)
        }
    }
    for cancel_at_end in [false, true] {
        let directory = Directory::new();
        let db = database(&directory);
        let cancel = CancellationToken::new();
        let baseline = db.reserved_memory_bytes();
        let mut token = None;
        let error = db
            .import_csv(
                "facts",
                FailsAtEnd {
                    input: VALID,
                    cancel: cancel_at_end.then_some(&cancel),
                },
                limits(),
                &cancel,
                |transaction| {
                    token = Some(transaction);
                    Ok(())
                },
            )
            .unwrap_err();
        if cancel_at_end {
            assert!(matches!(error, Error::Cancelled));
        } else {
            assert!(
                matches!(error, Error::Io { source, .. } if source.kind() == std::io::ErrorKind::BrokenPipe)
            );
        }
        assert_eq!(
            db.resolve_commit(token.unwrap()).unwrap(),
            CommitResolution::Aborted
        );
        assert_eq!(db.generation(), 1);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
        check_rows(&db, &[]);
        db.import_csv("facts", VALID, limits(), &CancellationToken::new(), |_| {
            Ok(())
        })
        .unwrap();
    }
}

#[test]
fn append_limit_refusal_discards_an_earlier_batch() {
    for encoded in [false, true] {
        let directory = Directory::new();
        let db = database(&directory);
        let mut bounds = limits();
        if encoded {
            bounds.append.encoded_bytes = 1;
        } else {
            bounds.append.batches = 1;
        }
        let mut token = None;
        assert!(
            db.import_csv(
                "facts",
                VALID,
                bounds,
                &CancellationToken::new(),
                |transaction| {
                    token = Some(transaction);
                    Ok(())
                }
            )
            .is_err()
        );
        assert_eq!(
            db.resolve_commit(token.unwrap()).unwrap(),
            CommitResolution::Aborted
        );
        assert_eq!(db.generation(), 1);
        check_rows(&db, &[]);
        db.import_csv("facts", VALID, limits(), &CancellationToken::new(), |_| {
            Ok(())
        })
        .unwrap();
    }
}

#[test]
fn wide_text_batch_splits_at_native_column_limit() {
    let directory = Directory::new();
    let db = database(&directory);
    let text = "x".repeat(65_536);
    let mut input = String::from("id,note,number,day\n");
    for id in 1..=8 {
        use std::fmt::Write;
        writeln!(input, "{id},{text},1,1970-01-01").unwrap();
    }
    let mut bounds = limits();
    bounds.csv.input_bytes = input.len() as u64;
    bounds.csv.record_bytes = 70_000;
    bounds.csv.field_bytes = 65_536;
    bounds.csv.batch_rows = 8;
    bounds.csv.batch_text_bytes = 524_288;
    bounds.append.encoded_bytes = 1_000_000;
    bounds.append.batches = 1;
    let mut token = None;
    assert!(
        db.import_csv(
            "facts",
            input.as_bytes(),
            bounds,
            &CancellationToken::new(),
            |transaction| {
                token = Some(transaction);
                Ok(())
            }
        )
        .is_err()
    );
    assert_eq!(
        db.resolve_commit(token.unwrap()).unwrap(),
        CommitResolution::Aborted
    );
    check_rows(&db, &[]);
    bounds.append.batches = 2;
    db.import_csv(
        "facts",
        input.as_bytes(),
        bounds,
        &CancellationToken::new(),
        |_| Ok(()),
    )
    .unwrap();
    let expected: Vec<_> = (1..=8)
        .map(|id| {
            [
                Value::Int64(id),
                Value::String(StringValue::new(&text)),
                Value::Double(1.0),
                Value::Date(DateValue::from_days_since_unix_epoch(0).unwrap()),
            ]
        })
        .collect();
    check_rows(&db, &expected);
}

mod failures;
mod wide;

#[test]
fn conversion_keeps_repeated_types_and_partial_validity_bytes_separate() {
    use std::fmt::Write;

    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("db"),
        Config::new(8_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let columns = [
        ("id", DataType::Int64, false),
        ("a", DataType::Int64, true),
        ("b", DataType::String, true),
        ("c", DataType::Double, true),
        ("d", DataType::Date, true),
        ("e", DataType::Int64, true),
        ("f", DataType::String, true),
        ("g", DataType::Double, true),
        ("h", DataType::Date, true),
    ]
    .map(|(name, data_type, nullable)| ColumnDeclaration {
        name,
        data_type,
        nullable,
    });
    let cancel = CancellationToken::new();
    db.declare_table("many", &columns, &cancel).unwrap();
    let baseline = db.reserved_memory_bytes();
    // Each bit names a nullable column, independently of other columns. Nine-row
    // batches exercise partial validity bytes and their reuse on later batches.
    let masks: [u8; 19] = [
        0x00, 0xff, 0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x55, 0xaa, 0xfe, 0xfd, 0xfb,
        0xf7, 0xef, 0xdf, 0xbf,
    ];
    let fields = [
        "10",
        "left",
        "1.5",
        "1970-01-02",
        "20",
        "right",
        "-0",
        "1969-12-31",
    ];
    let mut input = String::from("id,h,g,f,e,d,c,b,a\n");
    for (row, mask) in masks.iter().enumerate() {
        write!(input, "{row}").unwrap();
        for column in (0..8).rev() {
            let field = if mask & (1 << column) == 0 {
                r"\N"
            } else {
                fields[column]
            };
            write!(input, ",{field}").unwrap();
        }
        input.push('\n');
    }
    let mut bounds = limits();
    bounds.csv.batch_rows = 9;
    db.import_csv("many", input.as_bytes(), bounds, &cancel, |_| Ok(()))
        .unwrap();
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    let query = db.prepare("FROM many |> ORDER BY id").unwrap();
    let mut result = db.execute(&query, &cancel).unwrap();
    let mut rows = 0;
    let mut finished = false;
    let expected = [
        Value::Int64(10),
        Value::String(StringValue::new("left")),
        Value::Double(1.5),
        Value::Date(DateValue::from_days_since_unix_epoch(1).unwrap()),
        Value::Int64(20),
        Value::String(StringValue::new("right")),
        Value::Double(-0.0),
        Value::Date(DateValue::from_days_since_unix_epoch(-1).unwrap()),
    ];
    for _ in 0..10_000 {
        match result.step() {
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 9);
                for row in 0..batch.len() {
                    assert!(rows < masks.len());
                    assert_eq!(batch.value(row, 0), Some(Value::Int64(rows as i64)));
                    for (column, &value) in expected.iter().enumerate() {
                        let wanted = if masks[rows] & (1 << column) == 0 {
                            Value::Null
                        } else {
                            value
                        };
                        match wanted {
                            Value::Double(wanted) => assert!(matches!(
                                batch.value(row, column + 1), Some(Value::Double(actual))
                                if actual.to_bits() == wanted.to_bits()
                            )),
                            wanted => assert_eq!(batch.value(row, column + 1), Some(wanted)),
                        }
                    }
                    rows += 1;
                }
            }
            QueryStep::Progress => {}
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("{error:?}"),
        }
    }
    assert!(finished);
    assert_eq!(rows, masks.len());
    drop(result);
    drop(query);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close().unwrap();
}

#[test]
fn independent_wide_columns_choose_native_cuts_and_abort_without_a_prefix() {
    use std::fmt::Write;

    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("db"),
        Config::new(8_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let schema = [
        ColumnDeclaration {
            name: "id",
            data_type: DataType::Int64,
            nullable: false,
        },
        ColumnDeclaration {
            name: "a",
            data_type: DataType::String,
            nullable: true,
        },
        ColumnDeclaration {
            name: "b",
            data_type: DataType::String,
            nullable: true,
        },
    ];
    let cancel = CancellationToken::new();
    db.declare_table("facts", &schema, &cancel).unwrap();
    db.import_csv(
        "facts",
        &b"id,a,b\n99,kept-a,kept-b\n"[..],
        limits(),
        &cancel,
        |_| Ok(()),
    )
    .unwrap();
    let baseline = db.reserved_memory_bytes();
    let generation = db.generation();
    let a = "a".repeat(65_536);
    let b = "雪".repeat(21_845) + "x";
    assert_eq!(b.len(), 65_536);
    let mut input = String::from("b,id,a\n");
    for row in 0..20 {
        let left = if matches!(row, 0..=6 | 8) {
            a.as_str()
        } else if row % 2 == 1 {
            r"\N"
        } else {
            ""
        };
        let right = if matches!(row, 8..=14 | 16) {
            b.as_str()
        } else if row % 3 == 0 {
            r"\N"
        } else {
            "β"
        };
        writeln!(input, "{right},{row},{left}").unwrap();
    }
    let bounds = ImportLimits {
        csv: CsvLimits {
            input_bytes: input.len() as u64,
            rows: 20,
            record_bytes: 131_100,
            field_bytes: 65_536,
            batch_rows: 20,
            batch_text_bytes: 1_100_000,
        },
        append: AppendLimits {
            batches: 3,
            encoded_bytes: 2_000_000,
        },
    };
    {
        let required = CsvDecoder::<&[u8]>::required_memory(&schema, bounds.csv).unwrap();
        let _reservation = db
            .memory
            .reserve(required, "wide cut fixture decoder")
            .unwrap();
        let mut decoder =
            CsvDecoder::new(input.as_bytes(), &schema, bounds.csv, required, &cancel).unwrap();
        let conversion = Columns::new(&db.memory, &schema, 20).unwrap();
        let batch = decoder.next_batch(&cancel).unwrap().unwrap();
        assert_eq!(batch.row_count(), 20);
        // Eight maximum strings exceed 512 KiB once offsets and validity are
        // included. A forces the first cut; B independently forces the second.
        let mut start = 0;
        for end in [8, 16, 20] {
            assert_eq!(conversion.next_end(&batch, start, &cancel).unwrap(), end);
            start = end;
        }
        assert!(decoder.next_batch(&cancel).unwrap().is_none());
    }
    assert_eq!(db.reserved_memory_bytes(), baseline);
    let check = |expected: &[[Value<'_>; 3]]| {
        let query = db.prepare("FROM facts |> ORDER BY id").unwrap();
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut seen = 0;
        let mut finished = false;
        for _ in 0..10_000 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Rows(batch) => {
                    assert_eq!(batch.column_count(), 3);
                    for row in 0..batch.len() {
                        assert!(seen < expected.len());
                        for (column, &wanted) in expected[seen].iter().enumerate() {
                            assert_eq!(batch.value(row, column), Some(wanted));
                        }
                        seen += 1;
                    }
                }
                QueryStep::Finished => {
                    finished = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("{error:?}"),
            }
        }
        assert!(finished);
        assert_eq!(seen, expected.len());
    };
    let old = [
        Value::Int64(99),
        Value::String(StringValue::new("kept-a")),
        Value::String(StringValue::new("kept-b")),
    ];
    let mut short = bounds;
    short.append.batches = 2;
    let mut token = None;
    let error = db
        .import_csv("facts", input.as_bytes(), short, &cancel, |issued| {
            token = Some(issued);
            Ok(())
        })
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Resource {
            owner: "append batches",
            required: 3,
            limit: 2
        }
    ));
    assert_eq!(
        db.resolve_commit(token.unwrap()).unwrap(),
        CommitResolution::Aborted
    );
    assert_eq!(db.generation(), generation);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    check(&[old]);
    let commit = db
        .import_csv("facts", input.as_bytes(), bounds, &cancel, |_| Ok(()))
        .unwrap();
    assert_eq!(commit.generation(), generation + 1);
    assert_eq!(
        db.resolve_commit(commit.transaction()).unwrap(),
        CommitResolution::Durable(commit)
    );
    let mut expected: Vec<_> = (0..20)
        .map(|id| {
            let left = if [0, 1, 2, 3, 4, 5, 6, 8].contains(&id) {
                Value::String(StringValue::new(&a))
            } else if [7, 9, 11, 13, 15, 17, 19].contains(&id) {
                Value::Null
            } else {
                Value::String(StringValue::new(""))
            };
            let right = if [8, 9, 10, 11, 12, 13, 14, 16].contains(&id) {
                Value::String(StringValue::new(&b))
            } else if [0, 3, 6, 15, 18].contains(&id) {
                Value::Null
            } else {
                Value::String(StringValue::new("β"))
            };
            [Value::Int64(id), left, right]
        })
        .collect();
    expected.push(old);
    check(&expected);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close().unwrap();
}
