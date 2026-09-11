use super::{Directory, append, database, id, query_values};
use crate::effects::{DirectoryKind, Effect, Effects, Faults};
use crate::namespace::UNITS_NAME;
use crate::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, DataType,
    Database, Error,
};
use std::{cell::RefCell, rc::Rc};

#[test]
fn scratch_bootstrap_coexists_with_public_writer_and_pinned_reader() {
    use crate::scratch::Scratch;
    let directory = Directory::new();
    let db = Rc::new(database(&directory));
    append(&db, 1);
    let old = db.prepare("FROM facts").unwrap();
    let observed = Rc::new(std::cell::Cell::new(false));
    let captured = observed.clone();
    let shared = db.clone();
    let mut effects = Effects::with_faults(Faults {
        after_action: Some(Box::new(move |_, effect| {
            if effect == Effect::Load(crate::effects::LoadEffect::CreateStaging)
                && !captured.replace(true)
            {
                assert!(
                    shared
                        .path()
                        .join(crate::namespace::PRIVATE_NAME)
                        .join(crate::namespace::CATALOG_SCRATCH_NAMES[0])
                        .exists()
                );
                assert!(matches!(
                    Scratch::new(&shared, &CancellationToken::new(), &mut Effects::default()),
                    Err(Error::Contention("scratch bootstrap"))
                ));
                append(&shared, 2);
                assert_eq!(
                    query_values(&shared, &shared.prepare("FROM facts").unwrap()),
                    [1, 2]
                );
            }
        })),
        ..Faults::default()
    });
    let cancel = CancellationToken::new();
    let mut first = Scratch::new(&db, &cancel, &mut effects).unwrap();
    assert!(observed.get());
    drop(effects);
    assert_eq!(query_values(&db, &old), [1]);
    let mut transaction = db
        .begin_append(
            "facts",
            AppendLimits {
                batches: 1,
                encoded_bytes: 10_000,
            },
            &cancel,
        )
        .unwrap();
    let mut second = Scratch::new(&db, &cancel, &mut Effects::default()).unwrap();
    let baseline = db.reserved_temp_bytes();
    first
        .write(0, 0, b"one", &cancel, &mut Effects::default())
        .unwrap();
    second
        .write(1, 0, b"two", &cancel, &mut Effects::default())
        .unwrap();
    assert_eq!(db.reserved_temp_bytes(), baseline + 6);
    transaction
        .write(
            &[ColumnInput {
                values: ColumnValues::Int64(&[3]),
                validity: &[1],
            }],
            &cancel,
        )
        .unwrap();
    transaction.commit(&cancel).unwrap();
    let mut bytes = [0; 3];
    first
        .read(0, &mut bytes, 0, &cancel, &mut Effects::default())
        .unwrap();
    assert_eq!(&bytes, b"one");
    second
        .read(1, &mut bytes, 0, &cancel, &mut Effects::default())
        .unwrap();
    assert_eq!(&bytes, b"two");
    assert_eq!(db.reserved_temp_bytes(), 6);
    drop(first);
    assert_eq!(db.reserved_temp_bytes(), 3);
    drop(second);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert_eq!(query_values(&db, &old), [1]);
    assert_eq!(
        query_values(&db, &db.prepare("FROM facts").unwrap()),
        [1, 2, 3]
    );
}

#[test]
fn scratch_extent_admission_and_failed_writes_keep_exact_shared_charge() {
    use crate::scratch::Scratch;
    let directory = Directory::new();
    let db =
        Database::create_empty(&directory.0, crate::Config::new(2_000_000, 7).unwrap()).unwrap();
    let cancel = CancellationToken::new();
    let baseline = db.reserved_memory_bytes();
    let mut first = Scratch::new(&db, &cancel, &mut Effects::default()).unwrap();
    let mut second = Scratch::new(&db, &cancel, &mut Effects::default()).unwrap();
    first
        .write(0, 0, b"123", &cancel, &mut Effects::default())
        .unwrap();
    let mut failed = Effects::with_faults(Faults {
        fail_at: Some(0),
        ..Faults::default()
    });
    assert!(second.write(1, 0, b"4567", &cancel, &mut failed).is_err());
    assert_eq!(db.reserved_temp_bytes(), 7);
    assert_eq!(second.test_file(1).metadata().unwrap().len(), 0);
    let mut refused = Effects::default();
    assert!(matches!(
        first.write(0, 3, b"8", &cancel, &mut refused),
        Err(Error::Resource {
            required: 8,
            limit: 7,
            ..
        })
    ));
    assert_eq!(refused.count(), 0);
    assert_eq!(first.test_file(0).metadata().unwrap().len(), 3);
    second
        .write(1, 0, b"4567", &cancel, &mut Effects::default())
        .unwrap();
    first
        .write(0, 0, b"a", &cancel, &mut Effects::default())
        .unwrap();
    assert_eq!(db.reserved_temp_bytes(), 7);
    let mut byte = [0];
    assert!(matches!(
        first.read(0, &mut byte, 3, &cancel, &mut refused),
        Err(Error::Corrupt("scratch read exceeds admitted extent"))
    ));
    assert_eq!(refused.count(), 0);
    assert!(
        first
            .write(0, u64::MAX, b"x", &cancel, &mut refused)
            .is_err()
    );
    assert_eq!(refused.count(), 0);
    drop(second);
    assert_eq!(db.reserved_temp_bytes(), 3);
    drop(first);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert_eq!(db.reserved_memory_bytes(), baseline);
}

#[test]
fn scratch_constructor_failure_preserves_debt_until_exclusive_reopen() {
    use crate::scratch::Scratch;
    let control = Directory::new();
    let db = database(&control);
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    let mut effects = Effects::with_faults(Faults {
        action: Some(Box::new(move |i, e| captured.borrow_mut().push((i, e)))),
        ..Faults::default()
    });
    drop(Scratch::new(&db, &CancellationToken::new(), &mut effects).unwrap());
    let events = events.borrow();
    let create = events
        .iter()
        .find(|(_, e)| *e == Effect::Load(crate::effects::LoadEffect::CreateStaging))
        .unwrap()
        .0;
    let barrier = events
        .iter()
        .find(|(_, e)| *e == Effect::SyncDirectory(DirectoryKind::Private))
        .unwrap()
        .0;
    for index in 0..effects.count() {
        let directory = Directory::new();
        let db = database(&directory);
        append(&db, 1);
        let baseline = db.reserved_memory_bytes();
        let mut failure = Effects::with_faults(Faults {
            fail_at: Some(index),
            ..Faults::default()
        });
        assert!(
            Scratch::new(&db, &CancellationToken::new(), &mut failure).is_err(),
            "effect {index}"
        );
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        let pending = db.path().join(crate::namespace::PRIVATE_NAME);
        let entries: Vec<_> = std::fs::read_dir(&pending)
            .unwrap()
            .map(|e| e.unwrap())
            .collect();
        assert!(entries.len() <= 2);
        for entry in entries {
            assert_eq!(entry.metadata().unwrap().len(), 0);
        }
        let mut retry = Effects::default();
        if (create..=barrier).contains(&index) {
            assert!(matches!(
                Scratch::new(&db, &CancellationToken::new(), &mut retry),
                Err(Error::RecoveryRequired { .. })
            ));
            assert_eq!(retry.count(), 0);
        } else {
            drop(Scratch::new(&db, &CancellationToken::new(), &mut retry).unwrap());
        }
        // Isolated scratch debt owns no commit or query authority.
        append(&db, 2);
        assert_eq!(
            query_values(&db, &db.prepare("FROM facts").unwrap()),
            [1, 2]
        );
        db.close().unwrap();
        let reopened = Database::open(
            &directory.0,
            crate::Config::new(2_000_000, 2_000_000).unwrap(),
        )
        .unwrap();
        drop(
            Scratch::new(
                &reopened,
                &CancellationToken::new(),
                &mut Effects::default(),
            )
            .unwrap(),
        );
        assert_eq!(std::fs::read_dir(pending).unwrap().count(), 0);
        append(&reopened, 3);
    }
    println!(
        "shared scratch constructor failure effects={}",
        effects.count()
    );
}

#[test]
fn scratch_bootstrap_real_thread_excludes_only_another_bootstrap() {
    use crate::scratch::Scratch;
    use std::sync::{Arc, Barrier};
    let directory = Directory::new();
    let db = Arc::new(database(&directory));
    append(&db, 1);
    let old = db.prepare("FROM facts").unwrap();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let shared = db.clone();
    let worker_entered = entered.clone();
    let worker_release = release.clone();
    let worker = std::thread::spawn(move || {
        let mut first = true;
        let mut effects = Effects::with_faults(Faults {
            after_action: Some(Box::new(move |_, effect| {
                if first && effect == Effect::Load(crate::effects::LoadEffect::CreateStaging) {
                    first = false;
                    worker_entered.wait();
                    worker_release.wait();
                }
            })),
            ..Faults::default()
        });
        let cancel = CancellationToken::new();
        let mut scratch = Scratch::new(&shared, &cancel, &mut effects).unwrap();
        scratch.write(0, 0, b"ok", &cancel, &mut effects).unwrap();
    });
    entered.wait();
    // Capture assertions until after releasing/joining the worker, so an
    // unexpected result cannot strand the fixture at its barrier.
    let refused = matches!(
        Scratch::new(&db, &CancellationToken::new(), &mut Effects::default()),
        Err(Error::Contention("scratch bootstrap"))
    );
    let transaction = db.declare_table(
        "other",
        &[ColumnDeclaration {
            name: "v",
            data_type: DataType::Int64,
            nullable: false,
        }],
        &CancellationToken::new(),
    );
    release.wait();
    worker.join().unwrap();
    assert!(refused);
    transaction.unwrap();
    assert_eq!(query_values(&db, &old), [1]);
    assert_eq!(db.reserved_temp_bytes(), 0);
    drop(Scratch::new(&db, &CancellationToken::new(), &mut Effects::default()).unwrap());
}

#[test]
fn scratch_cancellation_before_growth_preserves_or_settles_namespace_debt() {
    use crate::scratch::Scratch;
    for after_unlink in [false, true] {
        let directory = Directory::new();
        let db = database(&directory);
        let cancel = Rc::new(CancellationToken::new());
        let shared = cancel.clone();
        let mut unlinks = 0;
        let mut effects = Effects::with_faults(Faults {
            after_action: Some(Box::new(move |_, effect| {
                if effect == Effect::Load(crate::effects::LoadEffect::RemoveStaging) {
                    unlinks += 1;
                }
                if (!after_unlink
                    && effect == Effect::Load(crate::effects::LoadEffect::CreateStaging))
                    || (after_unlink && unlinks == 2)
                {
                    shared.cancel();
                }
            })),
            ..Faults::default()
        });
        let result = Scratch::new(&db, &cancel, &mut effects);
        if after_unlink {
            assert!(matches!(result, Err(Error::Cancelled)));
        } else {
            assert!(matches!(result, Err(Error::RecoveryRequired { .. })));
        }
        assert_eq!(db.reserved_temp_bytes(), 0);
        let healthy = CancellationToken::new();
        let retry = Scratch::new(&db, &healthy, &mut Effects::default());
        if after_unlink {
            drop(retry.unwrap());
        } else {
            assert!(matches!(retry, Err(Error::RecoveryRequired { .. })));
        }
        // No I/O is permitted for an already cancelled admission.
        let mut stopped = Effects::default();
        assert!(matches!(
            Scratch::new(&db, &cancel, &mut stopped),
            Err(Error::Cancelled)
        ));
        assert_eq!(stopped.count(), 0);
    }
}

#[test]
fn scratch_path_admission_precedes_io_and_unwind_retains_debt() {
    use crate::scratch::Scratch;
    let directory = Directory::new();
    let db = database(&directory);
    let baseline = db.reserved_memory_bytes();
    let need = (2 * crate::path::MAX_PATH_BYTES) as u64;
    let held = db
        .memory
        .reserve(db.memory.limit() - baseline - need, "test pressure")
        .unwrap();
    drop(Scratch::new(&db, &CancellationToken::new(), &mut Effects::default()).unwrap());
    let one_byte = db.memory.reserve(1, "test pressure").unwrap();
    let mut refused = Effects::default();
    assert!(matches!(
        Scratch::new(&db, &CancellationToken::new(), &mut refused),
        Err(Error::Resource { .. })
    ));
    assert_eq!(refused.count(), 0);
    drop(one_byte);
    drop(held);
    let mut effects = Effects::with_faults(Faults {
        after_action: Some(Box::new(|_, effect| {
            if effect == Effect::Load(crate::effects::LoadEffect::CreateStaging) {
                panic!("scratch unwind fixture");
            }
        })),
        ..Faults::default()
    });
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = Scratch::new(&db, &CancellationToken::new(), &mut effects);
        }))
        .is_err()
    );
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert!(matches!(
        Scratch::new(&db, &CancellationToken::new(), &mut Effects::default()),
        Err(Error::RecoveryRequired { .. })
    ));
    db.close().unwrap();
    let reopened = Database::open(
        &directory.0,
        crate::Config::new(2_000_000, 2_000_000).unwrap(),
    )
    .unwrap();
    drop(
        Scratch::new(
            &reopened,
            &CancellationToken::new(),
            &mut Effects::default(),
        )
        .unwrap(),
    );
}

#[test]
fn scratch_recovery_rejects_unknown_nonempty_and_aliased_names() {
    for invalid in ["unknown-name", "nonempty-file", "hard-link", "directory"] {
        let directory = Directory::new();
        let db = database(&directory);
        append(&db, 1);
        db.close().unwrap();
        let private = directory.0.join(crate::namespace::PRIVATE_NAME);
        let name = if invalid == "unknown-name" {
            "unknown"
        } else {
            crate::namespace::CATALOG_SCRATCH_NAMES[0]
        };
        let path = private.join(name);
        if invalid == "directory" {
            std::fs::create_dir(&path).unwrap();
        } else {
            std::fs::write(
                &path,
                if invalid == "nonempty-file" {
                    b"invalid".as_slice()
                } else {
                    b"".as_slice()
                },
            )
            .unwrap();
        }
        if invalid == "hard-link" {
            std::fs::hard_link(
                &path,
                private.join(crate::namespace::CATALOG_SCRATCH_NAMES[1]),
            )
            .unwrap();
        }
        assert!(
            Database::open(
                &directory.0,
                crate::Config::new(2_000_000, 2_000_000).unwrap()
            )
            .is_err(),
            "scratch recovery accepted {invalid}"
        );
        assert!(path.exists(), "scratch recovery removed {invalid}");
    }
}

#[test]
fn scratch_debris_does_not_authorize_cleanup_of_a_corrupt_graph() {
    let directory = Directory::new();
    let db = database(&directory);
    append(&db, 1);
    db.close().unwrap();
    let pending = directory
        .0
        .join(crate::namespace::PRIVATE_NAME)
        .join(crate::namespace::CATALOG_SCRATCH_NAMES[0]);
    std::fs::write(&pending, []).unwrap();
    let unit = directory
        .0
        .join(UNITS_NAME)
        .join(std::str::from_utf8(&id(2, 1).name()).unwrap());
    let original = std::fs::read(&unit).unwrap();
    let mut corrupt = original.clone();
    corrupt[0] ^= 1;
    std::fs::write(&unit, corrupt).unwrap();
    assert!(
        Database::open(
            &directory.0,
            crate::Config::new(2_000_000, 2_000_000).unwrap()
        )
        .is_err()
    );
    assert!(pending.exists());
    std::fs::write(&unit, original).unwrap();
    let reopened = Database::open(
        &directory.0,
        crate::Config::new(2_000_000, 2_000_000).unwrap(),
    )
    .unwrap();
    assert!(!pending.exists());
    assert_eq!(
        query_values(&reopened, &reopened.prepare("FROM facts").unwrap()),
        [1]
    );
}
