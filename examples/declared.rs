//! Create a declared table, append typed rows, reopen, and query it.
//! Run with a new absolute database path; the database remains available afterward.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, QueryStep, Value,
};
use std::io::Write;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("supply a new absolute database path")?,
    );
    let config = Config::new(16_000_000, 8_000_000)?;
    let cancel = CancellationToken::new();
    let db = Database::create_empty(&path, config)?;
    db.declare_table(
        "sales",
        &[
            ColumnDeclaration {
                name: "region",
                data_type: DataType::String,
                nullable: false,
            },
            ColumnDeclaration {
                name: "amount",
                data_type: DataType::Int64,
                nullable: false,
            },
        ],
        &cancel,
    )?;
    let mut append = db.begin_append(
        "sales",
        AppendLimits {
            batches: 1,
            encoded_bytes: 100_000,
        },
        &cancel,
    )?;
    append.write(
        &[
            ColumnInput {
                values: ColumnValues::String(&["north", "south", "north"]),
                validity: &[0b111],
            },
            ColumnInput {
                values: ColumnValues::Int64(&[10, 20, 5]),
                validity: &[0b111],
            },
        ],
        &cancel,
    )?;
    append.commit(&cancel)?;
    db.close()?;

    let db = Database::open(&path, config)?;
    let query =
        db.prepare("FROM sales |> AGGREGATE SUM(amount) AS total GROUP AND ORDER BY region")?;
    let mut result = db.execute(&query, &cancel)?;
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    loop {
        match result.step() {
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let (Some(Value::String(region)), Some(Value::Int64(total))) =
                        (batch.value(row, 0), batch.value(row, 1))
                    else {
                        return Err("unexpected result schema or value".into());
                    };
                    writeln!(output, "{} {total}", region.as_str())?;
                }
            }
            QueryStep::Progress => (),
            QueryStep::Finished => break,
            QueryStep::Failed(_) => {
                return Err(result.into_error().ok_or("missing query error")?.into());
            }
        }
    }
    output.flush()?;
    drop(result);
    drop(query);
    db.close()?;
    Ok(())
}
