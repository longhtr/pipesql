//! Group stored Gregorian dates by year, retaining a separate NULL group.
//! Run with a new absolute database path; remove that database after the lesson.
use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, DateValue, QueryStep, Value,
};
use std::io::Write;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("supply a new absolute database path")?,
    );
    let config = Config::new(8_000_000, 4_000_000)?;
    let cancel = CancellationToken::new();
    let db = Database::create_empty(&path, config)?;
    db.declare_table(
        "events",
        &[
            ColumnDeclaration {
                name: "happened",
                data_type: DataType::Date,
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
    // 1999-12-31, 2000-02-29, 2000-12-31, 2001-01-01 and an ignored NULL payload.
    // DATE input uses validated signed days since 1970-01-01.
    let dates = [10_956, 11_016, 11_322, 11_323, 0]
        .map(|days| DateValue::from_days_since_unix_epoch(days).expect("literal date range"));
    let mut append = db.begin_append(
        "events",
        AppendLimits {
            batches: 1,
            encoded_bytes: 20_000,
        },
        &cancel,
    )?;
    append.write(
        &[
            ColumnInput {
                values: ColumnValues::Date(&dates),
                validity: &[0b01111],
            },
            ColumnInput {
                values: ColumnValues::Int64(&[10, 20, 5, 30, 40]),
                validity: &[0b11111],
            },
        ],
        &cancel,
    )?;
    append.commit(&cancel)?;
    db.close()?;

    let db = Database::open(&path, config)?;
    let baseline = db.reserved_memory_bytes();
    let query = db.prepare(include_str!("calendar_year.sql"))?;
    if query.result_column_count() != 3 {
        return Err("expected three result columns".into());
    }
    for (index, (name, nullable)) in [("calendar_year", true), ("total", true), ("entries", false)]
        .into_iter()
        .enumerate()
    {
        let column = query.result_column(index).ok_or("missing result column")?;
        if column.name != Some(name)
            || column.data_type != DataType::Int64
            || column.nullable != nullable
        {
            return Err("unexpected result schema".into());
        }
    }
    let expected = [
        (None, 40, 1),
        (Some(1999), 10, 1),
        (Some(2000), 25, 2),
        (Some(2001), 30, 1),
    ];
    let mut result = db.execute(&query, &cancel)?;
    let mut seen = 0;
    let mut finished = false;
    for _ in 0..10_000 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let (year, total, entries) =
                        *expected.get(seen).ok_or("too many year groups")?;
                    if batch.value(row, 0) != Some(year.map_or(Value::Null, Value::Int64))
                        || batch.value(row, 1) != Some(Value::Int64(total))
                        || batch.value(row, 2) != Some(Value::Int64(entries))
                    {
                        return Err("unexpected year group".into());
                    }
                    seen += 1;
                }
            }
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(_) => {
                return Err(result.into_error().ok_or("missing query error")?.into());
            }
        }
    }
    if !finished || seen != expected.len() {
        return Err("year grouping did not complete".into());
    }
    drop(result);
    drop(query);
    if db.reserved_memory_bytes() != baseline || db.reserved_temp_bytes() != 0 {
        return Err("query reservations remain live".into());
    }
    db.close()?;
    let mut output = std::io::stdout().lock();
    for (year, total, entries) in expected {
        match year {
            Some(year) => write!(output, "year={year}"),
            None => write!(output, "year=NULL"),
        }?;
        writeln!(output, " total={total} entries={entries}")?;
    }
    writeln!(output, "status=finished")?;
    output.flush()?;
    Ok(())
}
