//! Keep facts whose dimension key is missing, then group their amounts.
//! Supply a new absolute database path. The database remains available afterward.
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
            // The last region is NULL. Region 3 is present but absent from regions.
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
