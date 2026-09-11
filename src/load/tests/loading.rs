use crate::effects::{DirectoryKind, Effect, Effects, Faults, LoadEffect};
use crate::load::LOAD_MEMORY_BYTES;
use crate::load::staging::COLUMNS;
use crate::load::tests::{TempDir, config, independent_namespace_matches, row};
use crate::namespace::validate_namespace;
use crate::namespace::{PRIVATE_NAME, ROOT_A_NAME, ROOT_B_NAME, UNIT_NAME, UNITS_NAME, WAL_NAME};
use crate::storage_format::{self};
use crate::{CancellationToken, CommitResolution, Database, Error};
use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::FileExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const PAYLOAD_OFFSET_BYTES: usize = 28_672;

#[test]
fn staging_corruption_is_rejected_before_publication() {
    for name in COLUMNS.map(|column| column.name) {
        let temp = TempDir::new();
        let path = temp.0.join("database");
        let input = temp.0.join("lineitem.tbl");
        fs::write(&input, row("1", "100", "0.08", "1994-01-01")).unwrap();
        let mut database = Database::create(&path, config(2_000_000, 1_000_000)).unwrap();
        let stage = path.join(PRIVATE_NAME).join(name);
        let mut changed = false;
        let mut effects = Effects::with_faults(Faults {
            action: Some(Box::new(move |_, effect| {
                if !changed && effect == Effect::Load(LoadEffect::OpenStaging) {
                    let file = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(&stage)
                        .unwrap();
                    let mut byte = [0];
                    file.read_exact_at(&mut byte, 0).unwrap();
                    byte[0] ^= 1;
                    file.write_all_at(&byte, 0).unwrap();
                    changed = true;
                }
            })),
            ..Faults::default()
        });
        let result =
            database.load_lineitem_with_effects(&input, &CancellationToken::new(), &mut effects);
        assert!(
            matches!(result, Err(Error::Corrupt("staging checksum mismatch"))),
            "{name}: {result:?}"
        );
        independent_namespace_matches(&path, 0);
        assert_eq!(
            database.reserved_memory_bytes(),
            database.path_memory_bytes()
        );
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
}

#[test]
fn interrupted_load_admission_recovery_requires_reopen() {
    let temp = TempDir::new();
    let path = temp.0.join("database");
    let input = temp.0.join("input.tbl");
    fs::write(
        &input,
        b"1|1|1|1|1|100|0.08|0.01|N|O|1994-01-01|a|b|c|d|e|\n",
    )
    .unwrap();
    let mut database = Database::create(&path, config(2_000_000, 500_000_000)).unwrap();
    fs::remove_file(path.join(ROOT_B_NAME)).unwrap();
    let cut = Arc::new(AtomicU64::new(u64::MAX));
    let observed = Arc::clone(&cut);
    let mut trace = Effects::with_faults(Faults {
        action: Some(Box::new(move |index, effect| {
            if effect == Effect::SyncDirectory(DirectoryKind::Database) {
                observed.store(index, Ordering::Relaxed);
            }
        })),
        ..Faults::default()
    });
    validate_namespace(
        &path,
        database.lease(),
        Some(database.database_identity()),
        &database.memory,
        &mut trace,
    )
    .unwrap();
    let cut = cut.load(Ordering::Relaxed);
    assert_ne!(cut, u64::MAX);
    fs::remove_file(path.join(ROOT_B_NAME)).unwrap();
    let mut failed = Effects::with_faults(Faults {
        fail_at: Some(cut),
        ..Faults::default()
    });
    assert!(matches!(
        database.load_lineitem_with_effects(&input, &CancellationToken::new(), &mut failed),
        Err(Error::RecoveryRequired { .. })
    ));
    assert!(
        path.join(ROOT_B_NAME).exists(),
        "repair is visible despite failed barrier"
    );
    let mut retry = Effects::default();
    assert!(
        matches!(
            database.load_lineitem_with_effects(&input, &CancellationToken::new(), &mut retry),
            Err(Error::Unsupported(_))
        ),
        "same handle resumed after required recovery"
    );
    assert_eq!(retry.count(), 0);
    assert_eq!(
        database.reserved_memory_bytes(),
        database.path_memory_bytes()
    );
    assert_eq!(database.reserved_temp_bytes(), 0);
    database.close().unwrap();
    let mut database = Database::open(&path, config(2_000_000, 500_000_000)).unwrap();
    let commit = database
        .load_lineitem(&input, &CancellationToken::new())
        .unwrap();
    assert_eq!(
        database.resolve_commit(commit.transaction()).unwrap(),
        CommitResolution::Durable(commit)
    );
    database.close().unwrap();
}

#[test]
fn unwind_during_writer_mutation_keeps_handle_unavailable() {
    for admission in [false, true] {
        let temp = TempDir::new();
        let path = temp.0.join("database");
        let input = temp.0.join("input.tbl");
        fs::write(
            &input,
            b"1|1|1|1|1|100|0.08|0.01|N|O|1994-01-01|a|b|c|d|e|\n",
        )
        .unwrap();
        let mut database = Database::create(&path, config(2_000_000, 500_000_000)).unwrap();
        let target = if admission {
            fs::remove_file(path.join(ROOT_B_NAME)).unwrap();
            Effect::SyncDirectory(DirectoryKind::Database)
        } else {
            Effect::Load(LoadEffect::CreateStaging)
        };
        let mut effects = Effects::with_faults(Faults {
            action: Some(Box::new(move |_, effect| {
                if effect == target {
                    panic!("injected internal writer failure");
                }
            })),
            ..Faults::default()
        });
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            database.load_lineitem_with_effects(&input, &CancellationToken::new(), &mut effects)
        }));
        assert!(outcome.is_err());
        assert!(database.needs_reopen());
        assert_eq!(
            database.reserved_memory_bytes(),
            database.path_memory_bytes()
        );
        assert_eq!(database.reserved_temp_bytes() == 0, admission);
        assert!(matches!(
            database.load_lineitem(&input, &CancellationToken::new()),
            Err(Error::Unsupported(_))
        ));
        database.close().unwrap();
        let mut database = Database::open(&path, config(2_000_000, 500_000_000)).unwrap();
        database
            .load_lineitem(&input, &CancellationToken::new())
            .unwrap();
        database.close().unwrap();
    }
}

#[test]
fn changed_database_identity_refuses_before_load_cleanup() {
    let temp = TempDir::new();
    let path = temp.0.join("database");
    let foreign_path = temp.0.join("foreign");
    let mut database = Database::create(&path, config(2_000_000, 500_000_000)).unwrap();
    Database::create(&foreign_path, config(2_000_000, 500_000_000))
        .unwrap()
        .close()
        .unwrap();
    for name in [crate::namespace::CONTROL_NAME, ROOT_A_NAME, ROOT_B_NAME] {
        fs::copy(foreign_path.join(name), path.join(name)).unwrap();
    }
    let marker = path
        .join(PRIVATE_NAME)
        .join(crate::namespace::PRIVATE_RECOVERY_NAMES[0]);
    fs::write(&marker, b"foreign namespace private owner").unwrap();
    let input = temp.0.join("input.tbl");
    fs::write(
        &input,
        b"1|1|1|1|1|100|0.08|0.01|N|O|1994-01-01|a|b|c|d|e|\n",
    )
    .unwrap();
    assert!(matches!(
        database.load_lineitem(&input, &CancellationToken::new()),
        Err(Error::Corrupt(_))
    ));
    assert_eq!(
        fs::read(marker).unwrap(),
        b"foreign namespace private owner"
    );
    assert_eq!(
        database.reserved_memory_bytes(),
        database.path_memory_bytes()
    );
    assert_eq!(database.reserved_temp_bytes(), 0);
    database.close().unwrap();
}

#[test]
fn public_load_publishes_one_unit_and_reopens_generation_one() {
    let temp = TempDir::new();
    let database_path = temp.0.join("database");
    let input_path = temp.0.join("lineitem.tbl");
    fs::write(
        &input_path,
        format!(
            "{}{}",
            row("17.00", "21168.23", "0.04", "1996-03-13"),
            row("-0.0", "inf", "NaN", "1970-01-01")
        ),
    )
    .unwrap();
    let mut database = Database::create(&database_path, config(2_000_000, 1_000_000)).unwrap();
    let commit = database
        .load_lineitem(&input_path, &CancellationToken::new())
        .unwrap();
    assert_eq!(commit.generation(), 1);
    assert_eq!(database.generation(), 1);
    assert_eq!(
        database.reserved_memory_bytes(),
        database.path_memory_bytes()
    );
    assert_eq!(database.reserved_temp_bytes(), 0);
    assert_eq!(
        database.resolve_commit(commit.transaction()).unwrap(),
        CommitResolution::Durable(commit)
    );
    assert!(matches!(
        database.load_lineitem(&input_path, &CancellationToken::new()),
        Err(Error::Unsupported(_))
    ));

    let unit = fs::read(database_path.join(UNITS_NAME).join(UNIT_NAME)).unwrap();
    assert_eq!(unit.len(), 28_672 + 2 * 38);
    let mut offset = 28_672;
    assert_eq!(&unit[offset..offset + 8], &17.0_f64.to_bits().to_le_bytes());
    offset += 8;
    assert_eq!(
        &unit[offset..offset + 8],
        &(-0.0_f64).to_bits().to_le_bytes()
    );
    offset += 8;
    assert_eq!(
        &unit[offset..offset + 8],
        &21_168.23_f64.to_bits().to_le_bytes()
    );
    offset += 8;
    assert_eq!(
        &unit[offset..offset + 8],
        &f64::INFINITY.to_bits().to_le_bytes()
    );
    offset += 8;
    assert_eq!(&unit[offset..offset + 8], &0.04_f64.to_bits().to_le_bytes());
    offset += 8;
    assert_eq!(&unit[offset..offset + 8], &f64::NAN.to_bits().to_le_bytes());
    offset += 8;
    for _ in 0..2 {
        assert_eq!(&unit[offset..offset + 8], &8.0_f64.to_bits().to_le_bytes());
        offset += 8;
    }
    assert_eq!(&unit[offset..offset + 2], b"RR");
    offset += 2;
    assert_eq!(&unit[offset..offset + 2], b"FF");
    offset += 2;
    assert_eq!(&unit[offset..offset + 4], &9_568_i32.to_le_bytes());
    offset += 4;
    assert_eq!(&unit[offset..offset + 4], &0_i32.to_le_bytes());

    database.close().unwrap();
    let database = Database::open(&database_path, config(2_000_000, 1_000_000)).unwrap();
    assert_eq!(database.generation(), 1);
    assert_eq!(
        database.resolve_commit(commit.transaction()).unwrap(),
        CommitResolution::Durable(commit)
    );
    database.close().unwrap();
}

#[test]
fn load_memory_refusal_precedes_namespace_effects() {
    let temp = TempDir::new();
    let path = temp.0.join("database");
    let mut database = Database::create(&path, config(LOAD_MEMORY_BYTES - 1, 1_000_000)).unwrap();
    let marker = path
        .join(PRIVATE_NAME)
        .join(crate::namespace::PRIVATE_RECOVERY_NAMES[0]);
    fs::write(&marker, b"not admitted for cleanup").unwrap();
    let mut effects = Effects::default();
    assert!(matches!(
        database.load_lineitem_with_effects(
            &temp.0.join("missing-input"),
            &CancellationToken::new(),
            &mut effects
        ),
        Err(Error::Resource {
            owner: "lineitem load",
            ..
        })
    ));
    assert_eq!(effects.count(), 0);
    assert!(!database.needs_reopen());
    assert_eq!(fs::read(marker).unwrap(), b"not admitted for cleanup");
    assert_eq!(
        database.reserved_memory_bytes(),
        database.path_memory_bytes()
    );
    assert_eq!(database.reserved_temp_bytes(), 0);
    database.close().unwrap();
}

#[test]
fn cancellation_invalid_input_and_resource_refusal_publish_nothing() {
    let cases = [
        (2_000_000, 1_000_000, b"bad\n".as_slice(), false),
        (LOAD_MEMORY_BYTES - 1, 1_000_000, b"".as_slice(), false),
        (2_000_000, 28_671, b"".as_slice(), false),
        (
            2_000_000,
            28_747,
            b"1|2|3|4|17.00|21168.23|0.04|8|R|F|1996-03-13|12|13|14|15|16|\n".as_slice(),
            false,
        ),
        (2_000_000, 1_000_000, b"".as_slice(), true),
    ];
    for (memory, temporary, input, cancel) in cases {
        let temp = TempDir::new();
        let database_path = temp.0.join("database");
        let input_path = temp.0.join("lineitem.tbl");
        fs::write(&input_path, input).unwrap();
        let mut database = Database::create(&database_path, config(memory, temporary)).unwrap();
        let token = CancellationToken::new();
        if cancel {
            token.cancel();
        }
        assert!(database.load_lineitem(&input_path, &token).is_err());
        assert_eq!(database.generation(), 0);
        assert_eq!(
            database.reserved_memory_bytes(),
            database.path_memory_bytes()
        );
        assert_eq!(database.reserved_temp_bytes(), 0);
        assert_eq!(
            fs::metadata(database_path.join(WAL_NAME)).unwrap().len(),
            512
        );
        assert_eq!(
            fs::read_dir(database_path.join(UNITS_NAME))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            fs::read_dir(database_path.join(PRIVATE_NAME))
                .unwrap()
                .count(),
            0
        );
        database.close().unwrap();
    }
}

#[test]
fn load_crosses_double_and_date_block_boundaries_exactly() {
    const ROWS: u64 = 65_537;
    let temp = TempDir::new();
    let database_path = temp.0.join("database");
    let input_path = temp.0.join("lineitem.tbl");
    let source_row = row("17.00", "21168.23", "0.04", "1996-03-13");
    let mut input = String::new();
    let required = source_row
        .len()
        .checked_mul(usize::try_from(ROWS).unwrap())
        .unwrap();
    input.try_reserve_exact(required).unwrap();
    for _ in 0..ROWS {
        input.push_str(&source_row);
    }
    fs::write(&input_path, input).unwrap();
    let mut database = Database::create(&database_path, config(2_000_000, 6_000_000)).unwrap();
    database
        .load_lineitem(&input_path, &CancellationToken::new())
        .unwrap();
    let unit_path = database_path.join(UNITS_NAME).join(UNIT_NAME);
    let file = File::open(&unit_path).unwrap();
    let mut header = [0_u8; storage_format::HEADER_BYTES];
    let mut descriptor_bytes = [0_u8; storage_format::DESCRIPTOR_BYTES];
    file.read_exact_at(&mut header, 0).unwrap();
    file.read_exact_at(
        &mut descriptor_bytes,
        u64::try_from(storage_format::HEADER_BYTES).unwrap(),
    )
    .unwrap();
    let (metadata, _) = storage_format::decode_unit_metadata(&header, &descriptor_bytes).unwrap();
    assert_eq!(metadata.rows, ROWS);
    assert_eq!(metadata.descriptor_count(), 20);
    let sizes: Vec<u32> = metadata.descriptors[..20]
        .iter()
        .map(|descriptor| descriptor.bytes)
        .collect();
    assert_eq!(
        sizes,
        [
            262_144, 262_144, 8, 262_144, 262_144, 8, 262_144, 262_144, 8, 262_144, 262_144, 8,
            32_768, 32_768, 1, 32_768, 32_768, 1, 262_144, 4,
        ]
    );
    assert_eq!(
        file.metadata().unwrap().len(),
        28_672 + ROWS.checked_mul(38).unwrap()
    );
    database.close().unwrap();
    Database::open(&database_path, config(2_000_000, 6_000_000))
        .unwrap()
        .close()
        .unwrap();
}

#[test]
fn committed_unit_metadata_and_extent_mutations_obey_public_open_contract() {
    let temp = TempDir::new();
    let database_path = temp.0.join("database");
    let input_path = temp.0.join("lineitem.tbl");
    fs::write(&input_path, row("17.00", "21168.23", "0.04", "1996-03-13")).unwrap();
    let mut database = Database::create(&database_path, config(2_000_000, 1_000_000)).unwrap();
    database
        .load_lineitem(&input_path, &CancellationToken::new())
        .unwrap();
    database.close().unwrap();
    let unit_path = database_path.join(UNITS_NAME).join(UNIT_NAME);
    let original = fs::read(&unit_path).unwrap();
    let unit = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&unit_path)
        .unwrap();
    for index in 0..PAYLOAD_OFFSET_BYTES {
        let changed = [original[index] ^ 1];
        unit.write_all_at(&changed, u64::try_from(index).unwrap())
            .unwrap();
        assert!(
            Database::open(&database_path, config(2_000_000, 1_000_000)).is_err(),
            "unit metadata mutation {index} was accepted"
        );
        unit.write_all_at(&original[index..index + 1], u64::try_from(index).unwrap())
            .unwrap();
    }
    let payload_offset = u64::try_from(PAYLOAD_OFFSET_BYTES).unwrap();
    unit.write_all_at(&[original[PAYLOAD_OFFSET_BYTES] ^ 1], payload_offset)
        .unwrap();
    Database::open(&database_path, config(2_000_000, 1_000_000))
        .unwrap()
        .close()
        .unwrap();
    unit.write_all_at(
        &original[PAYLOAD_OFFSET_BYTES..PAYLOAD_OFFSET_BYTES + 1],
        payload_offset,
    )
    .unwrap();

    let exact_length = u64::try_from(original.len()).unwrap();
    unit.set_len(exact_length - 1).unwrap();
    assert!(Database::open(&database_path, config(2_000_000, 1_000_000)).is_err());
    unit.set_len(exact_length).unwrap();
    unit.write_all_at(&original[original.len() - 1..], exact_length - 1)
        .unwrap();
    unit.set_len(exact_length + 1).unwrap();
    assert!(Database::open(&database_path, config(2_000_000, 1_000_000)).is_err());
    unit.set_len(exact_length).unwrap();
    drop(unit);
    Database::open(&database_path, config(2_000_000, 1_000_000))
        .unwrap()
        .close()
        .unwrap();
}

#[test]
fn input_identity_changes_are_detected_between_passes_and_after_build() {
    for mutation in 0_u8..4 {
        let temp = TempDir::new();
        let database_path = temp.0.join("database");
        let input_path = temp.0.join("lineitem.tbl");
        fs::write(&input_path, row("17.00", "21168.23", "0.04", "1996-03-13")).unwrap();
        let mut database = Database::create(&database_path, config(2_000_000, 1_000_000)).unwrap();
        let path = input_path.clone();
        let mut open_count = 0_u8;
        let mut read_count = 0_u8;
        let mut final_count = 0_u8;
        let mut effects = Effects::with_faults(Faults {
            action: Some(Box::new(move |_, effect| match effect {
                Effect::Load(LoadEffect::OpenInput) => {
                    open_count += 1;
                    if open_count == 2 && mutation == 0 {
                        let mut bytes = fs::read(&path).unwrap();
                        bytes[0] = b'9';
                        fs::write(&path, bytes).unwrap();
                    } else if open_count == 2 && mutation == 1 {
                        let replacement = path.with_extension("replacement");
                        let mut bytes = fs::read(&path).unwrap();
                        bytes[0] = b'9';
                        fs::write(&replacement, bytes).unwrap();
                        fs::rename(replacement, &path).unwrap();
                    }
                }
                Effect::Load(LoadEffect::ReadInput) => {
                    read_count += 1;
                    if read_count == 3 && mutation == 2 {
                        OpenOptions::new()
                            .write(true)
                            .open(&path)
                            .unwrap()
                            .set_len(0)
                            .unwrap();
                    }
                }
                Effect::Load(LoadEffect::InspectInputFinal) => {
                    final_count += 1;
                    if final_count == 3 && mutation == 3 {
                        let mut bytes = fs::read(&path).unwrap();
                        bytes[0] = b'9';
                        fs::write(&path, bytes).unwrap();
                    }
                }
                _ => {}
            })),
            ..Faults::default()
        });
        let outcome = database.load_lineitem_with_effects(
            &input_path,
            &CancellationToken::new(),
            &mut effects,
        );
        assert!(
            matches!(outcome, Err(Error::Input { .. })),
            "mutation {mutation} returned {outcome:?}"
        );
        assert_eq!(database.generation(), 0);
        assert_eq!(
            database.reserved_memory_bytes(),
            database.path_memory_bytes()
        );
        assert_eq!(database.reserved_temp_bytes(), 0);
        database.close().unwrap();
        Database::open(&database_path, config(2_000_000, 1_000_000))
            .unwrap()
            .close()
            .unwrap();
    }
}

#[test]
fn empty_input_is_a_valid_committed_empty_unit() {
    let temp = TempDir::new();
    let database_path = temp.0.join("database");
    let input_path = temp.0.join("lineitem.tbl");
    fs::write(&input_path, []).unwrap();
    let mut database = Database::create(&database_path, config(2_000_000, 28_672)).unwrap();
    let commit = database
        .load_lineitem(&input_path, &CancellationToken::new())
        .unwrap();
    assert_eq!(commit.generation(), 1);
    assert_eq!(
        fs::metadata(database_path.join(UNITS_NAME).join(UNIT_NAME))
            .unwrap()
            .len(),
        28_672
    );
    database.close().unwrap();
    Database::open(&database_path, config(2_000_000, 28_672))
        .unwrap()
        .close()
        .unwrap();
}
