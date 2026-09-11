//! Serialized catalog reclamation. A validated inventory protects current and
//! pinned graphs before any unlink. Scratch is disposable; deletion becomes
//! durable only after the units directory barrier.
use super::{Active, Maintenance, SLOTS};
use crate::catalog::{self, ObjectId};
use crate::effects::{DirectoryKind, Effect, Effects};
use crate::error::io_error;
use crate::namespace::UNITS_NAME;
use crate::path::joined_path;
use crate::storage_format::{CatalogCommit, RootState};
use crate::{CancellationToken, Database, Error};
use pipesql_filesystem as filesystem;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

mod inventory;

#[derive(Clone, Copy, PartialEq)]
enum Dirty {
    None,
    Units,
}

impl Database {
    /// Reclaim obsolete objects in a declared-table database. Success reports
    /// durably removed names. Errors may leave partial cleanup; live snapshots
    /// and committed receipts remain protected. RecoveryRequired needs reopen.
    pub fn reclaim(&self, cancellation: &CancellationToken) -> Result<u64, Error> {
        self.reclaim_with_effects(cancellation, &mut Effects::default())
    }

    fn reclaim_with_effects(
        &self,
        cancellation: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<u64, Error> {
        cancellation.check()?;
        let maintenance = self.catalog_maintenance()?;
        let result = reclaim(&maintenance, cancellation, effects);
        if matches!(&result, Err(Error::RecoveryRequired { .. })) {
            self.registry()?.unavailable.store(true, Ordering::Release);
        }
        if !self.registry()?.unavailable() {
            maintenance.finish()?;
        }
        result
    }
}

fn reclaim(
    owner: &Maintenance<'_>,
    cancel: &CancellationToken,
    effects: &mut Effects,
) -> Result<u64, Error> {
    {
        let _paths = owner.database.memory.reserve(
            (2 * crate::path::MAX_PATH_BYTES) as u64,
            "reclamation validation paths",
        )?;
        owner
            .database
            .check_catalog_namespace(owner.prior, effects)?;
    }
    let scratch = crate::scratch::Scratch::new(owner.database, cancel, effects)?;
    // The scratch constructor releases its pathname charge before these owners.
    let _paths = owner.database.memory.reserve(
        (2 * crate::path::MAX_PATH_BYTES) as u64,
        "reclamation paths",
    )?;
    let objects = joined_path(owner.database.path(), UNITS_NAME)?;
    let mut dirty = Dirty::None;
    let result = (|| {
        let mut inventory = inventory::Inventory::from_scratch(owner, scratch)?;
        inventory.scan(cancel, effects)?;
        let mut removed = 0u64;
        while let Some(id) = inventory.next(cancel, effects)? {
            let name = id.name();
            let path = joined_path(
                &objects,
                std::str::from_utf8(&name).expect("ASCII object name"),
            )?;
            cancel.check()?;
            dirty = Dirty::Units;
            let unlink = Effect::RemoveCleanupFile;
            effects.before(unlink)?;
            filesystem::remove_file(path)
                .map_err(|e| io_error("remove obsolete catalog object", e))?;
            effects.after(unlink);
            removed = removed.checked_add(1).expect("namespace removal bound");
        }
        Ok(removed)
    })();
    // The inventory and its unlinked scratch owners are gone before this
    // barrier. Cancellation cannot bypass synchronization of attempted unlink.
    match dirty {
        Dirty::None => result,
        Dirty::Units => {
            match crate::namespace::sync_directory(&objects, effects, DirectoryKind::Units) {
                Ok(()) => result,
                Err(error) => Err(owner.fail(error)),
            }
        }
    }
}

#[derive(Clone, Copy)]
struct View {
    commit: Option<CatalogCommit>,
    data: bool,
    history: bool,
}

#[derive(Clone, Copy)]
enum Phase {
    History,
    Catalog,
    Schema,
    Index,
    Units,
    Finished,
    Failed,
}

struct Reachable<'a, 'db> {
    // The admitted writer prevents publication/reclamation while captured views
    // are traversed, even if a reader releases its pin after capture.
    writer: &'a Maintenance<'db>,
    views: [View; SLOTS],
    view: usize,
    table: usize,
    tables: usize,
    phase: Phase,
    index: Option<crate::table_data::Cursor>,
    bytes: Vec<u8>,
    objects: PathBuf,
    // Physical buffer/path/index owners disappear before their charge.
    _charge: crate::resources::Reservation<'a>,
}

impl<'a, 'db> Reachable<'a, 'db> {
    // One catalog buffer, one retained pathname and one transient object path.
    // The fixed inline cursor (including one index page) is separately bounded.
    const HEAP_ADMISSION: u64 = (catalog::MAX_BYTES + 2 * crate::path::MAX_PATH_BYTES) as u64;

    fn open(writer: &'a Maintenance<'db>) -> Result<Self, Error> {
        let registry = writer.database.registry()?;
        let views = {
            let state = registry.enter()?;
            assert!(matches!(state.active, Some(Active::Maintenance)));
            std::array::from_fn(|slot| {
                let current = slot == state.current;
                let RootState::Catalog(commit) = state.slots[slot].state else {
                    unreachable!("catalog registry contains catalog records")
                };
                View {
                    commit,
                    data: current || registry.data_pins[slot].load(Ordering::Acquire) != 0,
                    history: current || registry.resolution_pins[slot].load(Ordering::Acquire) != 0,
                }
            })
        };
        let mut charge = writer
            .database
            .memory
            .reserve(Self::HEAP_ADMISSION, "reclamation reference walk")?;
        let objects = joined_path(writer.database.path(), UNITS_NAME)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(catalog::MAX_BYTES)
            .map_err(|_| Error::Resource {
                owner: "reclamation catalog allocation",
                required: catalog::MAX_BYTES as u64,
                limit: writer.database.memory.limit(),
            })?;
        if bytes.capacity() != catalog::MAX_BYTES {
            return Err(Error::Resource {
                owner: "reclamation catalog capacity",
                required: bytes.capacity() as u64,
                limit: catalog::MAX_BYTES as u64,
            });
        }
        bytes.resize(catalog::MAX_BYTES, 0);
        charge.shrink_to(
            (bytes.capacity() + objects.capacity() + crate::path::MAX_PATH_BYTES) as u64,
        );
        Ok(Self {
            writer,
            views,
            view: 0,
            table: 0,
            tables: 0,
            phase: Phase::History,
            index: None,
            bytes,
            objects,
            _charge: charge,
        })
    }

    fn table(&self) -> Result<catalog::TableEntry<'_>, Error> {
        let reference = self.views[self.view].commit.expect("data view").catalog();
        let bytes = &self.bytes[..reference.bytes() as usize];
        // Re-decode a bounded, privately owned catalog instead of retaining a
        // self-reference. At most two decodes per table; no additional I/O.
        let catalog = catalog::decode(bytes, self.writer.database.database_identity(), reference)
            .map_err(|error| crate::error::map_format_error(error, bytes))?;
        catalog
            .table(self.table)
            .ok_or(Error::Corrupt("reference walk table index"))
    }

    fn next(
        &mut self,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Option<ObjectId>, Error> {
        if matches!(self.phase, Phase::Failed) {
            return Err(Error::Corrupt("reference walk has failed"));
        }
        let result = self.advance(cancel, effects);
        if result.is_err() {
            self.index = None;
            self.phase = Phase::Failed;
        }
        result
    }

    fn advance(
        &mut self,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Option<ObjectId>, Error> {
        // Empty views take at most three transitions each; an exhausted index
        // or empty table needs at most two more before an output or completion.
        for _ in 0..SLOTS * 3 + 3 {
            if matches!(self.phase, Phase::Finished) {
                return Ok(None);
            }
            cancel.check()?;
            if self.view == SLOTS {
                self.phase = Phase::Finished;
                return Ok(None);
            }
            let view = self.views[self.view];
            match self.phase {
                Phase::History => {
                    self.phase = Phase::Catalog;
                    if view.history
                        && let Some(commit) = view.commit
                    {
                        return Ok(Some(commit.successes().object()));
                    }
                }
                Phase::Catalog => {
                    if view.data
                        && let Some(commit) = view.commit
                    {
                        let catalog = catalog::read(
                            &self.objects,
                            self.writer.database.database_identity(),
                            commit.catalog(),
                            &mut self.bytes,
                            cancel,
                            effects,
                        )?;
                        self.tables = catalog.len();
                        self.table = 0;
                        self.phase = Phase::Schema;
                        return Ok(Some(commit.catalog().object()));
                    }
                    self.view += 1;
                    self.phase = Phase::History;
                }
                Phase::Schema => {
                    if self.table == self.tables {
                        self.view += 1;
                        self.phase = Phase::History;
                    } else {
                        let id = self.table()?.schema_object();
                        self.phase = Phase::Index;
                        return Ok(Some(id));
                    }
                }
                Phase::Index => {
                    let table = self.table()?;
                    if let Some(reference) = table.data() {
                        self.index = crate::table_data::open(
                            &self.objects,
                            self.writer.database.database_identity(),
                            table,
                            cancel,
                            effects,
                        )?;
                        assert!(self.index.is_some(), "nonempty table index");
                        self.phase = Phase::Units;
                        return Ok(Some(reference.object()));
                    }
                    self.table += 1;
                    self.phase = Phase::Schema;
                }
                Phase::Units => {
                    if let Some(unit) = self
                        .index
                        .as_mut()
                        .expect("index phase owns cursor")
                        .next(cancel, effects)?
                    {
                        return Ok(Some(unit.object()));
                    }
                    self.index = None;
                    self.table += 1;
                    self.phase = Phase::Schema;
                }
                Phase::Finished | Phase::Failed => unreachable!("terminal phase checked above"),
            }
        }
        unreachable!("reference walk advances within its transition bound")
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;
