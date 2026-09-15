//! Record a stop request that an operation observes at its own safe boundaries.
//!
//! The token does not interrupt a running thread or system call, or undo publication.
//! It only stores the request; each operation decides when returning Cancelled
//! is legal and how to release its work.

use crate::Error;
use std::sync::atomic::{AtomicBool, Ordering};

/// A thread-safe, one-way request for cooperative cancellation.
///
/// Share a token with an operation and call [`cancel`](Self::cancel) from another
/// thread to request that it stop at a cancellation boundary. The operation's
/// returned outcome remains authoritative; cancellation does not itself undo a
/// completed publication. Create a new token for an independent operation.
#[derive(Debug)]
pub struct CancellationToken {
    requested: AtomicBool,
}

impl CancellationToken {
    /// Create a token with no cancellation request.
    pub const fn new() -> Self {
        Self {
            requested: AtomicBool::new(false),
        }
    }

    /// Request cancellation. Repeated calls are harmless; the request cannot reset.
    pub fn cancel(&self) {
        self.requested.store(true, Ordering::Release);
    }

    /// Observe a caller-chosen cancellation boundary. Publication and recovery
    /// decide where stopping remains legal; the token owns only the request.
    pub(crate) fn check(&self) -> Result<(), Error> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Whether cancellation has been requested, independent of operation completion.
    pub fn is_cancelled(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}
