//! Prepare one checked SQL file, then explain its plan or stream its result.
//!
//! The shared `source` reader admits the complete file before preparation.
//! Preparation owns its literals and spans, so the source file closes before
//! explanation or execution. Explain formats logical metadata without reading rows.
//! Consume borrowed batches immediately and distinguish Progress, Rows, Finished
//! and Failed. Only Finished permits the final row count and success marker;
//! execution or sink failure can leave a visible prefix, never a complete result.

use super::output::{output_error, write_database_status, write_query_header, write_value};
use pipesql::{CancellationToken, Database, Error, PreparedQuery, QueryStep};
use std::io::Write;
use std::path::Path;

fn prepare_query_file<'db>(
    database: &'db Database,
    path: &Path,
) -> Result<PreparedQuery<'db>, Error> {
    let mut bytes = [0; super::source::MAX_SOURCE_BYTES + 1];
    database.prepare(super::source::read(path, &mut bytes)?)
}

pub(super) fn explain_query_file(
    database: &Database,
    path: &Path,
    output: &mut impl Write,
) -> Result<(), Error> {
    let prepared = prepare_query_file(database, path)?;
    write_database_status(output, "explaining", database)?;
    write!(output, "{}", prepared.logical_plan())
        .and_then(|()| writeln!(output, "status=explained"))
        .map_err(|source| output_error("write logical plan", source))
}

pub(super) fn execute_query_file(
    database: &Database,
    path: &Path,
    output: &mut impl Write,
) -> Result<(), Error> {
    let prepared = prepare_query_file(database, path)?;
    let cancellation = CancellationToken::new();
    let mut result = database.execute(&prepared, &cancellation)?;
    write_query_header(output, database, &prepared)?;
    let mut rows = 0_u64;
    loop {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Finished => break,
            QueryStep::Failed(_) => {
                return Err(result.into_error().expect("failed result owns its error"));
            }
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    output
                        .write_all(b"row=")
                        .map_err(|source| output_error("write query row", source))?;
                    for column in 0..batch.column_count() {
                        if column != 0 {
                            output
                                .write_all(b"|")
                                .map_err(|source| output_error("write query value", source))?;
                        }
                        let value = batch
                            .value(row, column)
                            .ok_or(Error::Corrupt("batch value is missing"))?;
                        write_value(output, &value)?;
                    }
                    output
                        .write_all(b"\n")
                        .map_err(|source| output_error("write query row", source))?;
                    rows = rows
                        .checked_add(1)
                        .ok_or(Error::Corrupt("CLI result row count overflow"))?;
                }
            }
        }
    }
    writeln!(output, "row_count={rows}\nstatus=queried")
        .map_err(|source| output_error("write query completion", source))
}
