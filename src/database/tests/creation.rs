//! Require creation to verify exactly the empty database it meant to write.
//!
//! Both legacy and catalog creation run through production initialization.
//! Cases corrupt newly written bytes, add pending names, change issuance or
//! replace the leased directory. Before/after byte snapshots and effect traces
//! check that validation performs no recovery writes and cleanup retains its
//! lease without deleting a replacement database.
//!
//! The issued-state control is valid for reopen but invalid for fresh creation;
//! accepting it here would hide a broken initializer. Fixtures come from the
//! parent `database::tests` module, and each case owns its mutation and expected
//! outcome. These checks do not substitute for independent format fixtures.

use super::*;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

fn create(path: &Path, catalog: bool, effects: &mut Effects) -> Database {
    if catalog {
        Database::create_catalog_with_effects(path, config(), effects)
    } else {
        Database::create_with_effects(path, config(), effects)
    }
    .unwrap()
}

fn initial(database: &Database, catalog: bool) -> storage_format::WalRecord {
    storage_format::WalRecord {
        database: database.database_identity(),
        issued: 0,
        state: if catalog {
            storage_format::RootState::Catalog(None)
        } else {
            storage_format::RootState::Empty
        },
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut found = BTreeMap::new();
    for directory in [
        root.to_path_buf(),
        root.join(UNITS_NAME),
        root.join(PRIVATE_NAME),
    ] {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let bytes = if entry.file_type().unwrap().is_dir() {
                None
            } else {
                Some(fs::read(entry.path()).unwrap())
            };
            found.insert(
                entry.path().strip_prefix(root).unwrap().to_path_buf(),
                bytes,
            );
        }
    }
    found
}

// Mutate a valid generation-zero namespace into a settled issued prefix of one.
// Literal offsets and a bitwise CRC keep this counterexample independent of the
// production encoder and snapshot-selection algorithm.
fn issue_one(root: &Path) {
    for name in [ROOT_A_NAME, ROOT_B_NAME, WAL_NAME] {
        let path = root.join(name);
        let mut bytes = fs::read(&path).unwrap();
        bytes[112..120].copy_from_slice(&1_u64.to_le_bytes());
        bytes[108..112].fill(0);
        let mut crc = !0_u32;
        for byte in &bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0x82f6_3b78 & 0_u32.wrapping_sub(crc & 1));
            }
        }
        bytes[108..112].copy_from_slice(&(!crc).to_le_bytes());
        fs::write(path, bytes).unwrap();
    }
}

#[test]
fn creation_retains_initial_file_and_directory_barriers_without_recovery() {
    for catalog in [false, true] {
        let temp = TempDir::new();
        let trace = Rc::new(RefCell::new(Vec::new()));
        let recorded = Rc::clone(&trace);
        let mut effects = Effects::with_faults(Faults {
            action: Some(Box::new(move |_, effect| {
                recorded.borrow_mut().push(effect)
            })),
            ..Faults::default()
        });
        create(&temp.database(), catalog, &mut effects)
            .close()
            .unwrap();
        let trace = trace.borrow();
        let barriers: Vec<_> = trace
            .iter()
            .copied()
            .filter(|effect| {
                matches!(
                    effect,
                    Effect::SyncLock | Effect::SyncMetadata(_) | Effect::SyncDirectory(_)
                )
            })
            .collect();
        assert_eq!(
            barriers,
            [
                Effect::SyncLock,
                Effect::SyncDirectory(DirectoryKind::Units),
                Effect::SyncDirectory(DirectoryKind::Private),
                Effect::SyncMetadata(MetadataKind::Wal),
                Effect::SyncMetadata(MetadataKind::Control),
                Effect::SyncMetadata(MetadataKind::RootA),
                Effect::SyncMetadata(MetadataKind::RootB),
                Effect::SyncDirectory(DirectoryKind::Database),
                Effect::SyncDirectory(DirectoryKind::Parent),
            ]
        );
        assert!(!trace.contains(&Effect::OpenWalForRecovery));
        assert!(!trace.contains(&Effect::RenameRepairedRoot));
        for kind in [
            MetadataKind::Wal,
            MetadataKind::Control,
            MetadataKind::RootA,
            MetadataKind::RootB,
        ] {
            assert_eq!(
                trace
                    .iter()
                    .filter(|&&effect| effect == Effect::WriteMetadata(kind))
                    .count(),
                1
            );
        }
    }
}

#[test]
fn created_validation_refuses_noninitial_names_bytes_and_authority_without_writes() {
    for catalog in [false, true] {
        for mutation in [
            "root-missing",
            "root-torn",
            "wal-torn",
            "pending-root",
            "scratch",
            "stage",
            "unit",
            "control",
            "lease",
            "database",
            "issued",
        ] {
            let temp = TempDir::new();
            let root = temp.database();
            let database = create(&root, catalog, &mut Effects::default());
            let mut expected = initial(&database, catalog);
            match mutation {
                "root-missing" => fs::remove_file(root.join(ROOT_A_NAME)).unwrap(),
                "root-torn" => fs::write(root.join(ROOT_B_NAME), []).unwrap(),
                "wal-torn" => fs::write(root.join(WAL_NAME), []).unwrap(),
                "pending-root" => fs::write(root.join("ROOT.A.next"), b"partial").unwrap(),
                "scratch" => fs::write(root.join(PRIVATE_NAME).join("SCRATCH.A"), []).unwrap(),
                "stage" => {
                    fs::write(root.join(PRIVATE_NAME).join("UNIT.next"), b"partial").unwrap()
                }
                "unit" => fs::write(root.join(UNITS_NAME).join(UNIT_NAME), b"partial").unwrap(),
                "control" => fs::write(root.join(CONTROL_NAME), b"invalid").unwrap(),
                "lease" => {
                    fs::rename(root.join(LOCK_NAME), temp.0.join("held-lock")).unwrap();
                    fs::write(root.join(LOCK_NAME), []).unwrap();
                }
                "database" => expected.database = DatabaseId::new([7; 16]).unwrap(),
                "issued" => issue_one(&root),
                _ => unreachable!(),
            }
            let before = snapshot(&root);
            let memory = database.reserved_memory_bytes();
            let result = validate_created_namespace(
                &root,
                &database.lease,
                &expected,
                &database.memory,
                &mut Effects::default(),
            );
            assert!(result.is_err(), "{catalog} {mutation}");
            assert_eq!(snapshot(&root), before, "{catalog} {mutation}");
            assert_eq!(database.reserved_memory_bytes(), memory);
            database.close().unwrap();
            if mutation == "issued" {
                // This is valid recovery state, but it is not the requested genesis.
                Database::open(&root, config()).unwrap().close().unwrap();
            }
        }
    }
}

#[test]
fn creation_validation_failure_cleans_up_while_holding_the_lease() {
    for catalog in [false, true] {
        let temp = TempDir::new();
        let root = temp.database();
        let changed = root.clone();
        let peer_checked = Rc::new(RefCell::new(false));
        let observed = Rc::clone(&peer_checked);
        let mut corrupted = false;
        let mut effects = Effects::with_faults(Faults {
            action: Some(Box::new(move |_, effect| {
                if !corrupted && effect == Effect::InspectDatabaseDirectory {
                    fs::write(changed.join(WAL_NAME), []).unwrap();
                    corrupted = true;
                }
                if effect == Effect::RemoveCleanupFile && !*observed.borrow() {
                    assert!(matches!(
                        Database::open(&changed, config()),
                        Err(Error::Locked)
                    ));
                    *observed.borrow_mut() = true;
                }
                assert_ne!(effect, Effect::OpenWalForRecovery);
                assert_ne!(effect, Effect::RenameRepairedRoot);
            })),
            ..Faults::default()
        });
        let result = if catalog {
            Database::create_catalog_with_effects(&root, config(), &mut effects)
        } else {
            Database::create_with_effects(&root, config(), &mut effects)
        };
        assert!(result.is_err());
        assert!(*peer_checked.borrow());
        assert!(!root.exists());
    }
}

#[test]
fn failed_creation_does_not_clean_a_replacement_lease() {
    let temp = TempDir::new();
    let root = temp.database();
    let changed = root.clone();
    let moved = temp.0.join("original-creation");
    let replacement = Rc::new(RefCell::new(None));
    let held = Rc::clone(&replacement);
    let before = Rc::new(RefCell::new(None));
    let captured = Rc::clone(&before);
    let mut replaced = false;
    let mut effects = Effects::with_faults(Faults {
        action: Some(Box::new(move |_, effect| {
            if !replaced && effect == Effect::InspectDatabaseDirectory {
                fs::rename(&changed, &moved).unwrap();
                *held.borrow_mut() = Some(Database::create_empty(&changed, config()).unwrap());
                *captured.borrow_mut() = Some(snapshot(&changed));
                replaced = true;
            }
        })),
        ..Faults::default()
    });
    assert!(matches!(
        Database::create_with_effects(&root, config(), &mut effects),
        Err(Error::CleanupRequired { .. })
    ));
    assert!(
        root.exists(),
        "creation cleanup deleted the replacement namespace"
    );
    assert_eq!(snapshot(&root), *before.borrow().as_ref().unwrap());
    replacement.borrow_mut().take().unwrap().close().unwrap();
}

#[test]
fn failed_cleanup_identity_inspection_preserves_created_files() {
    for catalog in [false, true] {
        let baseline = TempDir::new();
        let boundary = Rc::new(RefCell::new(None));
        let captured = Rc::clone(&boundary);
        let mut effects = Effects::with_faults(Faults {
            action: Some(Box::new(move |index, effect| {
                if effect == Effect::SyncDirectory(DirectoryKind::Parent) {
                    *captured.borrow_mut() = Some(index);
                }
            })),
            ..Faults::default()
        });
        create(&baseline.database(), catalog, &mut effects)
            .close()
            .unwrap();
        let fail_at = boundary.borrow().unwrap();
        // Cleanup must inspect both the database directory and its LOCK entry
        // before the held descriptor can authorize deletion through this path.
        for inspection in [1, 2] {
            let temp = TempDir::new();
            let root = temp.database();
            let observed = root.clone();
            let before = Rc::new(RefCell::new(None));
            let captured = Rc::clone(&before);
            let mut effects = Effects::with_faults(Faults {
                fail_at: Some(fail_at),
                second_fail_at: Some(fail_at + inspection),
                action: Some(Box::new(move |_, effect| {
                    if effect == Effect::SyncDirectory(DirectoryKind::Parent) {
                        *captured.borrow_mut() = Some(snapshot(&observed));
                    }
                    assert_ne!(effect, Effect::RemoveCleanupFile);
                })),
                ..Faults::default()
            });
            let result = if catalog {
                Database::create_catalog_with_effects(&root, config(), &mut effects)
            } else {
                Database::create_with_effects(&root, config(), &mut effects)
            };
            assert!(matches!(result, Err(Error::CleanupRequired { .. })));
            assert_eq!(snapshot(&root), *before.borrow().as_ref().unwrap());
            Database::open(&root, config()).unwrap().close().unwrap();
        }
    }
}
