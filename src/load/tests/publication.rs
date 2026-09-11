use crate::effects::{Effect, Effects, Faults, LoadEffect};
use crate::load::staging::COLUMNS;
use crate::load::tests::{TempDir, config, independent_crc32c, independent_namespace_matches, row};
use crate::namespace::{
    PRIVATE_NAME, ROOT_A_NAME, ROOT_B_NAME, UNIT_NAME, UNITS_NAME, WAL_NAME, inspect_namespace,
};
use crate::publication::{FailureStage, publish_snapshot};
use crate::storage_format::{self, Replica, Root, RootState, WalRecord};
use crate::{CancellationToken, CommitResolution, Database, Error, TransactionId};
use std::fs::{self};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[test]
fn issuance_after_data_uses_issuance_effects_and_preserves_reopened_data() {
    let temp = TempDir::new();
    let root = temp.0.join("db");
    let input = temp.0.join("lineitem.tbl");
    fs::write(&input, row("1", "100", "0.08", "1994-01-01")).unwrap();
    let limits = config(2_000_000, 1_000_000);
    let mut database = Database::create(&root, limits).unwrap();
    let committed = database
        .load_lineitem(&input, &CancellationToken::new())
        .unwrap();
    let before_unit = fs::read(root.join(UNITS_NAME).join(UNIT_NAME)).unwrap();
    let prior = inspect_namespace(
        &root,
        &crate::resources::MemoryAuthority::new(2_000_000),
        &mut Effects::default(),
    )
    .unwrap();
    let issuance = WalRecord {
        database: prior.database_id,
        issued: prior.issued + 1,
        state: prior.state,
    };
    let aborted = TransactionId::for_attempt(prior.database_id, issuance.issued).unwrap();
    let trace = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&trace);
    let mut effects = Effects::with_faults(Faults {
        action: Some(Box::new(move |_, effect| {
            captured.lock().unwrap().push(effect)
        })),
        ..Faults::default()
    });
    // Explicit trusted boundary: public load still refuses a second success.
    // Exercise its real publisher on the next issuance, then settle an abort.
    let previous = WalRecord {
        database: prior.database_id,
        issued: prior.issued,
        state: prior.state,
    };
    publish_snapshot(
        &root,
        previous,
        issuance,
        &CancellationToken::new(),
        &mut effects,
    )
    .unwrap_or_else(|failure| panic!("issuance failed: {:?}", failure.error));
    let trace = trace.lock().unwrap();
    for expected in [
        Effect::Load(LoadEffect::WriteIssuedWal),
        Effect::SyncMetadata(crate::effects::MetadataKind::IssuedWal),
        Effect::SyncMetadata(crate::effects::MetadataKind::IssuedRootA),
        Effect::SyncMetadata(crate::effects::MetadataKind::IssuedRootB),
        Effect::Load(LoadEffect::RenameIssuedRootA),
        Effect::Load(LoadEffect::RenameIssuedRootB),
    ] {
        assert!(
            trace.contains(&expected),
            "missing issuance effect {expected:?}"
        );
    }
    drop(trace);
    database.close().unwrap();
    let reopened = Database::open(&root, limits).unwrap();
    assert_eq!(
        reopened.resolve_commit(committed.transaction()).unwrap(),
        CommitResolution::Durable(committed)
    );
    assert_eq!(
        reopened.resolve_commit(aborted).unwrap(),
        CommitResolution::Aborted
    );
    assert_eq!(
        fs::read(root.join(UNITS_NAME).join(UNIT_NAME)).unwrap(),
        before_unit
    );
    let query = reopened
        .prepare(include_str!("../../../tests/fixtures/q6.pipe.sql"))
        .unwrap();
    let cancellation = CancellationToken::new();
    let mut result = reopened.execute(&query, &cancellation).unwrap();
    let mut rows = 0;
    let mut finished = false;
    for _ in 0..1024 {
        match result.step() {
            crate::QueryStep::Progress => (),
            crate::QueryStep::Rows(batch) => {
                assert_eq!(batch.len(), 1);
                assert_eq!(batch.value(0, 0), Some(crate::Value::Double(8.0)));
                rows += batch.len();
            }
            crate::QueryStep::Finished => {
                finished = true;
                break;
            }
            crate::QueryStep::Failed(error) => panic!("reopened query: {error:?}"),
        }
    }
    assert!(finished);
    assert_eq!(rows, 1);
}

#[test]
fn interrupted_issuance_after_data_preserves_prior_commit_through_reopen() {
    let mut cuts = None;
    let mut unknown = 0;
    let mut aborted = 0;
    // The successful pass discovers the finite real effect sequence; every
    // following run cuts exactly one effect in that sequence.
    for selected in 0..128_u64 {
        if cuts.is_some_and(|count| selected > count) {
            break;
        }
        let temp = TempDir::new();
        let root = temp.0.join("db");
        let input = temp.0.join("lineitem.tbl");
        fs::write(&input, row("1", "100", "0.08", "1994-01-01")).unwrap();
        let limits = config(2_000_000, 1_000_000);
        let mut database = Database::create(&root, limits).unwrap();
        let committed = database
            .load_lineitem(&input, &CancellationToken::new())
            .unwrap();
        let unit_before = fs::read(root.join(UNITS_NAME).join(UNIT_NAME)).unwrap();
        let namespace = inspect_namespace(
            &root,
            &crate::resources::MemoryAuthority::new(2_000_000),
            &mut Effects::default(),
        )
        .unwrap();
        let prior = WalRecord {
            database: namespace.database_id,
            issued: namespace.issued,
            state: namespace.state,
        };
        let next = WalRecord {
            issued: prior.issued + 1,
            ..prior
        };
        let token = TransactionId::for_attempt(prior.database, next.issued).unwrap();
        let mut effects = Effects::with_faults(Faults {
            fail_at: selected.checked_sub(1),
            ..Faults::default()
        });
        let outcome = publish_snapshot(&root, prior, next, &CancellationToken::new(), &mut effects);
        if selected == 0 {
            outcome.unwrap_or_else(|failure| panic!("baseline publication: {:?}", failure.error));
            assert!(effects.count() > 0 && effects.count() < 128);
            cuts = Some(effects.count());
        } else {
            assert!(outcome.is_err(), "cut {} not reached", selected - 1);
        }
        // This private producer boundary does not expose a usable writer
        // after failure. Release its lease and exercise ordinary recovery.
        database.close().unwrap();
        let reopened = Database::open(&root, limits).unwrap();
        assert_eq!(
            reopened.resolve_commit(committed.transaction()).unwrap(),
            CommitResolution::Durable(committed)
        );
        match reopened.resolve_commit(token) {
            Err(Error::NotFound) => unknown += 1,
            Ok(CommitResolution::Aborted) => aborted += 1,
            other => panic!("issuance without data commit: {other:?}"),
        }
        assert_eq!(
            fs::read(root.join(UNITS_NAME).join(UNIT_NAME)).unwrap(),
            unit_before
        );
        assert_eq!(reopened.generation(), 1);
        reopened.close().unwrap();
    }
    assert_eq!(unknown + aborted, cuts.unwrap() + 1);
    assert!(unknown > 0 && aborted > 0);
    eprintln!(
        "loaded issuance: {} effect cuts; {unknown} unissued and {aborted} settled abort observations",
        cuts.unwrap()
    );
}

#[test]
fn invalid_publication_transition_refuses_before_effects() {
    let database = storage_format::DatabaseId::new([7; 16]).unwrap();
    let prior = WalRecord {
        database,
        issued: 1,
        state: RootState::Empty,
    };
    for record in [
        prior,
        WalRecord { issued: 0, ..prior },
        WalRecord { issued: 3, ..prior },
        WalRecord {
            database: storage_format::DatabaseId::new([8; 16]).unwrap(),
            issued: 2,
            ..prior
        },
    ] {
        let mut effects = Effects::default();
        let failure = publish_snapshot(
            Path::new("/not-used-by-transition-check"),
            prior,
            record,
            &CancellationToken::new(),
            &mut effects,
        )
        .unwrap_err();
        assert!(matches!(
            failure.error,
            Error::Corrupt("invalid publication transition")
        ));
        assert_eq!(failure.stage, FailureStage::BeforePublication);
        assert_eq!(effects.count(), 0);
    }
}

#[test]
fn publication_refuses_a_fence_alias_added_after_admission() {
    for selected_open in [1, 2] {
        let temp = TempDir::new();
        let root = temp.0.join("db");
        let input = temp.0.join("lineitem.tbl");
        fs::write(&input, row("1", "2", "0.5", "1970-01-01")).unwrap();
        let limits = config(2_000_000, 1_000_000);
        let mut database = Database::create(&root, limits).unwrap();
        let outside = temp.0.join("outside-canary");
        let expected = temp.0.join("before");
        let canary = outside.clone();
        let saved = expected.clone();
        let wal = root.join(WAL_NAME);
        let mut opens = 0;
        let mut effects = Effects::with_faults(Faults {
            action: Some(Box::new(move |_, effect| {
                if effect == Effect::Load(LoadEffect::OpenWalPublication) {
                    opens += 1;
                    if opens == selected_open {
                        fs::hard_link(&wal, &canary).unwrap();
                        fs::copy(&wal, &saved).unwrap();
                    }
                }
            })),
            ..Faults::default()
        });
        assert!(
            database
                .load_lineitem_with_effects(&input, &CancellationToken::new(), &mut effects)
                .is_err()
        );
        assert_eq!(fs::read(&outside).unwrap(), fs::read(expected).unwrap());
        assert_eq!(
            database.reserved_memory_bytes(),
            database.path_memory_bytes()
        );
        database.close().unwrap();
        fs::remove_file(outside).unwrap();
        let mut reopened = Database::open(&root, limits).unwrap();
        assert_eq!(reopened.generation(), 0);
        reopened
            .load_lineitem(&input, &CancellationToken::new())
            .unwrap();
        assert_eq!(reopened.generation(), 1);
    }
}

#[test]
fn checksummed_unknown_root_fields_do_not_authorize_repair() {
    for (name, generation) in [
        (ROOT_A_NAME, 0_u64),
        (ROOT_A_NAME, 2),
        (WAL_NAME, 0),
        (WAL_NAME, 2),
    ] {
        let temp = TempDir::new();
        let path = temp.0.join("database");
        Database::create(&path, config(2_000_000, 1_000_000))
            .unwrap()
            .close()
            .unwrap();
        let mut bytes = fs::read(path.join(name)).unwrap();
        bytes[40..48].copy_from_slice(&generation.to_le_bytes());
        bytes[120] = 1;
        bytes[108..112].fill(0);
        let checksum = independent_crc32c(&bytes);
        bytes[108..112].copy_from_slice(&checksum.to_le_bytes());
        fs::write(path.join(name), &bytes).unwrap();
        let marker = path.join(PRIVATE_NAME).join(COLUMNS[0].name);
        fs::write(&marker, b"preserve unsupported authority").unwrap();
        assert!(
            matches!(
                Database::open(&path, config(2_000_000, 1_000_000)),
                Err(Error::Corrupt(_) | Error::UnsupportedGeneration(_))
            ),
            "checksummed unknown fields were repaired away"
        );
        assert_eq!(fs::read(path.join(name)).unwrap(), bytes);
        assert!(marker.exists());
    }
}

#[test]
fn exhausted_attempt_prefix_refuses_without_temporary_debt() {
    let temp = TempDir::new();
    let path = temp.0.join("database");
    let input = temp.0.join("input.tbl");
    fs::write(&input, row("1", "100", "0.08", "1994-01-01")).unwrap();
    let database = Database::create(&path, config(2_000_000, 1_000_000)).unwrap();
    let id = database.database_identity();
    database.close().unwrap();
    for (name, replica) in [(ROOT_A_NAME, Replica::A), (ROOT_B_NAME, Replica::B)] {
        fs::write(
            path.join(name),
            storage_format::encode_root(Root {
                database: id,
                issued: u64::MAX,
                replica,
                state: RootState::Empty,
            })
            .unwrap(),
        )
        .unwrap();
    }
    fs::write(
        path.join(WAL_NAME),
        storage_format::encode_wal(WalRecord {
            database: id,
            issued: u64::MAX,
            state: RootState::Empty,
        })
        .unwrap(),
    )
    .unwrap();
    let mut database = Database::open(&path, config(2_000_000, 1_000_000)).unwrap();
    let result = database.load_lineitem(&input, &CancellationToken::new());
    assert!(matches!(
        result,
        Err(Error::Unsupported("attempt identity capacity exhausted"))
    ));
    assert_eq!(
        database.reserved_memory_bytes(),
        database.path_memory_bytes()
    );
    assert_eq!(database.reserved_temp_bytes(), 0);
    assert!(!database.needs_reopen());
    independent_namespace_matches(&path, 0);
}

#[test]
fn root_fence_recovery_preserves_the_issued_prefix_or_refuses_without_cleanup() {
    for (b, fence, expected) in [
        (Some(1), Some(1), Some(1)),
        (Some(1), Some(2), Some(1)),
        (Some(2), Some(2), Some(2)),
        (Some(2), Some(1), None),
        (None, Some(1), Some(1)),
        (None, Some(2), None),
        (None, None, None),
        (Some(2), None, Some(2)),
    ] {
        let temp = TempDir::new();
        let path = temp.0.join("database");
        let database = Database::create(&path, config(2_000_000, 1_000_000)).unwrap();
        let database_id = database.database_identity();
        database.close().unwrap();
        let encoded = |issued, replica| {
            storage_format::encode_root(Root {
                database: database_id,
                issued,
                replica,
                state: RootState::Empty,
            })
            .unwrap()
        };
        fs::write(path.join(ROOT_A_NAME), encoded(1, Replica::A)).unwrap();
        if let Some(issued) = b {
            fs::write(path.join(ROOT_B_NAME), encoded(issued, Replica::B)).unwrap();
        } else {
            fs::remove_file(path.join(ROOT_B_NAME)).unwrap();
        }
        if let Some(issued) = fence {
            fs::write(
                path.join(WAL_NAME),
                storage_format::encode_wal(WalRecord {
                    database: database_id,
                    issued,
                    state: RootState::Empty,
                })
                .unwrap(),
            )
            .unwrap();
        } else {
            fs::write(path.join(WAL_NAME), b"torn").unwrap();
        }
        let marker = path.join(PRIVATE_NAME).join(COLUMNS[0].name);
        fs::write(&marker, b"must survive refusal").unwrap();
        let before_a = fs::read(path.join(ROOT_A_NAME)).unwrap();
        let before_b = fs::read(path.join(ROOT_B_NAME)).ok();
        let before_wal = fs::read(path.join(WAL_NAME)).unwrap();
        let result = Database::open(&path, config(2_000_000, 1_000_000));
        if let Some(issued) = expected {
            let database = result.unwrap();
            for sequence in 1..=3 {
                let result = database
                    .resolve_commit(TransactionId::for_attempt(database_id, sequence).unwrap());
                if sequence <= issued {
                    assert_eq!(result.unwrap(), CommitResolution::Aborted);
                } else {
                    assert!(matches!(result, Err(Error::NotFound)));
                }
            }
            independent_namespace_matches(&path, 0);
            assert!(!marker.exists());
        } else {
            assert!(matches!(result, Err(Error::Corrupt(_))));
            assert_eq!(fs::read(path.join(ROOT_A_NAME)).unwrap(), before_a);
            assert_eq!(fs::read(path.join(ROOT_B_NAME)).ok(), before_b);
            assert_eq!(fs::read(path.join(WAL_NAME)).unwrap(), before_wal);
            assert_eq!(fs::read(marker).unwrap(), b"must survive refusal");
        }
    }
}

#[test]
fn interrupted_prefix_repair_then_corruption_never_reuses_an_attempt() {
    fn fixture() -> (TempDir, PathBuf, storage_format::DatabaseId) {
        let temp = TempDir::new();
        let path = temp.0.join("database");
        let database = Database::create(&path, config(2_000_000, 1_000_000)).unwrap();
        let id = database.database_identity();
        database.close().unwrap();
        // Reachable issuance cut: A names prefix 2, B still names prefix 1.
        for (name, replica, issued) in [(ROOT_A_NAME, Replica::A, 2), (ROOT_B_NAME, Replica::B, 1)]
        {
            fs::write(
                path.join(name),
                storage_format::encode_root(Root {
                    database: id,
                    issued,
                    replica,
                    state: RootState::Empty,
                })
                .unwrap(),
            )
            .unwrap();
        }
        fs::write(
            path.join(WAL_NAME),
            storage_format::encode_wal(WalRecord {
                database: id,
                issued: 2,
                state: RootState::Empty,
            })
            .unwrap(),
        )
        .unwrap();
        (temp, path, id)
    }
    let trace = Arc::new(Mutex::new(Vec::with_capacity(128)));
    let observed = Arc::clone(&trace);
    let (_baseline, path, _) = fixture();
    let mut effects = Effects::with_faults(Faults {
        action: Some(Box::new(move |index, effect| {
            let mut trace = observed.lock().unwrap();
            assert!(
                trace.len() < 128,
                "recovery effect trace exceeds test bound"
            );
            trace.push((index, effect));
        })),
        ..Faults::default()
    });
    Database::open_with_effects(&path, config(2_000_000, 1_000_000), &mut effects)
        .unwrap()
        .close()
        .unwrap();
    let trace = trace.lock().unwrap();
    let first = trace
        .iter()
        .position(|(_, effect)| {
            *effect == Effect::CreateMetadata(crate::effects::MetadataKind::RootRepair)
        })
        .unwrap();
    let mut healed = 0;
    let mut refused = 0;
    for &(index, _) in &trace[first..] {
        for damage in [None, Some(ROOT_A_NAME), Some(ROOT_B_NAME), Some(WAL_NAME)] {
            let (temp, path, id) = fixture();
            let mut effects = Effects::with_faults(Faults {
                fail_at: Some(index),
                ..Faults::default()
            });
            assert!(
                Database::open_with_effects(&path, config(2_000_000, 1_000_000), &mut effects)
                    .is_err()
            );
            if let Some(name) = damage {
                fs::write(path.join(name), b"torn").unwrap();
            }
            let before =
                [ROOT_A_NAME, ROOT_B_NAME, WAL_NAME].map(|name| fs::read(path.join(name)).unwrap());
            match Database::open(&path, config(2_000_000, 1_000_000)) {
                Ok(mut database) => {
                    healed += 1;
                    for sequence in 1..=2 {
                        assert_eq!(
                            database
                                .resolve_commit(TransactionId::for_attempt(id, sequence).unwrap())
                                .unwrap(),
                            CommitResolution::Aborted
                        );
                    }
                    let input = temp.0.join("input.tbl");
                    fs::write(&input, row("1", "100", "0.08", "1994-01-01")).unwrap();
                    let commit = database
                        .load_lineitem(&input, &CancellationToken::new())
                        .unwrap();
                    assert_eq!(
                        commit.transaction().sequence(),
                        3,
                        "cut={index} damage={damage:?}"
                    );
                    for sequence in 1..=2 {
                        assert_eq!(
                            database
                                .resolve_commit(TransactionId::for_attempt(id, sequence).unwrap())
                                .unwrap(),
                            CommitResolution::Aborted
                        );
                    }
                }
                Err(Error::Corrupt(_)) if damage.is_some() => {
                    refused += 1;
                    assert_eq!(
                        [ROOT_A_NAME, ROOT_B_NAME, WAL_NAME]
                            .map(|name| fs::read(path.join(name)).unwrap()),
                        before
                    );
                }
                Err(error) => panic!("cut={index} damage={damage:?}: {error:?}"),
            }
        }
    }
    assert!(healed > 0 && refused > 0);
    assert_eq!(healed + refused, (trace.len() - first) * 4);
    eprintln!(
        "prefix_repair_cuts={} observations={} healed={healed} refused={refused}",
        trace.len() - first,
        healed + refused
    );
}
