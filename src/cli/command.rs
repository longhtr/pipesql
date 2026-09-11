//! CLI grammar and ownership transfer from captured arguments to one command.
use pipesql::{Config, Error, TransactionId};
use std::ffi::OsString;
use std::path::PathBuf;

pub(super) const MAX_ARGUMENT_BYTES: usize = 4_096;

pub(super) enum Operation {
    Create,
    Open,
    Load(PathBuf),
    Query(PathBuf),
    Resolve(TransactionId),
}

pub(super) struct Command {
    pub(super) operation: Operation,
    pub(super) database: PathBuf,
    pub(super) config: Config,
}

fn usage() -> &'static str {
    "usage: pipesql create|open|load|query|resolve --database ABSOLUTE_PATH [--input ABSOLUTE_TBL] [--query-file ABSOLUTE_PATH] [--transaction HEX_TOKEN] --memory-limit-bytes N --temp-limit-bytes N"
}

#[derive(Debug)]
pub(super) enum ArgumentError {
    Message(&'static str),
    // Move the already byte-validated option, never allocate diagnostic text.
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
                "create" | "open" | "load" | "query" | "resolve"
            ) =>
        {
            value
        }
        _ => return Err(usage().into()),
    };
    let mut database = None;
    let mut input = None;
    let mut query_file = None;
    let mut transaction = None;
    let mut memory_limit = None;
    let mut temp_limit = None;
    // A successful iteration consumes one of six previously unseen options.
    // Thus at most six iterations advance; the next pair must terminate in an
    // error. Argument capture separately bounds the entire native input.
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
            _ => return Err("unknown option".into()),
        }
    }
    let database = database.ok_or("--database is required")?;
    if operation != "load" && input.is_some() {
        return Err("--input is accepted only for load".into());
    }
    if operation != "query" && query_file.is_some() {
        return Err("--query-file is accepted only for query".into());
    }
    if operation != "resolve" && transaction.is_some() {
        return Err("--transaction is accepted only for resolve".into());
    }
    let operation = match operation.as_str() {
        "create" => Operation::Create,
        "open" => Operation::Open,
        "load" => Operation::Load(input.ok_or("--input is required for load")?),
        "query" => Operation::Query(query_file.ok_or("--query-file is required for query")?),
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

#[cfg(test)]
mod tests;
