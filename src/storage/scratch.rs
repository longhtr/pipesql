//! Provide two disposable files for work that does not fit in memory.
//!
//! Sorting and inventory passes can read one file while writing the other, then
//! exchange their roles. These files never become table data. Their contents may
//! be discarded when the operation ends, so writing them requires no durability
//! synchronization.
//!
//! Construction creates each file, keeps its descriptor open, then removes its
//! name. Unix permits reading and writing the unlinked file until the last
//! descriptor closes. The directory is synchronized before returning the pair,
//! so a crash cannot restore names for scratch that was already put into use.
//!
//! The database serializes creation because every pair uses the same two names.
//! Once their removal is durable, another pair can be created while this one is
//! still in use. Uncertain removal blocks further scratch creation until reopen;
//! a destructor records that requirement without attempting filesystem cleanup.
//!
//! Each file's reserved extent tracks the temporary-space budget. Writes reserve
//! any extension before I/O and retain it on failure. Reset truncates a file before
//! releasing its charge; drop closes both files before releasing the remaining
//! charges. Reads are limited to the reserved extent, which may include a failed
//! write and therefore does not itself prove that valid data exists there.

use crate::effects::{DirectoryKind, Effect, Effects, LoadEffect, QueryEffect};
use crate::error::{io_error, recovery_needed};
use crate::path::{MAX_PATH_BYTES, joined_path};
use crate::resources::Reservation;
use crate::storage::recovery::{
    PRIVATE_NAME, SCRATCH_NAMES, inspect_known_entries, sync_directory, validate_directory,
};
use crate::{CancellationToken, Database, Error, ErrorCause};
use pipesql_filesystem as filesystem;
use std::fs::File;
use std::sync::atomic::{AtomicU8, Ordering};

const IDLE: u8 = 0;
const CREATING: u8 = 1;
const REOPEN: u8 = 2;
const PATH_BYTES: u64 = (2 * MAX_PATH_BYTES) as u64;

/// Allow only one constructor to use the shared scratch filenames at a time.
/// Finished scratch owners no longer have names and do not hold this admission.
#[repr(transparent)]
pub(crate) struct Admission {
    state: AtomicU8,
}

enum AdmissionFailure {
    Creating,
    RecoveryRequired,
}

impl Admission {
    pub(crate) const fn new() -> Self {
        Self {
            state: AtomicU8::new(IDLE),
        }
    }

    fn begin(&self) -> Result<Bootstrap<'_>, AdmissionFailure> {
        match self
            .state
            .compare_exchange(IDLE, CREATING, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => Ok(Bootstrap {
                admission: self,
                requires_recovery: false,
            }),
            Err(CREATING) => Err(AdmissionFailure::Creating),
            Err(REOPEN) => Err(AdmissionFailure::RecoveryRequired),
            Err(_) => unreachable!("finite scratch bootstrap state"),
        }
    }
}

// Reserve pathname memory before grouping spends optional memory on its hash
// table. Keep that space available if grouping later spills. Reserving memory alone
// does not acquire permission to use the shared scratch filenames.
pub(crate) struct Creation<'db> {
    database: &'db Database,
    paths: Reservation<'db>,
}

impl<'db> Creation<'db> {
    pub(crate) const fn memory_requirement_bytes() -> u64 {
        PATH_BYTES
    }

    pub(crate) fn reserve(database: &'db Database) -> Result<Self, Error> {
        Ok(Self {
            database,
            paths: database.memory.reserve(PATH_BYTES, "scratch paths")?,
        })
    }

    pub(crate) fn create(
        self,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Scratch<'db>, Error> {
        Scratch::create(self.database, cancel, effects, Some(self.paths))
    }
}

struct Bootstrap<'a> {
    admission: &'a Admission,
    requires_recovery: bool,
}

impl Drop for Bootstrap<'_> {
    fn drop(&mut self) {
        // Once creation may have changed names, require durable removal before
        // allowing reuse. During unwinding, leave any filesystem repair to reopen.
        self.admission.state.store(
            if self.requires_recovery { REOPEN } else { IDLE },
            Ordering::Release,
        );
    }
}

pub(crate) struct Scratch<'a> {
    files: Option<[File; 2]>,
    database: &'a Database,
    extents: [u64; 2],
}

impl<'a> Scratch<'a> {
    pub(crate) fn new(
        database: &'a Database,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Self, Error> {
        Self::create(database, cancel, effects, None)
    }

    fn create(
        database: &'a Database,
        cancel: &CancellationToken,
        effects: &mut Effects,
        paths: Option<Reservation<'a>>,
    ) -> Result<Self, Error> {
        cancel.check()?;
        if database.needs_reopen() {
            return Err(recovery_needed(database.generation()));
        }
        let mut bootstrap = match database.scratch_admission.begin() {
            Ok(bootstrap) => bootstrap,
            Err(AdmissionFailure::Creating) => return Err(Error::Contention("scratch bootstrap")),
            Err(AdmissionFailure::RecoveryRequired) => {
                return Err(recovery_needed(database.generation()));
            }
        };
        let result = Self::create_files(database, cancel, effects, paths, &mut bootstrap);
        if bootstrap.requires_recovery {
            return Err(Error::RecoveryRequired {
                generation: database.generation(),
                source: ErrorCause::from_error(result.err().expect("unfinished bootstrap")),
            });
        }
        result
    }

    fn create_files(
        database: &'a Database,
        cancel: &CancellationToken,
        effects: &mut Effects,
        paths: Option<Reservation<'a>>,
        bootstrap: &mut Bootstrap<'_>,
    ) -> Result<Self, Error> {
        // Hold space for the directory path and one child path at the same time.
        // Declare the reservation first so both paths drop before it is released.
        let _paths = match paths {
            Some(paths) => {
                assert!(
                    paths.belongs_to(&database.memory) && paths.bytes() == PATH_BYTES,
                    "creation memory belongs to this database and covers both paths"
                );
                paths
            }
            None => database.memory.reserve(PATH_BYTES, "scratch paths")?,
        };
        let private = joined_path(database.path(), PRIVATE_NAME)?;
        let identity = validate_directory(&private, effects)?;
        let pending = inspect_known_entries(&private, identity, &SCRATCH_NAMES, effects)?;
        if pending.count != 0 {
            bootstrap.requires_recovery = true;
            return Err(Error::Corrupt("scratch bootstrap found unresolved names"));
        }
        let mut files: [Option<File>; 2] = [None, None];
        for (slot, name) in SCRATCH_NAMES.iter().enumerate() {
            cancel.check()?;
            let path = joined_path(&private, name)?;
            bootstrap.requires_recovery = true;
            let create = Effect::Load(LoadEffect::CreateStaging);
            effects.before(create)?;
            files[slot] = Some(
                filesystem::create_new_read_write(&path)
                    .map_err(|e| io_error("create scratch", e))?,
            );
            effects.after(create);
            cancel.check()?;
            let unlink = Effect::Load(LoadEffect::RemoveStaging);
            effects.before(unlink)?;
            filesystem::remove_file(&path).map_err(|e| io_error("unlink scratch", e))?;
            effects.after(unlink);
        }
        sync_directory(&private, effects, DirectoryKind::Private)?;
        bootstrap.requires_recovery = false;
        let scratch = Self::admit(
            database,
            files.map(|f| f.expect("both scratch files created")),
            effects,
        )?;
        cancel.check()?;
        Ok(scratch)
    }

    // Check that the pair is empty, unlinked and refers to two different files.
    // Production calls this only after creation; tests supply owned descriptors
    // here to exercise later I/O without repeating namespace construction.
    fn admit(
        database: &'a Database,
        files: [File; 2],
        effects: &mut Effects,
    ) -> Result<Self, Error> {
        let mut first_identity = None;
        for file in &files {
            effects.before(Effect::Load(LoadEffect::InspectStaging))?;
            let metadata =
                filesystem::file_metadata(file).map_err(|e| io_error("inspect scratch", e))?;
            if !metadata.file_type().is_file() || metadata.nlink() != 0 || !metadata.is_empty() {
                return Err(Error::Corrupt("scratch must be empty and unlinked"));
            }
            let identity = metadata.identity();
            if first_identity == Some(identity) {
                return Err(Error::Corrupt("scratch files alias"));
            }
            first_identity = Some(identity);
        }
        Ok(Self {
            files: Some(files),
            database,
            extents: [0; 2],
        })
    }

    #[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
    pub(crate) fn from_files(
        database: &'a Database,
        files: [File; 2],
        effects: &mut Effects,
    ) -> Result<Self, Error> {
        Self::admit(database, files, effects)
    }

    fn file(&self, slot: usize) -> &File {
        &self.files.as_ref().expect("scratch owns files")[slot]
    }

    #[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
    pub(crate) fn test_file(&self, slot: usize) -> &File {
        self.file(slot)
    }

    pub(crate) fn read(
        &self,
        slot: usize,
        bytes: &mut [u8],
        offset: u64,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        cancel.check()?;
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or(Error::Corrupt("scratch offset overflow"))?;
        if end > self.extents[slot] {
            return Err(Error::Corrupt("scratch read exceeds admitted extent"));
        }
        crate::effects::read_exact_at(
            self.file(slot),
            bytes,
            offset,
            Effect::Load(LoadEffect::ReadStaging),
            effects,
        )
    }

    pub(crate) fn write(
        &mut self,
        slot: usize,
        offset: u64,
        bytes: &[u8],
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        cancel.check()?;
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or(Error::Corrupt("scratch offset overflow"))?;
        if end > self.extents[slot] {
            self.database.temporary.reserve(end - self.extents[slot])?;
            // A write may fail after changing the file. Keep its full reservation
            // until a successful reset or close discards those bytes.
            self.extents[slot] = end;
        }
        crate::effects::write_all_at(
            self.file(slot),
            bytes,
            offset,
            Effect::Load(LoadEffect::WriteStaging),
            effects,
        )
    }

    pub(crate) fn reset(
        &mut self,
        slot: usize,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        cancel.check()?;
        let extent = *self
            .extents
            .get(slot)
            .ok_or(Error::Corrupt("scratch reset file slot"))?;
        let effect = Effect::Query(QueryEffect::ResetScratch);
        effects.before(effect)?;
        self.file(slot)
            .set_len(0)
            .map_err(|source| io_error("reset query scratch file", source))?;
        effects.after(effect);
        // Release only after successful truncation. On failure, retain the old
        // charge even if the syscall changed the file. The file is already
        // durably unlinked, so discarding its contents needs no synchronization.
        self.extents[slot] = 0;
        self.database.temporary.release(extent);
        Ok(())
    }
}

impl Drop for Scratch<'_> {
    fn drop(&mut self) {
        drop(self.files.take());
        self.database
            .temporary
            .release(self.extents[0] + self.extents[1]);
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;
