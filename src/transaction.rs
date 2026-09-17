//! Identify write attempts and report their committed outcomes.
//!
//! Database identity survives reopen. An attempt number identifies an issued
//! write, including one that aborts; a generation identifies a committed version.
//! These values preserve their byte representation without reading files.
//! Storage owns resolution of a saved token and the append's mutable state.

use crate::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IdentityError {
    Length,
    Zero,
}

/// Identifies a database across closes, reopens and pathname changes.
///
/// Creation assigns a nonzero 16-byte value. Obtain it from
/// [`crate::Database::database_identity`]; a generation number identifies a
/// committed version within that database, not the database itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DatabaseId([u8; 16]);

impl DatabaseId {
    pub(crate) fn new(bytes: [u8; 16]) -> Result<Self, IdentityError> {
        if bytes == [0; 16] {
            return Err(IdentityError::Zero);
        }
        Ok(Self(bytes))
    }

    pub(crate) fn from_slice(bytes: &[u8]) -> Result<Self, IdentityError> {
        let bytes: [u8; 16] = bytes.try_into().map_err(|_| IdentityError::Length)?;
        Self::new(bytes)
    }

    /// Return the persistent identity as a borrowed 16-byte array.
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// A token for identifying a transaction attempt and looking up its outcome.
///
/// Keep the token returned by a writer. If commit returns an uncertain outcome,
/// reopen the database and pass the token to [`crate::Database::resolve_commit`].
/// The token alone proves neither issuance nor success.
///
/// Its 24 bytes contain a database identity followed by a nonzero little-endian
/// attempt number. [`Display`](std::fmt::Display) produces 48 lowercase hex digits,
/// the form accepted by the CLI's `resolve --transaction` argument.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionId([u8; 24]);

impl TransactionId {
    pub(crate) fn for_attempt(database: DatabaseId, sequence: u64) -> Result<Self, IdentityError> {
        if sequence == 0 {
            return Err(IdentityError::Zero);
        }
        let mut bytes = [0; 24];
        bytes[..16].copy_from_slice(database.as_bytes());
        bytes[16..].copy_from_slice(&sequence.to_le_bytes());
        Ok(Self(bytes))
    }

    pub(crate) fn decode(bytes: [u8; 24]) -> Result<Self, IdentityError> {
        DatabaseId::from_slice(&bytes[..16])?;
        let value = Self(bytes);
        if value.sequence() == 0 {
            return Err(IdentityError::Zero);
        }
        Ok(value)
    }

    pub(crate) fn sequence(self) -> u64 {
        u64::from_le_bytes(self.0[16..].try_into().expect("fixed attempt sequence"))
    }

    pub(crate) fn belongs_to(self, database: DatabaseId) -> bool {
        self.0[..16] == database.0
    }

    /// Return the 24-byte representation that [`Self::from_bytes`] can decode.
    pub fn as_bytes(&self) -> &[u8; 24] {
        &self.0
    }
}

impl std::fmt::Display for TransactionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl TransactionId {
    /// Read an identifier saved with [`Self::as_bytes`].
    ///
    /// A zero database identifier or attempt number returns [`Error::InvalidTransactionId`].
    /// Accepted bytes do not prove that the transaction exists or committed.
    /// Use [`crate::Database::resolve_commit`] to check its recorded outcome.
    pub fn from_bytes(bytes: [u8; 24]) -> Result<Self, Error> {
        Self::decode(bytes).map_err(|_| Error::InvalidTransactionId)
    }
}

/// A receipt for a write that committed and completed the required synchronization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Commit {
    pub(crate) transaction: TransactionId,
    pub(crate) generation: u64,
}

impl Commit {
    /// The write attempt confirmed by this receipt.
    pub fn transaction(self) -> TransactionId {
        self.transaction
    }

    /// The database version created by this commit.
    pub fn generation(self) -> u64 {
        self.generation
    }
}

/// The stored outcome of a write attempt: aborted or durably committed.
/// Resolution returns an error if the outcome cannot be established. In
/// particular, an unknown identifier is not evidence that a write aborted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitResolution {
    Aborted,
    Durable(Commit),
}
