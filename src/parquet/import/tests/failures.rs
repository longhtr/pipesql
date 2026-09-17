//! Check importer failure outcomes across append's real filesystem effects.
//!
//! Healthy execution supplies the effect sequence. Each refusal starts with an
//! empty declared table, then recovery must expose either no rows or all 600
//! literal input values. Final-source failures also exercise cleanup and panic.
//!
//! Transaction resolution is checked against the recovered rows and generation.
//! Cleanup refusal must preserve the source error; final cancellation must abort,
//! while reader panic leaves the handle requiring reopen. These are effect-hook
//! and source controls, not a model of torn writes or power loss.

use super::*;
use crate::effects::{Effect, Faults, LoadEffect};
use std::sync::{Arc, Mutex};

#[test]
fn effect_refusals_recover_complete_old_or_new_tables() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    {
        let directory = Directory::new();
        let db = database(&directory);
        let captured = trace.clone();
        db.import_parquet_with_effects(
            "facts",
            Cursor::new(INPUT),
            limits(),
            &CancellationToken::new(),
            |_| Ok(()),
            &mut Effects::with_faults(Faults {
                action: Some(Box::new(move |index, effect| {
                    captured.lock().unwrap().push((index, effect));
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
    let (mut old, mut new) = (false, false);
    for &(cut, _) in trace.iter() {
        let directory = Directory::new();
        let db = database(&directory);
        let mut token = None;
        let error = db
            .import_parquet_with_effects(
                "facts",
                Cursor::new(INPUT),
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
                new = true;
                assert_eq!(commit.generation(), 2);
                check_rows(&db, 600);
            }
            Some(CommitResolution::Aborted) | None => {
                old = true;
                assert_eq!(db.generation(), 1);
                check_rows(&db, 0);
                db.import_parquet(
                    "facts",
                    Cursor::new(INPUT),
                    limits(),
                    &CancellationToken::new(),
                    |_| Ok(()),
                )
                .unwrap();
                check_rows(&db, 600);
            }
        }
    }
    assert!(old && new);
}

#[test]
fn cleanup_failure_retains_source_failure_and_recovers_without_a_prefix() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    {
        let directory = Directory::new();
        let db = database(&directory);
        let captured = trace.clone();
        let reader = FailsAtEnd {
            bytes: Cursor::new(INPUT),
            end_seeks: 0,
        };
        assert!(matches!(
            db.import_parquet_with_effects(
                "facts",
                reader,
                limits(),
                &CancellationToken::new(),
                |_| Ok(()),
                &mut Effects::with_faults(Faults {
                    action: Some(Box::new(move |index, effect| {
                        captured.lock().unwrap().push((index, effect));
                    })),
                    ..Faults::default()
                })
            ),
            Err(Error::Io { .. })
        ));
    }
    let cut = trace
        .lock()
        .unwrap()
        .iter()
        .find(|(_, effect)| *effect == Effect::RemoveCleanupFile)
        .expect("abort removes private units")
        .0;
    let directory = Directory::new();
    let db = database(&directory);
    let mut token = None;
    let reader = FailsAtEnd {
        bytes: Cursor::new(INPUT),
        end_seeks: 0,
    };
    let error = db
        .import_parquet_with_effects(
            "facts",
            reader,
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
        crate::CauseKind::Io {
            operation: "seek Parquet input",
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
    check_rows(&db, 0);
    db.import_parquet(
        "facts",
        Cursor::new(INPUT),
        limits(),
        &CancellationToken::new(),
        |_| Ok(()),
    )
    .unwrap();
    check_rows(&db, 600);
}

struct FinalAction<'a> {
    bytes: Cursor<&'static [u8]>,
    end_seeks: usize,
    cancel: Option<&'a CancellationToken>,
}

impl Read for FinalAction<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.bytes.read(bytes)
    }
}

impl Seek for FinalAction<'_> {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        if matches!(to, SeekFrom::End(0)) {
            self.end_seeks += 1;
            if self.end_seeks == 2 {
                if let Some(cancel) = self.cancel {
                    cancel.cancel();
                } else {
                    panic!("controlled final Parquet source panic");
                }
            }
        }
        self.bytes.seek(to)
    }
}

#[test]
fn final_cancellation_aborts_and_reader_panic_requires_reopen() {
    for panic in [false, true] {
        let directory = Directory::new();
        let db = database(&directory);
        let baseline = db.reserved_memory_bytes();
        let cancel = CancellationToken::new();
        let reader = FinalAction {
            bytes: Cursor::new(INPUT),
            end_seeks: 0,
            cancel: (!panic).then_some(&cancel),
        };
        let mut token = None;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            db.import_parquet("facts", reader, limits(), &cancel, |transaction| {
                token = Some(transaction);
                Ok(())
            })
        }));
        if panic {
            assert!(result.is_err());
            assert!(db.catalog_writer().is_err());
        } else {
            assert!(matches!(result.unwrap(), Err(Error::Cancelled)));
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
            check_rows(&db, 0);
        }
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
        check_rows(&db, 0);
        db.import_parquet(
            "facts",
            Cursor::new(INPUT),
            limits(),
            &CancellationToken::new(),
            |_| Ok(()),
        )
        .unwrap();
        check_rows(&db, 600);
    }
}
