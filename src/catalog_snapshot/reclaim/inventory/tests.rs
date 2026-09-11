//! External inventory membership, resource bounds, corruption, and effect cuts.
use super::super::tests::{Directory, append, database, id};
use super::{
    CHUNK_RECORDS, FAN_IN, Inventory, MAX_PAGES, MAX_RECORDS, MAX_RUNS, PAGE_BYTES, RECORD_BYTES,
    State,
};
use crate::catalog::ObjectId;
use crate::effects::{Effects, Faults};
use crate::namespace::UNITS_NAME;
use crate::{CancellationToken, Database, Error};
use std::collections::BTreeSet;
use std::fs::File;
use std::os::unix::fs::FileExt;

fn scratch(directory: &Directory) -> [File; 2] {
    std::array::from_fn(|slot| {
        let path = directory.0.join(format!("inventory-scratch-{slot}"));
        let file = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(path).unwrap();
        file
    })
}

fn garbage(
    inventory: &mut Inventory<'_, '_>,
    effects: &mut Effects,
) -> Result<BTreeSet<ObjectId>, Error> {
    let mut result = BTreeSet::new();
    for _ in 0..=crate::namespace::MAX_CATALOG_OBJECTS {
        match inventory.next(&CancellationToken::new(), effects)? {
            Some(id) => {
                assert!(result.insert(id));
            }
            None => return Ok(result),
        }
    }
    panic!("inventory output exceeds namespace bound");
}

fn roomy_database(directory: &Directory) -> Database {
    Database::create_empty(
        &directory.0,
        crate::Config::new(2_000_000, 100_000_000).unwrap(),
    )
    .unwrap()
}

#[test]
fn public_graph_inventory_separates_garbage_from_current_and_pinned_objects() {
    let directory = Directory::new();
    let db = database(&directory);
    append(&db, 1);
    let old = db.prepare("FROM facts").unwrap();
    append(&db, 2);
    let writer = db.catalog_maintenance().unwrap();
    let baseline = db.reserved_memory_bytes();
    let mut effects = Effects::default();
    let mut inventory = Inventory::new(&writer, scratch(&directory), &mut effects).unwrap();
    assert!(
        inventory
            .next(&CancellationToken::new(), &mut effects)
            .is_err()
    );
    inventory
        .scan(&CancellationToken::new(), &mut effects)
        .unwrap();
    assert_eq!(
        garbage(&mut inventory, &mut effects).unwrap(),
        [(1, 2), (1, 3), (2, 4)].map(|(a, o)| id(a, o)).into()
    );
    println!(
        "inventory_inline_bytes={} charged_memory={} charged_temp={} input_records={} merge_passes={}",
        std::mem::size_of::<Inventory<'_, '_>>(),
        db.reserved_memory_bytes() - baseline,
        db.reserved_temp_bytes(),
        inventory.input_records,
        inventory.merge_passes
    );
    drop(inventory);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    drop(old);
    let mut inventory = Inventory::new(&writer, scratch(&directory), &mut effects).unwrap();
    inventory
        .scan(&CancellationToken::new(), &mut effects)
        .unwrap();
    assert_eq!(
        garbage(&mut inventory, &mut effects).unwrap(),
        [(1, 2), (1, 3), (2, 2), (2, 3), (2, 4)]
            .map(|(a, o)| id(a, o))
            .into()
    );
    drop(inventory);
    writer.finish().unwrap();
    db.close().unwrap();
}

#[test]
fn missing_protected_leaf_refuses_the_complete_inventory() {
    let directory = Directory::new();
    let db = database(&directory);
    append(&db, 1);
    let path = directory
        .0
        .join(UNITS_NAME)
        .join(std::str::from_utf8(&id(2, 1).name()).unwrap());
    std::fs::remove_file(&path).unwrap();
    let writer = db.catalog_maintenance().unwrap();
    let mut effects = Effects::default();
    let mut inventory = Inventory::new(&writer, scratch(&directory), &mut effects).unwrap();
    assert!(matches!(
        inventory.scan(&CancellationToken::new(), &mut effects),
        Err(Error::Corrupt("protected reclamation object is missing"))
    ));
    let count = effects.count();
    assert!(
        inventory
            .next(&CancellationToken::new(), &mut effects)
            .is_err()
    );
    assert_eq!(effects.count(), count);
    drop(inventory);
    writer.finish().unwrap();
}

#[test]
fn external_merge_matches_independent_membership_and_detects_changed_output() {
    let directory = Directory::new();
    let db = roomy_database(&directory);
    let writer = db.catalog_maintenance().unwrap();
    let baseline = db.reserved_memory_bytes();
    let mut effects = Effects::default();
    let cancel = CancellationToken::new();
    let mut inventory = Inventory::new(&writer, scratch(&directory), &mut effects).unwrap();
    let count = CHUNK_RECORDS * FAN_IN + 17;
    for n in (1..=count).rev() {
        inventory
            .push(id(n as u64, 1), false, &cancel, &mut effects)
            .unwrap();
    }
    for n in (1..=count).filter(|n| n % 3 == 0) {
        inventory
            .push(id(n as u64, 1), true, &cancel, &mut effects)
            .unwrap();
        inventory
            .push(id(n as u64, 1), true, &cancel, &mut effects)
            .unwrap();
    }
    inventory.finish(&cancel, &mut effects).unwrap();
    assert_eq!(inventory.merge_passes, 2);
    let expected = (1..=count)
        .filter(|n| n % 3 != 0)
        .map(|n| id(n as u64, 1))
        .collect();
    assert_eq!(garbage(&mut inventory, &mut effects).unwrap(), expected);
    println!(
        "external_input_records={} unique_names={} memory={} temp={} passes={}",
        inventory.input_records,
        count,
        db.reserved_memory_bytes() - baseline,
        db.reserved_temp_bytes(),
        inventory.merge_passes
    );
    inventory.state = State::Ready;
    inventory.next_record = 0;
    inventory.loaded_page = None;
    inventory
        .scratch
        .test_file(inventory.source)
        .write_all_at(&[0xff], PAGE_BYTES as u64 + 7)
        .unwrap();
    assert!(matches!(
        garbage(&mut inventory, &mut effects),
        Err(Error::Corrupt("reclamation inventory page changed"))
    ));
    let stopped = effects.count();
    assert!(inventory.next(&cancel, &mut effects).is_err());
    assert_eq!(effects.count(), stopped);
    drop(inventory);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    writer.finish().unwrap();
}

#[test]
fn duplicate_namespace_and_damaged_input_run_never_finish() {
    let directory = Directory::new();
    let db = roomy_database(&directory);
    let writer = db.catalog_maintenance().unwrap();
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let mut inventory = Inventory::new(&writer, scratch(&directory), &mut effects).unwrap();
    // Put duplicate names in separate runs, so only merging can detect them.
    for n in 1..=CHUNK_RECORDS {
        inventory
            .push(id(n as u64, 1), false, &cancel, &mut effects)
            .unwrap();
    }
    inventory
        .push(id(1, 1), false, &cancel, &mut effects)
        .unwrap();
    assert!(matches!(
        inventory.finish(&cancel, &mut effects),
        Err(Error::Corrupt("duplicate reclamation namespace entry"))
    ));
    assert!(inventory.next(&cancel, &mut effects).is_err());
    drop(inventory);
    let mut inventory = Inventory::new(&writer, scratch(&directory), &mut effects).unwrap();
    for n in 1..=CHUNK_RECORDS + 1 {
        inventory
            .push(id(n as u64, 1), false, &cancel, &mut effects)
            .unwrap();
    }
    inventory
        .scratch
        .test_file(0)
        .write_all_at(&[3], 15)
        .unwrap();
    assert!(matches!(
        inventory.finish(&cancel, &mut effects),
        Err(Error::Corrupt("reclamation run checksum differs"))
    ));
    assert!(inventory.next(&cancel, &mut effects).is_err());
    drop(inventory);
    assert_eq!(db.reserved_temp_bytes(), 0);
    writer.finish().unwrap();
}

#[test]
fn failed_effects_and_resource_refusals_release_scratch() {
    let directory = Directory::new();
    let db = database(&directory);
    append(&db, 1);
    let writer = db.catalog_maintenance().unwrap();
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let mut trace = Effects::default();
    let mut inventory = Inventory::new(&writer, scratch(&directory), &mut trace).unwrap();
    inventory.scan(&cancel, &mut trace).unwrap();
    garbage(&mut inventory, &mut trace).unwrap();
    drop(inventory);
    for cut in 0..trace.count() {
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(cut),
            ..Faults::default()
        });
        let outcome = (|| {
            let mut inventory = Inventory::new(&writer, scratch(&directory), &mut effects)?;
            inventory.scan(&cancel, &mut effects)?;
            garbage(&mut inventory, &mut effects)?;
            Ok::<(), Error>(())
        })();
        assert!(outcome.is_err(), "effect cut {cut}");
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    let remaining = db.memory.limit() - baseline;
    let held = db
        .memory
        .reserve(remaining - Inventory::MEMORY_BYTES + 1, "test pressure")
        .unwrap();
    assert!(matches!(
        Inventory::new(&writer, scratch(&directory), &mut Effects::default()),
        Err(Error::Resource {
            owner: "reclamation inventory",
            ..
        })
    ));
    drop(held);
    db.temporary.reserve(db.temporary.limit()).unwrap();
    let mut inventory =
        Inventory::new(&writer, scratch(&directory), &mut Effects::default()).unwrap();
    assert!(matches!(
        inventory.scan(&cancel, &mut Effects::default()),
        Err(Error::Resource { .. })
    ));
    drop(inventory);
    assert_eq!(db.reserved_temp_bytes(), db.temporary.limit());
    db.temporary.release(db.temporary.limit());
    let mut inventory =
        Inventory::new(&writer, scratch(&directory), &mut Effects::default()).unwrap();
    cancel.cancel();
    assert!(matches!(
        inventory.scan(&cancel, &mut Effects::default()),
        Err(Error::Cancelled)
    ));
    drop(inventory);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    writer.finish().unwrap();
}

#[test]
fn maximum_input_uses_three_merge_passes_with_fixed_memory() {
    let directory = Directory::new();
    let db = roomy_database(&directory);
    let writer = db.catalog_maintenance().unwrap();
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let mut inventory = Inventory::new(&writer, scratch(&directory), &mut effects).unwrap();
    let start = std::time::Instant::now();
    // Synthetic codec/capacity boundary, not a claimed public catalog shape.
    for n in (1..=crate::namespace::MAX_CATALOG_OBJECTS).rev() {
        inventory
            .push(id(n as u64, 1), false, &cancel, &mut effects)
            .unwrap();
    }
    for n in 1..=crate::namespace::MAX_CATALOG_OBJECTS {
        inventory
            .push(id(n as u64, 1), true, &cancel, &mut effects)
            .unwrap();
    }
    for _ in 2 * crate::namespace::MAX_CATALOG_OBJECTS..MAX_RECORDS {
        inventory
            .push(id(1, 1), true, &cancel, &mut effects)
            .unwrap();
    }
    assert_eq!(inventory.input_records, MAX_RECORDS);
    inventory.finish(&cancel, &mut effects).unwrap();
    assert_eq!(inventory.merge_passes, 3);
    assert_eq!(
        inventory.runs[0].records,
        crate::namespace::MAX_CATALOG_OBJECTS
    );
    assert!(inventory.next(&cancel, &mut effects).unwrap().is_none());
    assert_eq!(
        db.reserved_memory_bytes() - baseline,
        Inventory::MEMORY_BYTES
    );
    assert!(db.reserved_temp_bytes() <= (2 * MAX_RECORDS * RECORD_BYTES) as u64);
    println!(
        "maximum_input_records={} runs_bound={} page_checksums_bound={} memory={} temp={} passes={} seconds={:.6}",
        MAX_RECORDS,
        MAX_RUNS,
        MAX_PAGES,
        Inventory::MEMORY_BYTES,
        db.reserved_temp_bytes(),
        inventory.merge_passes,
        start.elapsed().as_secs_f64()
    );
    drop(inventory);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    writer.finish().unwrap();
}

#[test]
fn every_two_run_merge_effect_failure_prevents_completion_and_releases_owners() {
    let directory = Directory::new();
    let db = roomy_database(&directory);
    let writer = db.catalog_maintenance().unwrap();
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let run = |effects: &mut Effects| -> Result<(), Error> {
        let mut inventory = Inventory::new(&writer, scratch(&directory), effects)?;
        for n in (1..=CHUNK_RECORDS + 1).rev() {
            inventory.push(id(n as u64, 1), false, &cancel, effects)?;
        }
        let result = inventory.finish(&cancel, effects);
        if result.is_err() {
            assert!(inventory.next(&cancel, effects).is_err());
        }
        result?;
        garbage(&mut inventory, effects)?;
        Ok(())
    };
    let mut trace = Effects::default();
    run(&mut trace).unwrap();
    for cut in 0..trace.count() {
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(cut),
            ..Faults::default()
        });
        assert!(run(&mut effects).is_err(), "merge effect cut {cut}");
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    println!("two_run_merge_effect_cuts={}", trace.count());
    writer.finish().unwrap();
}

#[test]
fn scratch_admission_rejects_aliasing_linked_and_nonempty_files() {
    let directory = Directory::new();
    let db = database(&directory);
    let writer = db.catalog_maintenance().unwrap();
    let baseline = db.reserved_memory_bytes();
    let [first, second] = scratch(&directory);
    let alias = first.try_clone().unwrap();
    assert!(matches!(
        Inventory::new(&writer, [first, alias], &mut Effects::default()),
        Err(Error::Corrupt("scratch files alias"))
    ));
    drop(second);
    let files = scratch(&directory);
    files[0].write_all_at(&[1], 0).unwrap();
    assert!(matches!(
        Inventory::new(&writer, files, &mut Effects::default()),
        Err(Error::Corrupt("scratch must be empty and unlinked"))
    ));
    let path = directory.0.join("linked-scratch");
    let linked = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    let [first, second] = scratch(&directory);
    assert!(matches!(
        Inventory::new(&writer, [linked, first], &mut Effects::default()),
        Err(Error::Corrupt("scratch must be empty and unlinked"))
    ));
    drop(second);
    std::fs::remove_file(path).unwrap();
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    writer.finish().unwrap();
}
