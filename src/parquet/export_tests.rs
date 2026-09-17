//! Exercise bounded Parquet output through real query cursors.
//!
//! Complete files are decoded for schema and every value; byte identity checks
//! show that cursor batching does not choose row-group boundaries. Literal page
//! and footer tests live beside their codecs; external-reader evidence is separate.
//!
//! Controlled writers fail, cancel or panic after progress to check owner release.
//! Admission refusal must write nothing, while late query errors must retain their
//! source spans even after earlier groups were written. Empty results still require
//! a valid schema and completed file.

use super::*;
use crate::parquet::{ParquetReadLimits, decoder::Decoder};
use crate::test_support::Directory;
use crate::{AppendLimits, Config, ParquetImportLimits};
use std::io::Cursor;

const INPUT: &[u8] = include_bytes!("../../test/data/parquet/plain-batches.parquet");
const SCHEMA: [ColumnDeclaration<'static>; 1] = [ColumnDeclaration {
    name: "id",
    data_type: DataType::Int64,
    nullable: false,
}];
const LIMITS: ParquetExportLimits = ParquetExportLimits {
    rows: 600,
    bytes: 20_000,
    row_group_rows: 300,
    row_group_text_bytes: 1,
    row_groups: 2,
    metadata_bytes: 2048,
};

fn database(directory: &Directory, populated: bool) -> Database {
    let db = Database::create_empty(
        &directory.0.join("db"),
        Config::new(8_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    db.declare_table("facts", &SCHEMA, &cancel).unwrap();
    if populated {
        db.import_parquet(
            "facts",
            Cursor::new(INPUT),
            ParquetImportLimits {
                parquet: ParquetReadLimits {
                    input_bytes: 10_000,
                    metadata_bytes: 2048,
                    row_groups: 1,
                    row_group_rows: 600,
                    row_group_bytes: 6000,
                    page_bytes: 1024,
                    rows: 600,
                },
                append: AppendLimits {
                    batches: 4,
                    encoded_bytes: 100_000,
                },
            },
            &cancel,
            |_| Ok(()),
        )
        .unwrap();
    }
    db
}

#[test]
fn complete_output_limits_and_cursor_independent_grouping() {
    let directory = Directory::new();
    let db = database(&directory, true);
    let cancel = CancellationToken::new();
    let ordered = db.prepare("FROM facts |> ORDER BY id").unwrap();
    let baseline = db.reserved_memory_bytes();
    let mut bytes = Vec::new();
    assert_eq!(
        db.export_parquet(&ordered, &mut bytes, LIMITS, &cancel)
            .unwrap(),
        600
    );
    assert_eq!(db.reserved_memory_bytes(), baseline);
    let query = db.prepare("FROM facts").unwrap();
    let mut unsorted = Vec::new();
    db.export_parquet(&query, &mut unsorted, LIMITS, &cancel)
        .unwrap();
    assert_eq!(bytes, unsorted);
    let read_limits = ParquetReadLimits {
        input_bytes: bytes.len() as u64,
        metadata_bytes: 2048,
        row_groups: 2,
        row_group_rows: 300,
        row_group_bytes: 3000,
        page_bytes: 3000,
        rows: 600,
    };
    let mut decoder = Decoder::new(
        Cursor::new(bytes.as_slice()),
        &SCHEMA,
        read_limits,
        1_000_000,
        &cancel,
    )
    .unwrap();
    let mut seen = 0;
    while let Some(batch) = decoder.next_batch(&cancel).unwrap() {
        for row in 0..batch.row_count() {
            assert_eq!(batch.value(row, 0), Some(Value::Int64(seen)));
            seen += 1;
        }
    }
    assert_eq!(seen, 600);
    let exact = ParquetExportLimits {
        bytes: bytes.len() as u64,
        ..LIMITS
    };
    unsorted.clear();
    db.export_parquet(&ordered, &mut unsorted, exact, &cancel)
        .unwrap();
    assert_eq!(bytes, unsorted);
    for limits in [
        ParquetExportLimits {
            bytes: exact.bytes - 1,
            ..exact
        },
        ParquetExportLimits { rows: 599, ..exact },
        ParquetExportLimits {
            row_groups: 1,
            ..exact
        },
        ParquetExportLimits {
            metadata_bytes: 1,
            ..exact
        },
    ] {
        let baseline = db.reserved_memory_bytes();
        let mut partial = Vec::new();
        assert!(matches!(
            db.export_parquet(&ordered, &mut partial, limits, &cancel),
            Err(Error::Resource { .. })
        ));
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert!(!partial.ends_with(b"PAR1"));
    }
}

#[test]
fn empty_result_and_invalid_column_names() {
    let directory = Directory::new();
    let db = database(&directory, false);
    let cancel = CancellationToken::new();
    let query = db.prepare("FROM facts").unwrap();
    let mut bytes = Vec::new();
    assert_eq!(
        db.export_parquet(
            &query,
            &mut bytes,
            ParquetExportLimits { rows: 0, ..LIMITS },
            &cancel
        )
        .unwrap(),
        0
    );
    assert_eq!(&bytes[..4], b"PAR1");
    assert_eq!(&bytes[bytes.len() - 4..], b"PAR1");
    let limits = ParquetReadLimits {
        input_bytes: 10_000,
        metadata_bytes: 2048,
        row_groups: 1,
        row_group_rows: 1,
        row_group_bytes: 100,
        page_bytes: 100,
        rows: 0,
    };
    let mut decoder = Decoder::new(
        Cursor::new(bytes.as_slice()),
        &SCHEMA,
        limits,
        1_000_000,
        &cancel,
    )
    .unwrap();
    assert!(decoder.next_batch(&cancel).unwrap().is_none());
    for sql in ["FROM facts |> SELECT id, id", "FROM facts |> SELECT id + 1"] {
        let query = db.prepare(sql).unwrap();
        let mut bytes = Vec::new();
        assert!(matches!(
            db.export_parquet(&query, &mut bytes, LIMITS, &cancel),
            Err(Error::Unsupported(_))
        ));
        assert!(bytes.is_empty());
    }
}

#[test]
fn complete_scalar_export_matches_external_reader_fixture() {
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
            name: "amount",
            data_type: DataType::Int64,
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
        ColumnDeclaration {
            name: "note",
            data_type: DataType::String,
            nullable: true,
        },
    ];
    let cancel = CancellationToken::new();
    db.declare_table("facts", &schema, &cancel).unwrap();
    db.import_parquet(
        "facts",
        Cursor::new(&include_bytes!("../../test/data/parquet/plain-v2.parquet")[..]),
        ParquetImportLimits {
            parquet: ParquetReadLimits {
                input_bytes: 100_000,
                rows: 8,
                metadata_bytes: 16_384,
                row_groups: 3,
                row_group_rows: 3,
                row_group_bytes: 70_000,
                page_bytes: 70_000,
            },
            append: AppendLimits {
                batches: 8,
                encoded_bytes: 200_000,
            },
        },
        &cancel,
        |_| Ok(()),
    )
    .unwrap();
    let query = db.prepare("FROM facts |> ORDER BY id").unwrap();
    let baseline = db.reserved_memory_bytes();
    let mut bytes = Vec::new();
    let limits = ParquetExportLimits {
        rows: 8,
        bytes: 100_000,
        row_group_rows: 3,
        row_group_text_bytes: 65_536,
        row_groups: 8,
        metadata_bytes: 16_384,
    };
    assert_eq!(
        db.export_parquet(&query, &mut bytes, limits, &cancel)
            .unwrap(),
        8
    );
    // PyArrow 22.0.0 independently checked every scalar and raw DOUBLE bit in
    // these exact bytes. Fixture provenance and its external check are retained.
    assert_eq!(
        bytes,
        include_bytes!("../../test/data/parquet/pipesql-v1.parquet")
    );
    assert_eq!(db.reserved_memory_bytes(), baseline);
    let mut partial = Vec::new();
    assert!(matches!(
        db.export_parquet(
            &query,
            &mut partial,
            ParquetExportLimits {
                row_group_text_bytes: 65_535,
                ..limits
            },
            &cancel
        ),
        Err(Error::Resource {
            owner: "Parquet row text bytes",
            required: 65_536,
            limit: 65_535
        })
    ));
    assert!(!partial.ends_with(b"PAR1"));
    assert_eq!(db.reserved_memory_bytes(), baseline);
}

#[test]
fn writer_failures_cancellation_and_panic_release_owners() {
    use crate::test_writer::Sink;
    let directory = Directory::new();
    let db = database(&directory, true);
    let query = db.prepare("FROM facts |> ORDER BY id").unwrap();
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let mut expected = Vec::new();
    db.export_parquet(&query, &mut expected, LIMITS, &cancel)
        .unwrap();
    let mut short = Sink {
        short: 1,
        ..Sink::default()
    };
    db.export_parquet(&query, &mut short, LIMITS, &cancel)
        .unwrap();
    assert_eq!(short.bytes, expected);
    assert_eq!(short.flushes, 1);
    for mut sink in [
        Sink {
            fail_after: 1111,
            ..Sink::default()
        },
        Sink {
            fail_after: 1111,
            failure: std::io::ErrorKind::Interrupted,
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
            db.export_parquet(&query, &mut sink, LIMITS, &cancel),
            Err(Error::Io { .. })
        ));
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert!(expected.starts_with(&sink.bytes));
        if sink.fail_flush {
            assert_eq!(sink.bytes, expected);
            assert_eq!(sink.flushes, 1);
        } else {
            assert_eq!(sink.flushes, 0);
        }
    }
    let mut sink = Sink {
        cancel: Some(&cancel),
        ..Sink::default()
    };
    assert!(matches!(
        db.export_parquet(&query, &mut sink, LIMITS, &cancel),
        Err(Error::Cancelled)
    ));
    assert_eq!(sink.bytes, b"PAR1");
    assert_eq!(sink.flushes, 0);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    let mut panic_sink = Sink {
        short: 17,
        panic_after: 1000,
        ..Sink::default()
    };
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            db.export_parquet(&query, &mut panic_sink, LIMITS, &CancellationToken::new())
        }))
        .is_err()
    );
    assert_eq!(panic_sink.flushes, 0);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    let mut retry = Vec::new();
    db.export_parquet(&query, &mut retry, LIMITS, &CancellationToken::new())
        .unwrap();
    assert_eq!(retry, expected);
}

#[test]
fn late_query_error_preserves_span_and_admission_refusal_writes_nothing() {
    let directory = Directory::new();
    let db = database(&directory, true);
    let sql = "FROM facts |> ORDER BY id |> SELECT id + 9223372036854775300 AS n";
    let query = db.prepare(sql).unwrap();
    let baseline = db.reserved_memory_bytes();
    let mut bytes = Vec::new();
    let error = db
        .export_parquet(&query, &mut bytes, LIMITS, &CancellationToken::new())
        .unwrap_err();
    let Error::ArithmeticOverflow {
        operation: "addition",
        span,
    } = error
    else {
        panic!("{error:?}")
    };
    assert_eq!(&sql[span.start()..span.end()], "id + 9223372036854775300");
    assert!(bytes.len() > 4 && !bytes.ends_with(b"PAR1"));
    assert_eq!(db.reserved_memory_bytes(), baseline);
    let held = db
        .reserve_memory(
            db.config().memory_limit_bytes() - baseline,
            "test held memory",
        )
        .unwrap();
    let mut bytes = Vec::new();
    assert!(matches!(
        db.export_parquet(&query, &mut bytes, LIMITS, &CancellationToken::new()),
        Err(Error::Resource {
            owner: "Parquet export buffers",
            ..
        })
    ));
    assert!(bytes.is_empty());
    drop(held);
    assert_eq!(db.reserved_memory_bytes(), baseline);
}
