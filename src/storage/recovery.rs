//! Validate the database directory and recover interrupted file changes.
//!
//! `CONTROL` identifies the database and its format. Two root files name the
//! committed snapshot. `WAL` records publication progress so recovery can decide
//! whether differing roots describe an interrupted commit or inconsistent data.
//! `format` owns their byte layouts and the snapshot-selection rules.
//!
//! Opening first selects a snapshot, then validates the metadata it references.
//! If that metadata is damaged, recovery refuses; choosing an older snapshot
//! could discard committed data. Column payloads are checked when read, not by
//! this directory check. Only an exclusive caller holding the database lease
//! may repair roots or remove unfinished files. Inspection calls never repair.
//!
//! Follow `check_namespace` for this sequence. Fresh creation instead calls
//! `validate_created_namespace`: its files must match the intended empty state,
//! without recovery hiding an initialization error. After catalog recovery,
//! `Database::open_with_effects` separately removes unfinished catalog objects
//! through `storage::construction`.

use crate::database::DatabaseLease;
use crate::effects::{DirectoryKind, Effect, Effects, MetadataKind};
use crate::error::{io_error, map_format_error, map_not_found, recovery_needed};
use crate::path::joined_path;
use crate::resources::MemoryAuthority;
use crate::storage::catalog;
use crate::storage::format;
use crate::storage::unit;
use crate::{CancellationToken, DatabaseId, Error, ErrorCause};
use pipesql_filesystem::{
    self as filesystem, Directory, DirectoryBuffer, FileIdentity as EntryIdentity,
};
use std::fs::File;
use std::io;
use std::path::Path;

pub(crate) const CONTROL_NAME: &str = "CONTROL";
pub(crate) const LOCK_NAME: &str = "LOCK";
pub(crate) const ROOT_A_NAME: &str = "ROOT.A";
pub(crate) const ROOT_B_NAME: &str = "ROOT.B";
pub(crate) const WAL_NAME: &str = "WAL";
pub(crate) const UNITS_NAME: &str = "units";
pub(crate) const PRIVATE_NAME: &str = "private";
pub(crate) const SCRATCH_NAMES: [&str; 2] = ["SCRATCH.A", "SCRATCH.B"];
pub(crate) const UNIT_NAME: &str = "0000000000000001.unit";
pub(crate) const PRIVATE_RECOVERY_NAMES: [&str; 8] = [
    "quantity.stage",
    "extendedprice.stage",
    "discount.stage",
    "tax.stage",
    "returnflag.stage",
    "linestatus.stage",
    "shipdate.stage",
    "UNIT.next",
];

// Loading can leave partial data in the first eight files. Query scratch must
// unlink its two names before writing data, so any surviving scratch name must
// still refer to an empty file. Keep those groups distinct for validation.
const LEGACY_PRIVATE_NAMES: [&str; 10] = [
    PRIVATE_RECOVERY_NAMES[0],
    PRIVATE_RECOVERY_NAMES[1],
    PRIVATE_RECOVERY_NAMES[2],
    PRIVATE_RECOVERY_NAMES[3],
    PRIVATE_RECOVERY_NAMES[4],
    PRIVATE_RECOVERY_NAMES[5],
    PRIVATE_RECOVERY_NAMES[6],
    PRIVATE_RECOVERY_NAMES[7],
    SCRATCH_NAMES[0],
    SCRATCH_NAMES[1],
];

/// Create exclusively, write and synchronize the file. The caller owns cleanup
/// on failure and must synchronize the parent directory to persist the name.
pub(crate) fn create_synced_file(
    path: impl AsRef<Path>,
    bytes: &[u8],
    effects: &mut Effects,
    kind: MetadataKind,
) -> Result<(), Error> {
    let create = Effect::CreateMetadata(kind);
    effects.before(create)?;
    let mut file = filesystem::create_new_read_write(path)
        .map_err(|source| io_error(create.name(), source))?;
    if !bytes.is_empty() {
        effects.write(&mut file, bytes, Effect::WriteMetadata(kind))?;
    }
    let sync = Effect::SyncMetadata(kind);
    effects.before(sync)?;
    filesystem::sync_all(&file).map_err(|source| io_error(sync.name(), source))
}

pub(crate) fn sync_directory(
    path: &Path,
    effects: &mut Effects,
    kind: DirectoryKind,
) -> Result<(), Error> {
    let open = Effect::OpenDirectory(kind);
    effects.before(open)?;
    let directory = filesystem::open_read(path).map_err(|source| io_error(open.name(), source))?;
    let sync = Effect::SyncDirectory(kind);
    effects.before(sync)?;
    filesystem::sync_all(&directory).map_err(|source| io_error(sync.name(), source))
}

/// Check cancellation before opening and before synchronizing. Once the host
/// synchronization call starts, this function waits for it to return.
pub(crate) fn sync_directory_cancellable(
    path: &Path,
    kind: DirectoryKind,
    cancellation: &CancellationToken,
    effects: &mut Effects,
) -> Result<(), Error> {
    cancellation.check()?;
    let open = Effect::OpenDirectory(kind);
    effects.before(open)?;
    let directory = filesystem::open_read(path).map_err(|source| io_error(open.name(), source))?;
    cancellation.check()?;
    let sync = Effect::SyncDirectory(kind);
    effects.before(sync)?;
    filesystem::sync_all(&directory).map_err(|source| io_error(sync.name(), source))
}

/// Inspect the current LOCK name without following a symlink. This does not
/// acquire a lock; callers must compare the result with their opened lease.
pub(crate) fn validate_lock_entry(
    root: &Path,
    effects: &mut Effects,
) -> Result<EntryIdentity, Error> {
    effects.before(Effect::InspectDatabaseDirectory)?;
    let metadata = filesystem::symlink_metadata(root)
        .map_err(|source| map_not_found("inspect database directory", source))?;
    if !metadata.file_type().is_dir() {
        return Err(Error::Corrupt("database path is not a directory"));
    }
    validate_regular_file(&joined_path(root, LOCK_NAME)?, 0, "invalid LOCK", effects)
}

/// Selected state after validation and, for exclusive callers, successful repair.
/// The fixed-schema reader uses projected_crc32c to verify its source columns;
/// catalog readers obtain per-column checksums from their unit metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Namespace {
    pub(crate) database_id: crate::DatabaseId,
    pub(crate) generation: u64,
    pub(crate) issued: u64,
    pub(crate) state: format::RootState,
    pub(crate) projected_crc32c: u32,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum NamespaceAccess {
    Exclusive {
        lock: EntryIdentity,
        database: Option<DatabaseId>,
    },
    ReadOnly,
    Query,
}

/// Validate and repair while the caller holds the database lease. Supply the
/// saved database identity when recovering through an existing handle; initial
/// open has no previously accepted identity and passes None.
pub(crate) fn recover_namespace(
    root: &Path,
    lease: &DatabaseLease,
    expected_database: Option<DatabaseId>,
    memory: &MemoryAuthority,
    effects: &mut Effects,
) -> Result<Namespace, Error> {
    check_namespace(
        root,
        effects,
        NamespaceAccess::Exclusive {
            lock: lease.identity(),
            database: expected_database,
        },
        memory,
    )
}

/// Require exactly the empty state creation intended, with no pending cleanup
/// or repair. This reads and validates files without changing them. The caller
/// must then synchronize the database directory and its parent before returning
/// the new handle; matching readback bytes alone do not establish durability.
pub(crate) fn validate_created_namespace(
    root: &Path,
    lease: &DatabaseLease,
    expected: &format::WalRecord,
    memory: &MemoryAuthority,
    effects: &mut Effects,
) -> Result<(), Error> {
    assert!(
        expected.issued == 0
            && matches!(
                expected.state,
                format::RootState::Empty | format::RootState::Catalog(None)
            )
    );
    let access = NamespaceAccess::Exclusive {
        lock: lease.identity(),
        database: Some(expected.database),
    };
    let authority = read_namespace_authority(root, access, effects)?;
    if authority.selected != *expected {
        return Err(Error::Corrupt(
            "created namespace differs from initial state",
        ));
    }
    let contents = validate_namespace_contents(root, &authority, access, memory, effects)?;
    require_settled_namespace(&authority, &contents)
}

/// Require settled roots and WAL without performing repairs. Fixed-schema
/// databases also require settled loading and scratch files; catalog inspection
/// leaves private scratch contents to exclusive recovery.
pub(crate) fn inspect_namespace(
    root: &Path,
    memory: &MemoryAuthority,
    effects: &mut Effects,
) -> Result<Namespace, Error> {
    check_namespace(root, effects, NamespaceAccess::ReadOnly, memory)
}

/// Inspect a query source while other readers may be creating scratch files.
/// Check the private directory itself but leave its changing contents alone.
/// Exclusive reopen validates and removes leftover scratch names.
pub(crate) fn inspect_query_namespace(
    root: &Path,
    memory: &MemoryAuthority,
    effects: &mut Effects,
) -> Result<Namespace, Error> {
    check_namespace(root, effects, NamespaceAccess::Query, memory)
}

struct NamespaceEntries {
    lock: Option<EntryIdentity>,
    control: Option<EntryIdentity>,
    root_a: Option<EntryIdentity>,
    root_b: Option<EntryIdentity>,
    wal: Option<EntryIdentity>,
    units: Option<EntryIdentity>,
    private: Option<EntryIdentity>,
    root_a_next: Option<EntryIdentity>,
    root_b_next: Option<EntryIdentity>,
    count: u8,
}

// End this stack frame before allocating unit-metadata buffers on the stack.
// The directory cursor needs 8 KiB of scratch; retaining it across the rest of
// validation would increase the peak. The returned identities are observations,
// not proof that their contents form a recoverable database.
#[inline(never)]
fn inspect_namespace_entries(
    root: &Path,
    effects: &mut Effects,
) -> Result<NamespaceEntries, Error> {
    effects.before(Effect::InspectDatabaseDirectory)?;
    let metadata = filesystem::symlink_metadata(root)
        .map_err(|source| map_not_found("inspect database directory", source))?;
    if !metadata.file_type().is_dir() {
        return Err(Error::Corrupt("database path is not a directory"));
    }
    effects.before(Effect::ListDatabaseDirectory)?;
    let mut directory_buffer = DirectoryBuffer::default();
    let mut entries = Directory::open(root, &mut directory_buffer)
        .map_err(|source| io_error("list database directory", source))?;
    let mut lock = None;
    let mut control = None;
    let mut root_a = None;
    let mut root_b = None;
    let mut wal = None;
    let mut units = None;
    let mut private = None;
    let mut root_a_next = None;
    let mut root_b_next = None;
    let mut count = 0_u8;
    loop {
        let next = entries.next_name();
        if matches!(&next, Ok(None)) {
            break;
        }
        effects.before(Effect::ReadDatabaseEntry)?;
        let name = next
            .map_err(|source| io_error("read database directory entry", source))?
            .expect("non-EOF directory step");
        count = count
            .checked_add(1)
            .ok_or(Error::Corrupt("too many namespace entries"))?;
        if count > 9 {
            return Err(Error::Corrupt("unexpected namespace entry"));
        }
        let path = joined_path(root, name)?;
        if name == LOCK_NAME {
            set_once(
                &mut lock,
                validate_regular_file(&path, 0, "invalid LOCK", effects)?,
            )?;
        } else if name == CONTROL_NAME {
            set_once(
                &mut control,
                validate_regular_file(
                    &path,
                    format::CONTROL_BYTES as u64,
                    "invalid CONTROL",
                    effects,
                )?,
            )?;
        } else if name == ROOT_A_NAME {
            set_once(
                &mut root_a,
                validate_regular_file_type(&path, "invalid ROOT.A", effects)?,
            )?;
        } else if name == ROOT_B_NAME {
            set_once(
                &mut root_b,
                validate_regular_file_type(&path, "invalid ROOT.B", effects)?,
            )?;
        } else if name == "ROOT.A.next" {
            set_once(
                &mut root_a_next,
                validate_regular_file_type(&path, "invalid ROOT.A.next", effects)?,
            )?;
        } else if name == "ROOT.B.next" {
            set_once(
                &mut root_b_next,
                validate_regular_file_type(&path, "invalid ROOT.B.next", effects)?,
            )?;
        } else if name == WAL_NAME {
            effects.before(Effect::InspectNamespaceEntry)?;
            let metadata = filesystem::symlink_metadata(&path)
                .map_err(|source| io_error("inspect namespace entry", source))?;
            validate_fence_owner(metadata.file_type().is_file(), metadata.nlink())?;
            if metadata.len() > format::WAL_BYTES as u64 {
                return Err(Error::Corrupt("invalid WAL"));
            }
            set_once(&mut wal, metadata.identity())?;
        } else if name == UNITS_NAME {
            set_once(&mut units, validate_directory(&path, effects)?)?;
        } else if name == PRIVATE_NAME {
            set_once(&mut private, validate_directory(&path, effects)?)?;
        } else {
            return Err(Error::Corrupt("unexpected namespace entry"));
        }
    }
    drop(entries);
    Ok(NamespaceEntries {
        lock,
        control,
        root_a,
        root_b,
        wal,
        units,
        private,
        root_a_next,
        root_b_next,
        count,
    })
}

// The snapshot selected from CONTROL, roots and WAL, plus the files that may
// need repair. Its referenced metadata must still pass validation before writes.
struct NamespaceAuthority {
    selected: format::WalRecord,
    fence: Option<format::WalRecord>,
    repair: Option<format::Replica>,
    // Presence of ROOT.A.next and ROOT.B.next, in that order.
    pending_roots: [bool; 2],
    wal: EntryIdentity,
    wal_length: u64,
    units: EntryIdentity,
    private: EntryIdentity,
}

struct ObservedDirectory {
    identity: EntryIdentity,
    entries: KnownEntries,
}

// Produced only after the selected snapshot's metadata and directory contents
// pass their checks. Cleanup describes permitted deletions, not work done yet.
struct ValidatedContents {
    generation: u64,
    projected_crc32c: u32,
    cleanup: NamespaceCleanup,
}

enum NamespaceCleanup {
    None,
    LegacyEmpty {
        units: ObservedDirectory,
        private: ObservedDirectory,
    },
    Scratch {
        private: KnownEntries,
    },
}

fn check_namespace(
    root: &Path,
    effects: &mut Effects,
    access: NamespaceAccess,
    memory: &MemoryAuthority,
) -> Result<Namespace, Error> {
    let authority = read_namespace_authority(root, access, effects)?;
    let contents = validate_namespace_contents(root, &authority, access, memory, effects)?;
    if !matches!(access, NamespaceAccess::Exclusive { .. }) {
        require_settled_namespace(&authority, &contents)?;
    } else {
        repair_namespace(root, &authority, &contents, effects).map_err(|source| {
            Error::RecoveryRequired {
                generation: contents.generation,
                source: ErrorCause::from_error(source),
            }
        })?;
    }
    Ok(Namespace {
        database_id: authority.selected.database,
        generation: contents.generation,
        issued: authority.selected.issued,
        state: authority.selected.state,
        projected_crc32c: contents.projected_crc32c,
    })
}

fn read_namespace_authority(
    root: &Path,
    access: NamespaceAccess,
    effects: &mut Effects,
) -> Result<NamespaceAuthority, Error> {
    let NamespaceEntries {
        lock,
        control,
        root_a,
        root_b,
        wal,
        units,
        private,
        root_a_next,
        root_b_next,
        count,
    } = inspect_namespace_entries(root, effects)?;
    // Renaming a database can leave its old LOCK descriptor open. Match the
    // current LOCK name before treating the held lease as permission to repair.
    if let NamespaceAccess::Exclusive { lock: expected, .. } = access
        && lock != Some(expected)
    {
        return Err(Error::Corrupt("namespace lease identity changed"));
    }
    let control = control.ok_or(Error::Corrupt("CONTROL is missing"))?;
    let control_bytes = read_fixed_file::<{ format::CONTROL_BYTES }>(
        &joined_path(root, CONTROL_NAME)?,
        control,
        effects,
        MetadataKind::Control,
    )?;
    let database_id = format::decode_control(&control_bytes)
        .map_err(|error| map_format_error(error, &control_bytes))?;
    if let NamespaceAccess::Exclusive {
        database: Some(expected),
        ..
    } = access
        && database_id != expected
    {
        return Err(Error::Corrupt("namespace database identity changed"));
    }

    // LOCK, CONTROL, WAL, units and private are mandatory. Roots and their
    // temporary replacements vary during recovery, but at least one root must exist.
    let required_count = 5_u8
        .checked_add(u8::from(root_a.is_some()))
        .and_then(|value| value.checked_add(u8::from(root_b.is_some())))
        .and_then(|value| value.checked_add(u8::from(root_a_next.is_some())))
        .and_then(|value| value.checked_add(u8::from(root_b_next.is_some())))
        .ok_or(Error::Corrupt("namespace count overflow"))?;
    let (wal, units, private) = match (lock, wal, units, private) {
        (Some(_), Some(wal), Some(units), Some(private))
            if count == required_count && (root_a.is_some() || root_b.is_some()) =>
        {
            (wal, units, private)
        }
        _ => return Err(Error::Corrupt("namespace is incomplete")),
    };
    let decoded_a = match root_a {
        Some(identity) => read_root_file(
            &joined_path(root, ROOT_A_NAME)?,
            identity,
            database_id,
            format::Replica::A,
            effects,
            MetadataKind::RootA,
        )?,
        None => None,
    };
    let decoded_b = match root_b {
        Some(identity) => read_root_file(
            &joined_path(root, ROOT_B_NAME)?,
            identity,
            database_id,
            format::Replica::B,
            effects,
            MetadataKind::RootB,
        )?,
        None => None,
    };
    let wal_length = metadata_length(
        &joined_path(root, WAL_NAME)?,
        wal,
        effects,
        MetadataKind::Wal,
    )?;
    let fence = read_fence(root, wal, database_id, effects)?;
    let (selected, repair) = format::select_snapshot(decoded_a, decoded_b, fence)
        .map_err(|_| Error::Corrupt("root/fence authority is inconsistent or insufficient"))?;
    if (u32::from_le_bytes(control_bytes[8..12].try_into().expect("CONTROL version"))
        == format::CATALOG_FORMAT_VERSION)
        != matches!(selected.state, format::RootState::Catalog(_))
    {
        return Err(Error::Corrupt("CONTROL and selected root formats differ"));
    }
    Ok(NamespaceAuthority {
        selected,
        fence,
        repair,
        pending_roots: [root_a_next.is_some(), root_b_next.is_some()],
        wal,
        wal_length,
        units,
        private,
    })
}

fn validate_namespace_contents(
    root: &Path,
    authority: &NamespaceAuthority,
    access: NamespaceAccess,
    memory: &MemoryAuthority,
    effects: &mut Effects,
) -> Result<ValidatedContents, Error> {
    let NamespaceAuthority {
        selected,
        units,
        private,
        ..
    } = *authority;
    let database_id = selected.database;
    let (generation, projected_crc32c, cleanup) = match selected.state {
        format::RootState::Catalog(commit) => {
            let objects = joined_path(root, UNITS_NAME)?;
            let mut scratch = catalog::Scratch::new(memory, "catalog recovery scratch")?;
            inspect_catalog_objects(
                &objects,
                units,
                selected.issued,
                MAX_CATALOG_OBJECTS,
                effects,
            )?;
            let cleanup = if matches!(access, NamespaceAccess::Exclusive { .. }) {
                scratch_cleanup(root, private, effects)?
            } else {
                NamespaceCleanup::None
            };

            catalog::validate_snapshot(
                &objects,
                selected,
                scratch.bytes(),
                &CancellationToken::new(),
                effects,
            )?;
            (commit.map_or(0, |v| v.generation()), 0, cleanup)
        }
        format::RootState::Empty => {
            let unit_entries = inspect_known_entries(
                &joined_path(root, UNITS_NAME)?,
                units,
                &[UNIT_NAME],
                effects,
            )?;
            let private_entries = if access == NamespaceAccess::Query {
                KnownEntries {
                    found: [false; 10],
                    count: 0,
                }
            } else {
                let path = joined_path(root, PRIVATE_NAME)?;
                let entries =
                    inspect_known_entries(&path, private, &LEGACY_PRIVATE_NAMES, effects)?;
                validate_scratch_names(&path, [entries.found[8], entries.found[9]], effects)?;
                entries
            };
            (
                0,
                0,
                NamespaceCleanup::LegacyEmpty {
                    units: ObservedDirectory {
                        identity: units,
                        entries: unit_entries,
                    },
                    private: ObservedDirectory {
                        identity: private,
                        entries: private_entries,
                    },
                },
            )
        }
        format::RootState::Data {
            transaction,
            rows,
            unit_bytes,
            unit_metadata_crc32c,
        } => {
            let _ = transaction; // The root decoder has already checked this receipt.
            require_directory_entries(
                &joined_path(root, UNITS_NAME)?,
                units,
                Some(UNIT_NAME),
                effects,
            )?;
            let cleanup = if access == NamespaceAccess::Query {
                NamespaceCleanup::None
            } else {
                scratch_cleanup(root, private, effects)?
            };
            let projected_crc32c = validate_unit_metadata(
                &joined_path(&joined_path(root, UNITS_NAME)?, UNIT_NAME)?,
                database_id,
                rows,
                unit_bytes,
                unit_metadata_crc32c,
                effects,
            )?;
            (1, projected_crc32c, cleanup)
        }
    };
    Ok(ValidatedContents {
        generation,
        projected_crc32c,
        cleanup,
    })
}

fn scratch_cleanup(
    root: &Path,
    private: EntryIdentity,
    effects: &mut Effects,
) -> Result<NamespaceCleanup, Error> {
    let path = joined_path(root, PRIVATE_NAME)?;
    let entries = inspect_known_entries(&path, private, &SCRATCH_NAMES, effects)?;
    validate_scratch_names(&path, [entries.found[0], entries.found[1]], effects)?;
    Ok(NamespaceCleanup::Scratch { private: entries })
}

fn validate_scratch_names(
    private: &Path,
    present: [bool; 2],
    effects: &mut Effects,
) -> Result<(), Error> {
    for (name, present) in SCRATCH_NAMES.into_iter().zip(present) {
        if present {
            effects.before(Effect::InspectNamespaceEntry)?;
            let metadata = filesystem::symlink_metadata(joined_path(private, name)?)
                .map_err(|error| io_error("inspect scratch recovery", error))?;
            // Scratch unlinks and synchronizes its names before writing payloads.
            // A nonempty named file therefore cannot be a leftover from that flow.
            if !metadata.file_type().is_file() || !metadata.is_empty() || metadata.nlink() != 1 {
                return Err(Error::Corrupt("scratch recovery ownership differs"));
            }
        }
    }
    Ok(())
}

fn require_settled_namespace(
    authority: &NamespaceAuthority,
    contents: &ValidatedContents,
) -> Result<(), Error> {
    let pending_cleanup = match &contents.cleanup {
        NamespaceCleanup::LegacyEmpty { units, private } => {
            units.entries.count != 0 || private.entries.count != 0
        }
        NamespaceCleanup::Scratch { private } => private.count != 0,
        _ => false,
    };
    if authority.repair.is_some()
        || authority.pending_roots.iter().any(|present| *present)
        || authority.fence != Some(authority.selected)
        || pending_cleanup
    {
        return Err(recovery_needed(contents.generation));
    }
    Ok(())
}

// Validate first, then establish durable roots before deleting unfinished work.
// Rewriting WAL afterward records the selected state. check_namespace converts
// any failure in this phase to RecoveryRequired because repairs may be partial.
fn repair_namespace(
    root: &Path,
    authority: &NamespaceAuthority,
    contents: &ValidatedContents,
    effects: &mut Effects,
) -> Result<(), Error> {
    let selected = authority.selected;
    reconcile_roots(
        root,
        selected.database,
        selected.issued,
        selected.state,
        authority.repair,
        authority.pending_roots,
        effects,
    )?;
    match &contents.cleanup {
        NamespaceCleanup::None => (),
        NamespaceCleanup::LegacyEmpty { units, private } => {
            recover_empty_mutations(root, units, private, effects)?;
        }
        NamespaceCleanup::Scratch { private } => {
            remove_known_entries(
                &joined_path(root, PRIVATE_NAME)?,
                &SCRATCH_NAMES,
                *private,
                effects,
            )?;
        }
    }
    let bytes =
        format::encode_wal(selected).map_err(|_| Error::Corrupt("cannot encode selected fence"))?;
    reconcile_fence(root, authority.wal, authority.wal_length, &bytes, effects)?;
    sync_directory(
        &joined_path(root, UNITS_NAME)?,
        effects,
        DirectoryKind::Units,
    )?;
    sync_directory(
        &joined_path(root, PRIVATE_NAME)?,
        effects,
        DirectoryKind::Private,
    )
}

// Count both retained generations and unfinished objects. Writers reserve room
// for new names against this limit before construction starts.
pub(crate) const MAX_CATALOG_OBJECTS: usize = 1_048_576;

#[inline(never)]
pub(crate) fn inspect_catalog_objects(
    path: &Path,
    expected: EntryIdentity,
    issued: u64,
    object_limit: usize,
    effects: &mut Effects,
) -> Result<(), Error> {
    assert!(object_limit <= MAX_CATALOG_OBJECTS);
    if validate_directory(path, effects)? != expected {
        return Err(Error::Corrupt("catalog object directory changed"));
    }
    effects.before(Effect::ListDatabaseDirectory)?;
    let mut buffer = DirectoryBuffer::default();
    // Allow the directory reader to see . and .. plus one extra object. That
    // extra entry lets this loop report the object limit instead of the lower
    // level reader's raw-entry limit.
    let mut directory = Directory::open_bounded(path, &mut buffer, object_limit + 3)
        .map_err(|e| io_error("list catalog objects", e))?;
    let mut count = 0usize;
    while let Some(name) = directory
        .next_name()
        .map_err(|e| io_error("read catalog object entry", e))?
    {
        effects.before(Effect::ReadDatabaseEntry)?;
        if count == object_limit {
            return Err(Error::Resource {
                owner: "catalog namespace objects",
                required: (object_limit + 1) as u64,
                limit: object_limit as u64,
            });
        }
        count += 1;
        let id = catalog::ObjectId::from_name(name.as_encoded_bytes())
            .map_err(|_| Error::Corrupt("unknown catalog object name"))?;
        if id.attempt() > issued {
            return Err(Error::Corrupt("catalog object exceeds issued prefix"));
        }
        let object_path = joined_path(path, name)?;
        effects.before(Effect::InspectNamespaceEntry)?;
        let metadata = filesystem::symlink_metadata(&object_path)
            .map_err(|e| io_error("inspect catalog object", e))?;
        if !metadata.file_type().is_file()
            || metadata.nlink() != 1
            || metadata.len() > unit::MAX_UNIT_BYTES as u64
        {
            return Err(Error::Corrupt(
                "catalog object ownership or extent is invalid",
            ));
        }
    }
    Ok(())
}

fn set_once<T>(slot: &mut Option<T>, value: T) -> Result<(), Error> {
    if slot.replace(value).is_some() {
        return Err(Error::Corrupt("duplicate namespace entry"));
    }
    Ok(())
}

pub(crate) fn validate_directory(
    path: &Path,
    effects: &mut Effects,
) -> Result<EntryIdentity, Error> {
    effects.before(Effect::InspectNamespaceEntry)?;
    let metadata = filesystem::symlink_metadata(path)
        .map_err(|source| io_error("inspect namespace entry", source))?;
    if !metadata.file_type().is_dir() {
        return Err(Error::Corrupt("namespace directory has invalid type"));
    }
    Ok(metadata.identity())
}

pub(crate) fn validate_regular_file(
    path: &Path,
    length: u64,
    message: &'static str,
    effects: &mut Effects,
) -> Result<EntryIdentity, Error> {
    effects.before(Effect::InspectNamespaceEntry)?;
    let metadata = filesystem::symlink_metadata(path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            Error::Corrupt("required namespace entry is missing")
        } else {
            io_error("inspect namespace entry", source)
        }
    })?;
    if !metadata.file_type().is_file() || metadata.len() != length {
        return Err(Error::Corrupt(message));
    }
    Ok(metadata.identity())
}

pub(crate) fn validate_regular_file_type(
    path: &Path,
    message: &'static str,
    effects: &mut Effects,
) -> Result<EntryIdentity, Error> {
    effects.before(Effect::InspectNamespaceEntry)?;
    let metadata = filesystem::symlink_metadata(path)
        .map_err(|source| io_error("inspect namespace entry", source))?;
    if !metadata.file_type().is_file() {
        return Err(Error::Corrupt(message));
    }
    Ok(metadata.identity())
}

/// Open the file observed during directory inspection and verify its identity
/// again. Return its current length so callers can apply the relevant format bound.
pub(crate) fn open_metadata(
    path: &Path,
    expected: EntryIdentity,
    effects: &mut Effects,
    kind: MetadataKind,
) -> Result<(File, u64), Error> {
    let open = Effect::OpenMetadata(kind);
    effects.before(open)?;
    let file = filesystem::open_read(path).map_err(|source| io_error(open.name(), source))?;
    let inspect = Effect::InspectOpenMetadata(kind);
    effects.before(inspect)?;
    let metadata =
        filesystem::file_metadata(&file).map_err(|source| io_error(inspect.name(), source))?;
    if metadata.identity() != expected || !metadata.file_type().is_file() {
        return Err(Error::Corrupt("metadata file changed while opening"));
    }
    if kind == MetadataKind::Wal {
        validate_fence_owner(metadata.file_type().is_file(), metadata.nlink())?;
    }
    Ok((file, metadata.len()))
}

fn read_fixed_file<const N: usize>(
    path: &Path,
    expected: EntryIdentity,
    effects: &mut Effects,
    kind: MetadataKind,
) -> Result<[u8; N], Error> {
    let (mut file, length) = open_metadata(path, expected, effects, kind)?;
    if length != N as u64 {
        return Err(Error::Corrupt("metadata file has invalid length"));
    }
    let mut bytes = [0_u8; N];
    effects.read(&mut file, &mut bytes, Effect::ReadMetadata(kind))?;
    Ok(bytes)
}

// None means recognizable physical damage that selection may be able to
// recover using the other root and WAL. Foreign identities, unsupported formats
// and invalid checksummed fields are errors, never fallback candidates.
fn decode_root_candidate(
    bytes: &[u8],
    database_id: crate::DatabaseId,
    replica: format::Replica,
) -> Result<Option<format::Root>, Error> {
    match format::decode_root(bytes) {
        Ok(root) if root.database == database_id && root.replica == replica => Ok(Some(root)),
        Ok(_) => Err(Error::Corrupt("root replica identity is foreign")),
        Err(error @ (format::FormatError::Version | format::FormatError::Generation(_))) => {
            Err(map_format_error(error, bytes))
        }
        Err(
            format::FormatError::Checksum
            | format::FormatError::Length
            | format::FormatError::Magic,
        ) => Ok(None),
        Err(_) => Err(Error::Corrupt(
            "checksummed root has invalid or unsupported fields",
        )),
    }
}

pub(crate) fn read_root_file(
    path: &Path,
    expected: EntryIdentity,
    database_id: crate::DatabaseId,
    replica: format::Replica,
    effects: &mut Effects,
    kind: MetadataKind,
) -> Result<Option<format::Root>, Error> {
    // A replaced file is an ownership error, not a damaged replica that the
    // surviving root may authorize us to overwrite.
    let (mut file, length) = open_metadata(path, expected, effects, kind)?;
    let prefix_length = usize::try_from(length.min(format::ROOT_BYTES as u64))
        .expect("bounded root prefix fits usize");
    let mut bytes = [0; format::ROOT_BYTES];
    if prefix_length != 0 {
        effects.read(
            &mut file,
            &mut bytes[..prefix_length],
            Effect::ReadMetadata(kind),
        )?;
    }
    // Decode the available prefix before classifying a wrong length as damage.
    // Otherwise extra or missing bytes could hide an unsupported format or a
    // valid root belonging to another database.
    let candidate = decode_root_candidate(&bytes[..prefix_length], database_id, replica)?;
    if length != format::ROOT_BYTES as u64 {
        return Ok(None);
    }
    Ok(candidate)
}

pub(crate) fn read_fence(
    root: &Path,
    identity: EntryIdentity,
    database: DatabaseId,
    effects: &mut Effects,
) -> Result<Option<format::WalRecord>, Error> {
    let (mut file, length) = open_metadata(
        &joined_path(root, WAL_NAME)?,
        identity,
        effects,
        MetadataKind::Wal,
    )?;
    if length > format::WAL_BYTES as u64 {
        return Err(Error::Corrupt("oversized fence"));
    }
    let length = usize::try_from(length).expect("bounded fence extent");
    let mut bytes = [0; format::WAL_BYTES];
    if length != 0 {
        effects.read(
            &mut file,
            &mut bytes[..length],
            Effect::ReadMetadata(MetadataKind::Wal),
        )?;
    }
    match format::decode_wal(&bytes[..length]) {
        Ok(record) if record.database == database => Ok(Some(record)),
        Ok(_) => Err(Error::Corrupt("foreign fence identity")),
        Err(error @ (format::FormatError::Version | format::FormatError::Generation(_))) => {
            Err(map_format_error(error, &bytes[..length]))
        }
        Err(
            format::FormatError::Checksum
            | format::FormatError::Length
            | format::FormatError::Magic,
        ) => Ok(None),
        Err(_) => Err(Error::Corrupt(
            "checksummed fence has invalid or unsupported fields",
        )),
    }
}

/// Require a single owner for WAL because recovery rewrites its bytes in place.
/// A second hard link would expose those writes through an unprotected pathname.
/// Call both when inspecting the name and after opening the file.
pub(crate) fn validate_fence_owner(regular_file: bool, links: u64) -> Result<(), Error> {
    if !regular_file || links != 1 {
        return Err(Error::Corrupt(
            "WAL fence must be a singly linked regular file",
        ));
    }
    Ok(())
}

/// Rewrite WAL to the selected snapshot, verify the bytes and synchronize them.
/// The caller must establish durable roots first and treat failure as incomplete
/// recovery. Recheck the observed file identity and length before writing.
pub(crate) fn reconcile_fence(
    root: &Path,
    identity: EntryIdentity,
    length: u64,
    bytes: &[u8; format::WAL_BYTES],
    effects: &mut Effects,
) -> Result<(), Error> {
    effects.before(Effect::OpenWalForRecovery)?;
    let mut file = filesystem::open_read_write(joined_path(root, WAL_NAME)?)
        .map_err(|source| io_error("open fence for recovery", source))?;
    effects.before(Effect::InspectWalForRecovery)?;
    let metadata = filesystem::file_metadata(&file)
        .map_err(|source| io_error("inspect fence for recovery", source))?;
    validate_fence_owner(metadata.file_type().is_file(), metadata.nlink())?;
    if metadata.identity() != identity
        || metadata.len() != length
        || length > format::WAL_BYTES as u64
    {
        return Err(Error::Corrupt("fence changed before recovery"));
    }
    effects.write(&mut file, bytes, Effect::WriteMetadata(MetadataKind::Wal))?;
    let mut observed = [0; format::WAL_BYTES];
    crate::effects::read_exact_at(
        &file,
        &mut observed,
        0,
        Effect::ReadMetadata(MetadataKind::Wal),
        effects,
    )?;
    if observed != *bytes {
        return Err(Error::Corrupt(
            "repaired fence differs from selected snapshot",
        ));
    }
    effects.before(Effect::SyncMetadata(MetadataKind::Wal))?;
    filesystem::sync_all(&file).map_err(|source| io_error("sync repaired fence", source))
}

fn metadata_length(
    path: &Path,
    expected: EntryIdentity,
    effects: &mut Effects,
    kind: MetadataKind,
) -> Result<u64, Error> {
    let inspect = Effect::InspectMetadata(kind);
    effects.before(inspect)?;
    let metadata =
        filesystem::symlink_metadata(path).map_err(|source| io_error(inspect.name(), source))?;
    if metadata.identity() != expected || !metadata.file_type().is_file() {
        return Err(Error::Corrupt("metadata file changed during validation"));
    }
    Ok(metadata.len())
}

// Positions refer to the caller's fixed list of allowed names. Retaining only
// presence bits bounds memory and lets cleanup use that same list.
#[derive(Clone, Copy)]
pub(crate) struct KnownEntries {
    pub(crate) found: [bool; 10],
    pub(crate) count: usize,
}

fn recover_empty_mutations(
    root: &Path,
    units: &ObservedDirectory,
    private: &ObservedDirectory,
    effects: &mut Effects,
) -> Result<(), Error> {
    let unit_entries = units.entries;
    let private_entries = private.entries;
    remove_known_entries(
        &joined_path(root, UNITS_NAME)?,
        &[UNIT_NAME],
        unit_entries,
        effects,
    )?;
    remove_known_entries(
        &joined_path(root, PRIVATE_NAME)?,
        &LEGACY_PRIVATE_NAMES,
        private_entries,
        effects,
    )?;
    if unit_entries.count != 0 {
        sync_directory(
            &joined_path(root, UNITS_NAME)?,
            effects,
            DirectoryKind::Units,
        )?;
    }
    if private_entries.count != 0 {
        sync_directory(
            &joined_path(root, PRIVATE_NAME)?,
            effects,
            DirectoryKind::Private,
        )?;
    }
    require_directory_entries(
        &joined_path(root, UNITS_NAME)?,
        units.identity,
        None,
        effects,
    )?;
    require_directory_entries(
        &joined_path(root, PRIVATE_NAME)?,
        private.identity,
        None,
        effects,
    )
}

pub(crate) fn inspect_known_entries(
    path: &Path,
    expected: EntryIdentity,
    allowed: &[&str],
    effects: &mut Effects,
) -> Result<KnownEntries, Error> {
    assert!(
        allowed.len() <= 10,
        "recovery allowlist exceeds fixed state"
    );
    effects.before(Effect::InspectSubdirectory)?;
    let metadata = filesystem::symlink_metadata(path)
        .map_err(|source| io_error("inspect database subdirectory", source))?;
    if metadata.identity() != expected || !metadata.file_type().is_dir() {
        return Err(Error::Corrupt(
            "database subdirectory changed before recovery",
        ));
    }
    effects.before(Effect::ListSubdirectory)?;
    let mut directory_buffer = DirectoryBuffer::default();
    let mut entries = Directory::open(path, &mut directory_buffer)
        .map_err(|source| io_error("list database subdirectory", source))?;
    let mut found = [false; 10];
    let mut count = 0_usize;
    loop {
        let next = entries.next_name();
        if matches!(&next, Ok(None)) {
            break;
        }
        effects.before(Effect::ReadSubdirectoryEntry)?;
        let name = next
            .map_err(|source| io_error("read database subdirectory entry", source))?
            .expect("non-EOF directory step");
        count = count
            .checked_add(1)
            .ok_or(Error::Corrupt("recovery entry count overflow"))?;
        if count > allowed.len() {
            return Err(Error::Corrupt("unexpected recovery entry"));
        }
        let Some(index) = allowed.iter().position(|allowed| name == *allowed) else {
            return Err(Error::Corrupt("unexpected recovery entry"));
        };
        if found[index] {
            return Err(Error::Corrupt("duplicate recovery entry"));
        }
        effects.before(Effect::InspectNamespaceEntry)?;
        let entry_metadata = filesystem::symlink_metadata(joined_path(path, name)?)
            .map_err(|source| io_error("inspect recovery entry", source))?;
        if !entry_metadata.file_type().is_file() {
            return Err(Error::Corrupt("recovery entry is not a regular file"));
        }
        found[index] = true;
    }
    Ok(KnownEntries { found, count })
}

fn remove_known_entries(
    path: &Path,
    allowed: &[&str],
    entries: KnownEntries,
    effects: &mut Effects,
) -> Result<(), Error> {
    for (index, name) in allowed.iter().enumerate() {
        if entries.found[index] {
            effects.before(Effect::RemoveRecoveryFile)?;
            filesystem::remove_file(joined_path(path, name)?)
                .map_err(|source| io_error("remove uncommitted recovery file", source))?;
        }
    }
    Ok(())
}

fn require_directory_entries(
    path: &Path,
    expected: EntryIdentity,
    one_name: Option<&str>,
    effects: &mut Effects,
) -> Result<(), Error> {
    effects.before(Effect::InspectSubdirectory)?;
    let metadata = filesystem::symlink_metadata(path)
        .map_err(|source| io_error("inspect database subdirectory", source))?;
    if metadata.identity() != expected || !metadata.file_type().is_dir() {
        return Err(Error::Corrupt(
            "database subdirectory changed during validation",
        ));
    }
    effects.before(Effect::ListSubdirectory)?;
    let mut directory_buffer = DirectoryBuffer::default();
    let mut entries = Directory::open(path, &mut directory_buffer)
        .map_err(|source| io_error("list database subdirectory", source))?;
    let mut found = 0_u8;
    loop {
        let next = entries.next_name();
        if matches!(&next, Ok(None)) {
            break;
        }
        effects.before(Effect::ReadSubdirectoryEntry)?;
        let name = next
            .map_err(|source| io_error("read database subdirectory entry", source))?
            .expect("non-EOF directory step");
        found = found
            .checked_add(1)
            .ok_or(Error::Corrupt("too many database subdirectory entries"))?;
        if found > u8::from(one_name.is_some()) || one_name != name.to_str() {
            return Err(Error::Corrupt("unexpected database subdirectory entry"));
        }
    }
    if found != u8::from(one_name.is_some()) {
        return Err(Error::Corrupt("database subdirectory is incomplete"));
    }
    Ok(())
}

// Check the fixed-schema unit header, descriptors and padding against the
// selected root. Payload verification belongs to the reader that uses a column.
fn validate_unit_metadata(
    path: &Path,
    database_id: crate::DatabaseId,
    rows: u64,
    unit_bytes: u64,
    unit_metadata_crc32c: u32,
    effects: &mut Effects,
) -> Result<u32, Error> {
    effects.before(Effect::InspectUnitEntry)?;
    let expected = filesystem::symlink_metadata(path)
        .map_err(|source| io_error("inspect unit entry", source))?;
    if !expected.file_type().is_file() || expected.len() != unit_bytes {
        return Err(Error::Corrupt("unit entry has invalid type or length"));
    }
    effects.before(Effect::OpenUnit)?;
    let mut file = filesystem::open_read(path).map_err(|source| io_error("open unit", source))?;
    effects.before(Effect::InspectOpenUnit)?;
    let opened =
        filesystem::file_metadata(&file).map_err(|source| io_error("inspect open unit", source))?;
    if opened.identity() != expected.identity() || opened.len() != unit_bytes {
        return Err(Error::Corrupt("unit changed while opening"));
    }
    let mut header = [0_u8; format::HEADER_BYTES];
    let mut descriptors = [0_u8; format::DESCRIPTOR_BYTES];
    let mut padding = [0_u8; format::UNIT_PADDING_BYTES];
    effects.read(&mut file, &mut header, Effect::ReadUnitHeader)?;
    effects.read(&mut file, &mut descriptors, Effect::ReadUnitDescriptors)?;
    effects.read(&mut file, &mut padding, Effect::ReadUnitPadding)?;
    if padding != [0; format::UNIT_PADDING_BYTES] {
        return Err(Error::Corrupt("unit metadata padding is nonzero"));
    }
    let (metadata, checksum) = format::decode_unit_summary(&header, &descriptors)
        .map_err(|error| map_format_error(error, &header))?;
    if metadata.database() != database_id
        || metadata.rows() != rows
        || checksum != unit_metadata_crc32c
    {
        return Err(Error::Corrupt("unit metadata disagrees with root"));
    }
    Ok(metadata.projected_crc32c())
}

/// Remove recognized temporary roots and replace the damaged or stale replica.
/// The caller must already have selected and validated the snapshot and hold
/// exclusive repair authority. Success includes synchronizing both root names.
pub(crate) fn reconcile_roots(
    root: &Path,
    database_id: crate::DatabaseId,
    issued: u64,
    state: format::RootState,
    repair: Option<format::Replica>,
    root_next: [bool; 2],
    effects: &mut Effects,
) -> Result<(), Error> {
    let mut removed_next = false;
    for (name, present) in [("ROOT.A.next", root_next[0]), ("ROOT.B.next", root_next[1])] {
        if present {
            effects.before(Effect::RemoveStaleRootNext)?;
            filesystem::remove_file(joined_path(root, name)?)
                .map_err(|source| io_error("remove stale root next", source))?;
            removed_next = true;
        }
    }
    if removed_next {
        sync_directory(root, effects, DirectoryKind::Database)?;
    }
    if let Some(replica) = repair {
        repair_root(root, database_id, issued, state, replica, effects)?;
    }
    // Equal roots may result from a rename whose directory sync failed earlier.
    // Synchronize even when no repair is needed, before cleanup or WAL rewrite.
    sync_directory(root, effects, DirectoryKind::Database)
}

fn repair_root(
    root: &Path,
    database_id: crate::DatabaseId,
    issued: u64,
    state: format::RootState,
    replica: format::Replica,
    effects: &mut Effects,
) -> Result<(), Error> {
    let name = match replica {
        format::Replica::A => ROOT_A_NAME,
        format::Replica::B => ROOT_B_NAME,
    };
    let next_name = match replica {
        format::Replica::A => "ROOT.A.next",
        format::Replica::B => "ROOT.B.next",
    };
    let encoded = format::encode_root(format::Root {
        database: database_id,
        issued,
        replica,
        state,
    })
    .map_err(|_| Error::Corrupt("failed to encode root repair"))?;
    // Write a separate file and read it back before replacing the root name.
    // A failed write or validation must not destroy the surviving committed root.
    let next = joined_path(root, next_name)?;
    create_synced_file(&next, &encoded, effects, MetadataKind::RootRepair)?;
    let expected = validate_regular_file(
        &next,
        format::ROOT_BYTES as u64,
        "invalid root repair",
        effects,
    )?;
    let observed = read_fixed_file::<{ format::ROOT_BYTES }>(
        &next,
        expected,
        effects,
        MetadataKind::RootRepair,
    )?;
    let decoded =
        format::decode_root(&observed).map_err(|error| map_format_error(error, &observed))?;
    if decoded.database != database_id
        || decoded.issued != issued
        || decoded.replica != replica
        || decoded.state != state
    {
        return Err(Error::Corrupt("root repair does not match selected state"));
    }
    effects.before(Effect::RenameRepairedRoot)?;
    filesystem::rename(&next, joined_path(root, name)?)
        .map_err(|source| io_error("rename repaired root", source))?;
    sync_directory(root, effects, DirectoryKind::Database)
}
