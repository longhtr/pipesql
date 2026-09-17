//! Check Parquet publication through actual append and query operations.
//!
//! The independent 600-row input crosses both page and borrowed-batch boundaries.
//! A final source error occurs after all private batches have been written; the
//! empty table, aborted receipt and same-handle retry check that none was published.
//!
//! Earlier failures are checked separately: malformed footer input cannot issue a
//! token, while a failing token callback must abort the issued transaction. Complete
//! imports are reopened and queried to check every row and the durable receipt.

use super::*;
use crate::test_support::Directory;
use crate::{CommitResolution, Config, DataType, QueryStep, Value};
use std::cell::Cell;
use std::io::{Cursor, SeekFrom};

const INPUT: &[u8] = include_bytes!("../../../test/data/parquet/plain-batches.parquet");

fn limits() -> ParquetImportLimits {
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
    }
}

fn database(directory: &Directory) -> Database {
    let db = Database::create_empty(
        &directory.0.join("db"),
        Config::new(8_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    db.declare_table(
        "facts",
        &[ColumnDeclaration {
            name: "id",
            data_type: DataType::Int64,
            nullable: false,
        }],
        &CancellationToken::new(),
    )
    .unwrap();
    db
}

fn check_rows(db: &Database, count: usize) {
    let query = db.prepare("FROM facts |> ORDER BY id").unwrap();
    let cancel = CancellationToken::new();
    let mut result = db.execute(&query, &cancel).unwrap();
    let mut seen = 0;
    for _ in 0..10_000 {
        match result.step() {
            QueryStep::Progress => {}
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 1);
                for row in 0..batch.len() {
                    assert!(seen < count);
                    assert_eq!(batch.value(row, 0), Some(Value::Int64(seen as i64)));
                    seen += 1;
                }
            }
            QueryStep::Finished => {
                assert_eq!(seen, count);
                return;
            }
            QueryStep::Failed(error) => panic!("{error:?}"),
        }
    }
    panic!("Parquet query did not finish");
}

#[test]
fn complete_import_reopens_with_all_values_and_resolved_receipt() {
    let directory = Directory::new();
    let db = database(&directory);
    let before = db.reserved_memory_bytes();
    let issued = Cell::new(None);
    let commit = db
        .import_parquet(
            "facts",
            Cursor::new(INPUT),
            limits(),
            &CancellationToken::new(),
            |token| {
                issued.set(Some(token));
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(db.reserved_memory_bytes(), before);
    check_rows(&db, 600);
    assert!(
        matches!(db.resolve_commit(issued.get().unwrap()).unwrap(), CommitResolution::Durable(receipt)
        if receipt.generation() == commit.generation())
    );
    db.close().unwrap();
    let db = Database::open(
        &directory.0.join("db"),
        Config::new(8_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    check_rows(&db, 600);
    db.close().unwrap();
}

struct FailsAtEnd {
    bytes: Cursor<&'static [u8]>,
    end_seeks: usize,
}

impl Read for FailsAtEnd {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.bytes.read(bytes)
    }
}

impl Seek for FailsAtEnd {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        if matches!(to, SeekFrom::End(0)) {
            self.end_seeks += 1;
            if self.end_seeks == 2 {
                return Err(std::io::ErrorKind::Other.into());
            }
        }
        self.bytes.seek(to)
    }
}

#[test]
fn final_source_failure_aborts_all_private_batches_and_allows_retry() {
    let directory = Directory::new();
    let db = database(&directory);
    let before = db.reserved_memory_bytes();
    let issued = Cell::new(None);
    let reader = FailsAtEnd {
        bytes: Cursor::new(INPUT),
        end_seeks: 0,
    };
    let error = db
        .import_parquet(
            "facts",
            reader,
            limits(),
            &CancellationToken::new(),
            |token| {
                issued.set(Some(token));
                Ok(())
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Io {
            operation: "seek Parquet input",
            ..
        }
    ));
    assert_eq!(db.reserved_memory_bytes(), before);
    assert_eq!(db.reserved_temp_bytes(), 0);
    check_rows(&db, 0);
    assert!(matches!(
        db.resolve_commit(issued.get().unwrap()).unwrap(),
        CommitResolution::Aborted
    ));
    db.import_parquet(
        "facts",
        Cursor::new(INPUT),
        limits(),
        &CancellationToken::new(),
        |_| Ok(()),
    )
    .unwrap();
    check_rows(&db, 600);
    db.close().unwrap();
}

#[test]
fn footer_refusal_precedes_issuance_and_callback_failure_aborts() {
    let directory = Directory::new();
    let db = database(&directory);
    let before = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let issued = Cell::new(None);
    let mut bounds = limits();
    bounds.parquet.rows = 599;
    assert!(matches!(
        db.import_parquet("facts", Cursor::new(INPUT), bounds, &cancel, |token| {
            issued.set(Some(token));
            Ok(())
        }),
        Err(Error::Resource { .. })
    ));
    assert!(issued.get().is_none());
    assert_eq!(db.reserved_memory_bytes(), before);
    assert!(matches!(
        db.import_parquet("facts", Cursor::new(INPUT), limits(), &cancel, |token| {
            issued.set(Some(token));
            Err(std::io::ErrorKind::BrokenPipe.into())
        }),
        Err(Error::Io {
            operation: "report Parquet transaction",
            ..
        })
    ));
    assert!(matches!(
        db.resolve_commit(issued.get().unwrap()).unwrap(),
        CommitResolution::Aborted
    ));
    assert_eq!(db.reserved_memory_bytes(), before);
    assert_eq!(db.reserved_temp_bytes(), 0);
    check_rows(&db, 0);
    db.import_parquet("facts", Cursor::new(INPUT), limits(), &cancel, |_| Ok(()))
        .unwrap();
    check_rows(&db, 600);
    db.close().unwrap();
}

mod failures;
mod interruption;
