//! Check snapshot lifetimes and transaction resolution during catalog changes.
//!
//! Old readers keep their generation while a writer publishes a newer one.
//! A real reader thread holds a snapshot across commit; effect callbacks acquire
//! additional snapshots during I/O to detect a registry lock held too long.
//!
//! Publication and issuance failures have different outcomes. After reopen, a
//! failed commit can resolve as durable or aborted. Failed issuance can leave an
//! unknown or aborted token, but never a successful receipt. Each campaign uses
//! recorded effects and requires both permitted outcomes to occur.
//!
//! Other tests exhaust generation slots, reader pins, lookup memory and writer
//! construction space. Existing snapshots must survive writer abandonment, while
//! new operations require recovery. Repeated lookup failures check that pins and
//! scratch allocations are released rather than gradually exhausting capacity.

use super::{Fixture, first_commit, graph, snapshot_rows, token};
use crate::effects::{Effects, Faults};
use crate::storage::recovery::{ROOT_A_NAME, WAL_NAME};
use crate::{CancellationToken, Commit, CommitResolution, Database, Error};
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, PartialEq)]
enum RegistryRelease {
    Normal,
    Poison,
    CoordinatorPanic,
}

// Admission may refuse a busy registry, but an existing owner must wait to
// settle its state. The holder keeps the mutex until the coordinator releases
// it; dropping endpoints during a panic also lets both workers leave.
fn complete_under_registry_contention(
    database: &Database,
    finish: impl FnOnce() -> Result<(), Error> + Send,
    release: RegistryRelease,
) -> Result<(), Error> {
    use std::sync::mpsc::{RecvTimeoutError, sync_channel};
    use std::time::Duration;

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        std::thread::scope(|scope| {
            let (locked, held) = sync_channel(1);
            let (unlock, released) = sync_channel(1);
            let (entering, entered) = sync_channel(1);
            let (completed, result) = sync_channel(1);
            let holder = scope.spawn(move || {
                let _guard = database.registry().unwrap().state.lock().unwrap();
                if locked.send(()).is_ok()
                    && released.recv().is_ok()
                    && release == RegistryRelease::Poison
                {
                    panic!("registry reader failure");
                }
            });
            held.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(matches!(
                database.catalog_snapshot(),
                Err(Error::Contention("catalog registry"))
            ));
            let finisher = scope.spawn(move || {
                entering.send(()).unwrap();
                let outcome = finish();
                // A coordinator panic can drop the receiver after completion
                // starts. Finishing the owner's state transition still matters.
                let _ = completed.send(outcome);
            });
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(
                matches!(
                    result.recv_timeout(Duration::from_millis(200)),
                    Err(RecvTimeoutError::Timeout)
                ),
                "completion returned while the registry mutex was held"
            );
            if release == RegistryRelease::CoordinatorPanic {
                panic!("registry coordinator failure");
            }
            unlock.send(()).unwrap();
            let held = holder.join();
            if release == RegistryRelease::Poison {
                assert_eq!(
                    held.unwrap_err().downcast_ref::<&str>(),
                    Some(&"registry reader failure")
                );
            } else {
                held.unwrap();
            }
            let outcome = result.recv_timeout(Duration::from_secs(5)).unwrap();
            finisher.join().unwrap();
            outcome
        })
    }));
    if release == RegistryRelease::CoordinatorPanic {
        assert_eq!(
            outcome.unwrap_err().downcast_ref::<&str>(),
            Some(&"registry coordinator failure")
        );
        Ok(())
    } else {
        outcome.unwrap()
    }
}

#[test]
fn registry_completion_waits_and_poisoning_preserves_old_views() {
    let case = std::env::var(crate::test_subprocess::CASE).unwrap_or_default();
    if case.is_empty() {
        for case in [
            "writer-release",
            "writer-poison",
            "writer-coordinator-panic",
            "maintenance-release",
            "maintenance-poison",
            "maintenance-coordinator-panic",
        ] {
            crate::test_subprocess::run(
                concat!(
                    module_path!(),
                    "::registry_completion_waits_and_poisoning_preserves_old_views"
                ),
                case,
                true,
                "registry completion settled; old view and reopen passed",
            );
        }
        return;
    }
    let (writer, release) = match case.as_str() {
        "writer-release" => (true, RegistryRelease::Normal),
        "writer-poison" => (true, RegistryRelease::Poison),
        "writer-coordinator-panic" => (true, RegistryRelease::CoordinatorPanic),
        "maintenance-release" => (false, RegistryRelease::Normal),
        "maintenance-poison" => (false, RegistryRelease::Poison),
        "maintenance-coordinator-panic" => (false, RegistryRelease::CoordinatorPanic),
        _ => panic!("unknown registry completion schedule"),
    };
    let fixture = Fixture::new();
    first_commit(&fixture);
    let config = crate::Config::new(2_000_000, 1_000_000).unwrap();
    let database = Database::open(fixture.root(), config).unwrap();
    let resident = database.reserved_memory_bytes();
    let old = database.catalog_snapshot().unwrap();
    assert_eq!(old.generation(), 1);
    let outcome = if writer {
        let mut owner = database.catalog_writer().unwrap();
        // No files are created: this charge isolates abort settlement from I/O.
        owner.reserve_construction(512).unwrap();
        complete_under_registry_contention(&database, move || owner.abort_unbuilt(), release)
    } else {
        let owner = database.catalog_maintenance().unwrap();
        complete_under_registry_contention(&database, move || owner.finish(), release)
    };
    if release == RegistryRelease::Poison {
        assert!(matches!(
            outcome,
            Err(Error::RecoveryRequired { generation: 1, .. })
        ));
        assert!(matches!(
            database.catalog_snapshot(),
            Err(Error::RecoveryRequired { generation: 1, .. })
        ));
        assert!(matches!(
            database.catalog_writer(),
            Err(Error::RecoveryRequired { generation: 1, .. })
        ));
        assert_eq!(database.reserved_temp_bytes(), if writer { 512 } else { 0 });
    } else {
        outcome.unwrap();
        database.catalog_writer().unwrap().abort_unbuilt().unwrap();
        database.catalog_maintenance().unwrap().finish().unwrap();
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
    assert_eq!(database.reserved_memory_bytes(), resident);
    assert_eq!(database.generation(), 1);
    assert_eq!(snapshot_rows(&old, &fixture.objects()), 4);
    drop(old);
    database.close().unwrap();
    let database = Database::open(fixture.root(), config).unwrap();
    assert_eq!(database.reserved_temp_bytes(), 0);
    let snapshot = database.catalog_snapshot().unwrap();
    assert_eq!(snapshot.generation(), 1);
    assert_eq!(snapshot_rows(&snapshot, &fixture.objects()), 4);
    drop(snapshot);
    database.catalog_writer().unwrap().abort_unbuilt().unwrap();
    database.catalog_maintenance().unwrap().finish().unwrap();
    database.close().unwrap();
    println!("registry completion settled; old view and reopen passed");
}

#[test]
fn database_snapshot_survives_real_publication_and_abort_gap() {
    let fixture = Fixture::new();
    let (_, unit) = first_commit(&fixture);
    let database = Arc::new(
        Database::open(
            fixture.root(),
            crate::Config::new(2_000_000, 1_000_000).unwrap(),
        )
        .unwrap(),
    );
    let mut aborted = database.catalog_writer().unwrap();
    assert_eq!(
        aborted
            .issue(&CancellationToken::new(), &mut Effects::default())
            .unwrap(),
        token(4)
    );
    assert!(matches!(
        database.resolve_commit(token(4)),
        Err(Error::Contention("transaction is active"))
    ));
    aborted.abort_unbuilt().unwrap();
    assert_eq!(
        database.resolve_commit(token(4)).unwrap(),
        CommitResolution::Aborted
    );
    // Reserve the writer's destination slot before any reader pins the current
    // generation. A reader arriving afterward must still retain the old view.
    let mut writer = database.catalog_writer().unwrap();
    // The fixture writes the objects directly. This reservation checks that
    // committing releases the construction charge; it does not size a real write.
    writer.reserve_construction(1_000_000).unwrap();
    let old = database.catalog_snapshot().unwrap();
    assert!(matches!(
        database.catalog_writer(),
        Err(Error::Contention("catalog writer"))
    ));
    let observer = database.clone();
    let calls = Arc::new(AtomicU64::new(0));
    let observed = calls.clone();
    let mut effects = Effects::with_faults(Faults {
        action: Some(Box::new(move |_, _| {
            let snapshot = observer
                .catalog_snapshot()
                .expect("no registry lock across I/O");
            assert_eq!(snapshot.generation(), 1);
            observed.fetch_add(1, Ordering::Relaxed);
        })),
        ..Faults::default()
    });
    assert_eq!(
        writer
            .issue(&CancellationToken::new(), &mut effects)
            .unwrap(),
        token(5)
    );
    let (next, _) = fixture.prepare(writer.issued_record().unwrap(), Some(unit));
    let committed = std::thread::scope(|scope| {
        let (ready, ready_rx) = std::sync::mpsc::channel();
        let (finished, finished_rx) = std::sync::mpsc::channel();
        let objects = fixture.objects();
        let reader_database = &database;
        let reader = scope.spawn(move || {
            let snapshot = reader_database.catalog_snapshot().unwrap();
            ready.send(()).unwrap();
            finished_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            assert_eq!(
                (snapshot.generation(), snapshot_rows(&snapshot, &objects)),
                (1, 4)
            );
        });
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        let result = writer.commit_prepared(graph(next), &CancellationToken::new(), &mut effects);
        finished.send(()).unwrap();
        reader.join().unwrap();
        result.unwrap()
    });
    assert!(calls.load(Ordering::Relaxed) > 0);
    assert_eq!(database.reserved_temp_bytes(), 0);
    drop(effects);
    let new = database.catalog_snapshot().unwrap();
    assert_eq!(
        (old.generation(), snapshot_rows(&old, &fixture.objects())),
        (1, 4)
    );
    assert_eq!(
        (new.generation(), snapshot_rows(&new, &fixture.objects())),
        (2, 8)
    );
    assert_eq!(database.generation(), 2);
    assert_eq!(
        database.resolve_commit(token(5)).unwrap(),
        CommitResolution::Durable(committed)
    );
    assert_eq!(
        database.resolve_commit(token(3)).unwrap(),
        CommitResolution::Durable(Commit {
            transaction: token(3),
            generation: 1
        })
    );
    assert_eq!(
        database.resolve_commit(token(4)).unwrap(),
        CommitResolution::Aborted
    );
    assert_eq!(
        database.reserved_memory_bytes(),
        crate::storage::snapshot::REGISTRY_BYTES + database.path_memory_bytes()
    );
    drop(old);
    drop(new);
    Arc::try_unwrap(database).ok().unwrap().close().unwrap();
}

#[test]
fn database_snapshot_capacity_and_abandoned_writer_preserve_old_views() {
    let fixture = Fixture::new();
    let (_, unit) = first_commit(&fixture);
    let database = Database::open(
        fixture.root(),
        crate::Config::new(2_000_000, 1_000_000).unwrap(),
    )
    .unwrap();
    let mut pins = vec![database.catalog_snapshot().unwrap()];
    for _ in 1..crate::storage::snapshot::SLOTS {
        let mut writer = database.catalog_writer().unwrap();
        writer
            .issue(&CancellationToken::new(), &mut Effects::default())
            .unwrap();
        let (next, _) = fixture.prepare(writer.issued_record().unwrap(), Some(unit));
        writer
            .commit_prepared(
                graph(next),
                &CancellationToken::new(),
                &mut Effects::default(),
            )
            .unwrap();
        pins.push(database.catalog_snapshot().unwrap());
    }
    let before = fixture.selected();
    assert!(matches!(
        database.catalog_writer(),
        Err(Error::Resource {
            owner: "catalog snapshot slots",
            ..
        })
    ));
    assert_eq!(fixture.selected(), before);
    let current = pins.last().unwrap();
    // Cloning a view consumes another pin but no additional generation slot.
    let clones: Vec<_> = (1..crate::storage::snapshot::MAX_PINS)
        .map(|_| current.try_clone().unwrap())
        .collect();
    assert!(matches!(
        current.try_clone(),
        Err(Error::Resource {
            owner: "catalog view pins",
            ..
        })
    ));
    drop(clones);
    drop(pins.remove(0));
    let writer = database.catalog_writer().unwrap();
    drop(writer);
    assert!(matches!(
        database.catalog_snapshot(),
        Err(Error::RecoveryRequired { .. })
    ));
    assert!(matches!(
        database.resolve_commit(token(3)),
        Err(Error::RecoveryRequired { .. })
    ));
    let clone = pins[0].try_clone().unwrap();
    assert_eq!(snapshot_rows(&clone, &fixture.objects()), 8);
    drop(clone);
    drop(pins);
    database.close().unwrap();
    let reopened = Database::open(
        fixture.root(),
        crate::Config::new(2_000_000, 1_000_000).unwrap(),
    )
    .unwrap();
    assert_eq!(
        reopened.generation(),
        crate::storage::snapshot::SLOTS as u64
    );
    reopened.close().unwrap();
}

#[test]
fn catalog_writer_failure_reopens_to_an_honest_receipt() {
    let config = crate::Config::new(2_000_000, 1_000_000).unwrap();
    let baseline = Fixture::new();
    let (_, unit) = first_commit(&baseline);
    let database = Database::open(baseline.root(), config).unwrap();
    let mut writer = database.catalog_writer().unwrap();
    writer
        .issue(&CancellationToken::new(), &mut Effects::default())
        .unwrap();
    let (next, _) = baseline.prepare(writer.issued_record().unwrap(), Some(unit));
    let mut effects = Effects::default();
    writer
        .commit_prepared(graph(next), &CancellationToken::new(), &mut effects)
        .unwrap();
    let cuts = effects.count();
    assert!(cuts > 0 && cuts < 128);
    println!("catalog_commit_effects={cuts}");
    database.close().unwrap();
    // Fresh fixtures isolate the effects of each injected failure. Reopen,
    // rather than the failed writer, decides whether the new commit survived.
    let mut old_seen = false;
    let mut new_seen = false;
    for cut in 0..cuts {
        let fixture = Fixture::new();
        let (_, unit) = first_commit(&fixture);
        let database = Database::open(fixture.root(), config).unwrap();
        let old = database.catalog_snapshot().unwrap();
        let mut writer = database.catalog_writer().unwrap();
        assert_eq!(
            writer
                .issue(&CancellationToken::new(), &mut Effects::default())
                .unwrap(),
            token(4)
        );
        let (next, _) = fixture.prepare(writer.issued_record().unwrap(), Some(unit));
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(cut),
            ..Faults::default()
        });
        assert!(
            writer
                .commit_prepared(graph(next), &CancellationToken::new(), &mut effects)
                .is_err(),
            "cut {cut}"
        );
        assert!(
            matches!(
                database.resolve_commit(token(4)),
                Err(Error::RecoveryRequired { .. })
            ),
            "cut {cut}"
        );
        assert_eq!(snapshot_rows(&old, &fixture.objects()), 4);
        assert_eq!(
            database.reserved_memory_bytes(),
            crate::storage::snapshot::REGISTRY_BYTES + database.path_memory_bytes()
        );
        drop(old);
        database.close().unwrap();
        let reopened = Database::open(fixture.root(), config).unwrap();
        assert_eq!(
            reopened.resolve_commit(token(3)).unwrap(),
            CommitResolution::Durable(Commit {
                transaction: token(3),
                generation: 1
            })
        );
        match reopened.resolve_commit(token(4)).unwrap() {
            CommitResolution::Aborted => {
                old_seen = true;
                assert_eq!(reopened.generation(), 1);
            }
            CommitResolution::Durable(commit) => {
                new_seen = true;
                assert_eq!(commit.generation(), 2);
                assert_eq!(reopened.generation(), 2);
            }
        }
        reopened.close().unwrap();
    }
    assert!(old_seen && new_seen);
}

#[test]
fn catalog_resolution_failures_release_index_pins_and_scratch() {
    let fixture = Fixture::new();
    first_commit(&fixture);
    let database = Database::open(
        fixture.root(),
        crate::Config::new(2_000_000, 1_000_000).unwrap(),
    )
    .unwrap();
    for _ in 0..crate::storage::snapshot::MAX_PINS * 2 {
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(0),
            ..Faults::default()
        });
        assert!(matches!(
            database.resolve_commit_with_effects(token(3), &mut effects),
            Err(Error::Io { .. })
        ));
        assert_eq!(
            database.reserved_memory_bytes(),
            crate::storage::snapshot::REGISTRY_BYTES + database.path_memory_bytes()
        );
    }
    assert_eq!(
        database.resolve_commit(token(3)).unwrap(),
        CommitResolution::Durable(Commit {
            transaction: token(3),
            generation: 1
        })
    );
    let mut writer = database.catalog_writer().unwrap();
    writer
        .issue(&CancellationToken::new(), &mut Effects::default())
        .unwrap();
    for _ in 0..crate::storage::snapshot::MAX_PINS * 2 {
        assert!(matches!(
            database.resolve_commit(token(4)),
            Err(Error::Contention("transaction is active"))
        ));
    }
    writer.abort_unbuilt().unwrap();
    assert_eq!(
        database.resolve_commit(token(4)).unwrap(),
        CommitResolution::Aborted
    );
    database.close().unwrap();
}

#[test]
fn catalog_issuance_failure_never_becomes_a_successful_receipt() {
    let config = crate::Config::new(2_000_000, 1_000_000).unwrap();
    let baseline = Fixture::new();
    first_commit(&baseline);
    let database = Database::open(baseline.root(), config).unwrap();
    let mut writer = database.catalog_writer().unwrap();
    let mut effects = Effects::default();
    writer
        .issue(&CancellationToken::new(), &mut effects)
        .unwrap();
    let cuts = effects.count();
    assert!(cuts > 0 && cuts < 128);
    println!("catalog_issuance_effects={cuts}");
    writer.abort_unbuilt().unwrap();
    database.close().unwrap();
    let mut unissued_seen = false;
    let mut aborted_seen = false;
    for cut in 0..cuts {
        let fixture = Fixture::new();
        first_commit(&fixture);
        let database = Database::open(fixture.root(), config).unwrap();
        let old = database.catalog_snapshot().unwrap();
        let mut writer = database.catalog_writer().unwrap();
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(cut),
            ..Faults::default()
        });
        assert!(
            writer
                .issue(&CancellationToken::new(), &mut effects)
                .is_err(),
            "cut {cut}"
        );
        drop(writer);
        assert!(matches!(
            database.resolve_commit(token(4)),
            Err(Error::RecoveryRequired { .. })
        ));
        assert_eq!(snapshot_rows(&old, &fixture.objects()), 4);
        drop(old);
        database.close().unwrap();
        let reopened = Database::open(fixture.root(), config).unwrap();
        match reopened.resolve_commit(token(4)) {
            Err(Error::NotFound) => unissued_seen = true,
            Ok(CommitResolution::Aborted) => aborted_seen = true,
            other => panic!("unexpected issuance outcome: {other:?}"),
        }
        assert_eq!(reopened.generation(), 1);
        reopened.close().unwrap();
    }
    assert!(unissued_seen && aborted_seen);
}

#[test]
fn catalog_writer_checks_held_lease_before_publication() {
    let fixture = Fixture::new();
    first_commit(&fixture);
    let database = Database::open(
        fixture.root(),
        crate::Config::new(2_000_000, 1_000_000).unwrap(),
    )
    .unwrap();
    let mut writer = database.catalog_writer().unwrap();
    let root = fs::read(fixture.root().join(ROOT_A_NAME)).unwrap();
    let wal = fs::read(fixture.root().join(WAL_NAME)).unwrap();
    // Keep the renamed file under Fixture ownership so a failed assertion
    // cannot leave the held inode outside the directory's cleanup boundary.
    let original_lock = fixture.root().join("held-lock");
    fs::rename(
        fixture.root().join(crate::storage::recovery::LOCK_NAME),
        &original_lock,
    )
    .unwrap();
    fs::write(fixture.root().join(crate::storage::recovery::LOCK_NAME), []).unwrap();
    assert!(matches!(
        writer.issue(&CancellationToken::new(), &mut Effects::default()),
        Err(Error::RecoveryRequired { .. })
    ));
    assert_eq!(fs::read(fixture.root().join(ROOT_A_NAME)).unwrap(), root);
    assert_eq!(fs::read(fixture.root().join(WAL_NAME)).unwrap(), wal);
    fs::remove_file(fixture.root().join(crate::storage::recovery::LOCK_NAME)).unwrap();
    fs::rename(
        original_lock,
        fixture.root().join(crate::storage::recovery::LOCK_NAME),
    )
    .unwrap();
    drop(writer);
    database.close().unwrap();
}

#[test]
fn catalog_registry_and_lookup_share_the_database_memory_limit() {
    let fixture = Fixture::new();
    first_commit(&fixture);
    let database = Database::open(
        fixture.root(),
        crate::Config::new(2_000_000, 1_000_000).unwrap(),
    )
    .unwrap();
    let path_bytes = database.path_memory_bytes();
    database.close().unwrap();
    let database = Database::open(
        fixture.root(),
        crate::Config::new(
            path_bytes
                + if cfg!(target_os = "macos") {
                    131_072
                } else {
                    65_536
                },
            1_000_000,
        )
        .unwrap(),
    )
    .unwrap();
    for _ in 0..crate::storage::snapshot::MAX_PINS * 2 {
        assert!(matches!(
            database.resolve_commit(token(3)),
            Err(Error::Resource {
                owner: "catalog resolution scratch",
                ..
            })
        ));
        assert_eq!(
            database.reserved_memory_bytes(),
            crate::storage::snapshot::REGISTRY_BYTES + database.path_memory_bytes()
        );
    }
    database.close().unwrap();
}

#[test]
fn catalog_construction_charge_does_not_invalidate_settled_receipts() {
    let fixture = Fixture::new();
    first_commit(&fixture);
    let database =
        Database::open(fixture.root(), crate::Config::new(2_000_000, 100).unwrap()).unwrap();
    let before = fs::read(fixture.root().join(WAL_NAME)).unwrap();
    let mut writer = database.catalog_writer().unwrap();
    assert!(matches!(
        writer.reserve_construction(101),
        Err(Error::Resource { .. })
    ));
    assert_eq!(database.reserved_temp_bytes(), 0);
    assert_eq!(fs::read(fixture.root().join(WAL_NAME)).unwrap(), before);
    writer.reserve_construction(100).unwrap();
    assert_eq!(database.reserved_temp_bytes(), 100);
    assert_eq!(
        database.resolve_commit(token(3)).unwrap(),
        CommitResolution::Durable(Commit {
            transaction: token(3),
            generation: 1,
        })
    );
    writer
        .issue(&CancellationToken::new(), &mut Effects::default())
        .unwrap();
    assert!(matches!(
        database.resolve_commit(token(4)),
        Err(Error::Contention("transaction is active"))
    ));
    writer.abort_unbuilt().unwrap();
    assert_eq!(database.reserved_temp_bytes(), 0);
    assert_eq!(
        database.resolve_commit(token(4)).unwrap(),
        CommitResolution::Aborted
    );
    let mut writer = database.catalog_writer().unwrap();
    writer.reserve_construction(100).unwrap();
    // Abandonment cannot certify that construction left no files. Keep the
    // temporary charge and refuse another writer until recovery.
    drop(writer);
    assert_eq!(database.reserved_temp_bytes(), 100);
    assert!(matches!(
        database.catalog_writer(),
        Err(Error::RecoveryRequired { .. })
    ));
}
