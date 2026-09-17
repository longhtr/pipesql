//! Read a schema file and submit one table declaration to the library.
//!
//! The first line is `table NAME`; each remaining line gives a column name,
//! lowercase type and `required` or `nullable`. Parsing borrows names from the
//! checked source buffer and fills a fixed array of at most 64 columns. Errors
//! identify the start of the offending line in bytes, including preceding newlines.
//!
//! Parse every line before calling `Database::declare_table`. The library then
//! validates names and types, admits resources and publishes the complete schema.
//! A malformed later line cannot leave earlier columns installed.
//!
//! Receipt output follows the committed declaration. If writing that output
//! fails, the table can already exist; a failed command does not imply rollback.

use super::output::{output_error, write_database_status};
use super::source;
use pipesql::{CancellationToken, ColumnDeclaration, DataType, Database, Error};
use std::io::Write;
use std::path::Path;

const MAX_COLUMNS: usize = 64;

struct Declaration<'text> {
    table: &'text str,
    columns: [ColumnDeclaration<'text>; MAX_COLUMNS],
    count: usize,
}

fn invalid(message: &'static str, offset: usize) -> Error {
    Error::Input {
        message,
        byte_offset: offset as u64,
    }
}

fn parse(source: &str) -> Result<Declaration<'_>, Error> {
    // Keep newline bytes in each slice so reported offsets refer to the source
    // file, including CRLF input, rather than to normalized lines.
    let mut lines = source.split_inclusive('\n');
    let header = lines
        .next()
        .ok_or_else(|| invalid("expected table NAME", 0))?;
    let mut words = header.split_ascii_whitespace();
    if words.next() != Some("table") {
        return Err(invalid("expected table NAME", 0));
    }
    let table = words
        .next()
        .ok_or_else(|| invalid("expected table NAME", 0))?;
    if words.next().is_some() {
        return Err(invalid("expected table NAME", 0));
    }
    let mut declaration = Declaration {
        table,
        columns: [ColumnDeclaration {
            name: "",
            data_type: DataType::Int64,
            nullable: false,
        }; MAX_COLUMNS],
        count: 0,
    };
    let mut offset = header.len();
    for line in lines {
        if declaration.count == MAX_COLUMNS {
            return Err(invalid("schema exceeds 64 columns", offset));
        }
        let mut words = line.split_ascii_whitespace();
        let name = words
            .next()
            .ok_or_else(|| invalid("expected NAME TYPE required|nullable", offset))?;
        let data_type = match words.next() {
            Some("int64") => DataType::Int64,
            Some("double") => DataType::Double,
            Some("string") => DataType::String,
            Some("date") => DataType::Date,
            _ => return Err(invalid("expected int64, double, string or date", offset)),
        };
        let nullable = match words.next() {
            Some("required") => false,
            Some("nullable") => true,
            _ => return Err(invalid("expected required or nullable", offset)),
        };
        if words.next().is_some() {
            return Err(invalid("unexpected schema field", offset));
        }
        declaration.columns[declaration.count] = ColumnDeclaration {
            name,
            data_type,
            nullable,
        };
        declaration.count += 1;
        offset += line.len();
    }
    if declaration.count == 0 {
        return Err(invalid("schema requires at least one column", offset));
    }
    Ok(declaration)
}

pub(super) fn declare_file(
    database: &Database,
    path: &Path,
    output: &mut impl Write,
) -> Result<(), Error> {
    let mut bytes = [0; source::MAX_SOURCE_BYTES + 1];
    let declaration = parse(source::read(path, &mut bytes)?)?;
    let commit = database.declare_table(
        declaration.table,
        &declaration.columns[..declaration.count],
        &CancellationToken::new(),
    )?;
    write_database_status(output, "declared", database)?;
    writeln!(output, "generation={}", commit.generation())
        .and_then(|()| writeln!(output, "transaction={}", commit.transaction()))
        .map_err(|source| output_error("write declaration status", source))
}

#[cfg(test)]
mod tests {
    //! Check parsed fields and error offsets against literals, including CRLF
    //! input, malformed later lines, and the 64-column limit. Name uniqueness
    //! belongs to the library; the width fixture deliberately repeats a name.

    use super::*;

    #[test]
    fn schema_types_and_late_errors_have_literal_expectations() {
        let schema = parse("table events\r\nid int64 required\r\nvalue double nullable\r\nlabel string nullable\r\nday date required").unwrap();
        assert_eq!(schema.table, "events");
        assert_eq!(schema.count, 4);
        for (column, (name, data_type, nullable)) in schema.columns[..4].iter().zip([
            ("id", DataType::Int64, false),
            ("value", DataType::Double, true),
            ("label", DataType::String, true),
            ("day", DataType::Date, false),
        ]) {
            assert_eq!(
                (column.name, column.data_type, column.nullable),
                (name, data_type, nullable)
            );
        }
        for (source, offset) in [
            ("", 0),
            ("table events\n", 13),
            ("table events extra\n", 0),
            ("table events\nx int64 required\ny bad nullable\n", 30),
            ("table events\nx int64 maybe\n", 13),
            ("table events\nx int64 required extra\n", 13),
            ("table events\n\n", 13),
        ] {
            assert!(
                matches!(parse(source), Err(Error::Input { byte_offset, .. }) if byte_offset == offset),
                "{source:?}"
            );
        }
        let full = format!("table t\n{}", "x int64 required\n".repeat(64));
        assert_eq!(parse(&full).unwrap().count, 64);
        let oversized = format!("{full}last date nullable\n");
        assert!(
            matches!(parse(&oversized), Err(Error::Input { byte_offset, .. }) if byte_offset == full.len() as u64)
        );
    }
}
