//! Inspect a prepared query, then execute it and check its answer.
//!
//! Run this against the sales database created by `declared.rs`. The query renames
//! amount, adds one to each present value, then sums them: 11 + 21 + 6 = 38.
//! Its logical plan shows the relations and column identities connecting those
//! operations. Printing that plan reads prepared metadata without executing SQL.
//!
//! The program checks the result schema, prints the plan, and requires exactly one
//! row containing 38 followed by completion. Reservation checks distinguish the
//! memory retained by the prepared query from the memory used during execution.
//! Pass the existing absolute database path:
//! `cargo run --release --offline --locked --example logical_plan -- /absolute/sales`.
//! Create it first with `cargo run --release --offline --locked --example declared
//! -- /absolute/sales`. Both commands must exit successfully; the caller owns cleanup.
//!
//! In the printed plan, source `r0` supplies amount as `c2`. The first SELECT
//! still exposes `c2`, now named subtotal. The next SELECT creates `c3` for
//! `subtotal + 1`, and aggregation creates `c4` for the total. The postfix program
//! `c2, 1, add` lists operands before their operation. It describes the expression,
//! not unconditional evaluation: CASE and COALESCE can skip unused branches.
//!
//! Preparation checks every referenced name without reading rows. An unknown
//! subtotal fails even if a later SELECT hides it; overflow in a row expression
//! fails only when that value is demanded. Source spans survive preparation so
//! errors can identify expressions after the caller discards the SQL string.
//! Follow `src/frontend/binding.rs` for preparation and `src/execution/planning.rs`
//! for the later mapping from semantic identities to physical value positions.

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
    // Display writes the borrowed plan directly to stdout. Calling execute
    // below is what starts evaluation; printing the plan does not read rows.
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
