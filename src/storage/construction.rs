//! Build immutable catalog objects and clean up attempts that never committed.
//!
//! Declaration and append write new files before publishing a catalog that
//! references them. Each name contains the transaction attempt and an ordinal.
//! `create_object` advances the caller's count as soon as creation succeeds;
//! a later write failure must not make cleanup forget that file.
//!
//! Before handing files to the publisher, a builder can remove its own created
//! prefix with `cleanup`. After the handoff, it must leave them for recovery:
//! deleting a possibly committed object could damage the database.
//!
//! `recover_construction` runs during exclusive reopen. It removes objects newer
//! than the selected committed attempt, while rejecting names beyond the issued
//! attempt number. Older unreferenced objects belong to reclamation, which checks
//! the catalog's references. Recovery collects names before deletion so it never
//! mutates a directory while enumerating it.

use crate::effects::{DirectoryKind, Effect, Effects};
use crate::error::io_error;
use crate::path::joined_path;
use crate::storage::catalog::{self, ObjectId};
use crate::storage::format::{RootState, WalRecord};
use crate::storage::recovery::UNITS_NAME;
use crate::{CancellationToken, Error};
use pipesql_filesystem as filesystem;
use std::fs::File;
use std::path::{Path, PathBuf};

const MAX_OBJECTS: u32 = catalog::MAX_UNITS + 3;

pub(super) fn object(attempt: u64, ordinal: u32) -> ObjectId {
    ObjectId::new(attempt, ordinal).expect("admitted attempt and fixed object ordinal")
}

fn object_path(objects: &Path, id: ObjectId) -> Result<PathBuf, Error> {
    joined_path(
        objects,
        std::str::from_utf8(&id.name()).expect("ASCII object name"),
    )
}

pub(super) fn create_object(
    objects: &Path,
    attempt: u64,
    created: &mut u32,
    cancel: &CancellationToken,
    effects: &mut Effects,
) -> Result<File, Error> {
    assert!(*created < MAX_OBJECTS);
    cancel.check()?;
    let effect = Effect::CreateMetadata(crate::effects::MetadataKind::CatalogObject);
    effects.before(effect)?;
    let file =
        filesystem::create_new_read_write(object_path(objects, object(attempt, *created + 1))?)
            .map_err(|error| io_error(effect.name(), error))?;
    // Even a following write/inspection failure owns this newly created name.
    *created += 1;
    Ok(file)
}

pub(super) fn sync_object(
    file: &File,
    cancel: &CancellationToken,
    effects: &mut Effects,
) -> Result<(), Error> {
    cancel.check()?;
    let effect = Effect::SyncMetadata(crate::effects::MetadataKind::CatalogObject);
    effects.before(effect)?;
    filesystem::sync_all(file).map_err(|error| io_error(effect.name(), error))?;
    effects.after(effect);
    Ok(())
}

pub(super) fn cleanup(
    objects: &Path,
    attempt: u64,
    created: u32,
    effects: &mut Effects,
) -> Result<(), Error> {
    assert!(created <= MAX_OBJECTS);
    if created == 0 {
        return Ok(());
    }

    // The caller must not have handed these files to commit_prepared. Each name
    // was created exclusively for this attempt, so no published catalog uses it.
    for ordinal in 1..=created {
        let effect = Effect::RemoveCleanupFile;
        effects.before(effect)?;
        filesystem::remove_file(object_path(objects, object(attempt, ordinal))?)
            .map_err(|error| io_error(effect.name(), error))?;
        effects.after(effect);
    }

    // Cancellation must not interrupt rollback. Synchronize the removals before
    // the builder releases their temporary-space reservation.
    crate::storage::recovery::sync_directory_cancellable(
        objects,
        DirectoryKind::Units,
        &CancellationToken::new(),
        effects,
    )
}

fn uncommitted_names(
    objects: &Path,
    committed_attempt: u64,
    issued: u64,
    mut output: Option<&mut Vec<ObjectId>>,
    effects: &mut Effects,
) -> Result<usize, Error> {
    effects.before(Effect::ListDatabaseDirectory)?;
    let mut buffer = pipesql_filesystem::DirectoryBuffer::default();
    let mut directory = pipesql_filesystem::Directory::open_bounded(
        objects,
        &mut buffer,
        crate::storage::recovery::MAX_CATALOG_OBJECTS + 3,
    )
    .map_err(|error| io_error("list uncommitted catalog objects", error))?;
    let mut seen = 0usize;
    let mut pending = 0usize;
    while let Some(name) = directory
        .next_name()
        .map_err(|error| io_error("read uncommitted catalog name", error))?
    {
        effects.before(Effect::ReadDatabaseEntry)?;
        if seen == crate::storage::recovery::MAX_CATALOG_OBJECTS {
            return Err(Error::Resource {
                owner: "catalog namespace objects",
                required: seen as u64 + 1,
                limit: seen as u64,
            });
        }
        seen += 1;
        let id = ObjectId::from_name(name.as_encoded_bytes())
            .map_err(|_| Error::Corrupt("unknown catalog recovery object"))?;
        if id.attempt() > issued {
            return Err(Error::Corrupt("catalog recovery object exceeds issuance"));
        }
        if id.attempt() > committed_attempt {
            pending += 1;
            if let Some(ids) = output.as_mut() {
                if ids.len() == ids.capacity() {
                    return Err(Error::Corrupt("catalog namespace changed during recovery"));
                }
                ids.push(id);
            }
        }
    }
    Ok(pending)
}

// Called only by exclusive Database::open after complete graph admission and
// repair of both roots. Never reachable from snapshot/receipt inspection.
pub(crate) fn recover_construction(
    root: &Path,
    selected: WalRecord,
    memory: &crate::resources::MemoryAuthority,
    effects: &mut Effects,
) -> Result<(), Error> {
    let RootState::Catalog(commit) = selected.state else {
        unreachable!("catalog open only")
    };
    let committed_attempt = commit.map_or(0, |commit| commit.transaction().sequence());
    let objects = joined_path(root, UNITS_NAME)?;
    let count = uncommitted_names(&objects, committed_attempt, selected.issued, None, effects)?;
    let bytes = count
        .checked_mul(std::mem::size_of::<ObjectId>())
        .expect("bounded namespace ID vector");
    let _charge = memory.reserve(bytes as u64, "catalog recovery object IDs")?;
    let mut ids = Vec::new();
    ids.try_reserve_exact(count).map_err(|_| Error::Resource {
        owner: "catalog recovery ID allocation",
        required: bytes as u64,
        limit: memory.limit(),
    })?;
    if ids.capacity() != count {
        return Err(Error::Resource {
            owner: "catalog recovery ID capacity",
            required: ids.capacity() as u64,
            limit: count as u64,
        });
    }
    if count != 0
        && uncommitted_names(
            &objects,
            committed_attempt,
            selected.issued,
            Some(&mut ids),
            effects,
        )? != count
    {
        return Err(Error::Corrupt("catalog namespace changed during recovery"));
    }

    // Collect before unlinking: directory cursors need not remain stable when
    // their directory changes. The vector has no growth beyond its reservation.
    for id in &ids {
        let effect = Effect::RemoveCleanupFile;
        effects.before(effect)?;
        filesystem::remove_file(object_path(&objects, *id)?)
            .map_err(|error| io_error(effect.name(), error))?;
        effects.after(effect);
    }

    // Also sync an empty set: previous unlink may be visible after a failed sync.
    crate::storage::recovery::sync_directory_cancellable(
        &objects,
        DirectoryKind::Units,
        &CancellationToken::new(),
        effects,
    )?;
    drop(ids);
    Ok(())
}
