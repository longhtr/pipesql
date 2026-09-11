//! Public operation outcomes and owned source locations.
use crate::{ErrorCause, TransactionId, error_cause, storage_format};
use std::{error::Error as StdError, fmt, io};

/// An owned half-open range of UTF-8 byte offsets in the submitted query.
/// The range does not borrow or retain the query text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceSpan {
    pub(crate) start: u16,
    pub(crate) end: u16,
}

impl SourceSpan {
    /// Inclusive start offset in the submitted query's UTF-8 bytes.
    pub fn start(self) -> usize {
        usize::from(self.start)
    }

    /// Exclusive end offset in the submitted query's UTF-8 bytes.
    pub fn end(self) -> usize {
        usize::from(self.end)
    }
}

/// A typed operation failure, including publication and cleanup outcomes.
///
/// Match the variant when deciding whether to correct input, release resources,
/// reopen the database, or resolve a transaction. Display text is diagnostic,
/// not a stable machine-readable protocol. Commit uncertainty and unfinished
/// cleanup retain their own variants rather than becoming ordinary I/O errors.
#[derive(Debug)]
pub enum Error {
    InvalidPath(&'static str),
    InvalidConfig(&'static str),
    InvalidTransactionId,
    Parse {
        message: &'static str,
        span: SourceSpan,
    },
    Bind {
        message: &'static str,
        span: SourceSpan,
    },
    Input {
        message: &'static str,
        byte_offset: u64,
    },
    /// An owner's resource requirement exceeded admission or physical capacity.
    /// The owner determines the unit; most limits are bytes, but native work
    /// limits count operations.
    Resource {
        owner: &'static str,
        required: u64,
        limit: u64,
    },
    Contention(&'static str),
    AlreadyExists,
    NotFound,
    Locked,
    Corrupt(&'static str),
    UnsupportedVersion(u32),
    UnsupportedGeneration(u64),
    Unsupported(&'static str),
    Cancelled,
    ArithmeticOverflow {
        operation: &'static str,
        span: SourceSpan,
    },
    /// Publication may have committed. Resolve this exact transaction token;
    /// this error does not establish that retrying the write is safe.
    CommitAmbiguous {
        transaction: TransactionId,
        source: ErrorCause,
    },
    Io {
        operation: &'static str,
        source: io::Error,
    },
    /// The primary operation failed and cleanup also failed. Both causes survive.
    CleanupRequired {
        primary: ErrorCause,
        cleanup: ErrorCause,
    },
    /// Recovery did not complete. The reported generation must not be bypassed
    /// by silently opening an older one.
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
            Self::NotFound => write!(formatter, "requested database or transaction was not found"),
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
            Self::ArithmeticOverflow { operation, span } => {
                error_cause::fmt_arithmetic(operation, *span, formatter)
            }
            Self::CommitAmbiguous {
                transaction,
                source,
            } => write!(
                formatter,
                "commit outcome is ambiguous for transaction {transaction}: {source}"
            ),
            Self::Io { operation, source } => error_cause::fmt_io(operation, source, formatter),
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

pub(crate) fn map_format_error(error: storage_format::FormatError, bytes: &[u8]) -> Error {
    match error {
        storage_format::FormatError::Version if bytes.len() >= 12 => {
            let version = u32::from_le_bytes(bytes[8..12].try_into().expect("four-byte version"));
            Error::UnsupportedVersion(version)
        }
        storage_format::FormatError::Generation(generation) => {
            Error::UnsupportedGeneration(generation)
        }
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
