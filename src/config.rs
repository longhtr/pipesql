//! Choose the memory and temporary-storage budgets for a database handle.
//!
//! Pass a `Config` when creating or opening a database. The memory budget is shared
//! by the handle, prepared queries, running queries, and writers. Each operation
//! reserves part of that budget before allocating its buffers. Two queries share
//! one limit; neither receives a separate budget of its own.
//!
//! The temporary-storage budget covers working files. For example, a sort can
//! write intermediate rows to disk when they do not all fit in memory. This
//! budget does not limit the size of the committed database. Space remains
//! charged while cleanup is unresolved, even if the operation that created the
//! files has returned an error.
//!
//! `Config::new` checks that both limits are nonzero. It does not allocate memory
//! or promise that an operation will fit: even opening a database needs space for
//! its resident state. The `resources` module maintains the live accounts and
//! refuses reservations that would exceed a limit. The limits measure PipeSQL's
//! reservations, not the surrounding process's total physical memory.

use crate::Error;

/// Resource limits shared by operations on one database handle.
///
/// These limits account for engine-owned memory and temporary data. They do not
/// bound caller allocations, process RSS, or filesystem metadata overhead.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Config {
    pub(crate) memory_limit_bytes: u64,
    pub(crate) temp_limit_bytes: u64,
}

impl Config {
    /// Set nonzero memory and temporary-space limits, both in bytes.
    ///
    /// Returns [`Error::InvalidConfig`] if either limit is zero. A valid
    /// configuration may still be too small to admit a particular operation.
    pub fn new(memory_limit_bytes: u64, temp_limit_bytes: u64) -> Result<Self, Error> {
        if memory_limit_bytes == 0 {
            return Err(Error::InvalidConfig("memory limit must be nonzero"));
        }
        if temp_limit_bytes == 0 {
            return Err(Error::InvalidConfig(
                "temporary-space limit must be nonzero",
            ));
        }
        Ok(Self {
            memory_limit_bytes,
            temp_limit_bytes,
        })
    }

    /// Maximum memory charge shared by the database and its live operations.
    pub fn memory_limit_bytes(self) -> u64 {
        self.memory_limit_bytes
    }

    /// Maximum charge for temporary data, including unresolved cleanup debt.
    pub fn temp_limit_bytes(self) -> u64 {
        self.temp_limit_bytes
    }
}
