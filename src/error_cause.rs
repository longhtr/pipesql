use crate::{Error, SourceSpan};
use std::{error::Error as StdError, fmt, io};

/// An owned, nonrecursive cause. Construction moves failure facts without
/// allocating or converting them to text. Returned causes belong to the caller.
#[derive(Debug)]
pub struct ErrorCause {
    kind: CauseKind,
    recovery_generation: Option<u64>,
}

/// Leaf diagnostic facts, without commit or cleanup outcome authority.
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
    ArithmeticOverflow {
        operation: &'static str,
        span: SourceSpan,
    },
    Io {
        operation: &'static str,
        source: io::Error,
    },
}

impl ErrorCause {
    pub fn kind(&self) -> &CauseKind {
        &self.kind
    }

    pub fn recovery_generation(&self) -> Option<u64> {
        self.recovery_generation
    }

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
            Error::ArithmeticOverflow { operation, span } => {
                CauseKind::ArithmeticOverflow { operation, span }
            }
            Error::Io { operation, source } => CauseKind::Io { operation, source },
            Error::RecoveryRequired { generation, source } => {
                // Create can preserve a failed recovery as its primary cause.
                // No producer nests a recovery context within another one.
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
                fmt_arithmetic(operation, *span, formatter)
            }
            Self::Io { operation, source } => fmt_io(operation, source, formatter),
        }
    }
}

// io::Error's native Display materializes an OS message String on the pinned
// toolchain. Render the stable kind and exact OS code without that allocation.
// Opaque custom errors retain their own Display contract; the engine's injected
// test errors use pre-owned text, not native message lookup.
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

// The error owns byte offsets, not source text or a diagnostic String.
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
