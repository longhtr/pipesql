//! Check table declaration, construction cleanup and recovery after failed cleanup.
//!
//! The internal writer accepts either explicit schema identities or declarations
//! whose identities it assigns. Tests inspect persisted schemas and old snapshots,
//! then reopen to verify committed identities and transaction receipts. Invalid
//! names, duplicate columns and cancellation must release the writer without
//! consuming an attempt number.
//!
//! A literal byte budget tests refusal before issuance. Construction failures
//! must remove new objects and allow another declaration. If unlinking or directory
//! synchronization also fails, the writer must keep its temporary charge and
//! require recovery. Reopen tests interrupt that cleanup again, then verify a
//! successful retry without changing any object in the committed graph.
//!
//! The 64-column case also runs on a thread whose reported stack size is checked.
//! This tests that declaration path under the requested stack bound, not the stack
//! needs of every operation or platform.

use super::{Fixture, declarations, object, token};
use crate::effects::{DirectoryKind, Effect, Effects, Faults};
use crate::storage::catalog;
use crate::storage::recovery::{UNITS_NAME, WAL_NAME};
use crate::storage::schema::{self, TableId};
use crate::{CancellationToken, CommitResolution, Database, Error, TransactionId};
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[test]
fn catalog_engine_creates_declared_tables_and_preserves_old_views() {
    let fixture = Fixture::new();
    let config = crate::Config::new(2_000_000, 1_000_000).unwrap();
    let database = Database::open(fixture.root(), config).unwrap();
    let cancel = CancellationToken::new();
    let first = database
        .catalog_writer()
        .unwrap()
        .create_table(
            "facts",
            TableId::new(17).unwrap(),
            &declarations(),
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let old = database.catalog_snapshot().unwrap();
    let second = database
        .catalog_writer()
        .unwrap()
        .create_table(
            "other",
            TableId::new(9).unwrap(),
            &declarations(),
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let mut buffer = [0; catalog::MAX_BYTES];
    let catalog = old
        .read_catalog(&mut buffer, &cancel, &mut Effects::default())
        .unwrap()
        .unwrap();
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog.table(0).unwrap().name(), "facts");
    assert_eq!(catalog.table(0).unwrap().rows(), 0);
    let mut schema_bytes = [0; schema::MAX_BYTES];
    let stored = catalog
        .read_schema(
            &fixture.objects(),
            0,
            &mut schema_bytes,
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    assert_eq!(stored.table(), TableId::new(17).unwrap());
    assert_eq!(stored.column(1).unwrap(), declarations()[1]);
    let before = fs::read(fixture.root().join(WAL_NAME)).unwrap();
    assert!(
        database
            .catalog_writer()
            .unwrap()
            .create_table(
                "FACTS",
                TableId::new(18).unwrap(),
                &declarations(),
                &cancel,
                &mut Effects::default()
            )
            .is_err()
    );
    assert_eq!(fs::read(fixture.root().join(WAL_NAME)).unwrap(), before);
    assert_eq!(database.reserved_temp_bytes(), 0);
    for commit in [first, second] {
        assert_eq!(
            database.resolve_commit(commit.transaction()).unwrap(),
            CommitResolution::Durable(commit)
        );
    }
    drop(old);
    database.close().unwrap();
    let database = Database::open(fixture.root(), config).unwrap();
    let view = database.catalog_snapshot().unwrap();
    let catalog = view
        .read_catalog(&mut buffer, &cancel, &mut Effects::default())
        .unwrap()
        .unwrap();
    assert_eq!(catalog.len(), 2);
    assert_eq!(catalog.table(0).unwrap().id(), TableId::new(9).unwrap());
    assert_eq!(catalog.table(1).unwrap().id(), TableId::new(17).unwrap());
    assert_eq!(
        database.resolve_commit(first.transaction()).unwrap(),
        CommitResolution::Durable(first)
    );
}

#[test]
fn catalog_engine_create_reserves_publication_peak_before_issuance() {
    // The peak holds a 160-byte schema, 192-byte catalog, 72-byte history and
    // one 4096-byte replacement root. Test the literal 4520-byte total and one
    // byte less, rather than deriving the expectation from admission code.
    for limit in [4519, 4520] {
        let fixture = Fixture::new();
        let database = Database::open(
            fixture.root(),
            crate::Config::new(2_000_000, limit).unwrap(),
        )
        .unwrap();
        let before = fs::read(fixture.root().join(WAL_NAME)).unwrap();
        let result = database.catalog_writer().unwrap().create_table(
            "facts",
            TableId::new(17).unwrap(),
            &declarations(),
            &CancellationToken::new(),
            &mut Effects::default(),
        );
        if limit == 4519 {
            assert!(matches!(
                result,
                Err(Error::Resource { required: 4520, .. })
            ));
            assert_eq!(fs::read(fixture.root().join(WAL_NAME)).unwrap(), before);
            assert_eq!(fs::read_dir(fixture.objects()).unwrap().count(), 0);
        } else {
            assert_eq!(result.unwrap().transaction(), token(3));
            assert_eq!(fs::read_dir(fixture.objects()).unwrap().count(), 3);
        }
        assert_eq!(database.reserved_temp_bytes(), 0);
        database.catalog_writer().unwrap().abort_unbuilt().unwrap();
    }
}

#[test]
fn catalog_engine_create_cleans_every_construction_failure() {
    let config = crate::Config::new(2_000_000, 1_000_000).unwrap();
    let baseline = Fixture::new();
    let database = Database::open(baseline.root(), config).unwrap();
    let trace = Arc::new(Mutex::new(Vec::new()));
    let observed = trace.clone();
    let mut effects = Effects::with_faults(Faults {
        action: Some(Box::new(move |index, effect| {
            observed.lock().unwrap().push((index, effect));
        })),
        ..Faults::default()
    });
    database
        .catalog_writer()
        .unwrap()
        .create_table(
            "facts",
            TableId::new(17).unwrap(),
            &declarations(),
            &CancellationToken::new(),
            &mut effects,
        )
        .unwrap();
    let trace = trace.lock().unwrap();
    let start = trace
        .iter()
        .position(|(_, effect)| {
            *effect == Effect::CreateMetadata(crate::effects::MetadataKind::CatalogObject)
        })
        .unwrap();
    // Cover construction through the first units-directory sync. Publication
    // and issuance failures have their own suites.
    let end = start
        + trace[start..]
            .iter()
            .position(|(_, effect)| *effect == Effect::SyncDirectory(DirectoryKind::Units))
            .unwrap();
    assert!(end - start < 64);
    let first_sync = trace[start..=end]
        .iter()
        .find(|(_, effect)| {
            *effect == Effect::SyncMetadata(crate::effects::MetadataKind::CatalogObject)
        })
        .unwrap()
        .0;
    for &(cut, _) in &trace[start..=end] {
        let fixture = Fixture::new();
        let database = Database::open(fixture.root(), config).unwrap();
        let result = database.catalog_writer().unwrap().create_table(
            "facts",
            TableId::new(17).unwrap(),
            &declarations(),
            &CancellationToken::new(),
            &mut Effects::with_faults(Faults {
                fail_at: Some(cut),
                ..Faults::default()
            }),
        );
        assert!(result.is_err(), "cut {cut}");
        assert_eq!(database.reserved_temp_bytes(), 0, "cut {cut}");
        assert_eq!(
            fs::read_dir(fixture.objects()).unwrap().count(),
            0,
            "cut {cut}"
        );
        assert_eq!(
            database.resolve_commit(token(3)).unwrap(),
            CommitResolution::Aborted
        );
        let retry = database
            .catalog_writer()
            .unwrap()
            .create_table(
                "facts",
                TableId::new(17).unwrap(),
                &declarations(),
                &CancellationToken::new(),
                &mut Effects::default(),
            )
            .unwrap();
        assert_eq!(retry.transaction(), token(4));
    }
    // Add a cleanup failure after the first object-sync failure. Removing a
    // name is not enough: if the directory sync fails, keep the construction
    // charge because deletion has not been established as durable.
    for offset in [1, 3] {
        let fixture = Fixture::new();
        let database = Database::open(fixture.root(), config).unwrap();
        let result = database.catalog_writer().unwrap().create_table(
            "facts",
            TableId::new(17).unwrap(),
            &declarations(),
            &CancellationToken::new(),
            &mut Effects::with_faults(Faults {
                fail_at: Some(first_sync),
                second_fail_at: Some(first_sync + offset),
                ..Faults::default()
            }),
        );
        assert!(matches!(result, Err(Error::CleanupRequired { .. })));
        assert!(database.reserved_temp_bytes() > 0);
        assert!(matches!(
            database.catalog_writer(),
            Err(Error::RecoveryRequired { .. })
        ));
        database.close().unwrap();
        let reopened = Database::open(fixture.root(), config).unwrap();
        assert_eq!(fs::read_dir(fixture.objects()).unwrap().count(), 0);
        assert_eq!(
            reopened.resolve_commit(token(3)).unwrap(),
            CommitResolution::Aborted
        );
        reopened.catalog_writer().unwrap().abort_unbuilt().unwrap();
    }
    println!("catalog_create_construction_effects={}", end - start + 1);
}

#[test]
fn catalog_engine_create_recovery_retries_unlink_and_sync() {
    let config = crate::Config::new(2_000_000, 1_000_000).unwrap();
    let prepare = || {
        let fixture = Fixture::new();
        let database = Database::open(fixture.root(), config).unwrap();
        database
            .catalog_writer()
            .unwrap()
            .create_table(
                "facts",
                TableId::new(17).unwrap(),
                &declarations(),
                &CancellationToken::new(),
                &mut Effects::default(),
            )
            .unwrap();
        let mut writer = database.catalog_writer().unwrap();
        writer
            .issue(&CancellationToken::new(), &mut Effects::default())
            .unwrap();
        // Write an unfinished object for the issued attempt, then abandon the
        // writer. Recovery may delete this object, but not the committed graph.
        fixture.put(&fixture.path(object(4, 1)), &[1]);
        drop(writer);
        database.close().unwrap();
        fixture
    };
    let baseline = prepare();
    let trace = Arc::new(Mutex::new(Vec::new()));
    let observed = trace.clone();
    Database::open_with_effects(
        baseline.root(),
        config,
        &mut Effects::with_faults(Faults {
            action: Some(Box::new(move |index, effect| {
                observed.lock().unwrap().push((index, effect));
            })),
            ..Faults::default()
        }),
    )
    .unwrap()
    .close()
    .unwrap();
    let trace = trace.lock().unwrap();
    let start = trace
        .iter()
        .position(|(_, effect)| *effect == Effect::RemoveCleanupFile)
        .unwrap();
    let end = start
        + trace[start..]
            .iter()
            .position(|(_, effect)| *effect == Effect::SyncDirectory(DirectoryKind::Units))
            .unwrap();
    assert_eq!(end - start, 2);
    for &(cut, _) in &trace[start..=end] {
        let fixture = prepare();
        let rooted: Vec<_> = (1..=3)
            .map(|ordinal| fs::read(fixture.path(object(3, ordinal))).unwrap())
            .collect();
        assert!(
            Database::open_with_effects(
                fixture.root(),
                config,
                &mut Effects::with_faults(Faults {
                    fail_at: Some(cut),
                    ..Faults::default()
                })
            )
            .is_err()
        );
        for (ordinal, bytes) in (1..=3).zip(&rooted) {
            assert_eq!(&fs::read(fixture.path(object(3, ordinal))).unwrap(), bytes);
        }
        // The failed open may already have removed the name. A retry still
        // has to synchronize the directory before reporting recovery complete.
        let barriers = Arc::new(AtomicU64::new(0));
        let observed = barriers.clone();
        let database = Database::open_with_effects(
            fixture.root(),
            config,
            &mut Effects::with_faults(Faults {
                action: Some(Box::new(move |_, effect| {
                    if effect == Effect::SyncDirectory(DirectoryKind::Units) {
                        observed.fetch_add(1, Ordering::Relaxed);
                    }
                })),
                ..Faults::default()
            }),
        )
        .unwrap();
        assert!(barriers.load(Ordering::Relaxed) > 0);
        assert!(!fixture.path(object(4, 1)).exists());
        assert_eq!(
            database.resolve_commit(token(4)).unwrap(),
            CommitResolution::Aborted
        );
        assert!(matches!(
            database.resolve_commit(token(3)).unwrap(),
            CommitResolution::Durable(_)
        ));
    }
}

#[test]
fn catalog_declaration_assigns_identities_across_abort_and_reopen() {
    use crate::ColumnDeclaration;
    let parent = Fixture::directory();
    let path = parent.root().join("declared-identities");
    let config = crate::Config::new(4_000_000, 2_000_000).unwrap();
    let db = Database::create_catalog_with_effects(&path, config, &mut Effects::default()).unwrap();
    let cancel = CancellationToken::new();
    let columns = [
        ColumnDeclaration {
            name: "zebra",
            data_type: crate::DataType::String,
            nullable: true,
        },
        ColumnDeclaration {
            name: "alpha",
            data_type: crate::DataType::Int64,
            nullable: false,
        },
    ];
    let first = db
        .catalog_writer()
        .unwrap()
        .declare_table("facts", &columns, &cancel, &mut Effects::default())
        .unwrap();
    let old = db.catalog_snapshot().unwrap();
    let mut writer = db.catalog_writer().unwrap();
    let aborted = writer.issue(&cancel, &mut Effects::default()).unwrap();
    writer.abort_unbuilt().unwrap();
    let second = db
        .catalog_writer()
        .unwrap()
        .declare_table("other", &columns, &cancel, &mut Effects::default())
        .unwrap();
    let mut bytes = [0; catalog::MAX_BYTES];
    assert_eq!(
        old.read_catalog(&mut bytes, &cancel, &mut Effects::default())
            .unwrap()
            .unwrap()
            .len(),
        1
    );
    drop(old);
    db.close().unwrap();
    let db = Database::open(&path, config).unwrap();
    assert_eq!(
        db.resolve_commit(aborted).unwrap(),
        CommitResolution::Aborted
    );
    for commit in [first, second] {
        assert_eq!(
            db.resolve_commit(commit.transaction()).unwrap(),
            CommitResolution::Durable(commit)
        );
    }
    let snapshot = db.catalog_snapshot().unwrap();
    let catalog = snapshot
        .read_catalog(&mut bytes, &cancel, &mut Effects::default())
        .unwrap()
        .unwrap();
    assert_eq!(catalog.len(), 2);
    let mut schema_bytes = [0; schema::MAX_BYTES];
    // The aborted attempt consumes table identity 2. Later declarations must
    // not recycle it, while each new schema starts its column IDs at 1.
    for (ordinal, expected_id) in [1, 3].into_iter().enumerate() {
        assert_eq!(catalog.table(ordinal).unwrap().id().value(), expected_id);
        let schema = catalog
            .read_schema(
                &path.join(UNITS_NAME),
                ordinal,
                &mut schema_bytes,
                &cancel,
                &mut Effects::default(),
            )
            .unwrap();
        for (index, expected) in columns.iter().enumerate() {
            let actual = schema.column(index).unwrap();
            assert_eq!(actual.id().value(), (index + 1) as u32);
            assert_eq!(actual.name(), expected.name);
            assert_eq!(actual.data_type(), expected.data_type);
            assert_eq!(actual.nullable(), expected.nullable);
        }
    }
}

#[test]
fn catalog_declaration_rejection_releases_writer_without_issuance() {
    use crate::ColumnDeclaration;
    use crate::storage::snapshot::REGISTRY_BYTES;
    let parent = Fixture::directory();
    let path = parent.root().join("rejected-declarations");
    let config = crate::Config::new(4_000_000, 2_000_000).unwrap();
    let db = Database::create_catalog_with_effects(&path, config, &mut Effects::default()).unwrap();
    let cancel = CancellationToken::new();
    let valid = ColumnDeclaration {
        name: "value",
        data_type: crate::DataType::Int64,
        nullable: false,
    };
    let invalid = ColumnDeclaration {
        name: "bad name",
        ..valid
    };
    for columns in [
        &[][..],
        &[invalid][..],
        &[valid, valid][..],
        &[valid; 65][..],
    ] {
        assert!(
            db.catalog_writer()
                .unwrap()
                .declare_table("facts", columns, &cancel, &mut Effects::default())
                .is_err()
        );
        assert_eq!(db.generation(), 0);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(
            db.reserved_memory_bytes(),
            REGISTRY_BYTES + db.path_memory_bytes()
        );
    }
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        db.catalog_writer().unwrap().declare_table(
            "facts",
            &[valid],
            &cancelled,
            &mut Effects::default()
        ),
        Err(Error::Cancelled)
    ));
    let first = db
        .catalog_writer()
        .unwrap()
        .declare_table("facts", &[valid], &cancel, &mut Effects::default())
        .unwrap();
    assert!(
        db.catalog_writer()
            .unwrap()
            .declare_table("FACTS", &[valid], &cancel, &mut Effects::default())
            .is_err()
    );
    let second = db
        .catalog_writer()
        .unwrap()
        .declare_table("other", &[valid], &cancel, &mut Effects::default())
        .unwrap();
    assert_eq!(
        first.transaction(),
        TransactionId::for_attempt(db.database_identity(), 1).unwrap()
    );
    assert_eq!(
        second.transaction(),
        TransactionId::for_attempt(db.database_identity(), 2).unwrap()
    );
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert_eq!(
        db.reserved_memory_bytes(),
        REGISTRY_BYTES + db.path_memory_bytes()
    );
}

#[test]
fn catalog_declaration_preserves_maximum_columns() {
    check_maximum_declaration(false);
}

#[test]
fn catalog_declaration_maximum_columns_has_bounded_reported_stack() {
    check_maximum_declaration(true);
}

fn check_maximum_declaration(small_stack: bool) {
    use crate::ColumnDeclaration;
    let parent = Fixture::directory();
    let path = parent.root().join("maximum-declaration");
    let config = crate::Config::new(4_000_000, 2_000_000).unwrap();
    let db = Database::create_catalog_with_effects(&path, config, &mut Effects::default()).unwrap();
    let names: Vec<_> = (0..64).map(|i| format!("column_{i}")).collect();
    let columns: Vec<_> = names
        .iter()
        .map(|name| ColumnDeclaration {
            name,
            data_type: crate::DataType::Int64,
            nullable: false,
        })
        .collect();
    let thread = if small_stack {
        std::thread::Builder::new().stack_size(pipesql_filesystem::TEST_SMALL_STACK_REQUEST_BYTES)
    } else {
        std::thread::Builder::new()
    };
    std::thread::scope(|scope| {
        thread
            .spawn_scoped(scope, || {
                if small_stack {
                    pipesql_filesystem::test_assert_small_stack();
                }
                db.catalog_writer()
                    .unwrap()
                    .declare_table(
                        "wide",
                        &columns,
                        &CancellationToken::new(),
                        &mut Effects::default(),
                    )
                    .unwrap();
            })
            .unwrap()
            .join()
            .unwrap();
    });
    let snapshot = db.catalog_snapshot().unwrap();
    let mut bytes = [0; catalog::MAX_BYTES];
    let catalog = snapshot
        .read_catalog(
            &mut bytes,
            &CancellationToken::new(),
            &mut Effects::default(),
        )
        .unwrap()
        .unwrap();
    let mut schema_bytes = [0; schema::MAX_BYTES];
    let schema = catalog
        .read_schema(
            &path.join(UNITS_NAME),
            0,
            &mut schema_bytes,
            &CancellationToken::new(),
            &mut Effects::default(),
        )
        .unwrap();
    assert_eq!(schema.len(), 64);
    assert_eq!(schema.column(63).unwrap().id().value(), 64);
}
