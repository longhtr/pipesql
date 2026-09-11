use super::*;
use crate::effects::Faults;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("pipesql-database-{}-{id}", std::process::id()));
        fs::create_dir(&path).expect("create test parent");
        Self(path)
    }

    fn database(&self) -> PathBuf {
        self.0.join("database")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn config() -> Config {
    Config::new(1_048_576, 1_048_576).expect("config")
}

#[test]
fn random_identity_validation_has_no_boxed_diagnostic() {
    match super::database_identity_from_random([0; 16]) {
        Err(super::Error::Io { operation, source }) => {
            assert_eq!(operation, "read database identity");
            assert_eq!(source.kind(), std::io::ErrorKind::InvalidData);
            assert_eq!(source.raw_os_error(), None);
            assert!(source.get_ref().is_none());
        }
        result => panic!("unexpected identity validation: {result:?}"),
    }
    for index in 0..16 {
        let mut bytes = [0; 16];
        bytes[index] = 1;
        assert_eq!(
            *super::database_identity_from_random(bytes)
                .unwrap()
                .as_bytes(),
            bytes
        );
    }
}

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn failed_creation_keeps_lease_until_cleanup_finishes() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    let baseline = TempDir::new();
    let boundary = Arc::new(AtomicU64::new(u64::MAX));
    let capture = Arc::clone(&boundary);
    let mut effects = Effects::with_faults(Faults {
        action: Some(Box::new(move |index, effect| {
            if effect == Effect::SyncDirectory(DirectoryKind::Parent) {
                capture.store(index, Ordering::Relaxed);
            }
        })),
        ..Faults::default()
    });
    Database::create_with_effects(&baseline.database(), config(), &mut effects)
        .unwrap()
        .close()
        .unwrap();
    let fail_at = boundary.load(Ordering::Relaxed);
    assert_ne!(fail_at, u64::MAX);
    let temp = TempDir::new();
    let path = temp.database();
    let peer_path = path.clone();
    let input = temp.0.join("input.tbl");
    fs::write(
        &input,
        b"1|1|1|1|1|100|0.08|0.01|N|O|1994-01-01|a|b|c|d|e|\n",
    )
    .unwrap();
    let committed = Arc::new(AtomicBool::new(false));
    let peer_committed = Arc::clone(&committed);
    let mut attempted = false;
    let mut effects = Effects::with_faults(Faults {
        fail_at: Some(fail_at),
        action: Some(Box::new(move |_, effect| {
            if !attempted && effect == Effect::RemoveCleanupFile {
                attempted = true;
                match Database::open(&peer_path, Config::new(2_000_000, 500_000_000).unwrap()) {
                    Ok(mut peer) => {
                        peer.load_lineitem(&input, &CancellationToken::new())
                            .unwrap();
                        peer_committed.store(true, Ordering::Relaxed);
                        peer.close().unwrap();
                    }
                    Err(Error::Locked) => {}
                    Err(error) => panic!("unexpected competing open: {error}"),
                }
            }
        })),
        ..Faults::default()
    });
    assert!(Database::create_with_effects(&path, config(), &mut effects).is_err());
    assert!(
        !committed.load(Ordering::Relaxed),
        "creation cleanup admitted a peer commit and then deleted its database"
    );
    assert!(!path.exists());
}

#[test]
fn stale_lease_cannot_recover_a_replacement_namespace() {
    use std::sync::{Arc, Mutex};
    let temp = TempDir::new();
    let path = temp.database();
    Database::create(&path, config()).unwrap().close().unwrap();
    let replacement = Arc::new(Mutex::new(None));
    let held = Arc::clone(&replacement);
    let original = path.clone();
    let moved = temp.0.join("old-database");
    let retained = path.join(PRIVATE_NAME).join(PRIVATE_RECOVERY_NAMES[0]);
    let marker = retained.clone();
    let mut replaced = false;
    let mut effects = Effects::with_faults(Faults {
        action: Some(Box::new(move |_, effect| {
            if !replaced && effect == Effect::LockDatabase {
                fs::rename(&original, &moved).unwrap();
                let database = Database::create(&original, config()).unwrap();
                fs::write(&marker, b"owned by the replacement lease").unwrap();
                *held.lock().unwrap() = Some(database);
                replaced = true;
            }
        })),
        ..Faults::default()
    });
    assert!(matches!(
        Database::open_with_effects(&path, config(), &mut effects),
        Err(Error::Corrupt(_))
    ));
    assert_eq!(
        fs::read(retained).unwrap(),
        b"owned by the replacement lease"
    );
}

#[test]
fn public_create_close_reopen_and_lock_alias() {
    let temp = TempDir::new();
    let path = temp.database();
    let database = Database::create(&path, config()).expect("create");
    assert_eq!(database.path(), fs::canonicalize(&path).unwrap());
    assert_ne!(*database.database_identity().as_bytes(), [0; 16]);
    assert_eq!(database.generation(), 0);
    assert!(matches!(
        Database::open(&path, config()),
        Err(Error::Locked)
    ));
    let alias = temp.0.join("alias");
    std::os::unix::fs::symlink(&path, &alias).expect("symlink alias");
    assert!(matches!(
        Database::open(&alias, config()),
        Err(Error::Locked)
    ));
    database.close().expect("close");
    Database::open(&alias, config())
        .expect("open alias")
        .close()
        .expect("close alias");
}

#[test]
fn wal_hard_link_refuses_before_repairing_an_outside_owner() {
    for truncated in [false, true] {
        let temp = TempDir::new();
        let path = temp.database();
        Database::create(&path, config()).unwrap().close().unwrap();
        let outside = temp.0.join("outside-canary");
        fs::hard_link(path.join(WAL_NAME), &outside).unwrap();
        if truncated {
            fs::write(&outside, []).unwrap();
        } else {
            fs::remove_file(path.join(ROOT_B_NAME)).unwrap();
        }
        let before = fs::read(&outside).unwrap();
        let result = Database::open(&path, config());
        assert!(
            matches!(result, Err(Error::Corrupt(_))),
            "linked mutable fence was admitted"
        );
        assert_eq!(fs::read(&outside).unwrap(), before);
        assert_eq!(path.join(ROOT_B_NAME).exists(), truncated);
        fs::remove_file(outside).unwrap();
        Database::open(&path, config()).unwrap().close().unwrap();
        assert_eq!(
            fs::metadata(path.join(WAL_NAME)).unwrap().len(),
            storage_format::WAL_BYTES as u64
        );
    }
}

#[test]
fn opened_fence_checks_link_ownership_again_before_recovery() {
    for point in [
        Effect::OpenMetadata(MetadataKind::Wal),
        Effect::OpenWalForRecovery,
    ] {
        let temp = TempDir::new();
        let path = temp.database();
        Database::create(&path, config()).unwrap().close().unwrap();
        let wal = path.join(WAL_NAME);
        fs::write(&wal, []).unwrap();
        let outside = temp.0.join("outside-canary");
        let canary = outside.clone();
        let mut linked = false;
        let mut effects = Effects::with_faults(Faults {
            action: Some(Box::new(move |_, effect| {
                if effect == point && !linked {
                    fs::hard_link(&wal, &canary).unwrap();
                    linked = true;
                }
            })),
            ..Faults::default()
        });
        assert!(matches!(
            Database::open_with_effects(&path, config(), &mut effects),
            Err(Error::Corrupt(_) | Error::RecoveryRequired { .. })
        ));
        assert_eq!(fs::read(&outside).unwrap(), []);
        fs::remove_file(outside).unwrap();
        Database::open(&path, config()).unwrap().close().unwrap();
    }
}

#[test]
fn review_reopen_must_retry_an_interrupted_repair_barrier() {
    let temp = TempDir::new();
    let path = temp.database();
    Database::create(&path, config()).unwrap().close().unwrap();
    fs::remove_file(path.join(ROOT_B_NAME)).unwrap();
    let first_barrier = std::sync::Arc::new(AtomicU64::new(0));
    let observed = first_barrier.clone();
    let mut trace = Effects::with_faults(Faults {
        action: Some(Box::new(move |index, effect| {
            if effect == Effect::SyncDirectory(DirectoryKind::Database)
                && observed.load(Ordering::Relaxed) == 0
            {
                observed.store(index, Ordering::Relaxed);
            }
        })),
        ..Faults::default()
    });
    Database::open_with_effects(&path, config(), &mut trace)
        .unwrap()
        .close()
        .unwrap();
    let cut = first_barrier.load(Ordering::Relaxed);
    assert!(cut > 0);
    fs::remove_file(path.join(ROOT_B_NAME)).unwrap();
    let mut failure = Effects::with_faults(Faults {
        fail_at: Some(cut),
        ..Faults::default()
    });
    assert!(matches!(
        Database::open_with_effects(&path, config(), &mut failure),
        Err(Error::RecoveryRequired { .. })
    ));
    assert!(path.join(ROOT_B_NAME).is_file());
    let barriers = std::sync::Arc::new(AtomicU64::new(0));
    let observed = barriers.clone();
    let mut retry = Effects::with_faults(Faults {
        action: Some(Box::new(move |_, effect| {
            if effect == Effect::SyncDirectory(DirectoryKind::Database) {
                observed.fetch_add(1, Ordering::Relaxed);
            }
        })),
        ..Faults::default()
    });
    Database::open_with_effects(&path, config(), &mut retry)
        .unwrap()
        .close()
        .unwrap();
    assert!(
        barriers.load(Ordering::Relaxed) > 0,
        "visible repair bytes cannot prove the failed directory barrier completed"
    );
}

#[test]
fn valid_foreign_root_is_not_repaired_as_replica_damage() {
    let temp = TempDir::new();
    let path = temp.database();
    let foreign = temp.0.join("foreign");
    Database::create(&path, config()).unwrap().close().unwrap();
    Database::create(&foreign, config())
        .unwrap()
        .close()
        .unwrap();
    for source in [foreign.join(ROOT_A_NAME), path.join(ROOT_B_NAME)] {
        let replacement = fs::read(source).unwrap();
        fs::write(path.join(ROOT_A_NAME), &replacement).unwrap();
        assert!(
            matches!(Database::open(&path, config()), Err(Error::Corrupt(_))),
            "a checksum-valid foreign database/replica must fail admission"
        );
        assert_eq!(fs::read(path.join(ROOT_A_NAME)).unwrap(), replacement);
        assert!(!path.join("ROOT.A.next").exists());
    }
}

#[test]
fn configuration_reports_capacity_not_live_reservations() {
    let temp = TempDir::new();
    let path = temp.database();
    let config = Config::new(1_000_000, 2_000_000).unwrap();
    let database = Database::create(&path, config).unwrap();
    let memory = database.reserve_memory(17, "configuration test").unwrap();
    assert_eq!(database.config(), config);
    assert_eq!(
        database.reserved_memory_bytes(),
        17 + database.path.capacity() as u64
    );
    drop(memory);
    database.temporary.reserve(19).unwrap();
    assert_eq!(database.config(), config);
    assert_eq!(database.reserved_temp_bytes(), 19);
    database.temporary.release(19);
    database.close().unwrap();
    let reopened_config = Config::new(3_000_000, 4_000_000).unwrap();
    let database = Database::open(&path, reopened_config).unwrap();
    assert_eq!(database.config(), reopened_config);
    database.close().unwrap();
}

#[test]
fn fallible_path_join_matches_unix_geometry_and_bounds() {
    let parts: &[&[u8]] = &[
        b"", b"/", b"//", b"a", b"a/", b"a//", b".", b"..", b"/a", b"a/b", b"\xff",
    ];
    for parent in parts {
        for child in parts {
            let parent = Path::new(OsStr::from_bytes(parent));
            let child = Path::new(OsStr::from_bytes(child));
            let joined = try_join_path(parent, child).unwrap();
            assert_eq!(joined, parent.join(child));
            assert!(joined.capacity() <= MAX_PATH_BYTES);
        }
    }
    let parent = "x".repeat(MAX_PATH_BYTES - 2);
    assert_eq!(
        try_join_path(Path::new(&parent), "y")
            .unwrap()
            .as_os_str()
            .len(),
        MAX_PATH_BYTES
    );
    assert_eq!(
        try_join_path(Path::new(&parent), "yy").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    let absolute = "/".to_owned() + &"x".repeat(MAX_PATH_BYTES - 1);
    assert_eq!(
        try_join_path(Path::new(&parent), &absolute).unwrap(),
        Path::new(&absolute)
    );
    assert_eq!(
        try_join_path(Path::new(&absolute), "").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn configuration_and_paths_are_bounded() {
    assert!(matches!(Config::new(0, 1), Err(Error::InvalidConfig(_))));
    assert!(matches!(Config::new(1, 0), Err(Error::InvalidConfig(_))));
    assert!(matches!(
        Database::open(Path::new("relative"), config()),
        Err(Error::InvalidPath(_))
    ));
    assert!(matches!(
        Database::open(Path::new("/tmp/../tmp/database"), config()),
        Err(Error::InvalidPath(_))
    ));
    for path in ["/tmp/./database", "/tmp/database/.", "/./tmp/database"] {
        assert!(matches!(
            validate_requested_path(Path::new(path)),
            Err(Error::InvalidPath(_))
        ));
    }
    let oversized = format!("/tmp/{}", "x".repeat(MAX_PATH_BYTES));
    assert!(matches!(
        Database::open(Path::new(&oversized), config()),
        Err(Error::InvalidPath(_))
    ));
    assert!(matches!(
        Database::open(Path::new(OsStr::from_bytes(b"/tmp/bad\0path")), config()),
        Err(Error::InvalidPath(_))
    ));
}

#[test]
fn create_refuses_existing_and_open_does_not_mutate_invalid_namespace() {
    let temp = TempDir::new();
    let path = temp.database();
    fs::create_dir(&path).expect("existing directory");
    assert!(matches!(
        Database::create(&path, config()),
        Err(Error::AlreadyExists)
    ));
    assert!(matches!(
        Database::open(&path, config()),
        Err(Error::Corrupt(_))
    ));
    assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
}

#[test]
fn empty_root_recovery_repairs_missing_corrupt_and_stale_replicas() {
    let temp = TempDir::new();
    let path = temp.database();
    let database = Database::create(&path, config()).unwrap();
    let database_id = database.database_identity();
    database.close().unwrap();
    let expected_a = storage_format::encode_root(storage_format::Root {
        database: database_id,
        issued: 0,
        replica: storage_format::Replica::A,
        state: storage_format::RootState::Empty,
    })
    .unwrap();
    fs::remove_file(path.join(ROOT_A_NAME)).unwrap();
    Database::open(&path, config()).unwrap().close().unwrap();
    assert_eq!(fs::read(path.join(ROOT_A_NAME)).unwrap(), expected_a);

    fs::write(path.join(ROOT_A_NAME), &expected_a[..100]).unwrap();
    Database::open(&path, config()).unwrap().close().unwrap();
    assert_eq!(fs::read(path.join(ROOT_A_NAME)).unwrap(), expected_a);

    let mut corrupt = expected_a;
    corrupt[200] ^= 0x80;
    fs::write(path.join(ROOT_A_NAME), corrupt).unwrap();
    Database::open(&path, config()).unwrap().close().unwrap();
    assert_eq!(fs::read(path.join(ROOT_A_NAME)).unwrap(), expected_a);

    fs::write(path.join("ROOT.A.next"), expected_a).unwrap();
    Database::open(&path, config()).unwrap().close().unwrap();
    assert!(!path.join("ROOT.A.next").exists());

    let mut unresolved = expected_a;
    unresolved[200] ^= 1;
    fs::write(path.join(ROOT_A_NAME), unresolved).unwrap();
    fs::write(path.join(WAL_NAME), [0_u8; storage_format::WAL_BYTES]).unwrap();
    assert!(matches!(
        Database::open(&path, config()),
        Err(Error::Corrupt(_))
    ));
    assert_eq!(fs::metadata(path.join(WAL_NAME)).unwrap().len(), 512);
    fs::write(path.join(ROOT_A_NAME), expected_a).unwrap();
    fs::write(path.join(WAL_NAME), []).unwrap();

    let mut bad_a = expected_a;
    bad_a[200] ^= 1;
    let mut bad_b = fs::read(path.join(ROOT_B_NAME)).unwrap();
    bad_b[200] ^= 1;
    fs::write(path.join(ROOT_A_NAME), bad_a).unwrap();
    fs::write(path.join(ROOT_B_NAME), bad_b).unwrap();
    assert!(matches!(
        Database::open(&path, config()),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn empty_root_recovery_removes_bounded_uncommitted_debris() {
    let temp = TempDir::new();
    let path = temp.database();
    Database::create(&path, config()).unwrap().close().unwrap();
    fs::write(path.join(WAL_NAME), [0_u8; storage_format::WAL_BYTES]).unwrap();
    fs::write(path.join(UNITS_NAME).join(UNIT_NAME), b"partial").unwrap();
    for name in PRIVATE_RECOVERY_NAMES {
        fs::write(path.join(PRIVATE_NAME).join(name), b"partial").unwrap();
    }
    Database::open(&path, config()).unwrap().close().unwrap();
    assert_eq!(fs::metadata(path.join(WAL_NAME)).unwrap().len(), 512);
    assert_eq!(fs::read_dir(path.join(UNITS_NAME)).unwrap().count(), 0);
    assert_eq!(fs::read_dir(path.join(PRIVATE_NAME)).unwrap().count(), 0);

    fs::write(path.join(PRIVATE_NAME).join("unknown"), []).unwrap();
    assert!(matches!(
        Database::open(&path, config()),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn changed_opened_root_identity_is_not_repairable_damage() {
    let temp = TempDir::new();
    let path = temp.database();
    Database::create(&path, config()).unwrap().close().unwrap();
    let root = path.join(ROOT_A_NAME);
    let foreign = storage_format::encode_root(storage_format::Root {
        database: DatabaseId::new([7; 16]).unwrap(),
        issued: 0,
        replica: storage_format::Replica::A,
        state: storage_format::RootState::Empty,
    })
    .unwrap();
    let replacement = temp.0.join("replacement-root");
    fs::write(&replacement, foreign).unwrap();
    let marker = path.join(PRIVATE_NAME).join(PRIVATE_RECOVERY_NAMES[0]);
    fs::write(&marker, b"retain before admission").unwrap();
    let destination = root.clone();
    let mut effects = Effects::with_faults(Faults {
        action: Some(Box::new(move |_, effect| {
            if effect == Effect::OpenMetadata(MetadataKind::RootA) {
                fs::rename(&replacement, &destination).unwrap();
            }
        })),
        ..Faults::default()
    });
    assert!(matches!(
        Database::open_with_effects(&path, config(), &mut effects),
        Err(Error::Corrupt(_))
    ));
    assert_eq!(fs::read(root).unwrap(), foreign);
    assert_eq!(fs::read(marker).unwrap(), b"retain before admission");
}

#[test]
fn wrong_root_extent_does_not_hide_recognizable_future_state() {
    for length in [
        12,
        storage_format::ROOT_BYTES - 1,
        storage_format::ROOT_BYTES,
        storage_format::ROOT_BYTES + 1,
    ] {
        let temp = TempDir::new();
        let path = temp.database();
        Database::create(&path, config()).unwrap().close().unwrap();
        let root = path.join(ROOT_A_NAME);
        let mut bytes = fs::read(&root).unwrap();
        bytes[8..12].copy_from_slice(&3_u32.to_le_bytes());
        bytes.resize(length, 0);
        fs::write(&root, &bytes).unwrap();
        let marker = path.join(PRIVATE_NAME).join(PRIVATE_RECOVERY_NAMES[0]);
        fs::write(&marker, b"retain before admission").unwrap();
        assert!(
            matches!(
                Database::open(&path, config()),
                Err(Error::UnsupportedVersion(3))
            ),
            "future root length {length} was repaired or misclassified"
        );
        assert_eq!(fs::read(root).unwrap(), bytes);
        assert_eq!(fs::read(marker).unwrap(), b"retain before admission");
    }
}

#[test]
fn trailing_root_bytes_do_not_hide_foreign_identity_or_generation() {
    for future_generation in [false, true] {
        let temp = TempDir::new();
        let path = temp.database();
        let database = Database::create(&path, config()).unwrap();
        let identity = database.database_identity();
        database.close().unwrap();
        let mut bytes = storage_format::encode_root(storage_format::Root {
            issued: 0,
            database: if future_generation {
                identity
            } else {
                DatabaseId::new([7; 16]).unwrap()
            },
            replica: storage_format::Replica::A,
            state: storage_format::RootState::Empty,
        })
        .unwrap();
        if future_generation {
            bytes[40..48].copy_from_slice(&2_u64.to_le_bytes());
            bytes[108..112].fill(0);
            let checksum = storage_format::crc32c(&bytes);
            bytes[108..112].copy_from_slice(&checksum.to_le_bytes());
        }
        let mut extended = bytes.to_vec();
        extended.push(0);
        fs::write(path.join(ROOT_A_NAME), &extended).unwrap();
        let outcome = Database::open(&path, config());
        assert!(if future_generation {
            matches!(outcome, Err(Error::UnsupportedGeneration(2)))
        } else {
            matches!(outcome, Err(Error::Corrupt(_)))
        });
        assert_eq!(fs::read(path.join(ROOT_A_NAME)).unwrap(), extended);
    }
}

#[test]
fn namespace_symlinks_and_unsupported_version_fail_closed() {
    for name in [LOCK_NAME, CONTROL_NAME] {
        let temp = TempDir::new();
        let path = temp.database();
        Database::create(&path, config()).unwrap().close().unwrap();
        let target = temp.0.join("outside");
        fs::write(&target, []).unwrap();
        fs::remove_file(path.join(name)).unwrap();
        std::os::unix::fs::symlink(&target, path.join(name)).unwrap();
        assert!(matches!(
            Database::open(&path, config()),
            Err(Error::Corrupt(_))
        ));
    }
    let temp = TempDir::new();
    let format_zero = temp.database();
    fs::create_dir(&format_zero).unwrap();
    fs::write(format_zero.join(LOCK_NAME), []).unwrap();
    let mut old_control = [0_u8; storage_format::CONTROL_BYTES];
    old_control[..8].copy_from_slice(b"PIPESQL\0");
    old_control[12] = storage_format::CONTROL_BYTES as u8;
    fs::write(format_zero.join(CONTROL_NAME), old_control).unwrap();
    assert!(matches!(
        Database::open(&format_zero, config()),
        Err(Error::UnsupportedVersion(0))
    ));

    let temp = TempDir::new();
    let path = temp.database();
    Database::create(&path, config()).unwrap().close().unwrap();
    fs::write(
        path.join(CONTROL_NAME),
        include_bytes!("../../tests/fixtures/rejected-single-table-format/CONTROL"),
    )
    .unwrap();
    assert!(matches!(
        Database::open(&path, config()),
        Err(Error::UnsupportedVersion(1))
    ));
    // These independently generated prototype namespaces were never public.
    // Retiring their implementation must not make them repairable current data.
    for (version, control) in [
        (
            3,
            include_bytes!("../../tests/fixtures/rejected-multi-table-format/CONTROL"),
        ),
        (
            5,
            include_bytes!("../../tests/fixtures/candidate-multi-table-format/CONTROL"),
        ),
    ] {
        fs::write(path.join(CONTROL_NAME), control).unwrap();
        let a = fs::read(path.join(ROOT_A_NAME)).unwrap();
        let b = fs::read(path.join(ROOT_B_NAME)).unwrap();
        let wal = fs::read(path.join(WAL_NAME)).unwrap();
        assert!(
            matches!(Database::open(&path, config()), Err(Error::UnsupportedVersion(observed)) if observed == version)
        );
        assert_eq!(fs::read(path.join(CONTROL_NAME)).unwrap(), control);
        assert_eq!(fs::read(path.join(ROOT_A_NAME)).unwrap(), a);
        assert_eq!(fs::read(path.join(ROOT_B_NAME)).unwrap(), b);
        assert_eq!(fs::read(path.join(WAL_NAME)).unwrap(), wal);
    }
    let mut bytes =
        include_bytes!("../../tests/fixtures/current-single-table-format/CONTROL").to_vec();
    bytes[8..12].copy_from_slice(&3_u32.to_le_bytes());
    fs::write(path.join(CONTROL_NAME), bytes).unwrap();
    assert!(matches!(
        Database::open(&path, config()),
        Err(Error::UnsupportedVersion(3))
    ));
}

#[test]
fn control_mutations_lengths_and_unexpected_entries_fail_closed() {
    for offset in 0..storage_format::CONTROL_BYTES {
        let temp = TempDir::new();
        let path = temp.database();
        Database::create(&path, config()).unwrap().close().unwrap();
        let control = path.join(CONTROL_NAME);
        let mut bytes = fs::read(&control).unwrap();
        bytes[offset] ^= 0x80;
        fs::write(control, bytes).unwrap();
        assert!(matches!(
            Database::open(&path, config()),
            Err(Error::Corrupt(_) | Error::UnsupportedVersion(_))
        ));
    }
    for length in 0..storage_format::CONTROL_BYTES {
        let temp = TempDir::new();
        let path = temp.database();
        Database::create(&path, config()).unwrap().close().unwrap();
        let control = fs::read(path.join(CONTROL_NAME)).unwrap();
        fs::write(path.join(CONTROL_NAME), &control[..length]).unwrap();
        assert!(matches!(
            Database::open(&path, config()),
            Err(Error::Corrupt(_))
        ));
    }
    let temp = TempDir::new();
    let path = temp.database();
    Database::create(&path, config()).unwrap().close().unwrap();
    let mut trailing = fs::read(path.join(CONTROL_NAME)).unwrap();
    trailing.push(0);
    fs::write(path.join(CONTROL_NAME), trailing).unwrap();
    assert!(matches!(
        Database::open(&path, config()),
        Err(Error::Corrupt(_))
    ));
    let temp = TempDir::new();
    let path = temp.database();
    Database::create(&path, config()).unwrap().close().unwrap();
    fs::write(path.join("unexpected"), []).unwrap();
    assert!(matches!(
        Database::open(&path, config()),
        Err(Error::Corrupt(_))
    ));
    let temp = TempDir::new();
    let path = temp.database();
    Database::create(&path, config()).unwrap().close().unwrap();
    fs::write(path.join(LOCK_NAME), [0]).unwrap();
    assert!(matches!(
        Database::open(&path, config()),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn every_create_effect_refuses_without_silent_debt() {
    let baseline_temp = TempDir::new();
    let baseline_path = baseline_temp.database();
    let mut baseline = Effects::default();
    Database::create_with_effects(&baseline_path, config(), &mut baseline)
        .expect("baseline")
        .close()
        .unwrap();
    assert_eq!(baseline.count(), 78);

    for index in 0..baseline.count() {
        let temp = TempDir::new();
        let path = temp.database();
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(index),
            ..Faults::default()
        });
        let outcome = Database::create_with_effects(&path, config(), &mut effects);
        assert!(outcome.is_err(), "effect {index} unexpectedly succeeded");
        assert!(
            !path.exists(),
            "effect {index} left unreported namespace debt"
        );
        Database::create(&path, config()).unwrap().close().unwrap();
    }
}

fn add_empty_generation_debris(path: &Path) {
    fs::write(path.join(WAL_NAME), [0_u8; storage_format::WAL_BYTES]).unwrap();
    fs::write(path.join(UNITS_NAME).join(UNIT_NAME), b"partial").unwrap();
    fs::write(path.join(PRIVATE_NAME).join("UNIT.next"), b"partial").unwrap();
}

#[test]
fn every_recovery_effect_releases_lock_and_remains_healable() {
    let baseline_temp = TempDir::new();
    let baseline_path = baseline_temp.database();
    Database::create(&baseline_path, config())
        .unwrap()
        .close()
        .unwrap();
    add_empty_generation_debris(&baseline_path);
    let mut baseline = Effects::default();
    Database::open_with_effects(&baseline_path, config(), &mut baseline)
        .unwrap()
        .close()
        .unwrap();
    assert_eq!(baseline.count(), 64);

    let temp = TempDir::new();
    let path = temp.database();
    Database::create(&path, config()).unwrap().close().unwrap();
    let mut recovery_required = 0_usize;
    for index in 0..baseline.count() {
        add_empty_generation_debris(&path);
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(index),
            ..Faults::default()
        });
        let outcome = Database::open_with_effects(&path, config(), &mut effects);
        recovery_required += usize::from(matches!(&outcome, Err(Error::RecoveryRequired { .. })));
        assert!(outcome.is_err(), "effect {index} unexpectedly succeeded");
        Database::open(&path, config()).unwrap().close().unwrap();
    }
    assert!(recovery_required > 0);
}

#[test]
fn every_open_and_close_effect_releases_ownership() {
    let temp = TempDir::new();
    let path = temp.database();
    Database::create(&path, config()).unwrap().close().unwrap();
    let mut baseline = Effects::default();
    Database::open_with_effects(&path, config(), &mut baseline)
        .unwrap()
        .close()
        .unwrap();
    assert_eq!(baseline.count(), 54);
    for index in 0..baseline.count() {
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(index),
            ..Faults::default()
        });
        assert!(Database::open_with_effects(&path, config(), &mut effects).is_err());
        Database::open(&path, config()).unwrap().close().unwrap();
    }
    let database = Database::open(&path, config()).unwrap();
    let mut effects = Effects::with_faults(Faults {
        fail_at: Some(0),
        ..Faults::default()
    });
    assert!(database.close_with_effects(&mut effects).is_err());
    Database::open(&path, config()).unwrap().close().unwrap();
}

fn cleanup_fixture(temp: &TempDir) -> PathBuf {
    let root = temp.0.join("private");
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join(UNITS_NAME)).unwrap();
    fs::create_dir(root.join(PRIVATE_NAME)).unwrap();
    fs::write(root.join(UNITS_NAME).join(UNIT_NAME), []).unwrap();
    for name in PRIVATE_RECOVERY_NAMES {
        fs::write(root.join(PRIVATE_NAME).join(name), []).unwrap();
    }
    for name in [
        "ROOT.A.next",
        "ROOT.B.next",
        ROOT_A_NAME,
        ROOT_B_NAME,
        WAL_NAME,
        CONTROL_NAME,
        LOCK_NAME,
    ] {
        fs::write(root.join(name), []).unwrap();
    }
    root
}

#[test]
fn every_cleanup_effect_is_bounded_and_reported() {
    let baseline = TempDir::new();
    let root = cleanup_fixture(&baseline);
    let mut effects = Effects::default();
    cleanup_created_namespace(&root, &mut effects).unwrap();
    assert_eq!(effects.count(), 21);

    for index in 0..effects.count() {
        let temp = TempDir::new();
        let root = cleanup_fixture(&temp);
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(index),
            ..Faults::default()
        });
        assert!(cleanup_created_namespace(&root, &mut effects).is_err());
    }
}

#[test]
fn injected_cleanup_failure_is_reported() {
    let baseline_temp = TempDir::new();
    let baseline_path = baseline_temp.database();
    let mut baseline = Effects::default();
    Database::create_with_effects(&baseline_path, config(), &mut baseline)
        .unwrap()
        .close()
        .unwrap();
    let mut reported = 0_usize;
    let mut retained_debt = 0_usize;
    for primary in 2..baseline.count() {
        let temp = TempDir::new();
        let path = temp.database();
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(primary),
            second_fail_at: Some(primary + 2),
            ..Faults::default()
        });
        if matches!(
            Database::create_with_effects(&path, config(), &mut effects),
            Err(Error::CleanupRequired { .. })
        ) {
            reported += 1;
            retained_debt += usize::from(path.exists());
        }
    }
    assert!(reported > 0);
    assert!(retained_debt > 0);
}

#[test]
fn bounded_short_control_io_fails_and_cleans_or_releases() {
    let baseline_temp = TempDir::new();
    let baseline_path = baseline_temp.database();
    let mut baseline = Effects::default();
    Database::create_with_effects(&baseline_path, config(), &mut baseline)
        .unwrap()
        .close()
        .unwrap();
    let mut create_short_count = 0;
    for index in 0..baseline.count() {
        let temp = TempDir::new();
        let path = temp.database();
        let mut effects = Effects::with_faults(Faults {
            short_at: Some(index),
            ..Faults::default()
        });
        if Database::create_with_effects(&path, config(), &mut effects).is_err() {
            create_short_count += 1;
            assert!(!path.exists());
        }
    }
    assert_eq!(create_short_count, 11);

    let temp = TempDir::new();
    let path = temp.database();
    Database::create(&path, config()).unwrap().close().unwrap();
    let mut baseline_open = Effects::default();
    Database::open_with_effects(&path, config(), &mut baseline_open)
        .unwrap()
        .close()
        .unwrap();
    let mut open_short_count = 0;
    for index in 0..baseline_open.count() {
        let mut effects = Effects::with_faults(Faults {
            short_at: Some(index),
            ..Faults::default()
        });
        if Database::open_with_effects(&path, config(), &mut effects).is_err() {
            open_short_count += 1;
        }
    }
    assert_eq!(open_short_count, 6);
    Database::open(&path, config()).unwrap().close().unwrap();
}
