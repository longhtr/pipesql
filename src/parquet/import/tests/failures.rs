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

#[test]
fn reader_destruction_precedes_publication() {
    fn objects(db: &Database) -> std::collections::BTreeMap<std::ffi::OsString, Vec<u8>> {
        std::fs::read_dir(db.path().join(crate::storage::recovery::UNITS_NAME))
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (entry.file_name(), std::fs::read(entry.path()).unwrap())
            })
            .collect()
    }
    struct DropsAfterInput<'a> {
        bytes: std::io::Cursor<&'static [u8]>,
        cancel: Option<&'a CancellationToken>,
        dropped: &'a Cell<bool>,
        db: &'a Database,
        old_units: usize,
    }
    impl Read for DropsAfterInput<'_> {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            self.bytes.read(bytes)
        }
    }
    impl Seek for DropsAfterInput<'_> {
        fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
            self.bytes.seek(position)
        }
    }
    impl Drop for DropsAfterInput<'_> {
        fn drop(&mut self) {
            assert_eq!(self.bytes.position(), self.bytes.get_ref().len() as u64);
            assert!(
                objects(self.db).len() > self.old_units,
                "import must own private units before reader destruction"
            );
            assert!(!self.dropped.replace(true));
            if let Some(cancel) = self.cancel {
                cancel.cancel();
            } else {
                panic!("controlled reader destructor panic");
            }
        }
    }
    for panic in [false, true] {
        let directory = Directory::new();
        let db = database(&directory);
        db.import_csv(
            "facts",
            b"id\n-1\n".as_slice(),
            crate::csv::ImportLimits {
                csv: crate::csv::CsvLimits {
                    input_bytes: 100,
                    rows: 1,
                    record_bytes: 32,
                    field_bytes: 8,
                    batch_rows: 1,
                    batch_text_bytes: 32,
                },
                append: limits().append,
            },
            &CancellationToken::new(),
            |_| Ok(()),
        )
        .unwrap();
        let baseline = db.reserved_memory_bytes();
        let retained = objects(&db);
        let generation = db.generation();
        let cancel = CancellationToken::new();
        let dropped = Cell::new(false);
        let mut token = None;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            db.import_parquet(
                "facts",
                DropsAfterInput {
                    bytes: std::io::Cursor::new(INPUT),
                    cancel: (!panic).then_some(&cancel),
                    dropped: &dropped,
                    db: &db,
                    old_units: retained.len(),
                },
                limits(),
                &cancel,
                |transaction| {
                    token = Some(transaction);
                    Ok(())
                },
            )
        }));
        assert!(dropped.get());
        if panic {
            let cause = result.expect_err("reader destructor panic must propagate");
            assert_eq!(
                cause.downcast_ref::<&str>(),
                Some(&"controlled reader destructor panic")
            );
            assert!(db.catalog_writer().is_err());
            // Unfinished writer drop retains the charge until recovery removes files.
            assert!(db.reserved_temp_bytes() > 0);
        } else {
            assert!(matches!(result.unwrap(), Err(Error::Cancelled)));
            assert_eq!(
                db.resolve_commit(token.unwrap()).unwrap(),
                CommitResolution::Aborted
            );
            assert_eq!(objects(&db), retained);
            assert_eq!(db.reserved_temp_bytes(), 0);
            check_rows_from(&db, -1, 1);
        }
        assert_eq!(db.generation(), generation);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        db.close().unwrap();
        let db = Database::open(
            &directory.0.join("db"),
            Config::new(8_000_000, 8_000_000).unwrap(),
        )
        .unwrap();
        assert_eq!(db.generation(), generation);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(
            db.resolve_commit(token.unwrap()).unwrap(),
            CommitResolution::Aborted
        );
        assert_eq!(objects(&db), retained);
        check_rows_from(&db, -1, 1);
        let baseline = db.reserved_memory_bytes();
        let mut retry = None;
        let committed = db
            .import_parquet(
                "facts",
                std::io::Cursor::new(INPUT),
                limits(),
                &CancellationToken::new(),
                |transaction| {
                    retry = Some(transaction);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(committed.generation(), generation + 1);
        assert!(
            matches!(db.resolve_commit(retry.unwrap()).unwrap(), CommitResolution::Durable(commit) if commit.generation() == generation + 1)
        );
        check_rows_from(&db, -1, 601);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}
