//! Errors returned by database operations and query evaluation.
//!
//! A failed write does not always mean that no data was committed. A filesystem
//! operation can fail after the commit has taken effect. Retrying the write
//! without checking its outcome could write the same data twice.
//!
//! [`Error`] keeps uncertain commits, failed cleanup and recovery requests distinct
//! so callers can handle each outcome. Their fields identify the transaction or
//! database version involved and retain the failures that need to be resolved.
//!
//! [`SourceSpan`] locates an error in query text. [`ErrorCause`] records the underlying
//! failure inside a commit, cleanup or recovery error.

mod cause;
pub use cause::{CauseKind, ErrorCause};

use crate::TransactionId;
use crate::storage::format;
use std::{error::Error as StdError, fmt, io};

/// A range of bytes in the submitted query, including `start` but excluding `end`.
///
/// Offsets count UTF-8 bytes from zero, not characters. For example, `2..5`
/// selects bytes 2, 3 and 4. This value stores only the offsets; the caller must
/// keep the query text if it needs to display the selected text.
/// Arithmetic errors identify the constant expression, computed definition or
/// demanded aggregate call, excluding its alias. Final SUM overflow identifies
/// SUM even when AVG shares its argument state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceSpan {
    pub(crate) start: u16,
    pub(crate) end: u16,
}

impl SourceSpan {
    /// The byte offset where the range begins.
    pub fn start(self) -> usize {
        usize::from(self.start)
    }

    /// The byte offset immediately after the range.
    pub fn end(self) -> usize {
        usize::from(self.end)
    }
}

/// A database or query failure that callers can inspect by matching its variant.
///
/// Use the variant and its fields to handle an error programmatically. The
/// formatted message is for people and may change; do not parse it to decide
/// whether to retry a write or recover a database.
#[derive(Debug)]
pub enum Error {
    InvalidPath(&'static str),
    InvalidConfig(&'static str),
    InvalidTransactionId,
    /// The query text does not match the supported grammar.
    Parse {
        message: &'static str,
        span: SourceSpan,
    },
    /// Parsed syntax cannot be resolved against its inputs or supported types.
    Bind {
        message: &'static str,
        span: SourceSpan,
    },
    /// Invalid input data or file. `byte_offset` counts bytes from zero;
    /// whole-file failures may use zero when no individual byte is responsible.
    Input {
        message: &'static str,
        byte_offset: u64,
    },
    /// An operation could not obtain the resources it needs within a limit.
    ///
    /// `owner` names the requirement being checked. `required` and `limit` use
    /// the same unit: usually bytes, but native work limits count operations.
    /// The limit may come from a configured budget or available physical capacity.
    Resource {
        owner: &'static str,
        required: u64,
        limit: u64,
    },
    Contention(&'static str),
    AlreadyExists,
    NotFound,
    Locked,
    /// Stored bytes or an internal representation violate a required invariant.
    Corrupt(&'static str),
    UnsupportedVersion(u32),
    UnsupportedGeneration(u64),
    Unsupported(&'static str),
    Cancelled,
    DivisionByZero {
        span: SourceSpan,
    },
    ArithmeticOverflow {
        operation: &'static str,
        span: SourceSpan,
    },
    /// An argument is outside the operation's domain, such as SQRT of a negative value.
    ArithmeticDomain {
        operation: &'static str,
        span: SourceSpan,
    },
    /// The write may have committed despite the reported failure.
    ///
    /// Close and reopen the database, then pass `transaction` to
    /// [`Database::resolve_commit`](crate::Database::resolve_commit) before retrying.
    /// Repeating the write without checking could duplicate committed data.
    /// `source` describes the failure that prevented a definite answer.
    CommitAmbiguous {
        transaction: TransactionId,
        source: ErrorCause,
    },
    Io {
        operation: &'static str,
        source: io::Error,
    },
    /// Cleanup failed after an earlier operation had already failed.
    ///
    /// `primary` records the original failure; `cleanup` records the cleanup
    /// failure. The database may still contain unfinished work. This error
    /// does not establish that the operation was fully rolled back.
    /// The standard error chain follows `primary`; inspect `cleanup` separately.
    CleanupRequired {
        primary: ErrorCause,
        cleanup: ErrorCause,
    },
    /// The database needs recovery for the reported generation.
    ///
    /// `generation` identifies the database version that recovery must account
    /// for; `source` explains why recovery is needed or could not finish.
    /// Opening an older version instead could hide committed changes.
    RecoveryRequired {
        generation: u64,
        source: ErrorCause,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(message) => write!(formatter, "invalid database path: {message}"),
            Self::InvalidConfig(message) => write!(formatter, "invalid configuration: {message}"),
            Self::InvalidTransactionId => write!(formatter, "invalid transaction identity"),
            Self::Parse { message, span } => write!(
                formatter,
                "parse error at bytes {}..{}: {message}",
                span.start, span.end
            ),
            Self::Bind { message, span } => write!(
                formatter,
                "bind error at bytes {}..{}: {message}",
                span.start, span.end
            ),
            Self::Input {
                message,
                byte_offset,
            } => write!(formatter, "input error at byte {byte_offset}: {message}"),
            Self::Resource {
                owner,
                required,
                limit,
            } => write!(
                formatter,
                "resource refusal for {owner}: required={required}, limit={limit}"
            ),
            Self::Contention(owner) => write!(formatter, "contention limit reached for {owner}"),
            Self::AlreadyExists => write!(formatter, "database path already exists"),
            Self::NotFound => write!(
                formatter,
                "requested database, table or transaction was not found"
            ),
            Self::Locked => write!(formatter, "database is already open"),
            Self::Corrupt(message) => write!(formatter, "corrupt database namespace: {message}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported database format version {version}")
            }
            Self::UnsupportedGeneration(generation) => {
                write!(formatter, "unsupported database generation {generation}")
            }
            Self::Unsupported(message) => write!(formatter, "unsupported operation: {message}"),
            Self::Cancelled => write!(formatter, "operation cancelled"),
            Self::DivisionByZero { span } => cause::fmt_division_by_zero(*span, formatter),
            Self::ArithmeticOverflow { operation, span } => {
                cause::fmt_arithmetic(operation, *span, formatter)
            }
            Self::ArithmeticDomain { operation, span } => {
                cause::fmt_arithmetic_domain(operation, *span, formatter)
            }
            Self::CommitAmbiguous {
                transaction,
                source,
            } => write!(
                formatter,
                "commit outcome is ambiguous for transaction {transaction}: {source}"
            ),
            Self::Io { operation, source } => cause::fmt_io(operation, source, formatter),
            Self::CleanupRequired { primary, cleanup } => {
                write!(
                    formatter,
                    "{primary}; namespace cleanup required: {cleanup}"
                )
            }
            Self::RecoveryRequired { generation, source } => {
                write!(
                    formatter,
                    "database recovery for generation {generation} must be retried: {source}"
                )
            }
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::CleanupRequired { primary, .. } => Some(primary),
            Self::RecoveryRequired { source, .. } | Self::CommitAmbiguous { source, .. } => {
                Some(source)
            }
            _ => None,
        }
    }
}

// Keep recognizable unsupported formats distinct from damaged current-format
// records. Recovery must not treat a future format as something it can repair.
pub(crate) fn map_format_error(error: format::FormatError, bytes: &[u8]) -> Error {
    match error {
        format::FormatError::Version if bytes.len() >= 12 => {
            let version = u32::from_le_bytes(bytes[8..12].try_into().expect("four-byte version"));
            Error::UnsupportedVersion(version)
        }
        format::FormatError::Generation(generation) => Error::UnsupportedGeneration(generation),
        _ => Error::Corrupt("snapshot/unit record validation failed"),
    }
}

pub(crate) fn recovery_needed(generation: u64) -> Error {
    Error::RecoveryRequired {
        generation,
        source: ErrorCause::from_error(Error::Unsupported(
            "close and reopen for exclusive recovery",
        )),
    }
}

pub(crate) fn map_not_found(operation: &'static str, source: io::Error) -> Error {
    if source.kind() == io::ErrorKind::NotFound {
        Error::NotFound
    } else {
        io_error(operation, source)
    }
}

pub(crate) fn io_error(operation: &'static str, source: io::Error) -> Error {
    Error::Io { operation, source }
}
