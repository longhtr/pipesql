//! Parse captured arguments into one command with owned paths and resource limits.
//!
//! The program-name slot is skipped. An operation is followed
//! by option/value pairs. Parsing rejects unknown or repeated options, missing
//! values and options for another operation, then asks `Config` to validate the
//! resource limits. No database is opened here.
//!
//! Paths retain their native bytes; the library checks their filesystem rules.
//! Errors store static messages or reuse an existing option string, avoiding a
//! fresh allocation to describe bad input. A parsed transaction token identifies
//! a request; only `Database::resolve_commit` can establish its recorded outcome.

use pipesql::{AppendLimits, Config, CsvLimits, Error, ExportLimits, ImportLimits, TransactionId};

#[path = "command/formats.rs"]
mod formats;
pub(super) use formats::{ExportFormat, ImportFormat};
use formats::{Format, PARQUET_OPTIONS};
use std::ffi::OsString;
use std::path::PathBuf;

pub(super) const MAX_ARGUMENT_BYTES: usize = 4_096;

pub(super) enum Operation {
    Create,
    CreateDeclared,
    Open,
    Declare(PathBuf),
    Schema(String),
    Load(PathBuf),
    Import {
        input: PathBuf,
        table: String,
        limits: ImportFormat,
    },
    Query(PathBuf),
    Export {
        query: PathBuf,
        limits: ExportFormat,
    },
    Explain(PathBuf),
    Resolve(TransactionId),
}

pub(super) struct Command {
    pub(super) operation: Operation,
    pub(super) database: PathBuf,
    pub(super) config: Config,
}

fn usage() -> &'static str {
    "usage: pipesql create|create-declared|declare|schema|open|load|import|query|export|explain|resolve --database ABSOLUTE_PATH [--input ABSOLUTE_TBL] [--query-file ABSOLUTE_PATH] [--schema-file ABSOLUTE_PATH] [--table NAME] [--transaction HEX_TOKEN] --memory-limit-bytes N --temp-limit-bytes N"
}

#[derive(Debug)]
pub(super) enum ArgumentError {
    Message(&'static str),
    // Reuse the option's allocation when reporting its missing value.
    MissingValue(String),
    InvalidValue {
        owner: &'static str,
        requirement: &'static str,
    },
    Library(Error),
}

impl From<&'static str> for ArgumentError {
    fn from(message: &'static str) -> Self {
        Self::Message(message)
    }
}

impl std::fmt::Display for ArgumentError {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message) => output.write_str(message),
            Self::MissingValue(option) => write!(output, "missing value after {option}"),
            Self::InvalidValue { owner, requirement } => write!(output, "{owner} {requirement}"),
            Self::Library(error) => std::fmt::Display::fmt(error, output),
        }
    }
}

pub(super) fn parse(arguments: impl Iterator<Item = OsString>) -> Result<Command, ArgumentError> {
    let mut arguments = arguments.skip(1);
    let operation = arguments
        .next()
        .ok_or_else(|| ArgumentError::from(usage()))?;
    check_argument(&operation)?;
    let operation = match operation.into_string() {
        Ok(value)
            if matches!(
                value.as_str(),
                "create"
                    | "create-declared"
                    | "declare"
                    | "schema"
                    | "open"
                    | "load"
                    | "import"
                    | "query"
                    | "export"
                    | "explain"
                    | "resolve"
            ) =>
        {
            value
        }
        _ => return Err(usage().into()),
    };
    let mut database = None;
    let mut input = None;
    let mut query_file = None;
    let mut schema_file = None;
    let mut table = None;
    let mut transaction = None;
    let mut memory_limit = None;
    let mut temp_limit = None;
    let mut import_bounds = [None; 8];
    let mut output_bytes = None;
    let mut format = None;
    let mut parquet_bounds = [None; 6];
    // Each accepted pair fills a fixed option slot. Repeating a slot fails, so
    // even a directly supplied iterator cannot keep this loop running forever.
    while let Some(option) = arguments.next() {
        check_argument(&option)?;
        let option = option
            .into_string()
            .map_err(|_| ArgumentError::from("option names must be UTF-8"))?;
        let value = match arguments.next() {
            Some(value) => value,
            None => return Err(ArgumentError::MissingValue(option)),
        };
        check_argument(&value)?;
        match option.as_str() {
            "--database" => {
                if database.replace(PathBuf::from(value)).is_some() {
                    return Err("--database may appear only once".into());
                }
            }
            "--input" => {
                if input.replace(PathBuf::from(value)).is_some() {
                    return Err("--input may appear only once".into());
                }
            }
            "--query-file" => {
                if query_file.replace(PathBuf::from(value)).is_some() {
                    return Err("--query-file may appear only once".into());
                }
            }
            "--schema-file" => {
                if schema_file.replace(PathBuf::from(value)).is_some() {
                    return Err("--schema-file may appear only once".into());
                }
            }
            "--table" => {
                let value = value
                    .into_string()
                    .map_err(|_| ArgumentError::from("--table must be UTF-8"))?;
                if table.replace(value).is_some() {
                    return Err("--table may appear only once".into());
                }
            }
            "--transaction" => {
                let value = parse_transaction(&value)?;
                if transaction.replace(value).is_some() {
                    return Err("--transaction may appear only once".into());
                }
            }
            "--memory-limit-bytes" => {
                let value = parse_u64(value, "--memory-limit-bytes")?;
                if memory_limit.replace(value).is_some() {
                    return Err("--memory-limit-bytes may appear only once".into());
                }
            }
            "--temp-limit-bytes" => {
                let value = parse_u64(value, "--temp-limit-bytes")?;
                if temp_limit.replace(value).is_some() {
                    return Err("--temp-limit-bytes may appear only once".into());
                }
            }
            "--format" => {
                let parsed = Format::parse(&value)?;
                if format.replace(parsed).is_some() {
                    return Err("--format may appear only once".into());
                }
            }
            name if PARQUET_OPTIONS.contains(&name) => {
                let index = PARQUET_OPTIONS
                    .iter()
                    .position(|&option| option == name)
                    .unwrap();
                let parsed = parse_u64(value, PARQUET_OPTIONS[index])?;
                if parquet_bounds[index].replace(parsed).is_some() {
                    return Err(ArgumentError::InvalidValue {
                        owner: PARQUET_OPTIONS[index],
                        requirement: "may appear only once",
                    });
                }
            }
            "--output-limit-bytes" => {
                let value = parse_u64(value, "--output-limit-bytes")?;
                if output_bytes.replace(value).is_some() {
                    return Err("--output-limit-bytes may appear only once".into());
                }
            }
            name if IMPORT_OPTIONS.contains(&name) => {
                let index = IMPORT_OPTIONS
                    .iter()
                    .position(|&option| option == name)
                    .unwrap();
                let parsed = parse_u64(value, IMPORT_OPTIONS[index])?;
                if import_bounds[index].replace(parsed).is_some() {
                    return Err(ArgumentError::InvalidValue {
                        owner: IMPORT_OPTIONS[index],
                        requirement: "may appear only once",
                    });
                }
            }
            _ => return Err("unknown option".into()),
        }
    }
    let database = database.ok_or("--database is required")?;
    if !matches!(operation.as_str(), "load" | "import") && input.is_some() {
        return Err("--input is accepted only for load or import".into());
    }
    if !matches!(operation.as_str(), "query" | "explain" | "export") && query_file.is_some() {
        return Err("--query-file is accepted only for query, explain or export".into());
    }
    if operation != "resolve" && transaction.is_some() {
        return Err("--transaction is accepted only for resolve".into());
    }
    if operation != "declare" && schema_file.is_some() {
        return Err("--schema-file is accepted only for declare".into());
    }
    if !matches!(operation.as_str(), "schema" | "import") && table.is_some() {
        return Err("--table is accepted only for schema or import".into());
    }
    if operation != "export" && output_bytes.is_some() {
        return Err("--output-limit-bytes is accepted only for export".into());
    }
    if !matches!(operation.as_str(), "import" | "export") && format.is_some() {
        return Err("--format is accepted only for import or export".into());
    }
    if format != Some(Format::Parquet) && parquet_bounds.iter().any(Option::is_some) {
        return Err("Parquet limits require --format parquet".into());
    }
    let export_rows = if operation == "export" {
        import_bounds[1].take()
    } else {
        None
    };
    if operation != "import" && import_bounds.iter().any(Option::is_some) {
        return Err("CSV and append limits are accepted only for import".into());
    }
    let operation = match operation.as_str() {
        "create" => Operation::Create,
        "create-declared" => Operation::CreateDeclared,
        "open" => Operation::Open,
        "declare" => {
            Operation::Declare(schema_file.ok_or("--schema-file is required for declare")?)
        }
        "schema" => Operation::Schema(table.ok_or("--table is required for schema")?),
        "load" => Operation::Load(input.ok_or("--input is required for load")?),
        "import" => Operation::Import {
            input: input.ok_or("--input is required for import")?,
            table: table.ok_or("--table is required for import")?,
            limits: match format.unwrap_or(Format::Csv) {
                Format::Csv => ImportFormat::Csv(import_limits(import_bounds)?),
                Format::Parquet => {
                    ImportFormat::Parquet(formats::import_limits(import_bounds, parquet_bounds)?)
                }
                Format::Jsonl => return Err("import format must be csv or parquet".into()),
            },
        },
        "query" => Operation::Query(query_file.ok_or("--query-file is required for query")?),
        "export" => Operation::Export {
            query: query_file.ok_or("--query-file is required for export")?,
            limits: {
                let common = ExportLimits {
                    rows: export_rows.ok_or("--row-limit is required for export")?,
                    bytes: output_bytes.ok_or("--output-limit-bytes is required for export")?,
                };
                match format.unwrap_or(Format::Jsonl) {
                    Format::Jsonl => ExportFormat::Jsonl(common),
                    Format::Parquet => {
                        ExportFormat::Parquet(formats::export_limits(common, parquet_bounds)?)
                    }
                    Format::Csv => return Err("export format must be jsonl or parquet".into()),
                }
            },
        },
        "explain" => Operation::Explain(query_file.ok_or("--query-file is required for explain")?),
        "resolve" => {
            Operation::Resolve(transaction.ok_or("--transaction is required for resolve")?)
        }
        _ => unreachable!("operation name was validated before options"),
    };
    let memory_limit = memory_limit.ok_or("--memory-limit-bytes is required")?;
    let temp_limit = temp_limit.ok_or("--temp-limit-bytes is required")?;
    let config = Config::new(memory_limit, temp_limit).map_err(ArgumentError::Library)?;
    Ok(Command {
        operation,
        database,
        config,
    })
}

fn parse_transaction(value: &OsString) -> Result<TransactionId, ArgumentError> {
    let text = value.as_encoded_bytes();
    let mut bytes = [0_u8; 24];
    if text.len() != bytes.len() * 2 {
        return Err("--transaction must contain exactly 48 hexadecimal digits".into());
    }
    for (output, pair) in bytes.iter_mut().zip(text.as_chunks::<2>().0) {
        let digit = |byte| match byte {
            b'0'..=b'9' => Ok(byte - b'0'),
            b'a'..=b'f' => Ok(byte - b'a' + 10),
            b'A'..=b'F' => Ok(byte - b'A' + 10),
            _ => Err(ArgumentError::from(
                "--transaction must contain exactly 48 hexadecimal digits",
            )),
        };
        *output = digit(pair[0])? * 16 + digit(pair[1])?;
    }
    TransactionId::from_bytes(bytes).map_err(ArgumentError::Library)
}

fn check_argument(value: &OsString) -> Result<(), ArgumentError> {
    if value.as_encoded_bytes().len() > MAX_ARGUMENT_BYTES {
        return Err("argument exceeds 4096 bytes".into());
    }
    Ok(())
}

fn parse_u64(value: OsString, owner: &'static str) -> Result<u64, ArgumentError> {
    value
        .into_string()
        .map_err(|_| ArgumentError::InvalidValue {
            owner,
            requirement: "must be UTF-8",
        })?
        .parse()
        .map_err(|_| ArgumentError::InvalidValue {
            owner,
            requirement: "must be an unsigned 64-bit integer",
        })
}

const IMPORT_OPTIONS: [&str; 8] = [
    "--input-limit-bytes",
    "--row-limit",
    "--record-limit-bytes",
    "--field-limit-bytes",
    "--batch-rows",
    "--batch-text-bytes",
    "--batch-limit",
    "--encoded-limit-bytes",
];

fn import_limits(values: [Option<u64>; 8]) -> Result<ImportLimits, ArgumentError> {
    let mut bounds = [0; 8];
    let maxima = [
        u64::MAX,
        u64::MAX,
        8_388_801,
        65_536,
        256,
        4_194_304,
        4_096,
        u64::MAX,
    ];
    for (index, value) in values.into_iter().enumerate() {
        bounds[index] = value.ok_or(ArgumentError::InvalidValue {
            owner: IMPORT_OPTIONS[index],
            requirement: "is required for import",
        })?;
        if bounds[index] == 0 || bounds[index] > maxima[index] {
            return Err(ArgumentError::InvalidValue {
                owner: IMPORT_OPTIONS[index],
                requirement: "is outside the supported positive range",
            });
        }
    }
    Ok(ImportLimits {
        csv: CsvLimits {
            input_bytes: bounds[0],
            rows: bounds[1],
            record_bytes: bounds[2] as u32,
            field_bytes: bounds[3] as u32,
            batch_rows: bounds[4] as u16,
            batch_text_bytes: bounds[5] as u32,
        },
        append: AppendLimits {
            batches: bounds[6] as u32,
            encoded_bytes: bounds[7],
        },
    })
}

#[cfg(test)]
#[path = "command/tests.rs"]
mod tests;
