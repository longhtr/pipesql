//! Let readers keep an old committed view while one writer publishes a new one.
//!
//! Catalog files are immutable. A snapshot selects a catalog and holds a pin,
//! a count that prevents reuse of its registry slot and deletion of its data.
//! Publication changes which slot new readers select; existing readers continue
//! using their pinned catalog. Receipt lookup uses a separate pin because it
//! needs transaction history, not the old table data.
//!
//! `Database::catalog_snapshot` selects a reader's view. `catalog_writer` reserves
//! a spare slot for one writer, which issues a transaction number before building
//! files. `Writer::commit_prepared` validates the completed files and publishes
//! them before making the new slot current. The registry mutex protects these
//! short changes to shared state; file operations happen outside the lock.
//!
//! Reclamation excludes writers but permits readers. Dropping an unfinished
//! writer or maintenance operation disables further admission: its files may
//! need cleanup or its commit may be uncertain. Reopen performs recovery before
//! another handle can use the database. Other storage modules build and remove
//! files; this module owns the rules for selecting and retaining live views.

use crate::effects::Effects;
use crate::error::io_error;
use crate::path::joined_path;
use crate::storage::catalog;
use crate::storage::format::{CatalogCommit, RootState, WalRecord};
use crate::storage::publication::{FailureStage, publish_snapshot};
use crate::storage::recovery::{UNITS_NAME, inspect_namespace};
use crate::{
    CancellationToken, Commit, CommitResolution, Database, Error, ErrorCause, TransactionId,
};
use pipesql_filesystem::{LockError, Mutex, MutexGuard};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

// Current state and pinned views share these slots. A writer needs a noncurrent,
// unpinned slot before claiming authority. Data and receipt pins have separate
// per-slot counters; capacity refusal differs from exhausting update attempts.
pub(crate) const SLOTS: usize = 4;
pub(crate) const MAX_PINS: u32 = 8;
const PIN_ATTEMPTS: usize = 8;
pub(crate) const REGISTRY_BYTES: u64 =
    (std::mem::size_of::<Registry>() + Mutex::<State>::allocation_bytes()) as u64;

#[derive(Clone, Copy)]
pub(super) enum Active {
    Writer {
        slot: usize,
        attempt: Option<TransactionId>,
    },
    Maintenance,
}

impl Active {
    fn writer_slot(self) -> usize {
        let Self::Writer { slot, .. } = self else {
            unreachable!("writer owns publication admission")
        };
        slot
    }

    fn transaction(self) -> Option<TransactionId> {
        match self {
            Self::Writer { attempt, .. } => attempt,
            Self::Maintenance => None,
        }
    }
}

pub(super) struct State {
    pub(super) current: usize,
    pub(super) slots: [WalRecord; SLOTS],
    pub(super) active: Option<Active>,
}

pub(crate) struct Registry {
    state: Mutex<State>,
    pub(super) data_pins: [AtomicU32; SLOTS],
    pub(super) resolution_pins: [AtomicU32; SLOTS],
    generation: AtomicU64,
    pub(super) unavailable: AtomicBool,
}

pub(super) fn generation(record: WalRecord) -> u64 {
    let RootState::Catalog(commit) = record.state else {
        unreachable!("registry admits catalog records only")
    };
    commit.map_or(0, CatalogCommit::generation)
}

impl Registry {
    /// Allocate one registry with explicit allocation failure. The single-element
    /// vector supplies stable ownership without an infallible boxed allocation;
    /// database callers borrow the registry, not the container.
    pub(crate) fn allocate(
        memory: &crate::resources::MemoryAuthority,
        record: WalRecord,
    ) -> Result<Vec<Self>, Error> {
        let charge = memory.reserve(REGISTRY_BYTES, "catalog snapshot registry")?;
        let mut owner = Vec::new();
        owner.try_reserve_exact(1).map_err(|_| Error::Resource {
            owner: "catalog registry allocation",
            required: REGISTRY_BYTES,
            limit: memory.limit(),
        })?;
        if owner.capacity() != 1 {
            return Err(Error::Resource {
                owner: "catalog registry capacity",
                required: owner.capacity() as u64,
                limit: 1,
            });
        }
        owner.push(Self {
            state: Mutex::new(State {
                current: 0,
                slots: [record; SLOTS],
                active: None,
            })
            .map_err(|error| io_error("initialize catalog registry mutex", error))?,
            data_pins: std::array::from_fn(|_| AtomicU32::new(0)),
            resolution_pins: std::array::from_fn(|_| AtomicU32::new(0)),
            generation: AtomicU64::new(generation(record)),
            unavailable: AtomicBool::new(false),
        });
        // Database takes the persistent charge with this physical owner and
        // releases it only after dropping the vector in Database::drop.
        std::mem::forget(charge);
        Ok(owner)
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub(crate) fn unavailable(&self) -> bool {
        self.unavailable.load(Ordering::Acquire)
    }

    // Refuse busy admission instead of waiting. Check unavailable again under
    // the lock because an unfinished owner can invalidate the registry while
    // this caller is acquiring it.
    pub(super) fn enter(&self) -> Result<MutexGuard<'_, State>, Error> {
        if self.unavailable() {
            return Err(crate::error::recovery_needed(self.generation()));
        }
        match self.state.try_lock() {
            Ok(Some(state)) if !self.unavailable() => Ok(state),
            Ok(Some(_)) | Err(LockError::Poisoned) => {
                Err(crate::error::recovery_needed(self.generation()))
            }
            Ok(None) => Err(Error::Contention("catalog registry")),
            Err(LockError::System(error)) => Err(io_error("try catalog registry mutex", error)),
        }
    }

    // An admitted writer or maintenance owner must settle its transition even
    // if a reader briefly holds the mutex. Unlike admission, completion may wait.
    // Lock holders do bounded metadata work without I/O or callbacks; progress
    // still depends on host scheduling and mutex fairness.
    fn finish_owned_write(&self) -> Result<MutexGuard<'_, State>, Error> {
        let state = self.state.lock().map_err(|error| match error {
            LockError::Poisoned => crate::error::recovery_needed(self.generation()),
            LockError::System(error) => io_error("lock catalog registry mutex", error),
        })?;
        if self.unavailable() {
            return Err(crate::error::recovery_needed(self.generation()));
        }
        Ok(state)
    }

    fn increment(pin: &AtomicU32) -> Result<(), Error> {
        let mut observed = pin.load(Ordering::Acquire);
        for _ in 0..PIN_ATTEMPTS {
            if observed >= MAX_PINS {
                return Err(Error::Resource {
                    owner: "catalog view pins",
                    required: u64::from(MAX_PINS) + 1,
                    limit: u64::from(MAX_PINS),
                });
            }
            match pin.compare_exchange_weak(
                observed,
                observed + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(actual) => observed = actual,
            }
        }
        Err(Error::Contention("catalog view pin"))
    }
}

// Copy the immutable commit description so reading it needs no registry lock.
// The pin protects its files, and the database borrow keeps the registry alive.
pub(crate) struct Snapshot<'db> {
    database: &'db Database,
    slot: usize,
    commit: Option<CatalogCommit>,
}

impl<'db> Snapshot<'db> {
    pub(crate) fn generation(&self) -> u64 {
        self.commit.map_or(0, CatalogCommit::generation)
    }

    pub(crate) fn state(&self) -> RootState {
        RootState::Catalog(self.commit)
    }

    // Pins belong to this live registry, not merely to a persistent database ID.
    pub(crate) fn belongs_to(&self, database: &Database) -> bool {
        std::ptr::eq(self.database, database)
    }

    #[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
    pub(crate) fn try_clone(&self) -> Result<Self, Error> {
        Registry::increment(&self.database.registry()?.data_pins[self.slot])?;
        Ok(Self {
            database: self.database,
            slot: self.slot,
            commit: self.commit,
        })
    }

    pub(crate) fn read_catalog<'a>(
        &self,
        bytes: &'a mut [u8],
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Option<catalog::Catalog<'a>>, Error> {
        self.commit
            .map(CatalogCommit::catalog)
            .map(|reference| {
                catalog::read(
                    &joined_path(self.database.path(), UNITS_NAME)?,
                    self.database.database_identity(),
                    reference,
                    bytes,
                    cancel,
                    effects,
                )
            })
            .transpose()
    }
}

impl Drop for Snapshot<'_> {
    fn drop(&mut self) {
        let registry = self
            .database
            .catalog_registry()
            .expect("snapshot owns database borrow");
        let previous = registry.data_pins[self.slot].fetch_sub(1, Ordering::AcqRel);
        assert!(previous > 0);
    }
}

// Capture issuance and the active attempt with the history being searched.
// Otherwise a concurrent writer could make a still-running attempt look aborted.
struct ResolutionView<'db> {
    database: &'db Database,
    slot: usize,
    record: WalRecord,
    active: Option<TransactionId>,
}

impl Drop for ResolutionView<'_> {
    fn drop(&mut self) {
        let registry = self
            .database
            .catalog_registry()
            .expect("resolution owns database borrow");
        let previous = registry.resolution_pins[self.slot].fetch_sub(1, Ordering::AcqRel);
        assert!(previous > 0);
    }
}

pub(crate) struct Writer<'db> {
    pub(super) database: &'db Database,
    slot: usize,
    pub(super) prior: WalRecord,
    pub(super) issued: Option<WalRecord>,
    finished: bool,
    temporary_bytes: u64,
}

pub(crate) struct Maintenance<'db> {
    pub(super) database: &'db Database,
    pub(super) prior: WalRecord,
    finished: bool,
}

impl Maintenance<'_> {
    pub(super) fn finish(mut self) -> Result<(), Error> {
        let mut state = self
            .database
            .registry()?
            .finish_owned_write()
            .map_err(|error| self.fail(error))?;
        assert!(matches!(state.active, Some(Active::Maintenance)));
        state.active = None;
        self.finished = true;
        Ok(())
    }

    pub(super) fn fail(&self, error: Error) -> Error {
        self.database.fail_catalog(self.prior, error)
    }
}

impl Drop for Maintenance<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.database
                .registry()
                .expect("maintenance owns registry")
                .unavailable
                .store(true, Ordering::Release);
        }
    }
}

impl Database {
    pub(super) fn catalog_maintenance(&self) -> Result<Maintenance<'_>, Error> {
        // Cleanup must remain possible with every snapshot slot pinned or the
        // transaction counter exhausted. It excludes writers but reserves no
        // new slot and issues no transaction.
        let mut state = self.registry()?.enter()?;
        if state.active.is_some() {
            return Err(Error::Contention("catalog writer"));
        }
        let prior = state.slots[state.current];
        state.active = Some(Active::Maintenance);
        Ok(Maintenance {
            database: self,
            prior,
            finished: false,
        })
    }

    pub(super) fn check_catalog_namespace(
        &self,
        expected: WalRecord,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        let root = self.path();
        if crate::storage::recovery::validate_lock_entry(root, effects)? != self.lease().identity()
        {
            return Err(Error::Corrupt("catalog writer lease identity changed"));
        }
        let observed = crate::storage::recovery::inspect_namespace(root, &self.memory, effects)?;
        let observed = WalRecord {
            database: observed.database_id,
            issued: observed.issued,
            state: observed.state,
        };
        if observed != expected {
            return Err(Error::Corrupt(
                "catalog writer namespace differs from admitted view",
            ));
        }
        Ok(())
    }

    fn fail_catalog(&self, prior: WalRecord, error: Error) -> Error {
        self.registry()
            .expect("catalog owner")
            .unavailable
            .store(true, Ordering::Release);
        Error::RecoveryRequired {
            generation: generation(prior),
            source: ErrorCause::from_error(error),
        }
    }

    pub(super) fn registry(&self) -> Result<&Registry, Error> {
        self.catalog_registry()
            .ok_or(Error::Unsupported("catalog database required"))
    }

    pub(crate) fn catalog_snapshot(&self) -> Result<Snapshot<'_>, Error> {
        let registry = self.registry()?;
        let state = registry.enter()?;
        let slot = state.current;
        let record = state.slots[slot];
        let RootState::Catalog(commit) = record.state else {
            unreachable!("catalog registry")
        };
        // Pin while holding the selection lock. A writer must not recycle the
        // selected slot between reading current and publishing the pin.
        Registry::increment(&registry.data_pins[slot])?;
        Ok(Snapshot {
            database: self,
            slot,
            commit,
        })
    }

    pub(crate) fn catalog_writer(&self) -> Result<Writer<'_>, Error> {
        let registry = self.registry()?;
        let mut state = registry.enter()?;
        if state.active.is_some() {
            return Err(Error::Contention("catalog writer"));
        }
        let prior = state.slots[state.current];
        if prior.issued == u64::MAX || generation(prior) == crate::storage::format::MAX_SUCCESSES {
            return Err(Error::Unsupported(
                "catalog issuance or success capacity exhausted",
            ));
        }
        // Never overwrite current or pinned history. Slot exhaustion is a
        // resource refusal; waiting for readers would make admission unbounded.
        let slot = (0..SLOTS)
            .find(|&slot| {
                slot != state.current
                    && registry.data_pins[slot].load(Ordering::Acquire) == 0
                    && registry.resolution_pins[slot].load(Ordering::Acquire) == 0
            })
            .ok_or(Error::Resource {
                owner: "catalog snapshot slots",
                required: SLOTS as u64 + 1,
                limit: SLOTS as u64,
            })?;
        state.active = Some(Active::Writer {
            slot,
            attempt: None,
        });
        Ok(Writer {
            database: self,
            slot,
            prior,
            issued: None,
            finished: false,
            temporary_bytes: 0,
        })
    }

    /// Check whether the identified write committed or aborted.
    ///
    /// This reads the stored transaction history without repairing files. Close
    /// and reopen first if a commit's outcome was uncertain. An identifier from
    /// another database, or one never issued, returns `NotFound`. An active append
    /// returns `Contention`; a database that needs recovery returns `RecoveryRequired`.
    pub fn resolve_commit(&self, transaction: TransactionId) -> Result<CommitResolution, Error> {
        self.resolve_commit_with_effects(transaction, &mut Effects::default())
    }

    pub(crate) fn resolve_commit_with_effects(
        &self,
        transaction: TransactionId,
        effects: &mut Effects,
    ) -> Result<CommitResolution, Error> {
        if self.needs_reopen() {
            return Err(crate::error::recovery_needed(self.generation()));
        }
        if self.catalog_registry().is_some() {
            // A healthy append reserves temporary space too. Ask the registry
            // whether work is active or needs cleanup, rather than treating every
            // reservation as evidence of a failed write.
            return self.resolve_catalog(transaction, effects);
        }
        if self.temporary.reserved() != 0 {
            return Err(crate::error::recovery_needed(self.generation()));
        }
        if !transaction.belongs_to(self.database_identity()) {
            return Err(Error::NotFound);
        }
        let namespace = inspect_namespace(self.path(), &self.memory, effects)?;
        if namespace.database_id != self.database_identity()
            || namespace.generation != self.generation()
        {
            return Err(Error::Corrupt("database changed before commit resolution"));
        }
        if transaction.sequence() > namespace.issued {
            return Err(Error::NotFound);
        }
        match namespace.state {
            RootState::Catalog(_) => unreachable!("catalog resolution captures registry facts"),
            RootState::Data {
                transaction: committed,
                ..
            } if committed == transaction => Ok(CommitResolution::Durable(Commit {
                transaction,
                generation: 1,
            })),
            _ => Ok(CommitResolution::Aborted),
        }
    }

    pub(crate) fn resolve_catalog(
        &self,
        transaction: TransactionId,
        effects: &mut Effects,
    ) -> Result<CommitResolution, Error> {
        let registry = self.registry()?;
        let view = {
            let state = registry.enter()?;
            let slot = state.current;
            Registry::increment(&registry.resolution_pins[slot])?;
            ResolutionView {
                database: self,
                slot,
                record: state.slots[slot],
                active: state.active.and_then(Active::transaction),
            }
        };
        if !transaction.belongs_to(view.record.database)
            || transaction.sequence() > view.record.issued
        {
            return Err(Error::NotFound);
        }
        if view.active == Some(transaction) {
            return Err(Error::Contention("transaction is active"));
        }
        let mut scratch = catalog::Scratch::new(&self.memory, "catalog resolution scratch")?;
        match crate::storage::history::find(
            &joined_path(self.path(), UNITS_NAME)?,
            view.record,
            transaction,
            scratch.bytes(),
            &CancellationToken::new(),
            effects,
        )? {
            Some(generation) => Ok(CommitResolution::Durable(Commit {
                transaction,
                generation,
            })),
            None => Ok(CommitResolution::Aborted),
        }
    }
}

impl Writer<'_> {
    // Construction admission occurs before issuance or file creation. The
    // prepared-graph test boundary may reserve zero; actual builders must pass
    // their checked peak extent. Refusal leaves this writer usable for abort.
    pub(crate) fn reserve_construction(&mut self, bytes: u64) -> Result<(), Error> {
        drop(self.database.registry()?.enter()?);
        if self.issued.is_some() {
            return Err(Error::Unsupported(
                "construction admission precedes issuance",
            ));
        }
        let next = self
            .temporary_bytes
            .checked_add(bytes)
            .ok_or(Error::Resource {
                owner: "catalog construction bytes",
                required: u64::MAX,
                limit: self.database.temporary.limit(),
            })?;
        self.database.temporary.reserve(bytes)?;
        self.temporary_bytes = next;
        Ok(())
    }

    fn check_namespace(&self, expected: WalRecord, effects: &mut Effects) -> Result<(), Error> {
        self.database.check_catalog_namespace(expected, effects)
    }

    pub(super) fn fail(&self, error: Error) -> Error {
        self.database.fail_catalog(self.prior, error)
    }

    // Persist the next attempt number without changing the committed catalog.
    // Aborting later must not allow that transaction number to be reused.
    pub(crate) fn issue(
        &mut self,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<TransactionId, Error> {
        let registry = self.database.registry()?;
        drop(registry.enter()?);
        if self.issued.is_some() {
            return Err(Error::Unsupported("writer attempt already issued"));
        }
        self.check_namespace(self.prior, effects)
            .map_err(|e| self.fail(e))?;
        let next = WalRecord {
            issued: self.prior.issued.checked_add(1).expect("admitted issuance"),
            ..self.prior
        };
        if let Err(failure) =
            publish_snapshot(self.database.path(), self.prior, next, cancel, effects)
        {
            return Err(self.fail(failure.error));
        }
        let mut state = registry.finish_owned_write().map_err(|e| self.fail(e))?;
        let active = state.active.as_mut().expect("writer owns active slot");
        assert_eq!(active.writer_slot(), self.slot);
        let transaction =
            TransactionId::for_attempt(next.database, next.issued).expect("admitted attempt");
        let Active::Writer { attempt, .. } = active else {
            unreachable!("writer owns admission")
        };
        *attempt = Some(transaction);
        // Issuance changes only the attempt prefix. Existing data pins still
        // refer to the same immutable catalog in this slot.
        let current = state.current;
        state.slots[current] = next;
        self.issued = Some(next);
        Ok(transaction)
    }

    #[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
    pub(crate) fn issued_record(&self) -> Option<WalRecord> {
        self.issued
    }

    // Release writer admission only after the builder has removed and synced
    // every private object it created. This controller owns no graph files.
    pub(crate) fn abort_unbuilt(mut self) -> Result<(), Error> {
        let mut state = self.database.registry()?.finish_owned_write()?;
        assert_eq!(
            state.active.expect("writer owns active slot").writer_slot(),
            self.slot
        );
        state.active = None;
        self.database.temporary.release(self.temporary_bytes);
        self.temporary_bytes = 0;
        self.finished = true;
        Ok(())
    }

    // Builders have closed and synced every dependency. Publication validates
    // the graph independently and owns the outcome from this point onward.
    pub(crate) fn commit_prepared(
        mut self,
        commit: CatalogCommit,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Commit, Error> {
        let prior = self
            .issued
            .ok_or(Error::Unsupported("commit requires issued attempt"))?;
        let next = WalRecord {
            state: RootState::Catalog(Some(commit)),
            ..prior
        };
        if next.transition_from(prior) != Some(crate::storage::format::PublicationKind::Commit) {
            return Err(Error::Corrupt(
                "prepared graph is not the writer's next commit",
            ));
        }
        self.check_namespace(prior, effects)
            .map_err(|e| self.fail(e))?;
        let mut scratch =
            catalog::Scratch::new(&self.database.memory, "catalog commit validation")?;
        catalog::validate_snapshot(
            &joined_path(self.database.path(), UNITS_NAME)?,
            next,
            scratch.bytes(),
            cancel,
            effects,
        )?;
        drop(scratch);
        if let Err(failure) = publish_snapshot(self.database.path(), prior, next, cancel, effects) {
            let error = self.fail(failure.error);
            return Err(if failure.stage == FailureStage::Uncertain {
                Error::CommitAmbiguous {
                    transaction: commit.transaction(),
                    source: ErrorCause::from_error(error),
                }
            } else {
                error
            });
        }
        // Disk publication has succeeded. Failure to update live visibility
        // cannot turn that durable commit into an abort.
        let registry = self.database.registry()?;
        let mut state = registry.finish_owned_write().map_err(|error| {
            let error = self.fail(error);
            Error::CommitAmbiguous {
                transaction: commit.transaction(),
                source: ErrorCause::from_error(error),
            }
        })?;
        assert_eq!(
            state.active.expect("writer owns slot").writer_slot(),
            self.slot
        );
        assert_eq!(registry.data_pins[self.slot].load(Ordering::Acquire), 0);
        assert_eq!(
            registry.resolution_pins[self.slot].load(Ordering::Acquire),
            0
        );
        state.slots[self.slot] = next;
        state.current = self.slot;
        state.active = None;
        registry
            .generation
            .store(commit.generation(), Ordering::Release);
        self.database.temporary.release(self.temporary_bytes);
        self.temporary_bytes = 0;
        self.finished = true;
        Ok(Commit {
            transaction: commit.transaction(),
            generation: commit.generation(),
        })
    }
}

impl Drop for Writer<'_> {
    fn drop(&mut self) {
        if !self.finished {
            // Drop cannot prove rollback. Retain any temporary-space debt and
            // prevent new operations until exclusive reopen examines the files.
            self.database
                .registry()
                .expect("writer owns registry")
                .unavailable
                .store(true, Ordering::Release);
        }
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;
