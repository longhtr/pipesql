//! Append ownership and committed-attempt receipts.

use crate::effects::Effects;
use crate::namespace::inspect_namespace;
use crate::storage_format::RootState;
use crate::{CancellationToken, ColumnInput, Database, Error, TransactionId, catalog_snapshot};

impl Database {
    /// Inspect an issued attempt without repairing persistent state.
    ///
    /// Returns `NotFound` for a foreign or unissued identity, `Contention` for an
    /// active append, and `RecoveryRequired` when the handle or namespace needs
    /// exclusive recovery. Close and reopen before resolving an uncertain commit.
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
            // The registry distinguishes a healthy active writer from retained
            // cleanup debt. Reserved construction bytes alone are not failure.
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
}

/// A bounded append transaction. Dropping it unfinished requires database reopen.
pub struct Append<'db> {
    pub(crate) inner: catalog_snapshot::append::Append<'db>,
}

impl Append<'_> {
    /// The issued identity to retain when publication has an uncertain outcome.
    pub fn transaction(&self) -> TransactionId {
        self.inner.transaction()
    }

    /// Write one typed batch, borrowing inputs in declaration order for this call.
    ///
    /// Columns must have equal positive row counts and match the declared types
    /// and validity rules in [`ColumnInput`]. Success creates private data; only
    /// [`Self::commit`] publishes it. Any failed write makes this append abort-only.
    /// Call [`Self::abort`] to clean up, or drop the owner and reopen the database.
    pub fn write(
        &mut self,
        columns: &[ColumnInput<'_>],
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        self.inner
            .write_columns(columns, cancel, &mut Effects::default())
    }

    /// Publish all written batches and return the durable commit receipt.
    /// An ambiguous result must be resolved using this append's transaction.
    pub fn commit(self, cancel: &CancellationToken) -> Result<Commit, Error> {
        self.inner.commit(cancel, &mut Effects::default())
    }

    /// Remove private construction and settle its required cleanup barriers.
    /// A failed cleanup requires database reopen and retains its temporary charge
    /// on the unavailable handle until close or recovery.
    pub fn abort(self) -> Result<(), Error> {
        self.inner.abort(&mut Effects::default())
    }
}

impl TransactionId {
    /// Decode a token previously obtained from [`Self::as_bytes`].
    ///
    /// Returns [`Error::InvalidTransactionId`] for a zero database identity or
    /// attempt sequence. Valid shape does not establish issuance or commitment;
    /// query the database with [`Database::resolve_commit`] for the outcome.
    pub fn from_bytes(bytes: [u8; 24]) -> Result<Self, Error> {
        Self::decode(bytes).map_err(|_| Error::InvalidTransactionId)
    }
}

/// A transaction whose publication completed the required durability barriers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Commit {
    pub(crate) transaction: TransactionId,
    pub(crate) generation: u64,
}

impl Commit {
    /// The issued attempt that produced this commit.
    pub fn transaction(self) -> TransactionId {
        self.transaction
    }

    /// The generation published by this transaction.
    pub fn generation(self) -> u64 {
        self.generation
    }
}

/// The settled outcome of an issued transaction in the retained history.
/// An unknown or unavailable outcome returns an error instead of `Aborted`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitResolution {
    Aborted,
    Durable(Commit),
}
