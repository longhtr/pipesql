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
fn invalid_reader_counts_abort_without_panicking_or_publishing_a_prefix() {
    fn object_bytes(db: &Database) -> std::io::Result<u64> {
        std::fs::read_dir(db.path().join(crate::storage::recovery::UNITS_NAME))?
            .try_fold(0, |sum, entry| Ok(sum + entry?.metadata()?.len()))
    }
    struct InvalidCount<'a> {
        prefix: &'a [u8],
        maximum: bool,
        db: &'a Database,
        initial_bytes: &'a Cell<Option<u64>>,
        private_bytes: &'a Cell<u64>,
        invalid_calls: &'a Cell<usize>,
    }
    impl Read for InvalidCount<'_> {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            if self.initial_bytes.get().is_none() {
                self.initial_bytes.set(Some(object_bytes(self.db)?));
            }
            if !self.prefix.is_empty() {
                return self.prefix.read(bytes);
            }
            self.private_bytes.set(object_bytes(self.db)?);
            self.invalid_calls.set(self.invalid_calls.get() + 1);
            bytes.fill(b'x');
            Ok(if self.maximum {
                usize::MAX
            } else {
                bytes.len() + 1
            })
        }
    }
    const OLD: &[u8] = b"id,note,number,day\n99,old,1.25,1970-01-02\n";
    let old = [
        Value::Int64(99),
        Value::String(StringValue::new("old")),
        Value::Double(1.25),
        Value::Date(DateValue::from_days_since_unix_epoch(1).unwrap()),
    ];
    for late in [false, true] {
        for maximum in [false, true] {
            for short_buffer in [false, true] {
                let directory = Directory::new();
                let db = database(&directory);
                let cancel = CancellationToken::new();
                db.import_csv("facts", OLD, limits(), &cancel, |_| Ok(()))
                    .unwrap();
                let baseline = db.reserved_memory_bytes();
                let generation = db.generation();
                let prefix = if late { VALID } else { &[] };
                let mut bounds = limits();
                bounds.csv.record_bytes = 8192;
                bounds.csv.field_bytes = 8192;
                bounds.csv.batch_text_bytes = 8192;
                if short_buffer {
                    bounds.csv.input_bytes = prefix.len() as u64 + 7;
                }
                let initial_bytes = Cell::new(None);
                let private_bytes = Cell::new(0);
                let invalid_calls = Cell::new(0);
                let mut token = None;
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    db.import_csv(
                        "facts",
                        InvalidCount {
                            prefix,
                            maximum,
                            db: &db,
                            initial_bytes: &initial_bytes,
                            private_bytes: &private_bytes,
                            invalid_calls: &invalid_calls,
                        },
                        bounds,
                        &cancel,
                        |transaction| {
                            token = Some(transaction);
                            Ok(())
                        },
                    )
                }));
                let error = outcome
                    .expect("invalid read count must not panic")
                    .unwrap_err();
                assert!(
                    matches!(error, Error::Input {
                    message: "CSV reader returned an invalid byte count",
                    byte_offset,
                } if byte_offset == prefix.len() as u64),
                    "{error:?}"
                );
                assert_eq!(invalid_calls.get(), 1);
                assert_eq!(private_bytes.get() > initial_bytes.get().unwrap(), late);
                assert_eq!(
                    db.resolve_commit(token.unwrap()).unwrap(),
                    CommitResolution::Aborted
                );
                assert_eq!(db.generation(), generation);
                assert_eq!(db.reserved_memory_bytes(), baseline);
                assert_eq!(db.reserved_temp_bytes(), 0);
                assert_eq!(object_bytes(&db).unwrap(), initial_bytes.get().unwrap());
                check_rows(&db, &[old]);
                db.import_csv(
                    "facts",
                    OLD,
                    limits(),
                    &CancellationToken::new(),
                    |_| Ok(()),
                )
                .unwrap();
                check_rows(&db, &[old, old]);
                assert_eq!(db.reserved_memory_bytes(), baseline);
                assert_eq!(db.reserved_temp_bytes(), 0);
            }
        }
    }
}

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
        const OLD: &[u8] = b"id,note,number,day\n0,old,1.25,1970-01-02\n";
        let old = [
            Value::Int64(0),
            Value::String(StringValue::new("old")),
            Value::Double(1.25),
            Value::Date(DateValue::from_days_since_unix_epoch(1).unwrap()),
        ];
        db.import_csv(
            "facts",
            OLD,
            limits(),
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
            db.import_csv(
                "facts",
                DropsAfterInput {
                    bytes: std::io::Cursor::new(VALID),
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
            check_rows(&db, &[old]);
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
        check_rows(&db, &[old]);
        let baseline = db.reserved_memory_bytes();
        let mut retry = None;
        let committed = db
            .import_csv(
                "facts",
                std::io::Cursor::new(VALID),
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
        check_rows(
            &db,
            &[
                old,
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
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}
