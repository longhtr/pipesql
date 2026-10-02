//! Shared fixture and fixed caller writer for native export refusal checks.
//!
//! Expected answers belong to each format's independent check. Allocation denial
//! in the final flush begins only after all output bytes have reached this writer.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, Config, DataType, Database,
    ParquetImportLimits, ParquetReadLimits,
};
use std::{io::Cursor, io::Write, path::Path, sync::atomic::Ordering};

const INPUT: &[u8] = include_bytes!("../../../../test/data/parquet/plain-v2.parquet");

pub(super) struct Output<'a> {
    pub(super) bytes: &'a mut [u8],
    pub(super) length: usize,
    pub(super) flushes: usize,
    pub(super) fail_flush: bool,
}

impl Write for Output<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let count = bytes.len().min(17).min(self.bytes.len() - self.length);
        self.bytes[self.length..self.length + count].copy_from_slice(&bytes[..count]);
        self.length += count;
        Ok(count)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flushes += 1;
        if self.fail_flush {
            // Fail only after complete output. Reporting this writer error and
            // releasing engine owners must not need another allocation.
            super::ALLOW.store(super::CALLS.load(Ordering::Relaxed), Ordering::Relaxed);
            super::DENY.store(true, Ordering::Relaxed);
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        Ok(())
    }
}

pub(super) fn database(root: &Path) -> (Database, Config) {
    std::fs::create_dir(root).unwrap();
    let path = root.join("database");
    let config = Config::new(32_000_000, 8_000_000).unwrap();
    let db = Database::create_empty(&path, config).unwrap();
    let cancel = CancellationToken::new();
    let schema = [
        ("id", DataType::Int64, false),
        ("amount", DataType::Int64, true),
        ("number", DataType::Double, true),
        ("day", DataType::Date, true),
        ("note", DataType::String, true),
    ]
    .map(|(name, data_type, nullable)| ColumnDeclaration {
        name,
        data_type,
        nullable,
    });
    db.declare_table("facts", &schema, &cancel).unwrap();
    db.import_parquet(
        "facts",
        Cursor::new(INPUT),
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
    // The supervisor compares authority after the exporting process exits.
    for name in ["CONTROL", "ROOT.A", "ROOT.B", "WAL"] {
        std::fs::copy(path.join(name), root.join(name)).unwrap();
    }
    (db, config)
}
