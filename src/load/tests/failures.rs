use crate::effects::{DirectoryKind, Effect, Effects, Faults, LoadEffect};
use crate::load::tests::{LOAD_EFFECT_COUNT, TempDir, config, independent_namespace_matches, row};
use crate::namespace::{ROOT_A_NAME, UNIT_NAME, UNITS_NAME};
use crate::{CancellationToken, Commit, CommitResolution, Database, Error, TransactionId};
use std::fs::{self, File};
use std::io::{self};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self};
use std::time::Duration;

#[test]
fn ambiguous_publication_preserves_diagnostic_cause() {
    let temp = TempDir::new();
    let input = temp.0.join("input.tbl");
    fs::write(&input, row("1", "100", "0.08", "1994-01-01")).unwrap();
    let cut = Arc::new(AtomicU64::new(u64::MAX));
    let captured = Arc::clone(&cut);
    let mut trace = Effects::with_faults(Faults {
        action: Some(Box::new(move |index, effect| {
            if effect == Effect::SyncMetadata(crate::effects::MetadataKind::RootB) {
                captured.store(index, Ordering::Relaxed);
            }
        })),
        ..Faults::default()
    });
    let limits = config(2_000_000, 1_000_000);
    let mut baseline = Database::create(&temp.0.join("baseline"), limits).unwrap();
    baseline
        .load_lineitem_with_effects(&input, &CancellationToken::new(), &mut trace)
        .unwrap();
    baseline.close().unwrap();
    let path = temp.0.join("database");
    let mut database = Database::create(&path, limits).unwrap();
    let mut effects = Effects::with_faults(Faults {
        fail_at: Some(cut.load(Ordering::Relaxed)),
        ..Faults::default()
    });
    let error = database
        .load_lineitem_with_effects(&input, &CancellationToken::new(), &mut effects)
        .unwrap_err();
    let Error::CommitAmbiguous { transaction, .. } = &error else {
        panic!("expected ambiguous data publication: {error}")
    };
    let transaction = *transaction;
    let cause = std::error::Error::source(&error)
        .and_then(|source| source.downcast_ref::<crate::ErrorCause>())
        .map(crate::ErrorCause::kind);
    assert!(
        matches!(cause, Some(crate::CauseKind::Io { operation: "sync ROOT.B", source }) if source.kind() == io::ErrorKind::Other),
        "ambiguous publication discarded its cause: {error:?}"
    );
    assert!(error.to_string().contains("sync ROOT.B"));
    database.close().unwrap();
    let reopened = Database::open(&path, limits).unwrap();
    assert!(
        matches!(reopened.resolve_commit(transaction).unwrap(), CommitResolution::Durable(commit) if commit.transaction() == transaction && commit.generation() == 1)
    );
    reopened.close().unwrap();
}

#[test]
fn ambiguous_publication_retains_reservation_until_recovery() {
    let temp = TempDir::new();
    let input = temp.0.join("lineitem.tbl");
    fs::write(&input, row("1", "100", "0.08", "1994-01-01")).unwrap();
    // Independent geometry: 28,672 metadata bytes plus two 38-byte copies.
    let limits = config(2_000_000, 28_748);
    let cuts = Arc::new(Mutex::new([None; 2]));
    let observed = Arc::clone(&cuts);
    let mut baseline = Database::create(&temp.0.join("baseline"), limits).unwrap();
    baseline
        .load_lineitem_with_effects(
            &input,
            &CancellationToken::new(),
            &mut Effects::with_faults(Faults {
                action: Some(Box::new(move |index, effect| {
                    let slot = match effect {
                        Effect::Load(LoadEffect::RenameRootA) => Some(0),
                        Effect::SyncMetadata(crate::effects::MetadataKind::RootB) => Some(1),
                        _ => None,
                    };
                    if let Some(slot) = slot {
                        observed.lock().unwrap()[slot] = Some(index);
                    }
                })),
                ..Faults::default()
            }),
        )
        .unwrap();
    assert_eq!(baseline.reserved_temp_bytes(), 0);
    baseline.close().unwrap();

    let cuts = *cuts.lock().unwrap();
    for (case, cut) in cuts.into_iter().enumerate() {
        let path = temp.0.join(format!("database-{case}"));
        let mut database = Database::create(&path, limits).unwrap();
        let outcome = database.load_lineitem_with_effects(
            &input,
            &CancellationToken::new(),
            &mut Effects::with_faults(Faults {
                fail_at: Some(cut.expect("both publication regions were observed")),
                ..Faults::default()
            }),
        );
        let Err(Error::CommitAmbiguous { transaction, .. }) = outcome else {
            panic!("expected ambiguous publication: {outcome:?}");
        };
        assert_eq!(
            database.reserved_memory_bytes(),
            database.path_memory_bytes()
        );
        assert_eq!(database.reserved_temp_bytes(), 28_748);
        assert_eq!(
            fs::metadata(path.join(UNITS_NAME).join(UNIT_NAME))
                .unwrap()
                .len(),
            28_710
        );
        let mut retry = Effects::default();
        assert!(matches!(
            database.load_lineitem_with_effects(&input, &CancellationToken::new(), &mut retry),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(retry.count(), 0);
        assert!(matches!(
            database.resolve_commit(transaction),
            Err(Error::RecoveryRequired { .. })
        ));
        assert_eq!(database.reserved_temp_bytes(), 28_748);
        database.close().unwrap();

        // Closing relinquishes the lease; it neither removes the unit nor
        // declares a settled outcome. Failed reopen must not erase that debt.
        assert_eq!(
            fs::metadata(path.join(UNITS_NAME).join(UNIT_NAME))
                .unwrap()
                .len(),
            28_710
        );
        assert!(
            Database::open_with_effects(
                &path,
                limits,
                &mut Effects::with_faults(Faults {
                    fail_at: Some(0),
                    ..Faults::default()
                })
            )
            .is_err()
        );
        assert_eq!(
            fs::metadata(path.join(UNITS_NAME).join(UNIT_NAME))
                .unwrap()
                .len(),
            28_710
        );
        let mut reopened = Database::open(&path, limits).unwrap();
        assert_eq!(reopened.reserved_temp_bytes(), 0);
        if case == 0 {
            assert_eq!(
                reopened.resolve_commit(transaction).unwrap(),
                CommitResolution::Aborted
            );
            assert_eq!(fs::read_dir(path.join(UNITS_NAME)).unwrap().count(), 0);
            let commit = reopened
                .load_lineitem(&input, &CancellationToken::new())
                .unwrap();
            assert_ne!(commit.transaction(), transaction);
            assert_eq!(reopened.reserved_temp_bytes(), 0);
            assert_eq!(
                reopened.resolve_commit(transaction).unwrap(),
                CommitResolution::Aborted
            );
        } else {
            assert_eq!(
                reopened.resolve_commit(transaction).unwrap(),
                CommitResolution::Durable(Commit {
                    transaction,
                    generation: 1
                })
            );
            assert_eq!(
                fs::metadata(path.join(UNITS_NAME).join(UNIT_NAME))
                    .unwrap()
                    .len(),
                28_710
            );
        }
        reopened.close().unwrap();
    }
}

#[test]
fn review_aborted_attempt_must_not_resolve_to_a_later_load() {
    let temp = TempDir::new();
    let input = temp.0.join("input.tbl");
    fs::write(&input, row("1", "100", "0.08", "1994-01-01")).unwrap();
    let cut = Arc::new(AtomicU64::new(u64::MAX));
    let captured = cut.clone();
    let mut trace = Effects::with_faults(Faults {
        action: Some(Box::new(move |index, effect| {
            if effect == Effect::Load(LoadEffect::RenameRootA) {
                captured.store(index, Ordering::Relaxed);
            }
        })),
        ..Faults::default()
    });
    let limits = config(2_000_000, 1_000_000);
    let mut baseline = Database::create(&temp.0.join("baseline"), limits).unwrap();
    baseline
        .load_lineitem_with_effects(&input, &CancellationToken::new(), &mut trace)
        .unwrap();
    baseline.close().unwrap();
    let path = temp.0.join("database");
    let mut database = Database::create(&path, limits).unwrap();
    let mut effects = Effects::with_faults(Faults {
        fail_at: Some(cut.load(Ordering::Relaxed)),
        ..Faults::default()
    });
    let Err(Error::CommitAmbiguous { transaction, .. }) =
        database.load_lineitem_with_effects(&input, &CancellationToken::new(), &mut effects)
    else {
        panic!("expected an ambiguous attempt");
    };
    database.close().unwrap();
    let mut database = Database::open(&path, limits).unwrap();
    assert_eq!(
        database.resolve_commit(transaction).unwrap(),
        CommitResolution::Aborted
    );
    fs::write(&input, row("1", "200", "0.08", "1994-01-01")).unwrap();
    let later = database
        .load_lineitem(&input, &CancellationToken::new())
        .unwrap();
    assert_eq!(
        database.resolve_commit(transaction).unwrap(),
        CommitResolution::Aborted
    );
    assert_ne!(later.transaction(), transaction);
}

#[test]
fn every_load_effect_has_a_typed_healable_outcome() {
    let baseline_temp = TempDir::new();
    let baseline_database = baseline_temp.0.join("database");
    let baseline_input = baseline_temp.0.join("lineitem.tbl");
    fs::write(
        &baseline_input,
        row("17.00", "21168.23", "0.04", "1996-03-13"),
    )
    .unwrap();
    let mut baseline_db =
        Database::create(&baseline_database, config(2_000_000, 1_000_000)).unwrap();
    let metadata_reads = Arc::new(AtomicU64::new(0));
    let observed_reads = Arc::clone(&metadata_reads);
    let mut baseline = Effects::with_faults(Faults {
        action: Some(Box::new(move |_, effect| {
            if effect == Effect::Load(LoadEffect::ReadPrivateUnitMetadata) {
                observed_reads.fetch_add(1, Ordering::Relaxed);
            }
        })),
        ..Faults::default()
    });
    baseline_db
        .load_lineitem_with_effects(&baseline_input, &CancellationToken::new(), &mut baseline)
        .unwrap();
    baseline_db.close().unwrap();
    assert_eq!(baseline.count(), LOAD_EFFECT_COUNT);
    assert_eq!(metadata_reads.load(Ordering::Relaxed), 3);

    let mut ambiguous = 0_usize;
    for index in 0..baseline.count() {
        let temp = TempDir::new();
        let database_path = temp.0.join("database");
        let input_path = temp.0.join("lineitem.tbl");
        fs::write(&input_path, row("17.00", "21168.23", "0.04", "1996-03-13")).unwrap();
        let mut database = Database::create(&database_path, config(2_000_000, 1_000_000)).unwrap();
        let transaction = TransactionId::for_attempt(database.database_identity(), 1).unwrap();
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(index),
            ..Faults::default()
        });
        let outcome = database.load_lineitem_with_effects(
            &input_path,
            &CancellationToken::new(),
            &mut effects,
        );
        ambiguous += usize::from(matches!(&outcome, Err(Error::CommitAmbiguous { .. })));
        assert!(outcome.is_err(), "effect {index} unexpectedly succeeded");
        assert_eq!(
            database.reserved_memory_bytes(),
            database.path_memory_bytes()
        );
        if matches!(&outcome, Err(Error::CommitAmbiguous { .. })) {
            assert_eq!(database.reserved_temp_bytes(), 28_748);
            assert!(matches!(
                database.load_lineitem(&input_path, &CancellationToken::new()),
                Err(Error::Unsupported(_))
            ));
        } else {
            assert_eq!(database.reserved_temp_bytes(), 0);
            assert_eq!(database.generation(), 0);
        }
        database.close().unwrap();
        let reopened = Database::open(&database_path, config(2_000_000, 1_000_000)).unwrap();
        independent_namespace_matches(&database_path, reopened.generation());
        let root_bytes = fs::read(database_path.join(ROOT_A_NAME)).unwrap();
        let issued = u64::from_le_bytes(root_bytes[112..120].try_into().unwrap());
        let resolution = reopened.resolve_commit(transaction);
        if issued == 0 {
            assert!(!matches!(&outcome, Err(Error::CommitAmbiguous { .. })));
            assert!(
                matches!(resolution, Err(Error::NotFound)),
                "unissued at effect {index}"
            );
        } else if !matches!(&outcome, Err(Error::CommitAmbiguous { .. })) {
            assert_eq!(
                resolution.unwrap(),
                CommitResolution::Aborted,
                "effect {index}"
            );
        } else {
            assert!(resolution.is_ok());
        }
        reopened.close().unwrap();
    }
    assert!(ambiguous > 0);
}

#[test]
fn short_load_io_is_either_progress_or_a_healable_typed_failure() {
    let mut succeeded = 0_usize;
    let mut failed = 0_usize;
    for index in 0..LOAD_EFFECT_COUNT {
        let temp = TempDir::new();
        let database_path = temp.0.join("database");
        let input_path = temp.0.join("lineitem.tbl");
        fs::write(&input_path, row("17.00", "21168.23", "0.04", "1996-03-13")).unwrap();
        let mut database = Database::create(&database_path, config(2_000_000, 1_000_000)).unwrap();
        let mut effects = Effects::with_faults(Faults {
            short_at: Some(index),
            ..Faults::default()
        });
        let outcome = database.load_lineitem_with_effects(
            &input_path,
            &CancellationToken::new(),
            &mut effects,
        );
        let unsettled = matches!(
            &outcome,
            Err(Error::CommitAmbiguous { .. } | Error::CleanupRequired { .. })
        );
        match outcome {
            Ok(_) => succeeded += 1,
            Err(_) => failed += 1,
        }
        assert_eq!(
            database.reserved_memory_bytes(),
            database.path_memory_bytes()
        );
        assert_eq!(
            database.reserved_temp_bytes(),
            if unsettled { 28_748 } else { 0 }
        );
        database.close().unwrap();
        Database::open(&database_path, config(2_000_000, 1_000_000))
            .unwrap()
            .close()
            .unwrap();
    }
    assert!(succeeded > 0);
    assert!(failed > 0);
}

#[test]
fn cancellation_at_each_effect_is_abort_before_root_and_ignored_after() {
    let mut cancelled = 0_usize;
    let mut committed = 0_usize;
    let mut issuance_cancelled = 0_usize;
    for index in 0..LOAD_EFFECT_COUNT {
        let temp = TempDir::new();
        let database_path = temp.0.join("database");
        let input_path = temp.0.join("lineitem.tbl");
        fs::write(&input_path, row("17.00", "21168.23", "0.04", "1996-03-13")).unwrap();
        let mut database = Database::create(&database_path, config(2_000_000, 1_000_000)).unwrap();
        let token = Arc::new(CancellationToken::new());
        let action_token = Arc::clone(&token);
        let mut effects = Effects::with_faults(Faults {
            action: Some(Box::new(move |effect_index, _| {
                if effect_index == index {
                    action_token.cancel();
                }
            })),
            ..Faults::default()
        });
        let outcome = database.load_lineitem_with_effects(&input_path, &token, &mut effects);
        match outcome {
            Ok(commit) => {
                committed += 1;
                assert_eq!(commit.generation(), 1);
                assert_eq!(database.generation(), 1);
            }
            Err(Error::Cancelled) => {
                cancelled += 1;
                assert_eq!(database.generation(), 0);
            }
            Err(Error::RecoveryRequired { source, .. })
                if matches!(source.kind(), crate::CauseKind::Cancelled) =>
            {
                issuance_cancelled += 1;
                assert!(database.needs_reopen());
                assert_eq!(database.generation(), 0);
            }
            other => panic!("effect {index} returned {other:?}"),
        }
        assert_eq!(
            database.reserved_memory_bytes(),
            database.path_memory_bytes()
        );
        assert_eq!(database.reserved_temp_bytes(), 0);
        let generation = database.generation();
        database.close().unwrap();
        let reopened = Database::open(&database_path, config(2_000_000, 1_000_000)).unwrap();
        assert_eq!(reopened.generation(), generation);
        independent_namespace_matches(&database_path, generation);
        reopened.close().unwrap();
    }
    assert!(cancelled > 0);
    assert!(committed > 0);
    assert!(issuance_cancelled > 0);
}

#[test]
fn every_load_cleanup_failure_retains_debt_and_reopen_heals() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let trace_action = Arc::clone(&trace);
    let baseline_temp = TempDir::new();
    let baseline_database = baseline_temp.0.join("database");
    let baseline_input = baseline_temp.0.join("lineitem.tbl");
    fs::write(
        &baseline_input,
        row("17.00", "21168.23", "0.04", "1996-03-13"),
    )
    .unwrap();
    let mut baseline_db =
        Database::create(&baseline_database, config(2_000_000, 1_000_000)).unwrap();
    baseline_db
        .load_lineitem_with_effects(
            &baseline_input,
            &CancellationToken::new(),
            &mut Effects::with_faults(Faults {
                action: Some(Box::new(move |index, effect| {
                    trace_action.lock().unwrap().push((index, effect));
                })),
                ..Faults::default()
            }),
        )
        .unwrap();
    baseline_db.close().unwrap();
    let primary = trace
        .lock()
        .unwrap()
        .iter()
        .find_map(|(index, effect)| {
            (*effect == Effect::Load(LoadEffect::WriteUnitPayload)).then_some(*index)
        })
        .unwrap();

    let probe_temp = TempDir::new();
    let probe_database = probe_temp.0.join("database");
    let probe_input = probe_temp.0.join("lineitem.tbl");
    fs::write(&probe_input, row("17.00", "21168.23", "0.04", "1996-03-13")).unwrap();
    let mut probe_db = Database::create(&probe_database, config(2_000_000, 1_000_000)).unwrap();
    let mut probe_effects = Effects::with_faults(Faults {
        fail_at: Some(primary),
        ..Faults::default()
    });
    assert!(
        probe_db
            .load_lineitem_with_effects(
                &probe_input,
                &CancellationToken::new(),
                &mut probe_effects,
            )
            .is_err()
    );
    let cleanup_end = probe_effects.count();
    probe_db.close().unwrap();
    assert_eq!((primary, cleanup_end), (107, 182));

    for cleanup_cut in primary + 1..cleanup_end {
        let temp = TempDir::new();
        let database_path = temp.0.join("database");
        let input_path = temp.0.join("lineitem.tbl");
        fs::write(&input_path, row("17.00", "21168.23", "0.04", "1996-03-13")).unwrap();
        let mut database = Database::create(&database_path, config(2_000_000, 1_000_000)).unwrap();
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(primary),
            second_fail_at: Some(cleanup_cut),
            ..Faults::default()
        });
        assert!(matches!(
            database.load_lineitem_with_effects(
                &input_path,
                &CancellationToken::new(),
                &mut effects,
            ),
            Err(Error::CleanupRequired { .. })
        ));
        assert_eq!(
            database.reserved_memory_bytes(),
            database.path_memory_bytes()
        );
        assert_eq!(database.reserved_temp_bytes(), 28_748);
        assert!(matches!(
            database.load_lineitem(&input_path, &CancellationToken::new()),
            Err(Error::Unsupported(_))
        ));
        database.close().unwrap();
        let reopened = Database::open(&database_path, config(2_000_000, 1_000_000)).unwrap();
        assert_eq!(reopened.generation(), 0);
        independent_namespace_matches(&database_path, 0);
        reopened.close().unwrap();
    }
}

#[test]
fn ambiguous_commit_survives_each_resolution_failure_and_heals() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let trace_action = Arc::clone(&trace);
    let baseline_temp = TempDir::new();
    let baseline_database = baseline_temp.0.join("database");
    let baseline_input = baseline_temp.0.join("lineitem.tbl");
    fs::write(
        &baseline_input,
        row("17.00", "21168.23", "0.04", "1996-03-13"),
    )
    .unwrap();
    let mut baseline_db =
        Database::create(&baseline_database, config(2_000_000, 1_000_000)).unwrap();
    baseline_db
        .load_lineitem_with_effects(
            &baseline_input,
            &CancellationToken::new(),
            &mut Effects::with_faults(Faults {
                action: Some(Box::new(move |index, effect| {
                    trace_action.lock().unwrap().push((index, effect));
                })),
                ..Faults::default()
            }),
        )
        .unwrap();
    baseline_db.close().unwrap();
    let publication_cut = trace
        .lock()
        .unwrap()
        .iter()
        .skip_while(|(_, effect)| *effect != Effect::Load(LoadEffect::RenameRootA))
        .find_map(|(index, effect)| {
            (*effect == Effect::SyncDirectory(DirectoryKind::Database)).then_some(*index)
        })
        .unwrap();

    let make_ambiguous = |database_path: &Path, input_path: &Path| {
        let mut database = Database::create(database_path, config(2_000_000, 1_000_000)).unwrap();
        let transaction = TransactionId::for_attempt(database.database_identity(), 1).unwrap();
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(publication_cut),
            ..Faults::default()
        });
        assert!(matches!(
            database.load_lineitem_with_effects(
                input_path,
                &CancellationToken::new(),
                &mut effects,
            ),
            Err(Error::CommitAmbiguous { .. })
        ));
        (database, transaction)
    };

    let probe_temp = TempDir::new();
    let probe_database = probe_temp.0.join("database");
    let probe_input = probe_temp.0.join("lineitem.tbl");
    fs::write(&probe_input, row("17.00", "21168.23", "0.04", "1996-03-13")).unwrap();
    let (probe_db, transaction) = make_ambiguous(&probe_database, &probe_input);
    assert!(matches!(
        probe_db.resolve_commit(transaction),
        Err(Error::RecoveryRequired { .. })
    ));
    probe_db.close().unwrap();
    let probe_db = Database::open(&probe_database, config(2_000_000, 1_000_000)).unwrap();
    let mut probe_effects = Effects::default();
    assert_eq!(
        probe_db
            .resolve_commit_with_effects(transaction, &mut probe_effects)
            .unwrap(),
        CommitResolution::Durable(Commit {
            transaction,
            generation: 1,
        })
    );
    let resolution_effects = probe_effects.count();
    probe_db.close().unwrap();
    assert_eq!(resolution_effects, 40);

    for resolution_cut in 0..resolution_effects {
        let temp = TempDir::new();
        let database_path = temp.0.join("database");
        let input_path = temp.0.join("lineitem.tbl");
        fs::write(&input_path, row("17.00", "21168.23", "0.04", "1996-03-13")).unwrap();
        let (database, transaction) = make_ambiguous(&database_path, &input_path);
        database.close().unwrap();
        let database = Database::open(&database_path, config(2_000_000, 1_000_000)).unwrap();
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(resolution_cut),
            ..Faults::default()
        });
        assert!(
            database
                .resolve_commit_with_effects(transaction, &mut effects)
                .is_err()
        );
        assert_eq!(
            database.resolve_commit(transaction).unwrap(),
            CommitResolution::Durable(Commit {
                transaction,
                generation: 1,
            })
        );
        independent_namespace_matches(&database_path, 1);
        database.close().unwrap();
    }
}

#[test]
fn process_death_at_publication_boundaries_recovers() {
    const CHILD: &str = "PIPESQL_LOAD_DEATH_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let database_path = PathBuf::from(std::env::var_os("PIPESQL_LOAD_DB").unwrap());
        let input_path = PathBuf::from(std::env::var_os("PIPESQL_LOAD_INPUT").unwrap());
        let marker = PathBuf::from(std::env::var_os("PIPESQL_LOAD_MARKER").unwrap());
        let target = std::env::var("PIPESQL_LOAD_CUT")
            .unwrap()
            .parse::<u64>()
            .unwrap();
        let mut database = Database::open(&database_path, config(2_000_000, 1_000_000)).unwrap();
        let mut effects = Effects::with_faults(Faults {
            after_action: Some(Box::new(move |index, _| {
                if index == target {
                    let file = File::create(&marker).unwrap();
                    file.sync_all().unwrap();
                    loop {
                        thread::sleep(Duration::from_secs(60));
                    }
                }
            })),
            ..Faults::default()
        });
        let outcome = database.load_lineitem_with_effects(
            &input_path,
            &CancellationToken::new(),
            &mut effects,
        );
        panic!("death child passed cut without being killed: {outcome:?}");
    }

    let baseline_temp = TempDir::new();
    let baseline_database = baseline_temp.0.join("database");
    let baseline_input = baseline_temp.0.join("lineitem.tbl");
    fs::write(
        &baseline_input,
        row("17.00", "21168.23", "0.04", "1996-03-13"),
    )
    .unwrap();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let action_observed = Arc::clone(&observed);
    let mut baseline_db =
        Database::create(&baseline_database, config(2_000_000, 1_000_000)).unwrap();
    baseline_db
        .load_lineitem_with_effects(
            &baseline_input,
            &CancellationToken::new(),
            &mut Effects::with_faults(Faults {
                after_action: Some(Box::new(move |index, effect| {
                    action_observed.lock().unwrap().push((index, effect));
                })),
                ..Faults::default()
            }),
        )
        .unwrap();
    baseline_db.close().unwrap();
    let mut generation = 0_u64;
    let mut issued = 0_u64;
    let cuts: Vec<(u64, u64, u64)> = observed
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(index, effect)| {
            if *effect == Effect::Load(LoadEffect::RenameRootA) {
                generation = 1;
            }
            if *effect == Effect::Load(LoadEffect::RenameIssuedRootA) {
                issued = 1;
            }
            matches!(
                effect,
                Effect::Load(LoadEffect::SyncPrivateUnit)
                    | Effect::Load(LoadEffect::LinkUnit)
                    | Effect::Load(LoadEffect::RemovePrivateUnit)
                    | Effect::Load(LoadEffect::RenameRootA)
                    | Effect::Load(LoadEffect::RenameRootB)
                    | Effect::SyncDirectory(DirectoryKind::Units)
                    | Effect::SyncDirectory(DirectoryKind::Private)
                    | Effect::SyncDirectory(DirectoryKind::Database)
                    | Effect::SyncMetadata(crate::effects::MetadataKind::Wal)
                    | Effect::SyncMetadata(crate::effects::MetadataKind::RootA)
                    | Effect::SyncMetadata(crate::effects::MetadataKind::RootB)
                    | Effect::SyncMetadata(crate::effects::MetadataKind::IssuedWal)
                    | Effect::SyncMetadata(crate::effects::MetadataKind::IssuedRootA)
                    | Effect::SyncMetadata(crate::effects::MetadataKind::IssuedRootB)
                    | Effect::Load(LoadEffect::RenameIssuedRootA)
                    | Effect::Load(LoadEffect::RenameIssuedRootB)
            )
            .then_some((*index, generation, issued))
        })
        .collect();
    assert_eq!(cuts.len(), 19);

    for (cut, expected_generation, expected_issued) in cuts {
        let temp = TempDir::new();
        let database_path = temp.0.join("database");
        let input_path = temp.0.join("lineitem.tbl");
        let marker = temp.0.join("paused");
        fs::write(&input_path, row("17.00", "21168.23", "0.04", "1996-03-13")).unwrap();
        Database::create(&database_path, config(2_000_000, 1_000_000))
            .unwrap()
            .close()
            .unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(
                concat!(
                    module_path!(),
                    "::process_death_at_publication_boundaries_recovers"
                )
                .strip_prefix("pipesql::")
                .expect("test module belongs to the library"),
            )
            .arg("--nocapture")
            .env(CHILD, "1")
            .env("PIPESQL_LOAD_DB", &database_path)
            .env("PIPESQL_LOAD_INPUT", &input_path)
            .env("PIPESQL_LOAD_MARKER", &marker)
            .env("PIPESQL_LOAD_CUT", cut.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut paused = false;
        for _ in 0..1_000 {
            if marker.exists() {
                paused = true;
                break;
            }
            if child.try_wait().unwrap().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        // Reap even a child which missed its cut; a failed test must not
        // leave a live process holding the database's lease.
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
        }
        child.wait().unwrap();
        assert!(paused, "child did not reach publication cut {cut}");

        let mut reopened = Database::open(&database_path, config(2_000_000, 1_000_000)).unwrap();
        assert_eq!(reopened.generation(), expected_generation, "cut {cut}");
        independent_namespace_matches(&database_path, expected_generation);
        let root = fs::read(database_path.join(ROOT_A_NAME)).unwrap();
        assert_eq!(
            u64::from_le_bytes(root[112..120].try_into().unwrap()),
            expected_issued,
            "cut {cut}"
        );
        if expected_generation == 0 {
            let later = reopened
                .load_lineitem(&input_path, &CancellationToken::new())
                .unwrap();
            assert_eq!(
                later.transaction().sequence(),
                expected_issued + 1,
                "cut {cut}"
            );
            if expected_issued != 0 {
                let aborted =
                    TransactionId::for_attempt(reopened.database_identity(), expected_issued)
                        .unwrap();
                assert_eq!(
                    reopened.resolve_commit(aborted).unwrap(),
                    CommitResolution::Aborted
                );
            }
        }
        reopened.close().unwrap();
    }
}
