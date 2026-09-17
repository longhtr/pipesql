//! Create, open and close a database.
//!
//! Applications use a `Database` handle to declare tables, append rows and prepare
//! queries. This module sets up the handle: it locates the database directory,
//! locks it against other handles, checks its files and establishes the memory
//! and temporary-space budgets shared by its operations.
//!
//! Start with [`Database::create_empty`] for a new database or [`Database::open`] for
//! an existing one. Creation writes the initial files; opening recovers interrupted
//! work when necessary. Both must finish successfully before returning a handle.
//!
//! Query processing and writes are implemented in other modules. This module
//! keeps the state they share alive and releases it when the handle is closed
//! or dropped.

use crate::effects::{DirectoryKind, Effect, Effects, MetadataKind};
use crate::error::{io_error, map_not_found};
use crate::path::{
    MAX_PATH_BYTES, joined_path, native_path_work_limit, try_join_path, validate_requested_path,
};
use crate::resources::{MemoryAuthority, Reservation, TemporaryAuthority};
use crate::storage::format;
use crate::storage::recovery::{
    CONTROL_NAME, LOCK_NAME, PRIVATE_NAME, PRIVATE_RECOVERY_NAMES, ROOT_A_NAME, ROOT_B_NAME,
    UNIT_NAME, UNITS_NAME, WAL_NAME, create_synced_file, recover_namespace, sync_directory,
    validate_created_namespace, validate_lock_entry,
};
use crate::storage::scratch;
use crate::storage::snapshot;
use crate::{
    Append, AppendLimits, CancellationToken, ColumnDeclaration, Commit, Config, DatabaseId, Error,
    ErrorCause, PreparedQuery, query,
};
use pipesql_filesystem::{self as filesystem, FileIdentity as EntryIdentity};
use std::fs::{File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

// The open LOCK file owns the advisory lock. Its identity lets later pathname
// checks detect replacement: holding an old file open does not lock a new file
// installed at the same name. Dropping File releases the held lock.
pub(crate) struct DatabaseLease {
    file: File,
    identity: EntryIdentity,
}

impl DatabaseLease {
    pub(crate) fn identity(&self) -> EntryIdentity {
        self.identity
    }

    fn acquire(
        file: File,
        expected: Option<EntryIdentity>,
        inspect: Effect,
        effects: &mut Effects,
    ) -> Result<Self, Error> {
        effects.before(inspect)?;
        let metadata =
            filesystem::file_metadata(&file).map_err(|source| io_error(inspect.name(), source))?;
        let identity = metadata.identity();
        if !metadata.file_type().is_file()
            || !metadata.is_empty()
            || expected.is_some_and(|expected| expected != identity)
        {
            return Err(Error::Corrupt("lock entry changed while opening"));
        }
        effects.before(Effect::LockDatabase)?;
        match File::try_lock(&file) {
            Ok(()) => Ok(Self { file, identity }),
            Err(TryLockError::WouldBlock) => Err(Error::Locked),
            Err(TryLockError::Error(source)) => Err(io_error("lock database", source)),
        }
    }
}

// Store the catalog's current version only in the registry. Keeping another
// copy here would risk returning an old version number after a commit.
#[derive(Clone, Copy)]
pub(crate) enum DatabaseState {
    Empty,
    Loaded,
    ReopenRequired,
    Catalog,
}

/// A handle to one database directory, holding its lock and resource budgets.
///
/// The lock prevents a second handle from opening the same database. Through this
/// handle, multiple readers can run alongside one writer. Each reader keeps its
/// chosen committed version of the data, called a snapshot.
///
/// Prepared queries and appends borrow this handle, so Rust prevents closing or
/// dropping it while they remain alive. Dropping the handle releases its lock
/// and resources. Recovery, if needed, happens when the database is reopened.
pub struct Database {
    catalog_registry: Option<Vec<snapshot::Registry>>,
    path: PathBuf,
    lease: DatabaseLease,
    pub(crate) memory: MemoryAuthority,
    pub(crate) temporary: TemporaryAuthority,
    pub(crate) scratch_admission: scratch::Admission,
    database_id: crate::DatabaseId,
    pub(crate) state: DatabaseState,
}

impl Database {
    pub(crate) fn catalog_registry(&self) -> Option<&snapshot::Registry> {
        self.catalog_registry.as_ref().map(|registry| &registry[0])
    }

    /// Create a new database with no tables.
    /// Use [`Self::declare_table`] to add a table before appending rows.
    pub fn create_empty(path: &Path, config: Config) -> Result<Self, Error> {
        Self::create_catalog_with_effects(path, config, &mut Effects::default())
    }

    /// Add an empty table with the supplied column names, types and NULL rules.
    /// A successful return confirms that the declaration is committed and durable.
    /// If the commit is uncertain, [`Error::CommitAmbiguous`] identifies the
    /// transaction to resolve before retrying.
    /// The caller can release the supplied names and columns after this call.
    ///
    /// A database holds at most 64 tables, each with 1–64 columns. Names contain
    /// 1–32 ASCII letters, digits or underscores and start with a letter or
    /// underscore. Table names and column names within a table must be unique
    /// ignoring ASCII case. Invalid declarations fail before transaction issuance;
    /// once issued, an attempt number is never reused, even after an abort.
    /// Legacy databases do not support declarations.
    pub fn declare_table(
        &self,
        name: &str,
        columns: &[ColumnDeclaration<'_>],
        cancel: &CancellationToken,
    ) -> Result<Commit, Error> {
        self.catalog_writer()?
            .declare_table(name, columns, cancel, &mut Effects::default())
    }

    /// Start an append with limits on its batch count and encoded data size.
    /// This reserves resources and records a transaction identifier before rows
    /// are written. Finish the returned [`Append`] by committing or aborting it.
    /// The table name is matched ignoring ASCII case. Invalid or unknown names
    /// release writer admission without issuing an attempt. Only one writer may
    /// be active; readers keep their existing snapshots during an append.
    pub fn begin_append(
        &self,
        name: &str,
        limits: AppendLimits,
        cancel: &CancellationToken,
    ) -> Result<Append<'_>, Error> {
        self.catalog_writer()?
            .begin_append_named(name, limits, cancel, &mut Effects::default())
    }

    /// Create a database for the legacy fixed-schema lineitem loader.
    /// Load its rows with [`Self::load_lineitem`]. For declared tables, use
    /// [`Self::create_empty`] instead.
    pub fn create(path: &Path, config: Config) -> Result<Self, Error> {
        let mut effects = Effects::default();
        Self::create_with_effects(path, config, &mut effects)
    }

    /// Open an existing database after locking it and checking its stored files.
    /// Recover interrupted work when needed. If validation, recovery or the
    /// required file synchronization fails, return an error without a handle.
    pub fn open(path: &Path, config: Config) -> Result<Self, Error> {
        let mut effects = Effects::default();
        Self::open_with_effects(path, config, &mut effects)
    }

    /// The absolute database path after resolving symbolic links.
    /// The returned path borrows storage kept by this handle.
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    #[cfg(test)]
    pub(crate) fn path_memory_bytes(&self) -> u64 {
        self.path.capacity() as u64
    }

    /// The configured memory and temporary-space limits.
    /// These are the total budgets, not the amounts currently available.
    pub fn config(&self) -> Config {
        Config {
            memory_limit_bytes: self.memory.limit(),
            temp_limit_bytes: self.temporary.limit(),
        }
    }

    /// Bytes currently reserved from this database's memory budget.
    /// Includes the stored path and catalog state, as well as reservations held
    /// by queries and writes.
    /// This counter does not measure allocator overhead or total process memory.
    pub fn reserved_memory_bytes(&self) -> u64 {
        self.memory.reserved()
    }

    /// Bytes currently reserved from this database's temporary-space budget.
    /// A failed write can retain a reservation until recovery deals with its files.
    /// Closing discards the counter; it does not perform that recovery.
    pub fn reserved_temp_bytes(&self) -> u64 {
        self.temporary.reserved()
    }

    /// The identifier assigned at creation and stored in the database files.
    /// It remains the same when the database is reopened.
    pub fn database_identity(&self) -> DatabaseId {
        self.database_id
    }

    /// The database version currently recorded by this handle.
    /// After an uncertain commit, this number cannot tell whether that write
    /// committed. Reopen for recovery, then use [`Self::resolve_commit`] to check.
    pub fn generation(&self) -> u64 {
        if let Some(registry) = &self.catalog_registry {
            return registry[0].generation();
        }
        match self.state {
            DatabaseState::Loaded => 1,
            DatabaseState::Empty | DatabaseState::ReopenRequired => 0,
            DatabaseState::Catalog => unreachable!("catalog handle owns a registry"),
        }
    }

    pub(crate) fn needs_reopen(&self) -> bool {
        if self
            .catalog_registry
            .as_ref()
            .is_some_and(|r| r[0].unavailable())
        {
            return true;
        }
        matches!(self.state, DatabaseState::ReopenRequired)
    }

    pub(crate) fn catalog_generation(&self) -> u64 {
        self.generation()
    }

    /// Prepare a query by checking its syntax, resolving names and validating its plan.
    /// This does not execute the query. For declared tables, the prepared query
    /// keeps its chosen database version available until it is dropped, even if
    /// later writes commit new data. The caller can release `source` after this
    /// call; error spans retain byte offsets, not the original SQL.
    pub fn prepare<'database>(
        &'database self,
        source: &str,
    ) -> Result<PreparedQuery<'database>, Error> {
        if matches!(self.state, DatabaseState::Catalog) {
            return query::prepare_catalog(self, source);
        }
        query::prepare(self, source)
    }

    /// Close the database and release its lock and resources.
    /// The handle is consumed even if closing returns an error. Closing does not
    /// recover interrupted work or determine whether an uncertain write committed.
    pub fn close(self) -> Result<(), Error> {
        self.close_with_effects(&mut Effects::default())
    }

    fn close_with_effects(self, effects: &mut Effects) -> Result<(), Error> {
        effects.before(Effect::UnlockDatabase)?;
        File::unlock(&self.lease.file).map_err(|source| Error::Io {
            operation: "unlock database",
            source,
        })?;
        Ok(())
    }

    pub(crate) fn lease(&self) -> &DatabaseLease {
        &self.lease
    }

    pub(crate) fn reserve_memory(
        &self,
        bytes: u64,
        owner: &'static str,
    ) -> Result<Reservation<'_>, Error> {
        self.memory.reserve(bytes, owner)
    }

    pub(crate) fn create_with_effects(
        path: &Path,
        config: Config,
        effects: &mut Effects,
    ) -> Result<Self, Error> {
        Self::create_initial_with_effects(path, config, format::RootState::Empty, effects)
    }

    pub(crate) fn create_catalog_with_effects(
        path: &Path,
        config: Config,
        effects: &mut Effects,
    ) -> Result<Self, Error> {
        Self::create_initial_with_effects(path, config, format::RootState::Catalog(None), effects)
    }

    fn create_initial_with_effects(
        path: &Path,
        config: Config,
        initial: format::RootState,
        effects: &mut Effects,
    ) -> Result<Self, Error> {
        assert!(matches!(
            initial,
            format::RootState::Empty | format::RootState::Catalog(None)
        ));
        validate_requested_path(path)?;
        let file_name = path
            .file_name()
            .ok_or(Error::InvalidPath("database path has no final component"))?;
        let parent = path
            .parent()
            .ok_or(Error::InvalidPath("database path has no parent"))?;
        let memory = MemoryAuthority::new(config.memory_limit_bytes);
        let mut path_charge = memory.reserve(MAX_PATH_BYTES as u64, "database pathname")?;
        effects.before(Effect::CanonicalizeParent)?;
        let parent =
            filesystem::canonicalize(parent, &mut crate::path::CanonicalizeScratch::new(&memory))
                .map_err(|source| match source {
                filesystem::CanonicalizeError::Io(source) => {
                    map_not_found("canonicalize database parent", source)
                }
                filesystem::CanonicalizeError::WorkLimit => native_path_work_limit(),
                filesystem::CanonicalizeError::Scratch(source) => source,
            })?;
        let root = joined_path(&parent, file_name)?;
        path_charge.shrink_to(root.capacity() as u64);
        let database_id = read_database_id(effects)?;
        effects.before(Effect::CreateDatabaseDirectory)?;
        match filesystem::create_dir(&root) {
            Ok(()) => {}
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                return Err(Error::AlreadyExists);
            }
            Err(source) => return Err(io_error("create database directory", source)),
        }
        // Keep the lock here so a later initialization failure cannot release it
        // before cleanup. Otherwise another opener could enter during cleanup.
        // Before the lock is acquired, the incomplete files cannot pass opening
        // validation. Even registry allocation must finish while we hold the lock.
        let mut lease = None;
        let initialized = (|| {
            finish_create(
                &root,
                &parent,
                format::WalRecord {
                    database: database_id,
                    issued: 0,
                    state: initial,
                },
                &mut lease,
                &memory,
                effects,
            )?;
            let catalog_registry = if matches!(initial, format::RootState::Catalog(_)) {
                Some(snapshot::Registry::allocate(
                    &memory,
                    format::WalRecord {
                        database: database_id,
                        issued: 0,
                        state: initial,
                    },
                )?)
            } else {
                None
            };
            Ok(catalog_registry)
        })();
        match initialized {
            Ok(catalog_registry) => {
                // Transfer the reservation to Database::drop without releasing
                // it while the retained path still occupies memory.
                std::mem::forget(path_charge);
                Ok(Self {
                    catalog_registry,
                    path: root,
                    lease: lease.take().expect("successful creation owns its lease"),
                    memory,
                    temporary: TemporaryAuthority::new(config.temp_limit_bytes),
                    scratch_admission: scratch::Admission::new(),
                    database_id,
                    state: match initial {
                        format::RootState::Catalog(_) => DatabaseState::Catalog,
                        format::RootState::Empty => DatabaseState::Empty,
                        format::RootState::Data { .. } => unreachable!("empty genesis"),
                    },
                })
            }
            // Cleanup can fail too. Retain both errors so the caller knows why
            // creation stopped and why its files could not be fully removed.
            Err(primary) => match cleanup_failed_creation(&root, lease.as_ref(), effects) {
                Ok(()) => Err(primary),
                Err(cleanup) => Err(Error::CleanupRequired {
                    primary: ErrorCause::from_error(primary),
                    cleanup: ErrorCause::from_error(cleanup),
                }),
            },
        }
    }

    pub(crate) fn open_with_effects(
        path: &Path,
        config: Config,
        effects: &mut Effects,
    ) -> Result<Self, Error> {
        validate_requested_path(path)?;
        let memory = MemoryAuthority::new(config.memory_limit_bytes);
        let mut path_charge = memory.reserve(MAX_PATH_BYTES as u64, "database pathname")?;
        effects.before(Effect::CanonicalizeDatabase)?;
        let root =
            filesystem::canonicalize(path, &mut crate::path::CanonicalizeScratch::new(&memory))
                .map_err(|source| match source {
                    filesystem::CanonicalizeError::Io(source) => {
                        map_not_found("canonicalize database path", source)
                    }
                    filesystem::CanonicalizeError::WorkLimit => native_path_work_limit(),
                    filesystem::CanonicalizeError::Scratch(source) => source,
                })?;
        path_charge.shrink_to(root.capacity() as u64);
        let expected_lock = validate_lock_entry(&root, effects)?;
        let lock_path = joined_path(&root, LOCK_NAME)?;
        effects.before(Effect::OpenLock)?;
        let lock = filesystem::open_read(lock_path)
            .map_err(|source| io_error("open database lock", source))?;
        let lease =
            DatabaseLease::acquire(lock, Some(expected_lock), Effect::InspectOpenLock, effects)?;
        // Recovery checks the held lock's identity before changing names.
        // Keep the lease through recovery and registry construction; any failure
        // drops it without exposing a partially initialized Database.
        let namespace = recover_namespace(&root, &lease, None, &memory, effects)?;
        let catalog_registry = if matches!(namespace.state, format::RootState::Catalog(_)) {
            crate::storage::construction::recover_construction(
                &root,
                format::WalRecord {
                    database: namespace.database_id,
                    issued: namespace.issued,
                    state: namespace.state,
                },
                &memory,
                effects,
            )?;
            Some(snapshot::Registry::allocate(
                &memory,
                format::WalRecord {
                    database: namespace.database_id,
                    issued: namespace.issued,
                    state: namespace.state,
                },
            )?)
        } else {
            None
        };
        // As in creation, Database::drop now owns release of the path charge.
        std::mem::forget(path_charge);
        Ok(Self {
            catalog_registry,
            path: root,
            lease,
            memory,
            temporary: TemporaryAuthority::new(config.temp_limit_bytes),
            scratch_admission: scratch::Admission::new(),
            database_id: namespace.database_id,
            state: match namespace.state {
                format::RootState::Catalog(_) => DatabaseState::Catalog,
                format::RootState::Empty => DatabaseState::Empty,
                format::RootState::Data { .. } => DatabaseState::Loaded,
            },
        })
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        // Free each allocation before returning its bytes to the memory budget.
        // Reversing this order would count memory as available while still in use.
        if let Some(registry) = self.catalog_registry.take() {
            drop(registry);
            let bytes = snapshot::REGISTRY_BYTES;
            self.memory
                .release(bytes, "catalog registry reservation underflow");
        }
        let path = std::mem::take(&mut self.path);
        let bytes = path.capacity() as u64;
        drop(path);
        self.memory
            .release(bytes, "database pathname reservation underflow");
    }
}

fn read_database_id(effects: &mut Effects) -> Result<crate::DatabaseId, Error> {
    effects.before(Effect::OpenRandom)?;
    let mut random = filesystem::open_read("/dev/urandom")
        .map_err(|source| io_error("open random identity source", source))?;
    let mut bytes = [0_u8; 16];
    effects.read(&mut random, &mut bytes, Effect::ReadIdentity)?;
    database_identity_from_random(bytes)
}

fn database_identity_from_random(bytes: [u8; 16]) -> Result<crate::DatabaseId, Error> {
    crate::DatabaseId::new(bytes)
        .map_err(|_| io_error("read database identity", io::ErrorKind::InvalidData.into()))
}

fn finish_create(
    root: &Path,
    parent: &Path,
    initial: format::WalRecord,
    lease_slot: &mut Option<DatabaseLease>,
    memory: &MemoryAuthority,
    effects: &mut Effects,
) -> Result<(), Error> {
    let database_id = initial.database;
    let lock_path = joined_path(root, LOCK_NAME)?;
    effects.before(Effect::CreateLock)?;
    let lock = filesystem::create_new_read_write(&lock_path)
        .map_err(|source| io_error("create database lock", source))?;
    *lease_slot = Some(DatabaseLease::acquire(
        lock,
        None,
        Effect::InspectCreatedLock,
        effects,
    )?);
    let lease = lease_slot.as_ref().expect("initialization owns its lease");
    effects.before(Effect::SyncLock)?;
    filesystem::sync_all(&lease.file).map_err(|source| io_error("sync database lock", source))?;

    effects.before(Effect::CreateUnitsDirectory)?;
    filesystem::create_dir(joined_path(root, UNITS_NAME)?)
        .map_err(|source| io_error("create units directory", source))?;
    effects.before(Effect::CreatePrivateDirectory)?;
    filesystem::create_dir(joined_path(root, PRIVATE_NAME)?)
        .map_err(|source| io_error("create private directory", source))?;
    sync_directory(
        &joined_path(root, UNITS_NAME)?,
        effects,
        DirectoryKind::Units,
    )?;
    sync_directory(
        &joined_path(root, PRIVATE_NAME)?,
        effects,
        DirectoryKind::Private,
    )?;

    let genesis =
        format::encode_wal(initial).map_err(|_| Error::Corrupt("cannot encode genesis fence"))?;
    create_synced_file(
        joined_path(root, WAL_NAME)?,
        &genesis,
        effects,
        MetadataKind::Wal,
    )?;
    let control = match initial.state {
        format::RootState::Catalog(_) => format::encode_catalog_control(database_id),
        _ => format::encode_control(database_id),
    };
    create_synced_file(
        joined_path(root, CONTROL_NAME)?,
        &control,
        effects,
        MetadataKind::Control,
    )?;
    let root_a = format::encode_root(format::Root {
        database: database_id,
        replica: format::Replica::A,
        issued: 0,
        state: initial.state,
    })
    .map_err(|_| Error::Corrupt("failed to encode empty ROOT.A"))?;
    create_synced_file(
        joined_path(root, ROOT_A_NAME)?,
        &root_a,
        effects,
        MetadataKind::RootA,
    )?;
    let root_b = format::encode_root(format::Root {
        database: database_id,
        replica: format::Replica::B,
        issued: 0,
        state: initial.state,
    })
    .map_err(|_| Error::Corrupt("failed to encode empty ROOT.B"))?;
    create_synced_file(
        joined_path(root, ROOT_B_NAME)?,
        &root_b,
        effects,
        MetadataKind::RootB,
    )?;

    // Read back the files and require the exact empty database we intended to
    // create. Recovery could conceal a creation error by accepting a repaired
    // state, so creation validates without repairing. Synchronize the directory
    // entries as well as file contents before reporting success.
    validate_created_namespace(root, lease, &initial, memory, effects)?;
    sync_directory(root, effects, DirectoryKind::Database)?;
    sync_directory(parent, effects, DirectoryKind::Parent)?;
    Ok(())
}

fn cleanup_failed_creation(
    root: &Path,
    lease: Option<&DatabaseLease>,
    effects: &mut Effects,
) -> Result<(), Error> {
    // A lock protects the opened file, even if someone replaces its pathname.
    // Before deleting anything, check that LOCK still names the file we locked;
    // otherwise cleanup could remove files belonging to a replacement database.
    if let Some(lease) = lease
        && validate_lock_entry(root, effects)? != lease.identity()
    {
        return Err(Error::Corrupt("creation cleanup lease identity changed"));
    }
    cleanup_created_namespace(root, effects)
        .map_err(|source| io_error("cleanup created namespace", source))
}

// Remove only names initialization can own. Continue after an individual failure
// to clean up the remaining files, but retain the first error for the caller.
// Unknown entries prevent directory removal instead of being recursively deleted.
fn cleanup_created_namespace(root: &Path, effects: &mut Effects) -> Result<(), io::Error> {
    let mut first = None;
    for (directory, names) in [
        (UNITS_NAME, [UNIT_NAME].as_slice()),
        (PRIVATE_NAME, PRIVATE_RECOVERY_NAMES.as_slice()),
    ] {
        for name in names {
            record_cleanup(
                effects.before_io(Effect::RemoveCleanupFile).and_then(|_| {
                    filesystem::remove_file(try_join_path(&try_join_path(root, directory)?, name)?)
                }),
                &mut first,
            );
        }
        record_cleanup(
            effects
                .before_io(Effect::RemoveCleanupDirectory)
                .and_then(|_| filesystem::remove_dir(try_join_path(root, directory)?)),
            &mut first,
        );
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
        record_cleanup(
            effects
                .before_io(Effect::RemoveCleanupFile)
                .and_then(|_| filesystem::remove_file(try_join_path(root, name)?)),
            &mut first,
        );
    }
    record_cleanup(
        effects
            .before_io(Effect::RemoveCleanupDirectory)
            .and_then(|_| filesystem::remove_dir(root)),
        &mut first,
    );
    if let Some(parent) = root.parent() {
        let synced = effects
            .before_io(Effect::OpenDirectory(DirectoryKind::Parent))
            .and_then(|_| filesystem::open_read(parent))
            .and_then(|directory| {
                effects.before_io(Effect::SyncDirectory(DirectoryKind::Parent))?;
                filesystem::sync_all(&directory)
            });
        record_cleanup(synced, &mut first);
    }
    match first {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn record_cleanup(outcome: Result<(), io::Error>, first: &mut Option<io::Error>) {
    if let Err(error) = outcome
        && error.kind() != io::ErrorKind::NotFound
        && first.is_none()
    {
        *first = Some(error);
    }
}

#[cfg(test)]
mod tests;
