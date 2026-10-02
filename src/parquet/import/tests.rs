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
use std::collections::BTreeMap;
use std::io::{Cursor, SeekFrom};
use std::path::{Path, PathBuf};

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

// Preflight must preserve the complete namespace and file contents, including
// issuance metadata. An unchanged generation or absent callback is insufficient.
fn database_bytes(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut entries = BTreeMap::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(root.join(directory)).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path().strip_prefix(root).unwrap().to_owned();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                pending.push(path.clone());
                entries.insert(path, None);
            } else {
                assert!(kind.is_file());
                entries.insert(path, Some(std::fs::read(entry.path()).unwrap()));
            }
        }
    }
    entries
}

fn check_rows(db: &Database, count: usize) {
    check_rows_from(db, 0, count);
}

fn check_rows_from(db: &Database, first: i64, count: usize) {
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
                    assert_eq!(batch.value(row, 0), Some(Value::Int64(first + seen as i64)));
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
    let stored = database_bytes(db.path());
    let second = TransactionId::for_attempt(db.database_identity(), 2).unwrap();
    let third = TransactionId::for_attempt(db.database_identity(), 3).unwrap();
    let cancel = CancellationToken::new();
    let issued = Cell::new(None);
    let mut bounds = limits();
    bounds.parquet.rows = 599;
    assert!(matches!(
        db.import_parquet("facts", Cursor::new(INPUT), bounds, &cancel, |token| {
            issued.set(Some(token));
            Ok(())
        }),
        Err(Error::Resource {
            owner: "Parquet rows",
            ..
        })
    ));
    assert!(issued.get().is_none());
    assert_eq!(database_bytes(db.path()), stored);
    assert!(matches!(db.resolve_commit(second), Err(Error::NotFound)));
    assert_eq!(db.generation(), 1);
    assert_eq!(db.reserved_memory_bytes(), before);
    assert_eq!(db.reserved_temp_bytes(), 0);
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
    assert_eq!(issued.get(), Some(second));
    assert!(matches!(
        db.resolve_commit(second).unwrap(),
        CommitResolution::Aborted
    ));
    assert!(matches!(db.resolve_commit(third), Err(Error::NotFound)));
    assert_eq!(db.reserved_memory_bytes(), before);
    assert_eq!(db.reserved_temp_bytes(), 0);
    check_rows(&db, 0);
    let commit = db
        .import_parquet("facts", Cursor::new(INPUT), limits(), &cancel, |token| {
            assert_eq!(token, third);
            Ok(())
        })
        .unwrap();
    assert_eq!(commit.transaction(), third);
    assert_eq!(commit.generation(), 2);
    assert_eq!(
        db.resolve_commit(third).unwrap(),
        CommitResolution::Durable(commit)
    );
    check_rows(&db, 600);
    db.close().unwrap();
}

mod failures;
mod interruption;

#[test]
fn conversion_admission_failure_precedes_receipt_and_releases_decoder() {
    let directory = Directory::new();
    let db = database(&directory);
    let baseline = db.reserved_memory_bytes();
    let stored = database_bytes(db.path());
    let second = TransactionId::for_attempt(db.database_identity(), 2).unwrap();
    let schema = [ColumnDeclaration {
        name: "id",
        data_type: DataType::Int64,
        nullable: false,
    }];
    let decoder = Decoder::<Cursor<&[u8]>>::required_memory(&schema, limits().parquet).unwrap();
    let account = crate::resources::MemoryAuthority::new(8_000_000);
    let columns = Columns::new(&account, &schema, BATCH_ROWS).unwrap();
    let conversion = account.reserved();
    drop(columns);
    let held = db
        .memory
        .reserve(
            db.memory.limit() - baseline - decoder - conversion + 1,
            "test held memory",
        )
        .unwrap();
    let issued = Cell::new(false);
    let result = db.import_parquet(
        "facts",
        Cursor::new(INPUT),
        limits(),
        &CancellationToken::new(),
        |_| {
            issued.set(true);
            Ok(())
        },
    );
    assert!(matches!(
        result,
        Err(Error::Resource {
            owner: "import columns",
            ..
        })
    ));
    assert!(!issued.get());
    assert_eq!(db.generation(), 1);
    assert_eq!(db.reserved_memory_bytes(), baseline + held.bytes());
    assert_eq!(db.reserved_temp_bytes(), 0);
    drop(held);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(database_bytes(db.path()), stored);
    assert!(matches!(db.resolve_commit(second), Err(Error::NotFound)));
    let commit = db
        .import_parquet(
            "facts",
            Cursor::new(INPUT),
            limits(),
            &CancellationToken::new(),
            |token| {
                assert_eq!(token, second);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(commit.transaction(), second);
    assert_eq!(commit.generation(), 2);
    assert_eq!(
        db.resolve_commit(second).unwrap(),
        CommitResolution::Durable(commit)
    );
    check_rows(&db, 600);
    db.close().unwrap();
}
