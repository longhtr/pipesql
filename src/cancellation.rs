//! Ask a running operation to stop.
//!
//! Share a `CancellationToken` with the operation and call `cancel` from another
//! thread. The operation checks the token at points where it can stop safely.
//! This is cooperative cancellation: setting the flag does not interrupt the
//! thread or a system call already in progress.
//!
//! The token records the request. The operation decides when to stop and cleans
//! up its work. Check the operation's result to learn what happened; requesting
//! cancellation cannot undo a commit.

use crate::Error;
use std::sync::atomic::{AtomicBool, Ordering};

/// A cancellation request that can be shared across threads.
///
/// Once cancelled, a token stays cancelled. Repeated calls to [`cancel`](Self::cancel)
/// have no further effect. Use a new token for an independent operation.
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

    /// Set the cancellation flag. This does not wait for the operation to stop.
    pub fn cancel(&self) {
        self.requested.store(true, Ordering::Release);
    }

    /// Return `Cancelled` if a request has been made.
    /// Call only where the operation can safely stop and perform cleanup.
    pub(crate) fn check(&self) -> Result<(), Error> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Whether `cancel` has been called. This does not tell whether work has stopped.
    pub fn is_cancelled(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}
