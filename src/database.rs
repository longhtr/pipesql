//! Database lifetime: path and lease admission, initialization, reopen, and close.
//!
//! Creation retains its lease until all initialization or failed cleanup finishes.
//! The handle owns resident charges; query and writer borrows keep it alive.

use crate::effects::{DirectoryKind, Effect, Effects, MetadataKind};
use crate::error::{io_error, map_not_found};
use crate::namespace::{
    CONTROL_NAME, LOCK_NAME, PRIVATE_NAME, PRIVATE_RECOVERY_NAMES, ROOT_A_NAME, ROOT_B_NAME,
    UNIT_NAME, UNITS_NAME, WAL_NAME, create_synced_file, sync_directory, validate_lock_entry,
    validate_namespace,
};
use crate::path::{
    MAX_PATH_BYTES, joined_path, native_path_work_limit, try_join_path, validate_requested_path,
};
use crate::resources::{MemoryAuthority, Reservation, TemporaryAuthority};
use crate::{
    Append, AppendLimits, CancellationToken, ColumnDeclaration, Commit, Config, DatabaseId, Error,
    ErrorCause, PreparedQuery, catalog_snapshot, frontend, scratch, storage_format,
};
use pipesql_filesystem::{self as filesystem, FileIdentity as EntryIdentity};
use std::fs::{File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

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

// Catalog generations live only in the registry. This marker cannot become a
// stale second generation cache after publication.
#[derive(Clone, Copy)]
pub(crate) enum DatabaseState {
    Empty,
    Loaded,
    ReopenRequired,
    Catalog,
}

/// An open database with an exclusive filesystem lease and bounded resources.
///
/// Shared references permit immutable readers and one serialized catalog writer.
/// Prepared queries and active appends borrow the handle. Dropping it releases
/// its resources and lease; unresolved publication or cleanup still needs reopen.
pub struct Database {
    catalog_registry: Option<Vec<catalog_snapshot::Registry>>,
    path: PathBuf,
    lease: DatabaseLease,
    pub(crate) memory: MemoryAuthority,
    pub(crate) temporary: TemporaryAuthority,
    pub(crate) scratch_admission: scratch::Admission,
    database_id: storage_format::DatabaseId,
    pub(crate) state: DatabaseState,
}

impl Database {
    pub(crate) fn catalog_registry(&self) -> Option<&catalog_snapshot::Registry> {
        self.catalog_registry.as_ref().map(|registry| &registry[0])
    }

    /// Create an empty database for declared tables and repeated appends.
    pub fn create_empty(path: &Path, config: Config) -> Result<Self, Error> {
        Self::create_catalog_with_effects(path, config, &mut Effects::default())
    }

    /// Declare an empty table and durably publish its schema.
    /// Names and declarations are borrowed only for this call.
    pub fn declare_table(
        &self,
        name: &str,
        columns: &[ColumnDeclaration<'_>],
        cancel: &CancellationToken,
    ) -> Result<Commit, Error> {
        self.catalog_writer()?
            .declare_table(name, columns, cancel, &mut Effects::default())
    }

    /// Admit a bounded append and issue its transaction identity.
    /// The returned owner must be committed or explicitly aborted.
    pub fn begin_append(
        &self,
        name: &str,
        limits: AppendLimits,
        cancel: &CancellationToken,
    ) -> Result<Append<'_>, Error> {
        Ok(Append {
            inner: self.catalog_writer()?.begin_append_named(
                name,
                limits,
                cancel,
                &mut Effects::default(),
            )?,
        })
    }

    /// Create the legacy fixed-schema database used by [`Self::load_lineitem`].
    pub fn create(path: &Path, config: Config) -> Result<Self, Error> {
        let mut effects = Effects::default();
        Self::create_with_effects(path, config, &mut effects)
    }

    /// Acquire the lease and complete namespace validation and recovery.
    /// No usable handle escapes if recovery or its durability barriers fail.
    pub fn open(path: &Path, config: Config) -> Result<Self, Error> {
        let mut effects = Effects::default();
        Self::open_with_effects(path, config, &mut effects)
    }

    /// The admitted canonical database path, owned by this handle.
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    #[cfg(test)]
    pub(crate) fn path_memory_bytes(&self) -> u64 {
        self.path.capacity() as u64
    }

    /// Configured limits, independent of current reservations.
    pub fn config(&self) -> Config {
        Config {
            memory_limit_bytes: self.memory.limit(),
            temp_limit_bytes: self.temporary.limit(),
        }
    }

    /// Current logical memory charges, including resident database ownership.
    /// This is not a measurement of allocator overhead or process memory.
    pub fn reserved_memory_bytes(&self) -> u64 {
        self.memory.reserved()
    }

    /// Current temporary-space charges, including unresolved construction debt.
    /// Closing the handle releases its account without settling persistent files.
    pub fn reserved_temp_bytes(&self) -> u64 {
        self.temporary.reserved()
    }

    /// The persistent identity assigned when the database was created.
    pub fn database_identity(&self) -> DatabaseId {
        self.database_id
    }

    /// The handle's current generation, not proof of a transaction's outcome.
    /// Resolve uncertain commits with [`Self::resolve_commit`] after recovery.
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

    /// Parse, bind, and independently validate a query against its input snapshot.
    /// A catalog query pins that generation until the prepared query is dropped.
    pub fn prepare<'database>(
        &'database self,
        source: &str,
    ) -> Result<PreparedQuery<'database>, Error> {
        if matches!(self.state, DatabaseState::Catalog) {
            return frontend::prepare_catalog(self, source);
        }
        frontend::prepare(self, source)
    }

    /// Release the database lease and consume the handle, including on failure.
    /// This does not resolve an uncertain commit or perform recovery.
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
        Self::create_initial_with_effects(path, config, storage_format::RootState::Empty, effects)
    }

    pub(crate) fn create_catalog_with_effects(
        path: &Path,
        config: Config,
        effects: &mut Effects,
    ) -> Result<Self, Error> {
        Self::create_initial_with_effects(
            path,
            config,
            storage_format::RootState::Catalog(None),
            effects,
        )
    }

    fn create_initial_with_effects(
        path: &Path,
        config: Config,
        initial: storage_format::RootState,
        effects: &mut Effects,
    ) -> Result<Self, Error> {
        assert!(matches!(
            initial,
            storage_format::RootState::Empty | storage_format::RootState::Catalog(None)
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
        let parent = filesystem::canonicalize(parent).map_err(|source| match source {
            filesystem::CanonicalizeError::Io(source) => {
                map_not_found("canonicalize database parent", source)
            }
            filesystem::CanonicalizeError::WorkLimit => native_path_work_limit(),
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
        // After acquisition, authority survives every initialization failure
        // until cleanup is complete. Before acquisition the namespace is
        // incomplete and cannot admit a public database handle.
        let mut lease = None;
        // Keep the lease in this slot through all fallible initialization,
        // including registry allocation, so error cleanup retains exclusivity.
        let initialized = (|| {
            finish_create(
                &root,
                &parent,
                storage_format::WalRecord {
                    database: database_id,
                    issued: 0,
                    state: initial,
                },
                &mut lease,
                &memory,
                effects,
            )?;
            let catalog_registry = if matches!(initial, storage_format::RootState::Catalog(_)) {
                Some(catalog_snapshot::Registry::allocate(
                    &memory,
                    storage_format::WalRecord {
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
                        storage_format::RootState::Catalog(_) => DatabaseState::Catalog,
                        storage_format::RootState::Empty => DatabaseState::Empty,
                        storage_format::RootState::Data { .. } => unreachable!("empty genesis"),
                    },
                })
            }
            Err(primary) => match cleanup_created_namespace(&root, effects) {
                Ok(()) => Err(primary),
                Err(cleanup) => Err(Error::CleanupRequired {
                    primary: ErrorCause::from_error(primary),
                    cleanup: ErrorCause::from_error(Error::Io {
                        operation: "cleanup created namespace",
                        source: cleanup,
                    }),
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
        let root = filesystem::canonicalize(path).map_err(|source| match source {
            filesystem::CanonicalizeError::Io(source) => {
                map_not_found("canonicalize database path", source)
            }
            filesystem::CanonicalizeError::WorkLimit => native_path_work_limit(),
        })?;
        path_charge.shrink_to(root.capacity() as u64);
        let expected_lock = validate_lock_entry(&root, effects)?;
        let lock_path = joined_path(&root, LOCK_NAME)?;
        effects.before(Effect::OpenLock)?;
        let lock = filesystem::open_read(lock_path)
            .map_err(|source| io_error("open database lock", source))?;
        let lease =
            DatabaseLease::acquire(lock, Some(expected_lock), Effect::InspectOpenLock, effects)?;
        let namespace = validate_namespace(&root, &lease, None, &memory, effects)?;
        let catalog_registry = if matches!(namespace.state, storage_format::RootState::Catalog(_)) {
            catalog_snapshot::recover_construction(
                &root,
                storage_format::WalRecord {
                    database: namespace.database_id,
                    issued: namespace.issued,
                    state: namespace.state,
                },
                &memory,
                effects,
            )?;
            Some(catalog_snapshot::Registry::allocate(
                &memory,
                storage_format::WalRecord {
                    database: namespace.database_id,
                    issued: namespace.issued,
                    state: namespace.state,
                },
            )?)
        } else {
            None
        };
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
                storage_format::RootState::Catalog(_) => DatabaseState::Catalog,
                storage_format::RootState::Empty => DatabaseState::Empty,
                storage_format::RootState::Data { .. } => DatabaseState::Loaded,
            },
        })
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        if let Some(registry) = self.catalog_registry.take() {
            drop(registry);
            let bytes = catalog_snapshot::REGISTRY_BYTES;
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

fn read_database_id(effects: &mut Effects) -> Result<storage_format::DatabaseId, Error> {
    effects.before(Effect::OpenRandom)?;
    let mut random = filesystem::open_read("/dev/urandom")
        .map_err(|source| io_error("open random identity source", source))?;
    let mut bytes = [0_u8; 16];
    effects.read(&mut random, &mut bytes, Effect::ReadIdentity)?;
    database_identity_from_random(bytes)
}

fn database_identity_from_random(bytes: [u8; 16]) -> Result<storage_format::DatabaseId, Error> {
    storage_format::DatabaseId::new(bytes)
        .map_err(|_| io_error("read database identity", io::ErrorKind::InvalidData.into()))
}

fn finish_create(
    root: &Path,
    parent: &Path,
    initial: storage_format::WalRecord,
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

    let genesis = storage_format::encode_wal(initial)
        .map_err(|_| Error::Corrupt("cannot encode genesis fence"))?;
    create_synced_file(
        joined_path(root, WAL_NAME)?,
        &genesis,
        effects,
        MetadataKind::Wal,
    )?;
    let control = match initial.state {
        storage_format::RootState::Catalog(_) => {
            storage_format::encode_catalog_control(database_id)
        }
        _ => storage_format::encode_control(database_id),
    };
    create_synced_file(
        joined_path(root, CONTROL_NAME)?,
        &control,
        effects,
        MetadataKind::Control,
    )?;
    let root_a = storage_format::encode_root(storage_format::Root {
        database: database_id,
        replica: storage_format::Replica::A,
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
    let root_b = storage_format::encode_root(storage_format::Root {
        database: database_id,
        replica: storage_format::Replica::B,
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

    validate_namespace(root, lease, Some(database_id), memory, effects)?;
    sync_directory(root, effects, DirectoryKind::Database)?;
    sync_directory(parent, effects, DirectoryKind::Parent)?;
    Ok(())
}

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
