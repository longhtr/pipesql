//! Exercise deletion and recovery while readers retain old catalog versions.
//!
//! Reclamation runs against real database files. Tests fail each operation in a
//! healthy trace, then check old query results and committed transaction receipts.
//! Cancellation after a deletion must still synchronize the directory; a failed
//! synchronization requires reopen.
//!
//! A separate reader thread runs while reclamation is paused after an unlink.
//! Subprocess cases exit at selected file-operation boundaries and reopen the
//! database without running destructors. The operating system stays alive, so
//! these cases test process interruption, not power loss.

use super::{Directory, append, database, query_values, result_values};
use crate::effects::{DirectoryKind, Effect, Effects, Faults};
use crate::{CancellationToken, CommitResolution, Database, Error};
use std::{cell::RefCell, path::Path, rc::Rc};

#[test]
fn active_receipt_lookup_survives_real_reclamation_after_publication() {
    let directory = Directory::new();
    let db = Rc::new(database(&directory));
    let first = append(&db, 1);
    let shared = db.clone();
    let mut fired = false;
    let mut effects = Effects::with_faults(Faults {
        action: Some(Box::new(move |_, effect| {
            if !fired && effect == Effect::ReadMetadata(crate::effects::MetadataKind::CatalogObject)
            {
                fired = true;
                append(&shared, 2);
                assert_eq!(shared.reclaim(&CancellationToken::new()).unwrap(), 4);
            }
        })),
        ..Faults::default()
    });
    assert_eq!(
        db.resolve_catalog(first.transaction(), &mut effects)
            .unwrap(),
        CommitResolution::Durable(first)
    );
    drop(effects);
    assert_eq!(db.reclaim(&CancellationToken::new()).unwrap(), 1);
    assert_eq!(
        query_values(&db, &db.prepare("FROM facts").unwrap()),
        [1, 2]
    );
}

#[test]
fn every_cleanup_effect_failure_preserves_live_reads_and_heals_on_reopen() {
    let control = Directory::new();
    let db = database(&control);
    append(&db, 1);
    let old = db.prepare("FROM facts").unwrap();
    append(&db, 2);
    let mut trace = Effects::default();
    assert_eq!(
        db.reclaim_with_effects(&CancellationToken::new(), &mut trace)
            .unwrap(),
        3
    );
    drop(old);
    drop(db);
    for cut in 0..trace.count() {
        let directory = Directory::new();
        let db = database(&directory);
        let first = append(&db, 1);
        let old = db.prepare("FROM facts").unwrap();
        let second = append(&db, 2);
        let cancel = CancellationToken::new();
        let mut live = db.execute(&old, &cancel).unwrap();
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(cut),
            ..Faults::default()
        });
        assert!(
            db.reclaim_with_effects(&cancel, &mut effects).is_err(),
            "cleanup cut {cut}"
        );
        assert_eq!(result_values(&mut live), [1], "live query at cut {cut}");
        assert_eq!(db.reserved_temp_bytes(), 0);
        drop(live);
        drop(old);
        db.close().unwrap();
        let reopened = Database::open(
            &directory.0.join("database"),
            crate::Config::new(2_000_000, 2_000_000).unwrap(),
        )
        .unwrap();
        assert_eq!(
            reopened.resolve_commit(first.transaction()).unwrap(),
            CommitResolution::Durable(first)
        );
        assert_eq!(
            reopened.resolve_commit(second.transaction()).unwrap(),
            CommitResolution::Durable(second)
        );
        reopened.reclaim(&cancel).unwrap();
        let third = append(&reopened, 3);
        assert_eq!(
            third.transaction().sequence(),
            second.transaction().sequence() + 1
        );
        assert_eq!(
            query_values(&reopened, &reopened.prepare("FROM facts").unwrap()),
            [1, 2, 3]
        );
        reopened.close().unwrap();
    }
    println!("public_cleanup_effect_cuts={}", trace.count());
}

#[test]
fn cancellation_after_unlink_syncs_cleanup_and_allows_another_writer() {
    let mut barrier = Vec::new();
    for refusal in [None, Some(0), Some(1)] {
        let directory = Directory::new();
        let db = database(&directory);
        let first = append(&db, 1);
        let old = db.prepare("FROM facts").unwrap();
        let second = append(&db, 2);
        let generation = db.generation();
        let resident = db.reserved_memory_bytes();
        let reading = CancellationToken::new();
        let mut live = db.execute(&old, &reading).unwrap();
        let cancel = Rc::new(CancellationToken::new());
        let requested = cancel.clone();
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = events.clone();
        let removals = Rc::new(std::cell::Cell::new(0));
        let removed = removals.clone();
        // Observe the barrier call boundary and inject refusals there. Native
        // synchronization mapping is checked by the separate syscall campaign.
        let mut effects = Effects::with_faults(Faults {
            fail_at: refusal.map(|index| barrier[index]),
            action: Some(Box::new(move |index, effect| {
                captured.borrow_mut().push((index, effect));
            })),
            after_action: Some(Box::new(move |_, effect| {
                if effect == Effect::RemoveCleanupFile {
                    removed.set(removed.get() + 1);
                    requested.cancel();
                }
            })),
            ..Faults::default()
        });
        let outcome = db.reclaim_with_effects(&cancel, &mut effects);
        assert!(cancel.is_cancelled());
        assert_eq!(removals.get(), 1, "cancel after one successful unlink");
        let events = events.borrow();
        let unlink = events
            .iter()
            .position(|(_, effect)| *effect == Effect::RemoveCleanupFile)
            .unwrap();
        let tail = &events[unlink + 1..];
        let expected = [
            Effect::OpenDirectory(DirectoryKind::Units),
            Effect::SyncDirectory(DirectoryKind::Units),
        ];
        let count = if refusal == Some(0) { 1 } else { 2 };
        assert_eq!(
            tail.iter().map(|(_, effect)| *effect).collect::<Vec<_>>(),
            expected[..count]
        );
        if let Some(index) = refusal {
            let Err(Error::RecoveryRequired {
                generation: actual,
                source,
            }) = outcome
            else {
                panic!("barrier failure must override cancellation: {outcome:?}")
            };
            assert_eq!(actual, generation);
            assert!(
                matches!(source.kind(), crate::CauseKind::Io { operation, source } if *operation == expected[index].name() && source.kind() == std::io::ErrorKind::Other)
            );
            assert!(db.needs_reopen());
            assert!(matches!(
                db.catalog_writer(),
                Err(Error::RecoveryRequired { .. })
            ));
        } else {
            barrier = tail.iter().map(|(index, _)| *index).collect();
            assert!(matches!(outcome, Err(Error::Cancelled)));
            assert!(!db.needs_reopen());
        }
        drop(events);
        assert_eq!(db.generation(), generation);
        assert_eq!(db.reserved_temp_bytes(), 0);
        // This execution was admitted before maintenance. A failed barrier must
        // not destroy its protected old files, even while new writes are blocked.
        assert_eq!(result_values(&mut live), [1]);
        drop(live);
        assert_eq!(db.reserved_memory_bytes(), resident);
        let third = refusal.is_none().then(|| append(&db, 3));
        drop(old);
        db.close().unwrap();
        let reopened = Database::open(
            &directory.0.join("database"),
            crate::Config::new(2_000_000, 2_000_000).unwrap(),
        )
        .unwrap();
        for commit in [first, second] {
            assert_eq!(
                reopened.resolve_commit(commit.transaction()).unwrap(),
                CommitResolution::Durable(commit)
            );
        }
        assert_eq!(
            query_values(&reopened, &reopened.prepare("FROM facts").unwrap()),
            if third.is_some() {
                vec![1, 2, 3]
            } else {
                vec![1, 2]
            }
        );
        let third = third.unwrap_or_else(|| append(&reopened, 3));
        assert_eq!(
            reopened.resolve_commit(third.transaction()).unwrap(),
            CommitResolution::Durable(third)
        );
        assert_eq!(
            query_values(&reopened, &reopened.prepare("FROM facts").unwrap()),
            [1, 2, 3]
        );
        assert_eq!(reopened.reserved_temp_bytes(), 0);
        reopened.close().unwrap();
    }
}

#[test]
fn failed_unlink_followed_by_failed_sync_requires_reopen() {
    let control = Directory::new();
    let db = database(&control);
    append(&db, 1);
    append(&db, 2);
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    let mut effects = Effects::with_faults(Faults {
        action: Some(Box::new(move |index, effect| {
            captured.borrow_mut().push((index, effect))
        })),
        ..Faults::default()
    });
    db.reclaim_with_effects(&CancellationToken::new(), &mut effects)
        .unwrap();
    let first_unlink = events
        .borrow()
        .iter()
        .find(|(_, e)| *e == Effect::RemoveCleanupFile)
        .unwrap()
        .0;
    drop(db);
    // After the injected unlink failure, the next effects open and synchronize
    // the directory. Fail the sync too to distinguish an I/O error from a
    // cleanup result that cannot be considered durable.
    for second in [None, Some(first_unlink + 2)] {
        let directory = Directory::new();
        let db = database(&directory);
        append(&db, 1);
        append(&db, 2);
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(first_unlink),
            second_fail_at: second,
            ..Faults::default()
        });
        let result = db.reclaim_with_effects(&CancellationToken::new(), &mut effects);
        assert!(result.is_err());
        assert_eq!(db.needs_reopen(), second.is_some());
        if second.is_some() {
            assert!(matches!(result, Err(Error::RecoveryRequired { .. })));
        }
        db.close().unwrap();
        let reopened = Database::open(
            &directory.0.join("database"),
            crate::Config::new(2_000_000, 2_000_000).unwrap(),
        )
        .unwrap();
        assert_eq!(
            query_values(&reopened, &reopened.prepare("FROM facts").unwrap()),
            [1, 2]
        );
        assert_eq!(reopened.reclaim(&CancellationToken::new()).unwrap(), 5);
    }
}

#[test]
fn process_death_during_scratch_creation_or_unlink_heals_on_reopen() {
    const DIRECTORY: &str = "PIPESQL_RECLAIM_DEATH_DIRECTORY";
    const STAGE: &str = "PIPESQL_RECLAIM_DEATH_STAGE";
    if let Some(path) = std::env::var_os(DIRECTORY) {
        let db = Database::open(
            Path::new(&path),
            crate::Config::new(2_000_000, 2_000_000).unwrap(),
        )
        .unwrap();
        let _old = db.prepare("FROM facts").unwrap();
        append(&db, 3);
        let stage = std::env::var(STAGE).unwrap();
        let before_stage = stage.clone();
        let mut effects = Effects::with_faults(Faults {
            action: Some(Box::new(move |_, effect| {
                if (before_stage == "scratch-sync"
                    && effect == Effect::SyncDirectory(DirectoryKind::Private))
                    || (before_stage == "scratch-admit"
                        && effect == Effect::Load(crate::effects::LoadEffect::InspectStaging))
                {
                    std::process::exit(73);
                }
            })),
            after_action: Some(Box::new(move |_, effect| {
                if (stage == "scratch"
                    && effect == Effect::Load(crate::effects::LoadEffect::CreateStaging))
                    || (stage == "scratch-unlink"
                        && effect == Effect::Load(crate::effects::LoadEffect::RemoveStaging))
                    || (stage == "unlink" && effect == Effect::RemoveCleanupFile)
                {
                    std::process::exit(73);
                }
            })),
            ..Faults::default()
        });
        db.reclaim_with_effects(&CancellationToken::new(), &mut effects)
            .unwrap();
        panic!("process-death boundary was not reached");
    }
    for stage in [
        "scratch",
        "scratch-unlink",
        "scratch-sync",
        "scratch-admit",
        "unlink",
    ] {
        let directory = Directory::new();
        let db = database(&directory);
        let first = append(&db, 1);
        let second = append(&db, 2);
        db.close().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(
                concat!(
                    module_path!(),
                    "::process_death_during_scratch_creation_or_unlink_heals_on_reopen"
                )
                .strip_prefix("pipesql::")
                .unwrap(),
            )
            .arg("--test-threads=1")
            .env(DIRECTORY, directory.0.join("database"))
            .env(STAGE, stage)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        // A harness that selected no test would also exit successfully. Require
        // the boundary's own exit code before interpreting the reopened files.
        assert_eq!(status.code(), Some(73));
        let reopened = Database::open(
            &directory.0.join("database"),
            crate::Config::new(2_000_000, 2_000_000).unwrap(),
        )
        .unwrap();
        for commit in [first, second] {
            assert_eq!(
                reopened.resolve_commit(commit.transaction()).unwrap(),
                CommitResolution::Durable(commit)
            );
        }
        assert_eq!(
            query_values(&reopened, &reopened.prepare("FROM facts").unwrap()),
            [1, 2, 3]
        );
        reopened.reclaim(&CancellationToken::new()).unwrap();
        append(&reopened, 4);
        assert_eq!(
            query_values(&reopened, &reopened.prepare("FROM facts").unwrap()),
            [1, 2, 3, 4]
        );
    }
}

#[test]
fn real_reader_thread_runs_during_unlink_and_another_writer_is_refused() {
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;
    let directory = Directory::new();
    let db = database(&directory);
    let first = append(&db, 1);
    let old = db.prepare("FROM facts").unwrap();
    append(&db, 2);
    let (ready_send, ready_receive) = sync_channel(1);
    let (done_send, done_receive) = sync_channel(1);
    let mut fired = false;
    let mut effects = Effects::with_faults(Faults {
        after_action: Some(Box::new(move |_, effect| {
            if !fired && effect == Effect::RemoveCleanupFile {
                fired = true;
                ready_send.send(()).unwrap();
                done_receive.recv_timeout(Duration::from_secs(10)).unwrap();
            }
        })),
        ..Faults::default()
    });
    std::thread::scope(|scope| {
        let db = &db;
        let old = &old;
        let reader = scope.spawn(move || {
            ready_receive.recv_timeout(Duration::from_secs(10)).unwrap();
            assert_eq!(query_values(db, old), [1]);
            assert_eq!(
                db.resolve_commit(first.transaction()).unwrap(),
                CommitResolution::Durable(first)
            );
            assert!(matches!(
                db.catalog_writer(),
                Err(Error::Contention("catalog writer"))
            ));
            done_send.send(()).unwrap();
        });
        assert_eq!(
            db.reclaim_with_effects(&CancellationToken::new(), &mut effects)
                .unwrap(),
            3
        );
        reader.join().unwrap();
    });
    drop(old);
    assert_eq!(db.reclaim(&CancellationToken::new()).unwrap(), 2);
    db.close().unwrap();
}
