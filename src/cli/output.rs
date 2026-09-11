//! CLI status, schema, and value encoding. Write failure terminates the command.
use pipesql::{DataType, Database, Error, PreparedQuery, Value};
use std::io::{self, Write};

pub(super) fn output_error(operation: &'static str, source: io::Error) -> Error {
    Error::Io { operation, source }
}

pub(super) fn write_database_status(
    output: &mut impl Write,
    status: &str,
    database: &Database,
) -> Result<(), Error> {
    writeln!(output, "status={status}")
        .and_then(|()| writeln!(output, "database={}", database.path().display()))
        .and_then(|()| {
            writeln!(
                output,
                "memory_limit_bytes={}",
                database.config().memory_limit_bytes()
            )
        })
        .and_then(|()| {
            writeln!(
                output,
                "temp_limit_bytes={}",
                database.config().temp_limit_bytes()
            )
        })
        .map_err(|source| output_error("write command status", source))
}

pub(super) fn write_query_header(
    output: &mut impl Write,
    database: &Database,
    prepared: &PreparedQuery<'_>,
) -> Result<(), Error> {
    write_database_status(output, "querying", database)?;
    writeln!(output, "column_count={}", prepared.result_column_count())
        .map_err(|source| output_error("write query schema", source))?;
    output
        .write_all(b"columns=")
        .map_err(|source| output_error("write query schema", source))?;
    for index in 0..prepared.result_column_count() {
        if index != 0 {
            output
                .write_all(b"|")
                .map_err(|source| output_error("write query schema", source))?;
        }
        let column = prepared
            .result_column(index)
            .ok_or(Error::Corrupt("result column is missing"))?;
        let data_type = match column.data_type {
            DataType::Double => "double",
            DataType::Int64 => "int64",
            DataType::Date => "date",
            DataType::String => "string",
        };
        write!(
            output,
            "{}:{data_type}:{}",
            column.name.unwrap_or(""),
            if column.nullable {
                "nullable"
            } else {
                "required"
            }
        )
        .map_err(|source| output_error("write query schema", source))?;
    }
    output
        .write_all(b"\n")
        .map_err(|source| output_error("write query schema", source))
}

pub(super) fn write_value(output: &mut impl Write, value: &Value<'_>) -> Result<(), Error> {
    let result = (|| -> io::Result<()> {
        match value {
            Value::Null => output.write_all(b"null"),
            Value::Int64(value) => write!(output, "int64:{value}"),
            Value::Double(value) => write!(output, "double:{value}:{:016x}", value.to_bits()),
            Value::Date(value) => write!(output, "date:{value}"),
            Value::String(value) => {
                output.write_all(b"string:")?;
                for byte in value.as_str().as_bytes() {
                    write!(output, "{byte:02x}")?;
                }
                Ok(())
            }
        }
    })();
    result.map_err(|source| output_error("write query value", source))
}
