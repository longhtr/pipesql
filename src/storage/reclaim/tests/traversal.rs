//! Check which files reclamation must preserve for readers and receipt lookups.
//!
//! The production Reachable cursor walks small committed histories. Literal
//! object-ID sets distinguish the current catalog, old query data and pinned
//! receipt history. Releasing a pin after capture must not remove its files from
//! that walk; the next walk can omit files no longer needed.
//!
//! Resource refusal, cancellation and read failures must release the walk's
//! owners without changing directory names. A modified later index page must
//! fail even after the cursor has returned a valid prefix. Actual deletion is
//! tested in the sibling cleanup suite.

use super::super::{Phase, Reachable};
use super::{Directory, append, database, id};
use crate::effects::{Effect, Effects, Faults};
use crate::storage::catalog::ObjectId;
use crate::storage::recovery::UNITS_NAME;
use crate::{
    AppendLimits, CancellationToken, ColumnInput, ColumnValues, CommitResolution, Database, Error,
};
use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

fn collect(
    walk: &mut Reachable<'_, '_>,
    effects: &mut Effects,
) -> Result<BTreeSet<ObjectId>, Error> {
    let mut found = BTreeSet::new();
    for _ in 0..100 {
        match walk.next(&CancellationToken::new(), effects)? {
            Some(object) => {
                found.insert(object);
            }
            None => return Ok(found),
        }
    }
    panic!("small reference fixture exceeded output bound");
}

fn names(db: &Database) -> BTreeSet<std::ffi::OsString> {
    std::fs::read_dir(db.path().join(UNITS_NAME))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect()
}

fn descriptors() -> usize {
    std::fs::read_dir("/dev/fd").unwrap().count()
}

#[test]
fn empty_database_finishes_without_io() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("database"),
        crate::Config::new(2_000_000, 2_000_000).unwrap(),
    )
    .unwrap();
    let writer = db.catalog_maintenance().unwrap();
    let baseline = db.reserved_memory_bytes();
    let mut effects = Effects::default();
    let mut walk = Reachable::open(&writer).unwrap();
    assert!(collect(&mut walk, &mut effects).unwrap().is_empty());
    assert_eq!(effects.count(), 0);
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(walk.next(&cancel, &mut effects).unwrap().is_none());
    drop(walk);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    writer.finish().unwrap();
    db.close().unwrap();
}

#[test]
fn current_and_pinned_views_preserve_shared_objects_without_retaining_released_views() {
    let directory = Directory::new();
    let db = database(&directory);
    let empty = db.prepare("FROM facts").unwrap();
    let first = append(&db, 1);
    let old = db.prepare("FROM facts").unwrap();
    let second = append(&db, 2);
    assert_eq!(
        (
            first.transaction().sequence(),
            second.transaction().sequence()
        ),
        (2, 3)
    );
    let before = names(&db);
    let writer = db.catalog_maintenance().unwrap();
    let baseline = db
        .reserved_memory_bytes()
        .checked_sub(empty.accounted_memory_bytes())
        .unwrap()
        .checked_sub(old.accounted_memory_bytes())
        .unwrap();
    let fds = descriptors();
    let mut walk = Reachable::open(&writer).unwrap();
    // Releasing these queries must not alter the walk's captured object set.
    // Maintenance keeps that set safe until this traversal finishes.
    drop(empty);
    drop(old);
    let mut found = BTreeSet::new();
    for _ in 0..100 {
        match walk
            .next(&CancellationToken::new(), &mut Effects::default())
            .unwrap()
        {
            Some(object) => {
                found.insert(object);
            }
            None => break,
        }
        assert!(
            descriptors() <= fds + 1,
            "only one retained index descriptor"
        );
    }
    assert!(matches!(walk.phase, Phase::Finished));
    assert_eq!(
        found,
        [
            (1, 1),
            (1, 2),
            (2, 1),
            (2, 2),
            (2, 3),
            (3, 1),
            (3, 2),
            (3, 3),
            (3, 4)
        ]
        .map(|(a, o)| id(a, o))
        .into()
    );
    assert!(
        walk.next(&CancellationToken::new(), &mut Effects::default())
            .unwrap()
            .is_none()
    );
    println!(
        "reference_walk_inline_bytes={} peak_charge={}",
        std::mem::size_of::<Reachable<'_, '_>>(),
        db.reserved_memory_bytes().checked_sub(baseline).unwrap()
    );
    drop(walk);
    assert_eq!(descriptors(), fds);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    let mut walk = Reachable::open(&writer).unwrap();
    assert_eq!(
        collect(&mut walk, &mut Effects::default()).unwrap(),
        [(1, 1), (2, 1), (3, 1), (3, 2), (3, 3), (3, 4)]
            .map(|(a, o)| id(a, o))
            .into()
    );
    drop(walk);
    assert_eq!(names(&db), before);
    writer.finish().unwrap();
}

#[test]
fn in_flight_resolution_protects_its_history_across_publication() {
    let directory = Directory::new();
    let db = Rc::new(database(&directory));
    let first = append(&db, 1);
    let captured = Rc::new(RefCell::new(None));
    let output = captured.clone();
    let shared = db.clone();
    // Pause resolution at its first file read, publish, then inspect protection
    // before resolution resumes. This is a controlled interleaving on one thread.
    let mut fired = false;
    let mut effects = Effects::with_faults(Faults {
        action: Some(Box::new(move |_, effect| {
            if !fired && effect == Effect::ReadMetadata(crate::effects::MetadataKind::CatalogObject)
            {
                fired = true;
                append(&shared, 2);
                let writer = shared.catalog_maintenance().unwrap();
                let mut walk = Reachable::open(&writer).unwrap();
                *output.borrow_mut() = Some(collect(&mut walk, &mut Effects::default()).unwrap());
                drop(walk);
                writer.finish().unwrap();
            }
        })),
        ..Faults::default()
    });
    assert_eq!(
        db.resolve_catalog(first.transaction(), &mut effects)
            .unwrap(),
        CommitResolution::Durable(first)
    );
    assert_eq!(
        captured.borrow().as_ref().unwrap(),
        &[(1, 1), (2, 1), (2, 4), (3, 1), (3, 2), (3, 3), (3, 4)]
            .map(|(a, o)| id(a, o))
            .into()
    );
    drop(effects);
    let writer = db.catalog_maintenance().unwrap();
    let mut walk = Reachable::open(&writer).unwrap();
    assert!(
        !collect(&mut walk, &mut Effects::default())
            .unwrap()
            .contains(&id(2, 4))
    );
    drop(walk);
    writer.finish().unwrap();
}

#[test]
fn refusal_cancellation_and_metadata_failures_are_read_only_and_release_owners() {
    let directory = Directory::new();
    let db = database(&directory);
    append(&db, 1);
    let before = names(&db);
    let writer = db.catalog_maintenance().unwrap();
    let baseline = db.reserved_memory_bytes();
    let held = db
        .reserve_memory(
            db.config().memory_limit_bytes() - baseline - Reachable::HEAP_ADMISSION + 1,
            "force walk refusal",
        )
        .unwrap();
    assert!(matches!(
        Reachable::open(&writer),
        Err(Error::Resource {
            owner: "reclamation reference walk",
            ..
        })
    ));
    drop(held);
    let mut trace = Effects::default();
    let mut walk = Reachable::open(&writer).unwrap();
    collect(&mut walk, &mut trace).unwrap();
    drop(walk);
    for cut in 0..trace.count() {
        let mut walk = Reachable::open(&writer).unwrap();
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(cut),
            ..Faults::default()
        });
        assert!(collect(&mut walk, &mut effects).is_err(), "read cut {cut}");
        let stopped = effects.count();
        assert!(matches!(
            walk.next(&CancellationToken::new(), &mut effects),
            Err(Error::Corrupt("reference walk has failed"))
        ));
        assert_eq!(effects.count(), stopped);
        drop(walk);
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    let cancel = CancellationToken::new();
    cancel.cancel();
    let mut walk = Reachable::open(&writer).unwrap();
    assert!(matches!(
        walk.next(&cancel, &mut Effects::default()),
        Err(Error::Cancelled)
    ));
    drop(walk);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(names(&db), before);
    writer.finish().unwrap();
    db.close().unwrap();
}

#[test]
fn paged_index_corruption_never_turns_a_partial_prefix_into_completion() {
    let directory = Directory::new();
    let db = database(&directory);
    let cancel = CancellationToken::new();
    let mut transaction = db
        .begin_append(
            "facts",
            AppendLimits {
                batches: 65,
                encoded_bytes: 1_000_000,
            },
            &cancel,
        )
        .unwrap();
    for value in 0..65 {
        transaction
            .write(
                &[ColumnInput {
                    values: ColumnValues::Int64(&[value]),
                    validity: &[1],
                }],
                &cancel,
            )
            .unwrap();
    }
    let commit = transaction.commit(&cancel).unwrap();
    let writer = db.catalog_maintenance().unwrap();
    let baseline = db.reserved_memory_bytes();
    let mut walk = Reachable::open(&writer).unwrap();
    let found = collect(&mut walk, &mut Effects::default()).unwrap();
    assert_eq!(found.len(), 69);
    for ordinal in 1..=65 {
        assert!(found.contains(&id(commit.transaction().sequence(), ordinal)));
    }
    drop(walk);
    let mut walk = Reachable::open(&writer).unwrap();
    let mut effects = Effects::default();
    for _ in 0..5 {
        walk.next(&cancel, &mut effects).unwrap().unwrap();
    }

    // Admission has checked the full index; the cursor currently holds page one.
    // Change page two to test validation when it is reread, not initial admission.
    let index = walk.views[walk.view]
        .commit
        .unwrap()
        .transaction()
        .sequence();
    let name = id(index, 66).name();
    let path = walk.objects.join(std::str::from_utf8(&name).unwrap());
    let original = std::fs::read(&path).unwrap();
    let mut damaged = original.clone();
    damaged[crate::storage::table::HEADER as usize + crate::storage::table::SCRATCH_BYTES] ^= 1;
    std::fs::write(&path, &damaged).unwrap();
    let error = collect(&mut walk, &mut effects).unwrap_err();
    assert!(matches!(
        error,
        Error::Corrupt("table-data page changed after admission")
    ));
    assert!(matches!(walk.phase, Phase::Failed));
    assert!(walk.next(&cancel, &mut effects).is_err());
    drop(walk);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(std::fs::read(&path).unwrap(), damaged);
    std::fs::write(&path, &original).unwrap();
    writer.finish().unwrap();
}
