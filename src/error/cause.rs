//! Preserve the original failure when reporting a commit, cleanup or recovery error.
//!
//! Cleanup can fail while handling another error. The caller needs both failures:
//! what went wrong first, and what prevented cleanup. `Error` stores the overall
//! outcome; `ErrorCause` keeps each underlying failure and its details.
//!
//! A cause stores one failure kind and, when relevant, the database version that
//! needs recovery. It cannot contain another cause. This limits how much state
//! error reporting can retain and lets conversion reuse existing values without
//! allocating memory, including when the original failure was lack of memory.

use crate::{Error, SourceSpan};
use std::{error::Error as StdError, fmt, io};

/// The underlying failure and an optional database version needing recovery.
/// Stores the details directly, without allocating or formatting a message.
/// These details belong to the caller receiving the error.
#[derive(Debug)]
pub struct ErrorCause {
    kind: CauseKind,
    recovery_generation: Option<u64>,
}

/// The kind of failure and its details, such as a query position or an I/O error.
/// Commit and cleanup outcomes belong to [`Error`], not this enum.
#[derive(Debug)]
pub enum CauseKind {
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
    DivisionByZero {
        span: SourceSpan,
    },
    ArithmeticOverflow {
        operation: &'static str,
        span: SourceSpan,
    },
    ArithmeticDomain {
        operation: &'static str,
        span: SourceSpan,
    },
    Io {
        operation: &'static str,
        source: io::Error,
    },
}

impl ErrorCause {
    /// Inspect the failure's kind and details without formatting a message.
    pub fn kind(&self) -> &CauseKind {
        &self.kind
    }

    /// The database version associated with a recovery failure, if one was recorded.
    pub fn recovery_generation(&self) -> Option<u64> {
        self.recovery_generation
    }

    // Move an ordinary error into a cause. Commit ambiguity and failed cleanup
    // must remain full errors: conversion would lose the transaction identifier
    // or one of the two failures. Treat either input as a programming mistake.
    pub(crate) fn from_error(error: Error) -> Self {
        let kind = match error {
            Error::InvalidPath(message) => CauseKind::InvalidPath(message),
            Error::InvalidConfig(message) => CauseKind::InvalidConfig(message),
            Error::InvalidTransactionId => CauseKind::InvalidTransactionId,
            Error::Parse { message, span } => CauseKind::Parse { message, span },
            Error::Bind { message, span } => CauseKind::Bind { message, span },
            Error::Input {
                message,
                byte_offset,
            } => CauseKind::Input {
                message,
                byte_offset,
            },
            Error::Resource {
                owner,
                required,
                limit,
            } => CauseKind::Resource {
                owner,
                required,
                limit,
            },
            Error::Contention(owner) => CauseKind::Contention(owner),
            Error::AlreadyExists => CauseKind::AlreadyExists,
            Error::NotFound => CauseKind::NotFound,
            Error::Locked => CauseKind::Locked,
            Error::Corrupt(message) => CauseKind::Corrupt(message),
            Error::UnsupportedVersion(version) => CauseKind::UnsupportedVersion(version),
            Error::UnsupportedGeneration(generation) => {
                CauseKind::UnsupportedGeneration(generation)
            }
            Error::Unsupported(message) => CauseKind::Unsupported(message),
            Error::Cancelled => CauseKind::Cancelled,
            Error::DivisionByZero { span } => CauseKind::DivisionByZero { span },
            Error::ArithmeticOverflow { operation, span } => {
                CauseKind::ArithmeticOverflow { operation, span }
            }
            Error::ArithmeticDomain { operation, span } => {
                CauseKind::ArithmeticDomain { operation, span }
            }
            Error::Io { operation, source } => CauseKind::Io { operation, source },
            Error::RecoveryRequired { generation, source } => {
                // Creation may report failed recovery followed by failed cleanup.
                // Keep the recovery version alongside the original failure, but
                // reject a second nested version that this type cannot represent.
                assert!(
                    source.recovery_generation.is_none(),
                    "recovery cause context must be singular"
                );
                return Self {
                    kind: source.kind,
                    recovery_generation: Some(generation),
                };
            }
            Error::CommitAmbiguous { .. } | Error::CleanupRequired { .. } => {
                unreachable!("commit and cleanup outcomes must not become diagnostic causes")
            }
        };
        Self {
            kind,
            recovery_generation: None,
        }
    }
}

impl fmt::Display for ErrorCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(generation) = self.recovery_generation {
            write!(
                formatter,
                "database recovery for generation {generation} must be retried: "
            )?;
        }
        self.kind.fmt(formatter)
    }
}

impl StdError for ErrorCause {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&self.kind)
    }
}

impl fmt::Display for CauseKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(message) => write!(formatter, "invalid database path: {message}"),
            Self::InvalidConfig(message) => write!(formatter, "invalid configuration: {message}"),
            Self::InvalidTransactionId => write!(formatter, "invalid transaction identity"),
            Self::Parse { message, span } => write!(
                formatter,
                "parse error at bytes {}..{}: {message}",
                span.start(),
                span.end()
            ),
            Self::Bind { message, span } => write!(
                formatter,
                "bind error at bytes {}..{}: {message}",
                span.start(),
                span.end()
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
            Self::DivisionByZero { span } => fmt_division_by_zero(*span, formatter),
            Self::ArithmeticOverflow { operation, span } => {
                fmt_arithmetic(operation, *span, formatter)
            }
            Self::ArithmeticDomain { operation, span } => {
                fmt_arithmetic_domain(operation, *span, formatter)
            }
            Self::Io { operation, source } => fmt_io(operation, source, formatter),
        }
    }
}

// Formatting a native io::Error allocates its OS message on the pinned toolchain.
// Print the error kind and OS code instead, so reporting a memory failure does
// not require another allocation. Custom errors format themselves; injected
// test errors already own their text and do not look up an OS message.
pub(super) fn fmt_io(
    operation: &str,
    source: &io::Error,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    if let Some(code) = source.raw_os_error() {
        write!(
            formatter,
            "{operation}: {} (os error {code})",
            source.kind()
        )
    } else if let Some(cause) = source.get_ref() {
        write!(formatter, "{operation}: {cause}")
    } else {
        write!(formatter, "{operation}: {}", source.kind())
    }
}

impl StdError for CauseKind {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

// Write the stored query offsets directly to the caller's formatter. Reporting
// an arithmetic failure does not need the query text or a new message buffer.
pub(super) fn fmt_arithmetic(
    operation: &str,
    span: SourceSpan,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    write!(
        formatter,
        "arithmetic overflow during {operation} at bytes {}..{}",
        span.start(),
        span.end()
    )
}

pub(super) fn fmt_arithmetic_domain(
    operation: &str,
    span: SourceSpan,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    write!(
        formatter,
        "arithmetic domain error during {operation} at bytes {}..{}",
        span.start(),
        span.end()
    )
}

pub(super) fn fmt_division_by_zero(
    span: SourceSpan,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    write!(
        formatter,
        "division by zero at bytes {}..{}",
        span.start(),
        span.end()
    )
}

#[cfg(test)]
mod tests;
