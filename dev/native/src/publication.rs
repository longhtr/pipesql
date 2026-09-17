//! Refuse a selected publication rename before it changes filesystem names.
//!
//! The isolated CLI arms a single-threaded interval. Four renames publish the
//! issuance and data roots; a fifth call means this observer no longer covers the
//! sequence and terminates the process. Position zero counts without refusing.
//! Stopping returns the observed count for the driver to check. No allocation,
//! formatting or loader work occurs while armed, and setup rejects overlap with
//! other modes. The observer selects the fault; the CLI campaign judges its outcome.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static ACTIVE: AtomicBool = AtomicBool::new(false);
static TARGET: AtomicU32 = AtomicU32::new(0);
static CALLS: AtomicU32 = AtomicU32::new(0);

pub(super) fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

#[unsafe(no_mangle)]
pub extern "C" fn publication_start(position: u32) {
    if position > 4
        || active()
        || super::ACTIVE.load(Ordering::Relaxed)
        || super::initialization::active()
        || super::io::active()
        || super::sync::active()
    {
        super::stop(90);
    }
    TARGET.store(position, Ordering::Relaxed);
    CALLS.store(0, Ordering::Relaxed);
    ACTIVE.store(true, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub extern "C" fn publication_stop() -> u32 {
    if !ACTIVE.swap(false, Ordering::Relaxed) {
        super::stop(90);
    }
    CALLS.load(Ordering::Relaxed)
}

pub(super) fn refuse() -> bool {
    if !active() {
        return false;
    }
    let call = CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    if call > 4 {
        super::stop(91);
    }
    call == TARGET.load(Ordering::Relaxed)
}
