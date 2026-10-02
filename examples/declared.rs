//! Create a sales table, append four rows, reopen it and print regional totals.
//!
//! This is the basic write-and-query flow for a declared table. Column declarations
//! specify names, types and whether NULL is allowed. The batch supplies values and
//! separate validity bits; commit publishes the batch so later queries can see it.
//! Reopening demonstrates that the committed rows can be read from storage.
//!
//! The final north row has no amount. Its region still contributes to COUNT(*),
//! while SUM(amount) and COUNT(amount) use only the two present amounts. The query
//! loop prints borrowed result rows and continues until completion or failure.
//! Pass a new absolute database path as the first argument; it remains available
//! for later examples. Run `cargo run --release --offline --locked --example
//! declared -- /absolute/new-sales`. The caller owns cleanup. Expected totals are
//! north 15 (three rows, two present amounts) and south 20 (one present row).

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
                nullable: true,
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
    // Bit zero describes row zero. Region has four set bits; amount has only
    // three, so its final zero is a placeholder for NULL, not a sale of zero.
    append.write(
        &[
            ColumnInput {
                values: ColumnValues::String(&["north", "south", "north", "north"]),
                validity: &[0b1111],
            },
            ColumnInput {
                values: ColumnValues::Int64(&[10, 20, 5, 0]),
                validity: &[0b111],
            },
        ],
        &cancel,
    )?;
    append.commit(&cancel)?;
    db.close()?;

    let db = Database::open(&path, config)?;
    let query =
        db.prepare("FROM sales |> AGGREGATE SUM(amount) AS total, COUNT(*) AS n, COUNT(amount) AS present GROUP AND ORDER BY region")?;
    let mut result = db.execute(&query, &cancel)?;
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    // Progress means the query did work without returning a batch. Rows are
    // borrowed from the result, so consume them before stepping again.
    loop {
        match result.step() {
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let (
                        Some(Value::String(region)),
                        Some(Value::Int64(total)),
                        Some(Value::Int64(n)),
                        Some(Value::Int64(present)),
                    ) = (
                        batch.value(row, 0),
                        batch.value(row, 1),
                        batch.value(row, 2),
                        batch.value(row, 3),
                    )
                    else {
                        return Err("unexpected result schema or value".into());
                    };
                    writeln!(
                        output,
                        "{} total={total} rows={n} present={present}",
                        region.as_str()
                    )?;
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
