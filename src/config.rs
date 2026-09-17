//! Set the memory and temporary-space limits for a database.
//!
//! Pass a `Config` to `Database::create_empty` or `Database::open`. The handle,
//! its queries and its writes share these budgets. Starting a second query does
//! not give it a separate allowance.
//!
//! The temporary-space limit covers working files, such as rows written to disk
//! while sorting a result larger than memory. It does not limit the size of the
//! committed database. The memory limit counts reservations made by PipeSQL;
//! it does not measure the entire process's physical memory use.
//!
//! This module checks and stores the configured limits. `resources` tracks how
//! much of each budget is reserved as operations run.

use crate::Error;

/// Resource limits shared by operations on one database handle.
///
/// Caller-owned buffers and filesystem metadata are outside these budgets.
/// They are not a limit on the operating system's reported process memory use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Config {
    pub(crate) memory_limit_bytes: u64,
    pub(crate) temp_limit_bytes: u64,
}

impl Config {
    /// Set nonzero memory and temporary-space limits, both in bytes.
    ///
    /// Returns [`Error::InvalidConfig`] if either limit is zero. A valid
    /// configuration may still be too small to open a database or run a query.
    /// Constructing a `Config` does not allocate the requested memory.
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

    /// The memory budget in bytes, shared by the database and its operations.
    pub fn memory_limit_bytes(self) -> u64 {
        self.memory_limit_bytes
    }

    /// The temporary-space budget in bytes.
    /// Space stays reserved when failed cleanup leaves working files behind.
    pub fn temp_limit_bytes(self) -> u64 {
        self.temp_limit_bytes
    }
}
