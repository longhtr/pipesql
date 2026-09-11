//! Bootstrap, authoritative graph admission, and corruption before repair.
use super::{
    Fixture, append_columns, database, declarations, first_commit, genesis, graph, object, publish,
    second_admission, snapshot_cells, token,
};
use crate::catalog::{self, ObjectId};
use crate::catalog_schema::TableId;
use crate::effects::{Effects, Faults};
use crate::namespace::{ROOT_A_NAME, ROOT_B_NAME, UNITS_NAME, WAL_NAME};
use crate::storage_format::{self, CatalogCommit, Replica, Root};
use crate::{CancellationToken, Commit, CommitResolution, Database, Error};
use pipesql_filesystem as filesystem;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[test]
fn independent_catalog_roots_reach_the_complete_graph_and_history() {
    let fixture = Fixture::new();
    for (id, bytes) in [
        (
            object(3, 1),
            include_bytes!("../../../tests/fixtures/catalog-schema/columns.bin").as_slice(),
        ),
        (
            object(3, 2),
            include_bytes!("../../../tests/fixtures/catalog-schema/native-unit.bin").as_slice(),
        ),
        (
            object(5, 3),
            include_bytes!("../../../tests/fixtures/catalog-roots/table-data.bin").as_slice(),
        ),
        (
            object(5, 4),
            include_bytes!("../../../tests/fixtures/catalog-roots/catalog.bin").as_slice(),
        ),
        (
            object(5, 5),
            include_bytes!("../../../tests/fixtures/catalog-roots/successes.bin").as_slice(),
        ),
    ] {
        fixture.put(&fixture.path(id), bytes);
    }
    for (name, bytes) in [
        (
            ROOT_A_NAME,
            include_bytes!("../../../tests/fixtures/catalog-roots/ROOT.A").as_slice(),
        ),
        (
            ROOT_B_NAME,
            include_bytes!("../../../tests/fixtures/catalog-roots/ROOT.B").as_slice(),
        ),
        (
            WAL_NAME,
            include_bytes!("../../../tests/fixtures/catalog-roots/WAL").as_slice(),
        ),
    ] {
        fixture.put(&fixture.0.join(name), bytes);
    }
    let (snapshot, repair) = fixture.selected();
    assert_eq!(repair, None);
    assert_eq!(snapshot.issued, 6);
    assert_eq!(graph(snapshot).generation(), 2);
    fixture.assert_graph(snapshot, 4);
    assert_eq!(fixture.find(snapshot, 3), Some(1));
    assert_eq!(fixture.find(snapshot, 5), Some(2));
    assert_eq!(fixture.find(snapshot, 4), None);
    assert_eq!(fixture.find(snapshot, 6), None);
    for (replica, bytes) in [
        (
            Replica::A,
            include_bytes!("../../../tests/fixtures/catalog-roots/ROOT.A").as_slice(),
        ),
        (
            Replica::B,
            include_bytes!("../../../tests/fixtures/catalog-roots/ROOT.B").as_slice(),
        ),
    ] {
        let root = storage_format::decode_root(bytes).unwrap();
        assert_eq!(root.replica, replica);
        assert_eq!(storage_format::encode_root(root).unwrap(), bytes);
    }
    assert_eq!(
        storage_format::encode_wal(snapshot).unwrap(),
        include_bytes!("../../../tests/fixtures/catalog-roots/WAL").as_slice()
    );
    let genesis_bytes = include_bytes!("../../../tests/fixtures/catalog-roots/GENESIS");
    let root = storage_format::decode_root(genesis_bytes).unwrap();
    assert_eq!(root.fence(), genesis());
    assert_eq!(
        storage_format::encode_root(root).unwrap(),
        genesis_bytes.as_slice()
    );
}

#[test]
fn catalog_root_fields_and_history_shape_fail_with_matching_checksums() {
    let original = include_bytes!("../../../tests/fixtures/catalog-roots/ROOT.A");
    for (at, value) in [
        (33, 1),
        (40, 0),
        (48, 1),
        (56, 8),
        (72, 1),
        (80, 1),
        (112, 4),
        (120, 1),
        (128, 7),
        (136, 0),
        (140, 0),
        (148, 1),
        (152, 4),
        (160, 0),
        (164, 0),
        (172, 1),
        (176, 1),
    ] {
        let mut bad = *original;
        assert_ne!(bad[at], value, "mutation {at}");
        bad[at] = value;
        bad[108..112].fill(0);
        let crc = storage_format::crc32c(&bad);
        bad[108..112].copy_from_slice(&crc.to_le_bytes());
        assert!(storage_format::decode_root(&bad).is_err(), "mutation {at}");
    }
    for length in 0..original.len() {
        assert!(storage_format::decode_root(&original[..length]).is_err());
    }
    let mut newer = *original;
    newer[8..12].copy_from_slice(&(storage_format::CATALOG_FORMAT_VERSION + 1).to_le_bytes());
    assert!(matches!(
        storage_format::decode_root(&newer[..12]),
        Err(storage_format::FormatError::Version)
    ));
    let catalog = graph(storage_format::decode_root(original).unwrap().fence()).catalog();
    let successes = graph(storage_format::decode_root(original).unwrap().fence()).successes();
    assert!(CatalogCommit::new(database(), 0, token(5), catalog, successes).is_err());
    assert!(CatalogCommit::new(database(), 3, token(5), catalog, successes).is_err());
    assert!(CatalogCommit::new(database(), 2, token(1), catalog, successes).is_err());
}

#[test]
fn catalog_open_preserves_unresolved_graph_when_only_stale_root_survives() {
    // Observe every file and directory, including names created by a bad repair.
    fn namespace(path: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
        let mut entries = Vec::new();
        for directory in [
            Path::new(""),
            Path::new(UNITS_NAME),
            Path::new(crate::namespace::PRIVATE_NAME),
        ] {
            for entry in fs::read_dir(path.join(directory)).unwrap() {
                let entry = entry.unwrap();
                let name = directory.join(entry.file_name());
                let contents = if entry.file_type().unwrap().is_file() {
                    Some(fs::read(entry.path()).unwrap())
                } else {
                    assert!(entry.file_type().unwrap().is_dir());
                    None
                };
                entries.push((name, contents));
            }
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    for prior_table in [false, true] {
        for (stale_name, stale_replica, damaged_name) in [
            (ROOT_A_NAME, Replica::A, ROOT_B_NAME),
            (ROOT_B_NAME, Replica::B, ROOT_A_NAME),
        ] {
            for missing in [false, true] {
                let fixture = Fixture::directory();
                let path = fixture.0.join("database");
                let config = crate::Config::new(4_000_000, 2_000_000).unwrap();
                let db = Database::create_empty(&path, config).unwrap();
                let columns = [crate::ColumnDeclaration {
                    name: "value",
                    data_type: crate::DataType::Int64,
                    nullable: false,
                }];
                let cancel = CancellationToken::new();
                if prior_table {
                    db.declare_table("prior", &columns, &cancel).unwrap();
                }
                let mut stale =
                    storage_format::decode_root(&fs::read(path.join(stale_name)).unwrap()).unwrap();
                let commit = db.declare_table("facts", &columns, &cancel).unwrap();
                db.close().unwrap();
                let newest = fs::read(path.join(damaged_name)).unwrap();
                // Retain the issued prefix but the preceding committed graph:
                // this is the peer before the first committed-root replacement.
                stale.issued = commit.transaction().sequence();
                stale.replica = stale_replica;
                fs::write(
                    path.join(stale_name),
                    storage_format::encode_root(stale).unwrap(),
                )
                .unwrap();
                if missing {
                    fs::remove_file(path.join(damaged_name)).unwrap();
                } else {
                    let mut damaged = newest.clone();
                    damaged[0] ^= 1;
                    fs::write(path.join(damaged_name), damaged).unwrap();
                }
                let before = namespace(&path);
                for _ in 0..2 {
                    assert!(matches!(
                        Database::open(&path, config),
                        Err(Error::Corrupt(
                            "root/fence authority is inconsistent or insufficient"
                        ))
                    ));
                    assert_eq!(namespace(&path), before);
                }
                // Refused opens must release their lease. Restoring only the
                // newer root must retain the receipt and permit ordinary repair.
                fs::write(path.join(damaged_name), newest).unwrap();
                let db = Database::open(&path, config).unwrap();
                assert_eq!(db.generation(), commit.generation());
                assert_eq!(
                    db.resolve_commit(commit.transaction()).unwrap(),
                    CommitResolution::Durable(commit)
                );
                db.prepare("FROM facts |> SELECT value").unwrap();
                assert_eq!(db.reserved_temp_bytes(), 0);
                db.close().unwrap();
            }
        }
    }
}

#[test]
fn selected_graph_damage_refuses_before_root_repair() {
    let fixture = Fixture::new();
    let (first, unit) = first_commit(&fixture);
    let issued = second_admission(&fixture, first);
    let (second, _) = fixture.prepare(issued, Some(unit));
    publish(&fixture, issued, second);
    // A valid older peer makes repair necessary, while WAL selects the new graph.
    let older = storage_format::encode_root(Root {
        database: database(),
        issued: issued.issued,
        replica: Replica::B,
        state: issued.state,
    })
    .unwrap();
    fs::write(fixture.0.join(ROOT_B_NAME), older).unwrap();
    assert_eq!(fixture.selected(), (second, Some(Replica::B)));
    let names = [ROOT_A_NAME, ROOT_B_NAME, WAL_NAME];
    let before = names.map(|name| fs::read(fixture.0.join(name)).unwrap());
    for id in [
        object(3, 1),
        object(3, 2),
        object(5, 2),
        object(5, 3),
        object(5, 4),
        object(5, 5),
    ] {
        let path = fixture.path(id);
        let original = fs::read(&path).unwrap();
        let mut damaged = original.clone();
        damaged[0] ^= 1;
        fs::write(&path, damaged).unwrap();
        assert!(
            fixture.heal_metadata(&mut Effects::default()).is_err(),
            "{id:?}"
        );
        for (index, name) in names.iter().enumerate() {
            assert_eq!(
                fs::read(fixture.0.join(name)).unwrap(),
                before[index],
                "{id:?} {name}"
            );
        }
        assert!(!fixture.0.join("ROOT.B.next").exists());
        fs::write(path, original).unwrap();
    }
    assert_eq!(
        fixture.heal_metadata(&mut Effects::default()).unwrap(),
        second
    );
}

#[test]
fn graph_admission_bounds_and_effect_failures_are_read_only() {
    let fixture = Fixture::new();
    let (first, unit) = first_commit(&fixture);
    let issued = second_admission(&fixture, first);
    let (second, _) = fixture.prepare(issued, Some(unit));
    publish(&fixture, issued, second);
    let mut buffer = vec![0; catalog::SNAPSHOT_SCRATCH_BYTES];
    let mut baseline = Effects::default();
    catalog::validate_snapshot(
        &fixture.objects(),
        second,
        &mut buffer,
        &CancellationToken::new(),
        &mut baseline,
    )
    .unwrap();
    assert!(baseline.count() > 0 && baseline.count() < 128);
    for (mode, cut) in (0..3).flat_map(|mode| (0..baseline.count()).map(move |cut| (mode, cut))) {
        let cancel = Arc::new(CancellationToken::new());
        let peer = cancel.clone();
        let mut effects = match mode {
            0 => Effects::with_faults(Faults {
                fail_at: Some(cut),
                ..Faults::default()
            }),
            1 => Effects::with_faults(Faults {
                short_at: Some(cut),
                ..Faults::default()
            }),
            _ => Effects::with_faults(Faults {
                action: Some(Box::new(move |index, _| {
                    if index == cut {
                        peer.cancel();
                    }
                })),
                ..Faults::default()
            }),
        };
        let result = catalog::validate_snapshot(
            &fixture.objects(),
            second,
            &mut buffer,
            &cancel,
            &mut effects,
        );
        match mode {
            0 => assert!(matches!(result, Err(Error::Io { .. })), "cut {cut}"),
            1 => assert!(
                result.is_ok() || matches!(result, Err(Error::Io { .. })),
                "cut {cut}"
            ),
            _ => assert!(matches!(result, Err(Error::Cancelled)), "cut {cut}"),
        }
    }
    let mut effects = Effects::default();
    assert!(matches!(
        catalog::validate_snapshot(
            &fixture.objects(),
            second,
            &mut buffer[..catalog::SNAPSHOT_SCRATCH_BYTES - 1],
            &CancellationToken::new(),
            &mut effects
        ),
        Err(Error::Resource {
            owner: "catalog snapshot scratch bytes",
            ..
        })
    ));
    assert_eq!(effects.count(), 0);
    catalog::validate_snapshot(
        &fixture.objects(),
        genesis(),
        &mut [],
        &CancellationToken::new(),
        &mut effects,
    )
    .unwrap();
    assert_eq!(effects.count(), 0);
    assert_eq!(fixture.selected(), (second, None));
}

#[test]
fn database_open_resolves_catalog_history_and_owns_its_lease() {
    let fixture = Fixture::new();
    let (first, unit) = first_commit(&fixture);
    let issued = second_admission(&fixture, first);
    let (second, _) = fixture.prepare(issued, Some(unit));
    publish(&fixture, issued, second);
    // Older immutable objects are conservatively retained, including enough
    // names to cross the old 64-record directory boundary.
    for ordinal in 10..90 {
        fixture.put(&fixture.path(object(4, ordinal)), &[]);
    }
    fs::remove_file(fixture.0.join(ROOT_B_NAME)).unwrap();
    let path_bytes = pipesql_filesystem::canonicalize(&fixture.0)
        .unwrap()
        .capacity() as u64;
    let config = crate::Config::new(
        path_bytes
            + catalog::SNAPSHOT_SCRATCH_BYTES as u64
            + crate::catalog_snapshot::REGISTRY_BYTES,
        1_000_000,
    )
    .unwrap();
    let database = Database::open(&fixture.0, config).unwrap();
    assert_eq!(database.generation(), 2);
    assert_eq!(
        database.reserved_memory_bytes(),
        crate::catalog_snapshot::REGISTRY_BYTES + database.path_memory_bytes()
    );
    for (sequence, generation) in [(3, 1), (5, 2)] {
        assert_eq!(
            database.resolve_commit(token(sequence)).unwrap(),
            CommitResolution::Durable(Commit {
                transaction: token(sequence),
                generation
            })
        );
        assert_eq!(
            database.reserved_memory_bytes(),
            crate::catalog_snapshot::REGISTRY_BYTES + database.path_memory_bytes()
        );
    }
    assert_eq!(
        database.resolve_commit(token(4)).unwrap(),
        CommitResolution::Aborted
    );
    assert!(matches!(
        database.resolve_commit(token(6)),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        database.prepare("FROM absent"),
        Err(Error::Bind { .. })
    ));
    assert!(matches!(
        Database::open(&fixture.0, config),
        Err(Error::Locked)
    ));
    database.close().unwrap();
    let reopened = Database::open(&fixture.0, config).unwrap();
    assert_eq!(reopened.generation(), 2);
    reopened.close().unwrap();
    assert_eq!(fixture.selected(), (second, None));
}

#[test]
fn database_catalog_admission_refuses_before_repair() {
    let fixture = Fixture::new();
    let (first, _) = first_commit(&fixture);
    fs::remove_file(fixture.0.join(ROOT_B_NAME)).unwrap();
    let root = fs::read(fixture.0.join(ROOT_A_NAME)).unwrap();
    let wal = fs::read(fixture.0.join(WAL_NAME)).unwrap();
    let small = crate::Config::new(catalog::SNAPSHOT_SCRATCH_BYTES as u64 - 1, 1_000_000).unwrap();
    assert!(matches!(
        Database::open(&fixture.0, small),
        Err(Error::Resource {
            owner: "catalog recovery scratch",
            ..
        })
    ));
    let config = crate::Config::new(2_000_000, 1_000_000).unwrap();
    let schema_path = fixture.path(object(3, 1));
    let schema = fs::read(&schema_path).unwrap();
    fs::write(&schema_path, [0; 64]).unwrap();
    assert!(Database::open(&fixture.0, config).is_err());
    fs::write(&schema_path, schema).unwrap();
    let unexpected = fixture.objects().join("unknown");
    fs::write(&unexpected, []).unwrap();
    assert!(Database::open(&fixture.0, config).is_err());
    fs::remove_file(unexpected).unwrap();
    assert!(!fixture.0.join(ROOT_B_NAME).exists());
    assert_eq!(fs::read(fixture.0.join(ROOT_A_NAME)).unwrap(), root);
    assert_eq!(fs::read(fixture.0.join(WAL_NAME)).unwrap(), wal);
    let database = Database::open(&fixture.0, config).unwrap();
    assert_eq!(database.generation(), graph(first).generation());
    database.close().unwrap();
}

#[test]
fn catalog_namespace_names_and_object_capacity_are_bounded() {
    let fixture = Fixture::new();
    for id in [object(1, 1), object(u64::MAX, u32::MAX)] {
        assert_eq!(ObjectId::from_name(&id.name()).unwrap(), id);
    }
    for name in [
        b"0000000000000000-00000001.obj".as_slice(),
        b"0000000000000001-00000000.obj",
        b"000000000000000A-00000001.obj",
        b"../ROOT.A",
        b"0000000000000001-00000001.obj/",
    ] {
        assert!(ObjectId::from_name(name).is_err());
    }
    fixture.put(&fixture.path(object(1, 1)), &[]);
    fixture.put(&fixture.path(object(1, 2)), &[]);
    let identity = filesystem::symlink_metadata(fixture.objects())
        .unwrap()
        .identity();
    assert!(matches!(
        crate::namespace::inspect_catalog_objects(
            &fixture.objects(),
            identity,
            2,
            1,
            &mut Effects::default()
        ),
        Err(Error::Resource {
            owner: "catalog namespace objects",
            required: 2,
            limit: 1
        })
    ));
    crate::namespace::inspect_catalog_objects(
        &fixture.objects(),
        identity,
        2,
        2,
        &mut Effects::default(),
    )
    .unwrap();
    fs::hard_link(fixture.path(object(1, 1)), fixture.path(object(1, 3))).unwrap();
    assert!(
        crate::namespace::inspect_catalog_objects(
            &fixture.objects(),
            identity,
            2,
            3,
            &mut Effects::default()
        )
        .is_err()
    );
    fs::remove_file(fixture.path(object(1, 3))).unwrap();
    fixture.put(&fixture.path(object(3, 1)), &[]);
    assert!(
        crate::namespace::inspect_catalog_objects(
            &fixture.objects(),
            identity,
            2,
            3,
            &mut Effects::default()
        )
        .is_err()
    );
}

#[test]
fn catalog_bootstrap_builds_a_fresh_database_and_reopens_values() {
    assert_eq!(
        storage_format::encode_catalog_control(database()),
        *include_bytes!("../../../tests/fixtures/catalog-roots/CONTROL")
    );
    let parent = Fixture::directory();
    let path = parent.0.join("fresh");
    let config = crate::Config::new(2_000_000, 1_000_000).unwrap();
    let database =
        Database::create_catalog_with_effects(&path, config, &mut Effects::default()).unwrap();
    let identity = database.database_identity();
    assert_eq!(database.generation(), 0);
    assert_eq!(
        database.reserved_memory_bytes(),
        crate::catalog_snapshot::REGISTRY_BYTES + database.path_memory_bytes()
    );
    assert!(matches!(Database::open(&path, config), Err(Error::Locked)));
    let cancel = CancellationToken::new();
    let created = database
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
    assert_eq!(created.transaction().sequence(), 1);
    let appended = database
        .catalog_writer()
        .unwrap()
        .append(
            TableId::new(17).unwrap(),
            &append_columns(),
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    assert_eq!(appended.transaction().sequence(), 2);
    let expected = vec![
        (Some(1f64.to_bits()), Some("雪".to_owned())),
        (Some((-0f64).to_bits()), Some(String::new())),
        (Some(f64::NAN.to_bits()), Some("abc".to_owned())),
        (Some(42f64.to_bits()), None),
    ];
    assert_eq!(
        snapshot_cells(
            &database.catalog_snapshot().unwrap(),
            &path.join(UNITS_NAME)
        ),
        expected
    );
    database.close().unwrap();
    let database = Database::open(&path, config).unwrap();
    assert_eq!(database.database_identity(), identity);
    assert_eq!(
        snapshot_cells(
            &database.catalog_snapshot().unwrap(),
            &path.join(UNITS_NAME)
        ),
        expected
    );
    for commit in [created, appended] {
        assert_eq!(
            database.resolve_commit(commit.transaction()).unwrap(),
            CommitResolution::Durable(commit)
        );
    }
}

#[test]
fn catalog_bootstrap_effect_failures_clean_before_retry() {
    let parent = Fixture::directory();
    let config = crate::Config::new(2_000_000, 1_000_000).unwrap();
    let path = parent.0.join("control");
    let mut effects = Effects::default();
    Database::create_catalog_with_effects(&path, config, &mut effects)
        .unwrap()
        .close()
        .unwrap();
    let count = effects.count();
    assert!(count < 128);
    for cut in 0..count {
        let path = parent.0.join(format!("cut-{cut}"));
        assert!(
            Database::create_catalog_with_effects(
                &path,
                config,
                &mut Effects::with_faults(Faults {
                    fail_at: Some(cut),
                    ..Faults::default()
                })
            )
            .is_err(),
            "cut {cut}"
        );
        assert!(!path.exists(), "cleanup at cut {cut}");
        Database::create_catalog_with_effects(&path, config, &mut Effects::default())
            .unwrap()
            .close()
            .unwrap();
        Database::open(&path, config).unwrap().close().unwrap();
    }
    let path = parent.0.join("memory-refusal");
    assert!(matches!(
        Database::create_catalog_with_effects(
            &path,
            crate::Config::new(1, 1).unwrap(),
            &mut Effects::default()
        ),
        Err(Error::Resource { .. })
    ));
    assert!(!path.exists());
    println!("catalog_bootstrap_effects={count}");
}
