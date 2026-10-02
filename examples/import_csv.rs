//! Import CSV as one transaction, then reopen and verify complete typed rows.
//!
//! Pass a new absolute database path. It remains available afterward. The token
//! is flushed before input is read; save it to resolve an interrupted attempt.
//! A failed import never commits only its earlier valid batches.
//!
//! The schema and CSV limits are explicit at the call site. After import, the
//! example resolves the receipt and consumes the reopened table through query
//! completion, checking NULL and empty text separately. The caller owns cleanup of
//! the new database, including any directory left by a failed invocation.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, CommitResolution, Config, CsvLimits,
    DataType, Database, ImportLimits, QueryStep, Value,
};
use std::io::Write;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("supply a new absolute database path")?,
    );
    let config = Config::new(4_000_000, 4_000_000)?;
    let cancel = CancellationToken::new();
    let db = Database::create_empty(&path, config)?;
    db.declare_table(
        "events",
        &[
            ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "label",
                data_type: DataType::String,
                nullable: true,
            },
        ],
        &cancel,
    )?;
    let bounds = ImportLimits {
        csv: CsvLimits {
            input_bytes: 1024,
            rows: 10,
            record_bytes: 128,
            field_bytes: 64,
            batch_rows: 2,
            batch_text_bytes: 128,
        },
        append: AppendLimits {
            batches: 4,
            encoded_bytes: 100_000,
        },
    };
    let input = b"label,id\n\"hello, world\",1\n\\N,2\n\"\",3\n";
    let mut output = std::io::stdout().lock();
    let commit = db.import_csv("events", &input[..], bounds, &cancel, |token| {
        writeln!(output, "transaction={token}").and_then(|()| output.flush())
    })?;
    db.close()?;
    let db = Database::open(&path, config)?;
    assert_eq!(
        db.resolve_commit(commit.transaction())?,
        CommitResolution::Durable(commit)
    );
    let query = db.prepare("FROM events |> ORDER BY id")?;
    let mut result = db.execute(&query, &cancel)?;
    let mut rows = 0;
    loop {
        match result.step() {
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 2);
                for row in 0..batch.len() {
                    assert!(rows < 3);
                    assert_eq!(batch.value(row, 0), Some(Value::Int64(rows + 1)));
                    match rows {
                        0 => assert!(
                            matches!(batch.value(row, 1), Some(Value::String(text)) if text.as_str() == "hello, world")
                        ),
                        1 => assert_eq!(batch.value(row, 1), Some(Value::Null)),
                        2 => assert!(
                            matches!(batch.value(row, 1), Some(Value::String(text)) if text.as_str().is_empty())
                        ),
                        _ => unreachable!(),
                    }
                    rows += 1;
                }
            }
            QueryStep::Progress => {}
            QueryStep::Finished => break,
            QueryStep::Failed(_) => return Err(result.into_error().expect("failed result").into()),
        }
    }
    assert_eq!(rows, 3);
    drop(result);
    drop(query);
    db.close()?;
    writeln!(
        output,
        "Committed and reopened all {rows} rows; NULL and empty text remain distinct."
    )?;
    output.flush()?;
    Ok(())
}
