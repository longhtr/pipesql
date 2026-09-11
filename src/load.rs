//! Legacy lineitem load admission and outcome ownership. Two input passes build
//! one private unit; the shared publisher and recovery settle its commit outcome.

use crate::effects::Effects;
use crate::load_input::MAX_CHUNK_BYTES;
use crate::namespace::validate_namespace;
use crate::publication::{FailureStage, publish_snapshot};
use crate::storage_format::{self, RootState, WalRecord};
use crate::{CancellationToken, Commit, Database, Error, TransactionId};
use input::{inspect_input, scan_pass};
use staging::{BLOCK_BYTES, STAGING_BUFFER_BYTES};
use std::path::Path;
use unit::{LoadContext, build_and_publish};

mod input;
mod staging;
mod unit;

const LOAD_MEMORY_BYTES: u64 = 1_867_776;
const ARENA_REQUESTED_BYTES: usize =
    MAX_CHUNK_BYTES + STAGING_BUFFER_BYTES + storage_format::DESCRIPTOR_BYTES + BLOCK_BYTES;
const ARENA_CAPACITY_CEILING: usize = 1_802_240;

impl Database {
    /// Load the fixed-schema lineitem table into an empty legacy database.
    ///
    /// Input must be an unchanged regular file at an absolute bounded path. Only
    /// one load may commit. An uncertain commit or failed cleanup retains its
    /// reservation and requires reopen before resolution or further work.
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
        // Recovery can mutate before input construction or temporary admission.
        // Keep the handle unavailable on any error or unwind from that phase;
        // visible repaired names alone cannot establish their failed durability.
        self.state = crate::database::DatabaseState::ReopenRequired;
        let namespace = validate_namespace(
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

        let source = inspect_input(input, effects)?;
        let first = scan_pass(
            &source,
            &mut arena[..MAX_CHUNK_BYTES],
            cancellation,
            effects,
            None,
        )?;
        let layout = storage_format::layout(first.rows).map_err(|_| Error::Input {
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
            // No data construction has begun. Issuance may be durable, but this
            // is not an ambiguous DATA commit and no attempt token is exposed.
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
                match validate_namespace(
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
        // A unit's directory name does not settle its owner. Keep the original
        // construction reservation until success or confirmed rollback; an
        // ambiguous publication may still leave an unreferenced unit to recover.
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
