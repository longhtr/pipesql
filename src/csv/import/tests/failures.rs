//! Check CSV import outcomes when append effects or input handling fail.
//!
//! A healthy import records the append effect sequence, then each refusal starts
//! from an empty table. Reopen and transaction resolution must agree with complete
//! old or new rows, never an imported prefix. Inspection before append uses its own
//! default effects: these cuts cover issuance, private writes, publication and
//! cleanup, not every OS call. Separate cases exercise reader panic, a failed abort
//! that retains the input cause, and memory refusal before transaction issuance.

use super::*;
use crate::effects::{Effect, Faults, LoadEffect};
use std::sync::{Arc, Mutex};

#[test]
fn import_effect_failures_recover_only_complete_old_or_new_tables() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    {
        let directory = Directory::new();
        let db = database(&directory);
        let captured = trace.clone();
        db.import_csv_with_effects(
            "facts",
            VALID,
            limits(),
            &CancellationToken::new(),
            |_| Ok(()),
            &mut Effects::with_faults(Faults {
                action: Some(Box::new(move |index, effect| {
                    captured.lock().unwrap().push((index, effect))
                })),
                ..Faults::default()
            }),
        )
        .unwrap();
    }
    let trace = trace.lock().unwrap();
    assert!(!trace.is_empty() && trace.len() < 500);
    let publication = trace
        .iter()
        .find(|(_, effect)| *effect == Effect::Load(LoadEffect::RenameRootA))
        .unwrap()
        .0;
    let mut old_seen = false;
    let mut new_seen = false;
    for &(cut, _) in trace.iter() {
        let directory = Directory::new();
        let db = database(&directory);
        let mut token = None;
        let error = db
            .import_csv_with_effects(
                "facts",
                VALID,
                limits(),
                &CancellationToken::new(),
                |transaction| {
                    token = Some(transaction);
                    Ok(())
                },
                &mut Effects::with_faults(Faults {
                    fail_at: Some(cut),
                    ..Faults::default()
                }),
            )
            .unwrap_err();
        assert_eq!(
            matches!(error, Error::CommitAmbiguous { .. }),
            cut >= publication,
            "cut {cut}: {error:?}"
        );
        if let Error::CommitAmbiguous { transaction, .. } = error {
            assert_eq!(Some(transaction), token);
        }
        db.close().unwrap();
        let db = Database::open(
            &directory.0.join("db"),
            Config::new(8_000_000, 8_000_000).unwrap(),
        )
        .unwrap();
        assert_eq!(db.reserved_temp_bytes(), 0);
        match token.map(|token| db.resolve_commit(token).unwrap()) {
            Some(CommitResolution::Durable(commit)) => {
                new_seen = true;
                assert_eq!(commit.generation(), 2);
                check_rows(
                    &db,
                    &[
                        [
                            Value::Int64(1),
                            Value::String(StringValue::new("one,\"x\"")),
                            Value::Double(1.5),
                            Value::Date(DateValue::from_days_since_unix_epoch(1).unwrap()),
                        ],
                        [Value::Int64(2), Value::Null, Value::Null, Value::Null],
                        [
                            Value::Int64(3),
                            Value::String(StringValue::new("")),
                            Value::Double(-0.0),
                            Value::Date(DateValue::from_days_since_unix_epoch(-1).unwrap()),
                        ],
                    ],
                );
            }
            Some(CommitResolution::Aborted) | None => {
                old_seen = true;
                assert_eq!(db.generation(), 1);
                check_rows(&db, &[]);
                db.import_csv("facts", VALID, limits(), &CancellationToken::new(), |_| {
                    Ok(())
                })
                .unwrap();
            }
        }
    }
    assert!(old_seen && new_seen);
}

#[test]
fn reader_panic_requires_reopen_and_discards_written_prefix() {
    struct PanicsAtEnd(&'static [u8]);
    impl Read for PanicsAtEnd {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            assert!(!self.0.is_empty(), "deliberate CSV reader panic");
            self.0.read(bytes)
        }
    }
    let directory = Directory::new();
    let db = database(&directory);
    let mut token = None;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        db.import_csv(
            "facts",
            PanicsAtEnd(VALID),
            limits(),
            &CancellationToken::new(),
            |transaction| {
                token = Some(transaction);
                Ok(())
            },
        )
    }));
    assert!(result.is_err());
    assert!(db.catalog_writer().is_err());
    db.close().unwrap();
    let db = Database::open(
        &directory.0.join("db"),
        Config::new(8_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    assert_eq!(
        db.resolve_commit(token.unwrap()).unwrap(),
        CommitResolution::Aborted
    );
    check_rows(&db, &[]);
    db.import_csv("facts", VALID, limits(), &CancellationToken::new(), |_| {
        Ok(())
    })
    .unwrap();
}

#[test]
fn cleanup_failure_retains_input_error_and_reopen_removes_prefix() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mut input = VALID.to_vec();
    input.extend_from_slice(b"bad,1970-01-01,invalid,1\n");
    {
        let directory = Directory::new();
        let db = database(&directory);
        let captured = trace.clone();
        assert!(matches!(
            db.import_csv_with_effects(
                "facts",
                &input[..],
                limits(),
                &CancellationToken::new(),
                |_| Ok(()),
                &mut Effects::with_faults(Faults {
                    action: Some(Box::new(move |index, effect| captured
                        .lock()
                        .unwrap()
                        .push((index, effect)))),
                    ..Faults::default()
                })
            ),
            Err(Error::Input { .. })
        ));
    }
    let cut = trace
        .lock()
        .unwrap()
        .iter()
        .find(|(_, effect)| *effect == Effect::RemoveCleanupFile)
        .map(|(index, _)| *index);
    // Construction cleanup names its unlink effect explicitly. A missing hook
    // must fail this test rather than silently skip the cleanup control.
    let cut = cut.expect("import abort removes a private file");
    let directory = Directory::new();
    let db = database(&directory);
    let mut token = None;
    let error = db
        .import_csv_with_effects(
            "facts",
            &input[..],
            limits(),
            &CancellationToken::new(),
            |transaction| {
                token = Some(transaction);
                Ok(())
            },
            &mut Effects::with_faults(Faults {
                fail_at: Some(cut),
                ..Faults::default()
            }),
        )
        .unwrap_err();
    let Error::CleanupRequired { primary, cleanup } = error else {
        panic!("{error:?}")
    };
    assert!(matches!(
        primary.kind(),
        crate::CauseKind::Input {
            message: "invalid CSV INT64",
            ..
        }
    ));
    assert!(matches!(
        cleanup.kind(),
        crate::CauseKind::Io {
            operation: "remove private namespace file",
            ..
        }
    ));
    assert!(db.catalog_writer().is_err());
    db.close().unwrap();
    let db = Database::open(
        &directory.0.join("db"),
        Config::new(8_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    assert_eq!(
        db.resolve_commit(token.unwrap()).unwrap(),
        CommitResolution::Aborted
    );
    assert_eq!(db.reserved_temp_bytes(), 0);
    check_rows(&db, &[]);
    db.import_csv("facts", VALID, limits(), &CancellationToken::new(), |_| {
        Ok(())
    })
    .unwrap();
}

#[test]
fn shared_memory_refusal_precedes_issuance_and_releases_ownership() {
    let directory = Directory::new();
    let db = database(&directory);
    let baseline = db.reserved_memory_bytes();
    let wal = std::fs::read(db.path().join("WAL")).unwrap();
    let pressure = db
        .memory
        .reserve(8_000_000 - baseline - 100_000, "test competing owner")
        .unwrap();
    let error = db
        .import_csv("facts", VALID, limits(), &CancellationToken::new(), |_| {
            panic!("admission must precede issuance")
        })
        .unwrap_err();
    assert!(matches!(error, Error::Resource { .. }), "{error:?}");
    assert_eq!(std::fs::read(db.path().join("WAL")).unwrap(), wal);
    assert_eq!(db.reserved_temp_bytes(), 0);
    drop(pressure);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    db.import_csv("facts", VALID, limits(), &CancellationToken::new(), |_| {
        Ok(())
    })
    .unwrap();
}
