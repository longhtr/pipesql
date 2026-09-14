//! Explain and execute query-flow.sql on the sales database made by declared.rs.
use pipesql::{CancellationToken, Config, DataType, Database, QueryStep, Value};
use std::io::Write;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let path = arguments
        .next()
        .ok_or("supply the absolute sales database path")?;
    let path = Path::new(&path);
    if arguments.next().is_some() || !path.is_absolute() {
        return Err("expected one absolute sales database path".into());
    }
    let db = Database::open(path, Config::new(8_000_000, 8_000_000)?)?;
    let resident = db.reserved_memory_bytes();
    let query = db.prepare(include_str!("query-flow.sql"))?;
    let column = query.result_column(0).ok_or("missing total column")?;
    if query.result_column_count() != 1
        || column.name != Some("total")
        || column.data_type != DataType::Int64
        || !column.nullable
    {
        return Err("unexpected total schema".into());
    }
    let prepared = db.reserved_memory_bytes();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    // The view borrows the prepared metadata and writes directly to this sink.
    // A report describes the plan; executing the query is a separate operation.
    write!(output, "{}", query.logical_plan())?;
    if db.reserved_memory_bytes() != prepared || db.reserved_temp_bytes() != 0 {
        return Err("formatting changed logical reservations".into());
    }
    let cancel = CancellationToken::new();
    let mut result = db.execute(&query, &cancel)?;
    let mut rows = 0;
    let mut finished = false;
    for _ in 0..10_000 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                if batch.column_count() != 1
                    || batch.len() != 1
                    || rows != 0
                    || batch.value(0, 0) != Some(Value::Int64(38))
                {
                    return Err("query-flow total differs from the literal oracle".into());
                }
                rows += 1;
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
    if !finished || rows != 1 {
        return Err("query-flow did not finish with one row".into());
    }
    drop(result);
    if db.reserved_memory_bytes() != prepared || db.reserved_temp_bytes() != 0 {
        return Err("execution did not release its reservations".into());
    }
    drop(query);
    if db.reserved_memory_bytes() != resident {
        return Err("preparation did not release its reservation".into());
    }
    db.close()?;
    writeln!(output, "total=38\nstatus=finished")?;
    output.flush()?;
    Ok(())
}
