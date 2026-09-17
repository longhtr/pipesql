//! Import, export and reopen a complete Parquet result using explicit limits.
//!
//! Pass a new absolute database path and a new output file path. Both remain
//! available afterward. The external fixture contains IDs 0 through 599; both
//! the original table and the table imported from the export must retain them.
//!
//! Transaction tokens are retained for resolution, and query checks consume every
//! batch through completion. The export file is created without overwriting an
//! existing path. Import and export errors propagate to the process result; callers
//! must not treat a partially written file as a successful transfer.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, CommitResolution, Config, DataType,
    Database, ParquetExportLimits, ParquetImportLimits, ParquetReadLimits, QueryStep, Value,
};
use std::fs::{File, OpenOptions};
use std::io::{Cursor, Write};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(args.next().ok_or("supply a new absolute database path")?);
    let output_path = PathBuf::from(args.next().ok_or("supply a new Parquet output path")?);
    if args.next().is_some() {
        return Err("expected a database path and an output path".into());
    }
    let config = Config::new(8_000_000, 8_000_000)?;
    let cancel = CancellationToken::new();
    let db = Database::create_empty(&path, config)?;
    for table in ["original", "copied"] {
        db.declare_table(
            table,
            &[ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            }],
            &cancel,
        )?;
    }
    let limits = ParquetImportLimits {
        parquet: ParquetReadLimits {
            input_bytes: 20_000,
            rows: 600,
            metadata_bytes: 2048,
            row_groups: 2,
            row_group_rows: 600,
            row_group_bytes: 6000,
            page_bytes: 6000,
        },
        append: AppendLimits {
            batches: 4,
            encoded_bytes: 100_000,
        },
    };
    let mut status = std::io::stdout().lock();
    let original = db.import_parquet(
        "original",
        Cursor::new(&include_bytes!("../test/data/parquet/plain-batches.parquet")[..]),
        limits,
        &cancel,
        |token| writeln!(status, "transaction={token}").and_then(|()| status.flush()),
    )?;
    let query = db.prepare("FROM original |> ORDER BY id")?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)?;
    assert_eq!(
        db.export_parquet(
            &query,
            &mut output,
            ParquetExportLimits {
                rows: 600,
                bytes: 20_000,
                row_group_rows: 300,
                row_group_text_bytes: 1,
                row_groups: 2,
                metadata_bytes: 2048,
            },
            &cancel,
        )?,
        600
    );
    drop(output);
    drop(query);
    let copied = db.import_parquet(
        "copied",
        File::open(&output_path)?,
        limits,
        &cancel,
        |token| writeln!(status, "transaction={token}").and_then(|()| status.flush()),
    )?;
    db.close()?;
    let db = Database::open(&path, config)?;
    for commit in [original, copied] {
        assert_eq!(
            db.resolve_commit(commit.transaction())?,
            CommitResolution::Durable(commit)
        );
    }
    for sql in ["FROM original |> ORDER BY id", "FROM copied |> ORDER BY id"] {
        let query = db.prepare(sql)?;
        let mut result = db.execute(&query, &cancel)?;
        let mut rows = 0;
        loop {
            match result.step() {
                QueryStep::Progress => {}
                QueryStep::Rows(batch) => {
                    assert_eq!(batch.column_count(), 1);
                    for row in 0..batch.len() {
                        assert!(rows < 600);
                        assert_eq!(batch.value(row, 0), Some(Value::Int64(rows)));
                        rows += 1;
                    }
                }
                QueryStep::Finished => break,
                QueryStep::Failed(_) => {
                    return Err(result.into_error().expect("failed result").into());
                }
            }
        }
        assert_eq!(rows, 600);
    }
    db.close()?;
    writeln!(
        status,
        "Imported, exported and reopened all 600 rows in both tables."
    )?;
    status.flush()?;
    Ok(())
}
