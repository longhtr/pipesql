//! Report regional sales while retaining sales with no matching region.
//!
//! The facts table has five sales; the regions table supplies names for IDs 1
//! and 2. A left join attaches those names but also keeps region 3 and the sale
//! whose region is NULL. Both unmatched rows receive a NULL name. Grouping by
//! that name combines their amounts into one group, printed as `unmatched`.
//!
//! The program creates both tables, commits their rows, reopens the database and
//! runs the report. It prints rows as they arrive, so check process success before
//! treating the output as complete. Pass a new absolute database path; the files
//! remain available afterward for caller cleanup. Run `cargo run --release
//! --offline --locked --example left_join -- /absolute/new-facts`.
//!
//! The NULL-label group totals 90 from amounts 40 and 50; north totals 30 from
//! two rows and south 30 from one. Additional SQL files use this same database:
//! `default-region.sql` replaces missing IDs with zero, giving (0, 90, 2),
//! (1, 30, 2), (2, 30, 1). `missing-regions.sql` returns NULL and 3 using EXCEPT
//! DISTINCT; `shared-regions.sql` returns 1 and 2 using INTERSECT DISTINCT.
//! `repeated-regions.sql` uses EXCEPT ALL, retaining NULL, one copy of 1, and 3.
//!
//! `sentinel-amounts.sql` treats amount 20 as missing using NULLIF: SUM is 130,
//! COUNT of the expression is 4, but COUNT(*) stays 5. `null-safe-region.sql`
//! excludes region 3 while retaining NULL, returning total 110 from four rows.
//! Ordinary `!= 3` would discard the NULL row too, returning 60 from three.
//! Run these with the CLI's `query --query-file` command and require complete
//! output and exit status zero, as with the primary report.

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
    let cancel = CancellationToken::new();
    let config = Config::new(8_000_000, 4_000_000)?;
    let db = Database::create_empty(&path, config)?;
    db.declare_table(
        "facts",
        &[
            ColumnDeclaration {
                name: "region",
                data_type: DataType::Int64,
                nullable: true,
            },
            ColumnDeclaration {
                name: "amount",
                data_type: DataType::Int64,
                nullable: false,
            },
        ],
        &cancel,
    )?;
    db.declare_table(
        "regions",
        &[
            ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "name",
                data_type: DataType::String,
                nullable: false,
            },
        ],
        &cancel,
    )?;
    let limits = AppendLimits {
        batches: 1,
        encoded_bytes: 10_000,
    };
    let mut append = db.begin_append("facts", limits, &cancel)?;
    append.write(
        &[
            // The last row has a clear validity bit, so its region is NULL.
            // Region 3 has a value but no matching row in the regions table.
            ColumnInput {
                values: ColumnValues::Int64(&[1, 1, 2, 3, 0]),
                validity: &[0b01111],
            },
            ColumnInput {
                values: ColumnValues::Int64(&[10, 20, 30, 40, 50]),
                validity: &[0b11111],
            },
        ],
        &cancel,
    )?;
    append.commit(&cancel)?;
    let mut append = db.begin_append("regions", limits, &cancel)?;
    append.write(
        &[
            ColumnInput {
                values: ColumnValues::Int64(&[1, 2]),
                validity: &[0b11],
            },
            ColumnInput {
                values: ColumnValues::String(&["north", "south"]),
                validity: &[0b11],
            },
        ],
        &cancel,
    )?;
    append.commit(&cancel)?;
    db.close()?;

    let db = Database::open(&path, config)?;
    let query = db.prepare(include_str!("left-join.sql"))?;
    let mut result = db.execute(&query, &cancel)?;
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    loop {
        match result.step() {
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let label = match batch.value(row, 0) {
                        Some(Value::String(name)) => name.as_str(),
                        Some(Value::Null) => "unmatched",
                        _ => return Err("unexpected region type".into()),
                    };
                    let (Some(Value::Int64(total)), Some(Value::Int64(n))) =
                        (batch.value(row, 1), batch.value(row, 2))
                    else {
                        return Err("unexpected aggregate values".into());
                    };
                    writeln!(output, "{label} total={total} rows={n}")?;
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
