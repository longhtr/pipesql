//! Load a lineitem text file into an empty legacy database.
//!
//! The loader reads the input twice so it can reserve enough disk space before
//! writing. The first pass validates rows and counts them. The second converts
//! seven fields into column files, using fixed buffers rather than keeping the
//! table in memory. `unit` combines those files into one storage unit, verifies
//! the bytes it wrote, and publishes the result through the shared commit code.
//!
//! This module owns the operation's memory and temporary-space reservations.
//! It also decides whether the database can be used after an error. A failure
//! before publication can be rolled back; an error during publication may leave
//! the commit uncertain. In that case the caller must reopen the database and
//! resolve the returned transaction token before deciding whether to retry.
//!
//! Legacy databases allow one successful load. Declared tables support repeated
//! writes through `storage::append` instead.

use crate::effects::Effects;
use crate::storage::format::{self, RootState, WalRecord};
use crate::storage::legacy::parser::MAX_CHUNK_BYTES;
use crate::storage::publication::{FailureStage, publish_snapshot};
use crate::storage::recovery::recover_namespace;
use crate::{CancellationToken, Commit, Database, Error, TransactionId};
use input::{inspect_input, scan_pass};
use staging::{BLOCK_BYTES, STAGING_BUFFER_BYTES};
use std::path::Path;
use unit::{LoadContext, build_and_publish};

mod input;
pub(super) mod parser;
mod staging;
mod unit;

const LOAD_MEMORY_BYTES: u64 = 1_867_776;
const ARENA_REQUESTED_BYTES: usize =
    MAX_CHUNK_BYTES + STAGING_BUFFER_BYTES + format::DESCRIPTOR_BYTES + BLOCK_BYTES;
const ARENA_CAPACITY_CEILING: usize = 1_802_240;

impl Database {
    /// Load the fixed-schema lineitem table into an empty legacy database.
    ///
    /// The input must be a regular file, with no final symlink, and must stay
    /// unchanged throughout the operation. Its absolute path and lineitem rows
    /// must satisfy the input limits in `docs/cli.md#legacy-input`.
    ///
    /// Success returns the committed transaction and generation. An uncertain
    /// commit returns [`Error::CommitAmbiguous`]; reopen and use
    /// [`Database::resolve_commit`] to determine its outcome. Uncertainty or failed
    /// cleanup retains the temporary-space charge and prevents further loading
    /// through this handle.
    pub fn load_lineitem(
        &mut self,
        input: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Commit, Error> {
        let mut effects = Effects::default();
        self.load_lineitem_with_effects(input, cancellation, &mut effects)
    }

    fn load_lineitem_with_effects(
        &mut self,
        input: &Path,
        cancellation: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Commit, Error> {
        if matches!(self.state, crate::database::DatabaseState::Catalog) {
            return Err(Error::Unsupported(
                "lineitem loading requires a legacy database",
            ));
        }
        if self.temporary.reserved() != 0 {
            return Err(Error::Unsupported(
                "reopen is required after incomplete load cleanup",
            ));
        }
        if self.needs_reopen() {
            return Err(Error::Unsupported(
                "reopen is required after unsettled namespace mutation",
            ));
        }
        if self.generation() != 0 {
            return Err(Error::Unsupported(
                "legacy databases admit exactly one successful lineitem load",
            ));
        }
        cancellation.check()?;
        let memory = self.memory.reserve(LOAD_MEMORY_BYTES, "lineitem load")?;
        // Recovery may change files and then fail to synchronize them. Mark the
        // handle unusable first, so an error or panic cannot expose that state.
        self.state = crate::database::DatabaseState::ReopenRequired;
        let namespace = recover_namespace(
            self.path(),
            self.lease(),
            Some(self.database_identity()),
            &self.memory,
            effects,
        )?;
        if namespace.generation != 0 {
            return Err(Error::Corrupt("database changed before load"));
        }
        self.state = crate::database::DatabaseState::Empty;
        let mut arena = Vec::new();
        arena
            .try_reserve_exact(ARENA_REQUESTED_BYTES)
            .map_err(|_| Error::Resource {
                owner: "lineitem load allocation",
                required: LOAD_MEMORY_BYTES,
                limit: self.memory.limit(),
            })?;
        let arena_capacity = u64::try_from(arena.capacity()).map_err(|_| Error::Resource {
            owner: "lineitem load allocator capacity",
            required: u64::MAX,
            limit: u64::try_from(ARENA_CAPACITY_CEILING).expect("arena capacity ceiling fits u64"),
        })?;
        if arena.capacity() < ARENA_REQUESTED_BYTES || arena.capacity() > ARENA_CAPACITY_CEILING {
            return Err(Error::Resource {
                owner: "lineitem load allocator capacity",
                required: arena_capacity,
                limit: u64::try_from(ARENA_CAPACITY_CEILING)
                    .expect("arena capacity ceiling fits u64"),
            });
        }
        arena.resize(ARENA_REQUESTED_BYTES, 0);

        let source = inspect_input(input, &self.memory, effects)?;
        let first = scan_pass(
            &source,
            &mut arena[..MAX_CHUNK_BYTES],
            cancellation,
            effects,
            None,
        )?;
        let layout = format::layout(first.rows).map_err(|_| Error::Input {
            message: "row count cannot be represented by format 4",
            byte_offset: first.input_bytes,
        })?;
        let issued = namespace
            .issued
            .checked_add(1)
            .ok_or(Error::Unsupported("attempt identity capacity exhausted"))?;
        let transaction = TransactionId::for_attempt(self.database_identity(), issued)
            .map_err(|_| Error::Corrupt("invalid next attempt"))?;
        self.temporary.reserve(layout.temporary_peak_bytes)?;
        self.state = crate::database::DatabaseState::ReopenRequired;
        // Persist the attempt number before constructing data. Recovery must
        // never reuse a number that could identify an earlier interrupted load.
        let issuance = WalRecord {
            database: self.database_identity(),
            issued,
            state: RootState::Empty,
        };
        let prior = WalRecord {
            database: namespace.database_id,
            issued: namespace.issued,
            state: namespace.state,
        };
        let root = self.path();
        if let Err(failure) = publish_snapshot(root, prior, issuance, cancellation, effects) {
            // Only the attempt number may have changed; no data has been built.
            // Reopen to recover metadata, without reporting an uncertain data commit.
            self.temporary.release(layout.temporary_peak_bytes);
            return Err(Error::RecoveryRequired {
                generation: 0,
                source: crate::ErrorCause::from_error(failure.error),
            });
        }
        let context = LoadContext {
            root,
            database_id: self.database_identity(),
            transaction,
            source: &source,
            first,
            layout,
            cancellation,
        };
        let operation = build_and_publish(&context, &mut arena, effects);
        let outcome = match operation {
            Ok(()) => Ok(Commit {
                transaction,
                generation: 1,
            }),
            Err(failure) if failure.stage == FailureStage::Uncertain => {
                Err(Error::CommitAmbiguous {
                    transaction,
                    source: crate::ErrorCause::from_error(failure.error),
                })
            }
            Err(failure) => {
                match recover_namespace(
                    root,
                    self.lease(),
                    Some(self.database_identity()),
                    &self.memory,
                    effects,
                ) {
                    Ok(namespace) if namespace.generation == 0 => Err(failure.error),
                    Ok(_) => Err(Error::CommitAmbiguous {
                        transaction,
                        source: crate::ErrorCause::from_error(failure.error),
                    }),
                    Err(cleanup) => Err(Error::CleanupRequired {
                        primary: crate::ErrorCause::from_error(failure.error),
                        cleanup: crate::ErrorCause::from_error(cleanup),
                    }),
                }
            }
        };
        // Release disk space only after commit or confirmed rollback. A failed
        // publication or cleanup may leave files that recovery still needs to
        // remove, even if they have already moved out of the private directory.
        if !matches!(
            &outcome,
            Err(Error::CommitAmbiguous { .. } | Error::CleanupRequired { .. })
        ) {
            self.temporary.release(layout.temporary_peak_bytes);
        }
        drop(arena);
        drop(memory);
        self.state = match &outcome {
            Ok(_) => crate::database::DatabaseState::Loaded,
            Err(Error::CommitAmbiguous { .. } | Error::CleanupRequired { .. }) => {
                crate::database::DatabaseState::ReopenRequired
            }
            Err(_) => crate::database::DatabaseState::Empty,
        };
        outcome
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;
