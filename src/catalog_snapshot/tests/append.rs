//! Streaming batch ownership, admission refusal, and cleanup through publication.
use super::{Fixture, append_columns, declarations, snapshot_cells, token};
use crate::catalog;
use crate::catalog_schema::{self, TableId};
use crate::effects::{DirectoryKind, Effect, Effects, Faults};
use crate::namespace::{UNITS_NAME, WAL_NAME};
use crate::native_unit::{self, InputColumn, InputValues};
use crate::{CancellationToken, CommitResolution, Database, Error};
use std::fs;
use std::sync::{Arc, Mutex};

fn positional_append_columns() -> [crate::catalog_snapshot::ColumnInput<'static>; 2] {
    use crate::catalog_snapshot::ColumnInput;
    // Fixture declaration order is note (ID 29), amount (ID 3), not ID order.
    [
        ColumnInput {
            values: InputValues::String(&["雪", "", "abc", "ignored"]),
            validity: &[7],
        },
        ColumnInput {
            values: InputValues::Double(&[1., -0., f64::NAN, 42.]),
            validity: &[15],
        },
    ]
}

fn streaming_database() -> (Fixture, Database, TableId) {
    let fixture = Fixture::directory();
    let db = Database::create_catalog_with_effects(
        &fixture.0.join("db"),
        crate::Config::new(4_000_000, 2_000_000).unwrap(),
        &mut Effects::default(),
    )
    .unwrap();
    let table = TableId::new(17).unwrap();
    db.catalog_writer()
        .unwrap()
        .create_table(
            "facts",
            table,
            &declarations(),
            &CancellationToken::new(),
            &mut Effects::default(),
        )
        .unwrap();
    (fixture, db, table)
}

#[test]
fn catalog_engine_append_preserves_values_snapshots_and_receipts() {
    let fixture = Fixture::new();
    let config = crate::Config::new(2_000_000, 1_000_000).unwrap();
    let database = Database::open(&fixture.0, config).unwrap();
    let cancel = CancellationToken::new();
    let table = TableId::new(17).unwrap();
    database
        .catalog_writer()
        .unwrap()
        .create_table(
            "facts",
            table,
            &declarations(),
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let first = database
        .catalog_writer()
        .unwrap()
        .append(table, &append_columns(), &cancel, &mut Effects::default())
        .unwrap();
    let old = database.catalog_snapshot().unwrap();
    let mut aborted = database.catalog_writer().unwrap();
    aborted.issue(&cancel, &mut Effects::default()).unwrap();
    aborted.abort_unbuilt().unwrap();
    let second = database
        .catalog_writer()
        .unwrap()
        .append(table, &append_columns(), &cancel, &mut Effects::default())
        .unwrap();
    let expected = vec![
        (Some(1f64.to_bits()), Some("雪".to_owned())),
        (Some((-0f64).to_bits()), Some(String::new())),
        (Some(f64::NAN.to_bits()), Some("abc".to_owned())),
        (Some(42f64.to_bits()), None),
    ];
    assert_eq!(snapshot_cells(&old, &fixture.objects()), expected);
    let new = database.catalog_snapshot().unwrap();
    assert_eq!(
        snapshot_cells(&new, &fixture.objects()),
        [expected.clone(), expected.clone()].concat()
    );
    assert_eq!(database.reserved_temp_bytes(), 0);
    assert_eq!(
        database.reserved_memory_bytes(),
        crate::catalog_snapshot::REGISTRY_BYTES + database.path_memory_bytes()
    );
    assert_eq!(
        database.resolve_commit(token(5)).unwrap(),
        CommitResolution::Aborted
    );
    drop(new);
    drop(old);
    database.close().unwrap();
    let database = Database::open(&fixture.0, config).unwrap();
    for commit in [first, second] {
        assert_eq!(
            database.resolve_commit(commit.transaction()).unwrap(),
            CommitResolution::Durable(commit)
        );
    }
    assert_eq!(
        snapshot_cells(&database.catalog_snapshot().unwrap(), &fixture.objects()),
        [expected.clone(), expected].concat()
    );
}

#[test]
fn catalog_engine_append_refuses_space_without_issuing() {
    let fixture = Fixture::new();
    let database =
        Database::open(&fixture.0, crate::Config::new(2_000_000, 4520).unwrap()).unwrap();
    let cancel = CancellationToken::new();
    let table = TableId::new(17).unwrap();
    database
        .catalog_writer()
        .unwrap()
        .create_table(
            "facts",
            table,
            &declarations(),
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let before = fs::read(fixture.0.join(WAL_NAME)).unwrap();
    assert!(matches!(
        database.catalog_writer().unwrap().append(
            table,
            &append_columns(),
            &cancel,
            &mut Effects::default()
        ),
        Err(Error::Resource { .. })
    ));
    assert_eq!(fs::read(fixture.0.join(WAL_NAME)).unwrap(), before);
    assert_eq!(fs::read_dir(fixture.objects()).unwrap().count(), 3);
    assert_eq!(database.reserved_temp_bytes(), 0);
    database.catalog_writer().unwrap().abort_unbuilt().unwrap();
}

#[test]
fn catalog_engine_append_cleans_construction_failures() {
    let config = crate::Config::new(2_000_000, 1_000_000).unwrap();
    let cancel = CancellationToken::new();
    let table = TableId::new(17).unwrap();
    let setup = || {
        let fixture = Fixture::new();
        let database = Database::open(&fixture.0, config).unwrap();
        database
            .catalog_writer()
            .unwrap()
            .create_table(
                "facts",
                table,
                &declarations(),
                &cancel,
                &mut Effects::default(),
            )
            .unwrap();
        database
            .catalog_writer()
            .unwrap()
            .append(table, &append_columns(), &cancel, &mut Effects::default())
            .unwrap();
        (fixture, database)
    };
    let (baseline, database) = setup();
    let trace = Arc::new(Mutex::new(Vec::new()));
    let observed = trace.clone();
    database
        .catalog_writer()
        .unwrap()
        .append(
            table,
            &append_columns(),
            &cancel,
            &mut Effects::with_faults(Faults {
                action: Some(Box::new(move |index, effect| {
                    observed.lock().unwrap().push((index, effect));
                })),
                ..Faults::default()
            }),
        )
        .unwrap();
    let trace = trace.lock().unwrap();
    let start = trace
        .iter()
        .position(|(_, effect)| {
            *effect == Effect::CreateMetadata(crate::effects::MetadataKind::CatalogObject)
        })
        .unwrap();
    let end = start
        + trace[start..]
            .iter()
            .position(|(_, effect)| *effect == Effect::SyncDirectory(DirectoryKind::Units))
            .unwrap();
    assert!(end - start < 128);
    for &(cut, _) in &trace[start..=end] {
        let (fixture, database) = setup();
        let old = database.catalog_snapshot().unwrap();
        let values = snapshot_cells(&old, &fixture.objects());
        assert!(
            database
                .catalog_writer()
                .unwrap()
                .append(
                    table,
                    &append_columns(),
                    &cancel,
                    &mut Effects::with_faults(Faults {
                        fail_at: Some(cut),
                        ..Faults::default()
                    })
                )
                .is_err(),
            "cut {cut}"
        );
        assert_eq!(database.reserved_temp_bytes(), 0, "cut {cut}");
        assert_eq!(
            fs::read_dir(fixture.objects()).unwrap().count(),
            7,
            "cut {cut}"
        );
        assert_eq!(snapshot_cells(&old, &fixture.objects()), values);
        assert_eq!(
            database.resolve_commit(token(5)).unwrap(),
            CommitResolution::Aborted
        );
        let retry = database
            .catalog_writer()
            .unwrap()
            .append(table, &append_columns(), &cancel, &mut Effects::default())
            .unwrap();
        assert_eq!(retry.transaction(), token(6));
    }
    println!("catalog_append_construction_effects={}", end - start + 1);
    drop(database);
    drop(baseline);
}

#[test]
fn catalog_streaming_append_reuses_inputs_and_publishes_once() {
    use crate::catalog_snapshot::AppendLimits;
    use crate::{QueryStep, Value};
    let parent = Fixture::directory();
    let path = parent.0.join("streaming-append");
    let config = crate::Config::new(4_000_000, 2_000_000).unwrap();
    let db = Database::create_catalog_with_effects(&path, config, &mut Effects::default()).unwrap();
    let cancel = CancellationToken::new();
    let table = TableId::new(17).unwrap();
    db.catalog_writer()
        .unwrap()
        .create_table(
            "facts",
            table,
            &declarations(),
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let old = crate::frontend::prepare_catalog(&db, "FROM facts |> SELECT note,amount").unwrap();
    let mut append = db
        .catalog_writer()
        .unwrap()
        .begin_append(
            table,
            AppendLimits {
                batches: 2,
                encoded_bytes: 100_000,
            },
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let token = append.transaction();
    assert!(matches!(
        db.resolve_catalog(token, &mut Effects::default()),
        Err(Error::Contention(_))
    ));
    assert!(db.reserved_temp_bytes() > 100_000);
    let mut text = String::from("first");
    let mut numbers = [1.0];
    for next in ["second", "overwritten"] {
        let text_values = [text.as_str()];
        let columns = [
            InputColumn {
                id: catalog_schema::ColumnId::new(29).unwrap(),
                values: InputValues::String(&text_values),
                validity: &[1],
            },
            InputColumn {
                id: catalog_schema::ColumnId::new(3).unwrap(),
                values: InputValues::Double(&numbers),
                validity: &[1],
            },
        ];
        append
            .write(&columns, &cancel, &mut Effects::default())
            .unwrap();
        text.clear();
        text.push_str(next);
        numbers[0] += 1.0;
        assert_eq!(db.generation(), 1);
        // A newly admitted reader may inspect the published snapshot while
        // private ingestion files and their reservations remain live.
        let mut reader = db.execute(&old, &cancel).unwrap();
        assert!(matches!(reader.step(), QueryStep::Finished));
    }
    let commit = append.commit(&cancel, &mut Effects::default()).unwrap();
    assert_eq!(commit.transaction(), token);
    assert_eq!(commit.generation(), 2);
    assert_eq!(db.reserved_temp_bytes(), 0);
    let mut old_reader = db.execute(&old, &cancel).unwrap();
    assert!(matches!(old_reader.step(), QueryStep::Finished));
    drop(old_reader);
    drop(old);
    db.close().unwrap();
    let db = Database::open(&path, config).unwrap();
    assert_eq!(
        db.resolve_catalog(token, &mut Effects::default()).unwrap(),
        crate::CommitResolution::Durable(commit)
    );
    let query = crate::frontend::prepare_catalog(&db, "FROM facts |> SELECT note,amount").unwrap();
    let mut reader = db.execute(&query, &cancel).unwrap();
    let mut rows = Vec::new();
    let mut finished = false;
    for _ in 0..64 {
        match reader.step() {
            QueryStep::Progress => {}
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let Some(Value::String(text)) = batch.value(row, 0) else {
                        panic!("STRING");
                    };
                    let Some(Value::Double(value)) = batch.value(row, 1) else {
                        panic!("DOUBLE");
                    };
                    rows.push((text.as_str().to_owned(), value));
                }
            }
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("query failed: {error:?}"),
        }
    }
    assert!(finished);
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        rows,
        [("first".to_owned(), 1.0), ("second".to_owned(), 2.0)]
    );
    drop(reader);
    drop(query);
    assert_eq!(
        db.reserved_memory_bytes(),
        crate::catalog_snapshot::REGISTRY_BYTES + db.path_memory_bytes()
    );
}

#[test]
fn catalog_streaming_append_limits_abort_and_recovery() {
    use crate::catalog_snapshot::AppendLimits;
    let (_fixture, db, table) = streaming_database();
    let cancel = CancellationToken::new();
    let objects = db.path().join(UNITS_NAME);
    let initial = fs::read(db.path().join(WAL_NAME)).unwrap();
    for limits in [
        AppendLimits {
            batches: 0,
            encoded_bytes: 1,
        },
        AppendLimits {
            batches: catalog::MAX_UNITS + 1,
            encoded_bytes: 1,
        },
        AppendLimits {
            batches: 1,
            encoded_bytes: 0,
        },
    ] {
        assert!(matches!(
            db.catalog_writer().unwrap().begin_append(
                table,
                limits,
                &cancel,
                &mut Effects::default()
            ),
            Err(Error::Resource { .. })
        ));
        assert_eq!(fs::read(db.path().join(WAL_NAME)).unwrap(), initial);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(
            db.reserved_memory_bytes(),
            crate::catalog_snapshot::REGISTRY_BYTES + db.path_memory_bytes()
        );
    }
    for bytes in [1, 100_000] {
        let mut append = db
            .catalog_writer()
            .unwrap()
            .begin_append(
                table,
                AppendLimits {
                    batches: 1,
                    encoded_bytes: bytes,
                },
                &cancel,
                &mut Effects::default(),
            )
            .unwrap();
        let token = append.transaction();
        if bytes > 1 {
            append
                .write(&append_columns(), &cancel, &mut Effects::default())
                .unwrap();
        }
        let mut effects = Effects::default();
        assert!(matches!(
            append.write(&append_columns(), &cancel, &mut effects),
            Err(Error::Resource { .. })
        ));
        assert_eq!(effects.count(), 0, "limits refuse before file effects");
        assert!(matches!(
            append.write(&append_columns(), &cancel, &mut effects),
            Err(Error::Unsupported(_))
        ));
        append.abort(&mut Effects::default()).unwrap();
        assert_eq!(
            db.resolve_catalog(token, &mut Effects::default()).unwrap(),
            crate::CommitResolution::Aborted
        );
        assert_eq!(fs::read_dir(&objects).unwrap().count(), 3);
        assert_eq!(db.generation(), 1);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(
            db.reserved_memory_bytes(),
            crate::catalog_snapshot::REGISTRY_BYTES + db.path_memory_bytes()
        );
    }
    // No encoding allocation can exceed the shared account after issuance.
    let mut append = db
        .catalog_writer()
        .unwrap()
        .begin_append(
            table,
            AppendLimits {
                batches: 2,
                encoded_bytes: 100_000,
            },
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let pressure = db
        .reserve_memory(
            db.config().memory_limit_bytes() - db.reserved_memory_bytes(),
            "deny streaming encoder",
        )
        .unwrap();
    let mut effects = Effects::default();
    assert!(matches!(
        append.write(&append_columns(), &cancel, &mut effects),
        Err(Error::Resource { .. })
    ));
    assert_eq!(effects.count(), 0);
    drop(pressure);
    append.abort(&mut Effects::default()).unwrap();
    // Drop is not rollback: reopen removes both unpublished unit files and
    // proves abort from the durable issued prefix and unchanged success history.
    let mut append = db
        .catalog_writer()
        .unwrap()
        .begin_append(
            table,
            AppendLimits {
                batches: 2,
                encoded_bytes: 100_000,
            },
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let token = append.transaction();
    for _ in 0..2 {
        append
            .write(&append_columns(), &cancel, &mut Effects::default())
            .unwrap();
    }
    drop(append);
    assert!(db.catalog_writer().is_err());
    assert!(db.reserved_temp_bytes() > 0);
    let path = db.path().to_owned();
    let config = db.config();
    db.close().unwrap();
    let db = Database::open(&path, config).unwrap();
    assert_eq!(
        db.resolve_catalog(token, &mut Effects::default()).unwrap(),
        crate::CommitResolution::Aborted
    );
    assert_eq!(fs::read_dir(objects).unwrap().count(), 3);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert_eq!(db.generation(), 1);
}

#[test]
fn catalog_streaming_append_second_batch_faults_clean_entire_prefix() {
    use crate::catalog_snapshot::AppendLimits;
    let (_fixture, db, table) = streaming_database();
    let cancel = CancellationToken::new();
    let limits = AppendLimits {
        batches: 2,
        encoded_bytes: 100_000,
    };
    let mut baseline = db
        .catalog_writer()
        .unwrap()
        .begin_append(table, limits, &cancel, &mut Effects::default())
        .unwrap();
    baseline
        .write(&append_columns(), &cancel, &mut Effects::default())
        .unwrap();
    let mut effects = Effects::default();
    baseline
        .write(&append_columns(), &cancel, &mut effects)
        .unwrap();
    let count = effects.count();
    assert!(count > 0 && count < 32);
    baseline.abort(&mut Effects::default()).unwrap();
    for cut in 0..count {
        let mut append = db
            .catalog_writer()
            .unwrap()
            .begin_append(table, limits, &cancel, &mut Effects::default())
            .unwrap();
        let token = append.transaction();
        append
            .write(&append_columns(), &cancel, &mut Effects::default())
            .unwrap();
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(cut),
            ..Faults::default()
        });
        assert!(
            append
                .write(&append_columns(), &cancel, &mut effects)
                .is_err(),
            "second batch cut {cut}"
        );
        assert_eq!(effects.count(), cut + 1);
        append.abort(&mut Effects::default()).unwrap();
        assert_eq!(
            db.resolve_catalog(token, &mut Effects::default()).unwrap(),
            crate::CommitResolution::Aborted
        );
        assert_eq!(fs::read_dir(db.path().join(UNITS_NAME)).unwrap().count(), 3);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(
            db.reserved_memory_bytes(),
            crate::catalog_snapshot::REGISTRY_BYTES + db.path_memory_bytes()
        );
        assert_eq!(db.generation(), 1);
    }
    let mut append = db
        .catalog_writer()
        .unwrap()
        .begin_append(table, limits, &cancel, &mut Effects::default())
        .unwrap();
    append
        .write(&append_columns(), &cancel, &mut Effects::default())
        .unwrap();
    let stopped = CancellationToken::new();
    stopped.cancel();
    assert!(matches!(
        append.write(&append_columns(), &stopped, &mut Effects::default()),
        Err(Error::Cancelled)
    ));
    append.abort(&mut Effects::default()).unwrap();
}

#[test]
fn catalog_streaming_append_growth_refusal_preserves_cleanup_ownership() {
    use crate::catalog_snapshot::AppendLimits;
    let (_fixture, db, table) = streaming_database();
    let cancel = CancellationToken::new();
    let mut append = db
        .catalog_writer()
        .unwrap()
        .begin_append(
            table,
            AppendLimits {
                batches: 2,
                encoded_bytes: 100_000,
            },
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    append
        .write(&append_columns(), &cancel, &mut Effects::default())
        .unwrap();
    let text = "x".repeat(native_unit::MAX_TEXT_BYTES);
    let strings = [text.as_str()];
    let columns = [
        InputColumn {
            id: catalog_schema::ColumnId::new(29).unwrap(),
            values: InputValues::String(&strings),
            validity: &[1],
        },
        InputColumn {
            id: catalog_schema::ColumnId::new(3).unwrap(),
            values: InputValues::Double(&[1.0]),
            validity: &[1],
        },
    ];
    let pressure = db
        .reserve_memory(
            db.config().memory_limit_bytes() - db.reserved_memory_bytes(),
            "deny encoder growth",
        )
        .unwrap();
    let mut effects = Effects::default();
    assert!(matches!(
        append.write(&columns, &cancel, &mut effects),
        Err(Error::Resource { .. })
    ));
    assert_eq!(effects.count(), 0);
    assert!(db.reserved_temp_bytes() > 0);
    drop(pressure);
    append.abort(&mut Effects::default()).unwrap();
    assert_eq!(fs::read_dir(db.path().join(UNITS_NAME)).unwrap().count(), 3);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert_eq!(
        db.reserved_memory_bytes(),
        crate::catalog_snapshot::REGISTRY_BYTES + db.path_memory_bytes()
    );
}

#[test]
#[cfg_attr(
    all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"),
    ignore = "GNU aarch64 pthread minimum exceeds the 64-KiB reported-stack ceiling"
)]
fn catalog_streaming_append_has_bounded_reported_stack() {
    use crate::catalog_snapshot::AppendLimits;
    let (_fixture, db, _) = streaming_database();
    let commit = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(48 * 1024)
            .spawn_scoped(scope, || {
                let reported = pipesql_filesystem::test_current_thread_stack_bytes();
                assert!(
                    reported > 0 && reported <= 65_536,
                    "streaming append stack: {reported}"
                );
                let cancel = CancellationToken::new();
                let mut append = db
                    .catalog_writer()
                    .unwrap()
                    .begin_append_named(
                        "facts",
                        AppendLimits {
                            batches: 2,
                            encoded_bytes: 100_000,
                        },
                        &cancel,
                        &mut Effects::default(),
                    )
                    .unwrap();
                for _ in 0..2 {
                    append
                        .write_columns(
                            &positional_append_columns(),
                            &cancel,
                            &mut Effects::default(),
                        )
                        .unwrap();
                }
                append.commit(&cancel, &mut Effects::default()).unwrap()
            })
            .unwrap()
            .join()
            .unwrap()
    });
    assert_eq!(commit.generation(), 2);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert_eq!(
        db.reserved_memory_bytes(),
        crate::catalog_snapshot::REGISTRY_BYTES + db.path_memory_bytes()
    );
}

#[test]
fn catalog_streaming_append_cleanup_failure_requires_recovery() {
    use crate::catalog_snapshot::AppendLimits;
    for cut in 0..3 {
        let (_fixture, db, table) = streaming_database();
        let cancel = CancellationToken::new();
        let mut append = db
            .catalog_writer()
            .unwrap()
            .begin_append(
                table,
                AppendLimits {
                    batches: 2,
                    encoded_bytes: 100_000,
                },
                &cancel,
                &mut Effects::default(),
            )
            .unwrap();
        let token = append.transaction();
        for _ in 0..2 {
            append
                .write(&append_columns(), &cancel, &mut Effects::default())
                .unwrap();
        }
        assert!(matches!(
            append.abort(&mut Effects::with_faults(Faults {
                fail_at: Some(cut),
                ..Faults::default()
            })),
            Err(Error::RecoveryRequired { .. })
        ));
        assert!(
            db.reserved_temp_bytes() > 0,
            "cleanup cut {cut} retains debt"
        );
        assert!(db.catalog_writer().is_err());
        let path = db.path().to_owned();
        let config = db.config();
        db.close().unwrap();
        let db = Database::open(&path, config).unwrap();
        assert_eq!(
            db.resolve_catalog(token, &mut Effects::default()).unwrap(),
            crate::CommitResolution::Aborted
        );
        assert_eq!(fs::read_dir(path.join(UNITS_NAME)).unwrap().count(), 3);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(db.generation(), 1);
    }
}

#[test]
fn catalog_streaming_append_commit_failure_cleans_all_five_objects() {
    use crate::catalog_snapshot::AppendLimits;
    let cancel = CancellationToken::new();
    let limits = AppendLimits {
        batches: 2,
        encoded_bytes: 100_000,
    };
    let (_baseline, db, table) = streaming_database();
    let mut append = db
        .catalog_writer()
        .unwrap()
        .begin_append(table, limits, &cancel, &mut Effects::default())
        .unwrap();
    for _ in 0..2 {
        append
            .write(&append_columns(), &cancel, &mut Effects::default())
            .unwrap();
    }
    let trace = Arc::new(Mutex::new(Vec::new()));
    let captured = trace.clone();
    append
        .commit(
            &cancel,
            &mut Effects::with_faults(Faults {
                action: Some(Box::new(move |index, effect| {
                    captured.lock().unwrap().push((index, effect))
                })),
                ..Faults::default()
            }),
        )
        .unwrap();
    let trace = trace.lock().unwrap();
    let last_create = trace
        .iter()
        .rfind(|(_, effect)| {
            *effect == Effect::CreateMetadata(crate::effects::MetadataKind::CatalogObject)
        })
        .unwrap()
        .0;
    let last_sync = trace
        .iter()
        .rfind(|(_, effect)| {
            *effect == Effect::SyncMetadata(crate::effects::MetadataKind::CatalogObject)
        })
        .unwrap()
        .0;
    let directory_sync = trace
        .iter()
        .find(|(_, effect)| *effect == Effect::SyncDirectory(DirectoryKind::Units))
        .unwrap()
        .0;
    for cut in [last_create, last_sync, directory_sync] {
        let (_fixture, db, table) = streaming_database();
        let mut append = db
            .catalog_writer()
            .unwrap()
            .begin_append(table, limits, &cancel, &mut Effects::default())
            .unwrap();
        let token = append.transaction();
        for _ in 0..2 {
            append
                .write(&append_columns(), &cancel, &mut Effects::default())
                .unwrap();
        }
        assert!(
            append
                .commit(
                    &cancel,
                    &mut Effects::with_faults(Faults {
                        fail_at: Some(cut),
                        ..Faults::default()
                    })
                )
                .is_err(),
            "commit construction cut {cut}"
        );
        assert_eq!(
            db.resolve_catalog(token, &mut Effects::default()).unwrap(),
            crate::CommitResolution::Aborted
        );
        assert_eq!(fs::read_dir(db.path().join(UNITS_NAME)).unwrap().count(), 3);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(
            db.reserved_memory_bytes(),
            crate::catalog_snapshot::REGISTRY_BYTES + db.path_memory_bytes()
        );
        assert_eq!(db.generation(), 1);
    }
}

#[test]
fn catalog_named_append_maps_stored_identities_and_reopens() {
    use crate::catalog_snapshot::{AppendLimits, REGISTRY_BYTES};
    let (_fixture, db, _) = streaming_database();
    let path = db.path().to_owned();
    let config = db.config();
    let cancel = CancellationToken::new();
    let limits = AppendLimits {
        batches: 2,
        encoded_bytes: 100_000,
    };
    let initial = fs::read(path.join(WAL_NAME)).unwrap();
    for name in [
        "missing",
        "",
        "bad name",
        "abcdefghijklmnopqrstuvwxyzabcdefg",
    ] {
        assert!(
            db.catalog_writer()
                .unwrap()
                .begin_append_named(name, limits, &cancel, &mut Effects::default())
                .is_err()
        );
        assert_eq!(fs::read(path.join(WAL_NAME)).unwrap(), initial);
        assert_eq!(
            db.reserved_memory_bytes(),
            REGISTRY_BYTES + db.path_memory_bytes()
        );
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    let mut append = db
        .catalog_writer()
        .unwrap()
        .begin_append_named("FaCtS", limits, &cancel, &mut Effects::default())
        .unwrap();
    let transaction = append.transaction();
    assert_eq!(transaction.sequence(), 2);
    for _ in 0..2 {
        append
            .write_columns(
                &positional_append_columns(),
                &cancel,
                &mut Effects::default(),
            )
            .unwrap();
    }
    let commit = append.commit(&cancel, &mut Effects::default()).unwrap();
    assert_eq!(commit.transaction(), transaction);
    db.close().unwrap();
    let db = Database::open(&path, config).unwrap();
    assert_eq!(
        db.resolve_commit(transaction).unwrap(),
        CommitResolution::Durable(commit)
    );
    let expected = vec![
        (Some(1f64.to_bits()), Some("雪".to_owned())),
        (Some((-0f64).to_bits()), Some(String::new())),
        (Some(f64::NAN.to_bits()), Some("abc".to_owned())),
        (Some(42f64.to_bits()), None),
    ];
    assert_eq!(
        snapshot_cells(&db.catalog_snapshot().unwrap(), &path.join(UNITS_NAME)),
        [expected.clone(), expected].concat()
    );
}

#[test]
fn catalog_named_append_invalid_second_batch_requires_abort() {
    use crate::catalog_snapshot::{AppendLimits, ColumnInput, REGISTRY_BYTES};
    let valid = positional_append_columns();
    let wrong_type = [valid[1], valid[0]];
    let wrong_rows = [
        valid[0],
        ColumnInput {
            values: InputValues::Double(&[1.]),
            validity: &[1],
        },
    ];
    let null_nonnullable = [
        valid[0],
        ColumnInput {
            validity: &[7],
            ..valid[1]
        },
    ];
    let invalid_bitmap = [
        ColumnInput {
            validity: &[255],
            ..valid[0]
        },
        valid[1],
    ];
    for bad in [
        &[][..],
        &valid[..1],
        &wrong_type,
        &wrong_rows,
        &null_nonnullable,
        &invalid_bitmap,
    ] {
        let (_fixture, db, _) = streaming_database();
        let cancel = CancellationToken::new();
        let mut append = db
            .catalog_writer()
            .unwrap()
            .begin_append_named(
                "facts",
                AppendLimits {
                    batches: 2,
                    encoded_bytes: 100_000,
                },
                &cancel,
                &mut Effects::default(),
            )
            .unwrap();
        let transaction = append.transaction();
        append
            .write_columns(&valid, &cancel, &mut Effects::default())
            .unwrap();
        let mut effects = Effects::default();
        assert!(append.write_columns(bad, &cancel, &mut effects).is_err());
        assert_eq!(effects.count(), 0);
        assert!(append.write_columns(&valid, &cancel, &mut effects).is_err());
        assert_eq!(effects.count(), 0);
        append.abort(&mut Effects::default()).unwrap();
        assert_eq!(
            db.resolve_commit(transaction).unwrap(),
            CommitResolution::Aborted
        );
        assert_eq!(db.generation(), 1);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(
            db.reserved_memory_bytes(),
            REGISTRY_BYTES + db.path_memory_bytes()
        );
        assert_eq!(fs::read_dir(db.path().join(UNITS_NAME)).unwrap().count(), 3);
    }
}
