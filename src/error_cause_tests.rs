use crate::{CauseKind, Error, ErrorCause, SourceSpan};
use std::{error::Error as StdError, io};

#[test]
fn leaf_facts_and_display_survive_inline_conversion() {
    let span = SourceSpan { start: 7, end: 19 };
    for error in [
        Error::InvalidPath("path"),
        Error::InvalidConfig("config"),
        Error::InvalidTransactionId,
        Error::Parse {
            message: "syntax",
            span,
        },
        Error::Bind {
            message: "binding",
            span,
        },
        Error::Input {
            message: "input",
            byte_offset: u64::MAX,
        },
        Error::Resource {
            owner: "owner",
            required: u64::MAX,
            limit: 17,
        },
        Error::Contention("owner"),
        Error::AlreadyExists,
        Error::NotFound,
        Error::Locked,
        Error::Corrupt("bytes"),
        Error::UnsupportedVersion(u32::MAX),
        Error::UnsupportedGeneration(u64::MAX),
        Error::Unsupported("operation"),
        Error::Cancelled,
        Error::ArithmeticOverflow {
            operation: "multiply",
            span,
        },
        Error::Io {
            operation: "write fence",
            source: io::Error::from_raw_os_error(28),
        },
    ] {
        let display = error.to_string();
        let facts = format!("{error:?}");
        let cause = ErrorCause::from_error(error);
        assert_eq!(cause.recovery_generation(), None);
        assert_eq!(cause.to_string(), display);
        assert_eq!(format!("{:?}", cause.kind()), facts);
        let leaf = cause.source().unwrap().downcast_ref::<CauseKind>().unwrap();
        if let CauseKind::Io { source, .. } = leaf {
            assert_eq!(source.raw_os_error(), Some(28));
            assert_eq!(
                leaf.source()
                    .unwrap()
                    .downcast_ref::<io::Error>()
                    .unwrap()
                    .raw_os_error(),
                Some(28)
            );
        }
    }
}

#[test]
fn cleanup_preserves_both_causes_and_primary_recovery_context() {
    let primary = Error::RecoveryRequired {
        generation: 1,
        source: ErrorCause::from_error(Error::Io {
            operation: "sync root",
            source: io::Error::from_raw_os_error(5),
        }),
    };
    let primary_text = primary.to_string();
    let cleanup = Error::Corrupt("unknown private entry");
    let cleanup_text = cleanup.to_string();
    let error = Error::CleanupRequired {
        primary: ErrorCause::from_error(primary),
        cleanup: ErrorCause::from_error(cleanup),
    };
    assert_eq!(
        error.to_string(),
        format!("{primary_text}; namespace cleanup required: {cleanup_text}")
    );
    let Error::CleanupRequired { primary, cleanup } = &error else {
        unreachable!()
    };
    assert_eq!(primary.recovery_generation(), Some(1));
    assert!(
        matches!(primary.kind(), CauseKind::Io { operation: "sync root", source } if source.raw_os_error() == Some(5))
    );
    assert!(matches!(
        cleanup.kind(),
        CauseKind::Corrupt("unknown private entry")
    ));
    assert_eq!(error.source().unwrap().to_string(), primary_text);
}

#[test]
fn arithmetic_cause_owns_the_original_source_range() {
    let text = String::from("# 雪\nFROM lineitem |> LIMIT 9223372036854775807 + 1");
    let start = text.find("922337").unwrap();
    let span = SourceSpan {
        start: start as u16,
        end: text.len() as u16,
    };
    let cause = ErrorCause::from_error(crate::scalar::ArithmeticFailure::Add.into_error(span));
    drop(text);
    assert!(
        matches!(cause.kind(), CauseKind::ArithmeticOverflow { operation: "addition", span: actual } if *actual == span)
    );
    assert_eq!(
        cause.to_string(),
        format!(
            "arithmetic overflow during addition at bytes {}..{}",
            span.start(),
            span.end()
        )
    );
}
