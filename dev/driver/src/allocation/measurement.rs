//! Sample live allocations and compare an isolated owner with its reservation.
//!
//! These snapshots do not observe peaks between calls. Workloads must keep other
//! owners still while attributing a difference; transient observers cover calls.
//!
//! Live includes open descriptors; Heap and Owner separate byte differences from
//! the corresponding database charge. Preparation uses a transient observer and
//! requires its entry and exit samples to agree with the surrounding snapshots.
//! These helpers share measurement mechanics, not workload-specific expected rows.

use super::{LIVE_REQUESTED, LIVE_USABLE};
use pipesql::{Database, Error, PreparedQuery};
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Live {
    pub(super) requested: usize,
    pub(super) usable: usize,
    pub(super) descriptors: usize,
}

impl Live {
    pub(super) fn now() -> Self {
        // Count open files before reading heap counters, so the directory
        // iterator's allocation is already freed. Its descriptor counts in both
        // the baseline and later samples and therefore cancels in the difference.
        let descriptors = std::fs::read_dir("/dev/fd").unwrap().count();
        Self {
            requested: LIVE_REQUESTED.load(Ordering::Relaxed),
            usable: LIVE_USABLE.load(Ordering::Relaxed),
            descriptors,
        }
    }
}

// Sample live heap bytes, not the peak between samples. A difference belongs to
// one owner only when other owners are prevented from changing during the interval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Heap {
    pub(super) requested: usize,
    pub(super) usable: usize,
}

impl Heap {
    pub(super) fn now() -> Self {
        Self {
            requested: LIVE_REQUESTED.load(Ordering::Relaxed),
            usable: LIVE_USABLE.load(Ordering::Relaxed),
        }
    }

    pub(super) fn increase_from(self, before: Self) -> Self {
        Self {
            requested: self.requested.checked_sub(before.requested).unwrap(),
            usable: self.usable.checked_sub(before.usable).unwrap(),
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct Owner {
    pub(super) charge: u64,
    pub(super) heap: Heap,
}

impl Owner {
    pub(super) fn report(self, label: &str, nonheap_reservation: u64) {
        assert!(self.heap.requested <= self.heap.usable);
        assert_eq!(
            self.charge,
            self.heap.requested as u64 + nonheap_reservation,
            "execution ownership attribution: {label}"
        );
        println!(
            "owner {label} charge={} requested={} usable={} nonheap-reservation={} rounding={}",
            self.charge,
            self.heap.requested,
            self.heap.usable,
            self.charge - self.heap.requested as u64,
            self.heap.usable - self.heap.requested,
        );
    }
}

// Only the allowance beyond the requested capacity belongs in this term.
pub(super) fn native_buffer_allowance(capacity: usize) -> usize {
    if cfg!(target_os = "macos") && capacity > 32_768 {
        capacity.div_ceil(16_384) * 32_768 - capacity
    } else {
        0
    }
}

pub(super) fn prepare_observed<'db>(
    db: &'db Database,
    sql: &str,
    label: &str,
    retained_allocations: usize,
    wrong_allowance: bool,
) -> Result<PreparedQuery<'db>, Error> {
    let before = Live::now();
    let query = db.prepare(sql)?;
    let after = Live::now();
    let requested = after.requested.checked_sub(before.requested).unwrap();
    let usable = after.usable.checked_sub(before.usable).unwrap();
    let inline = std::mem::size_of::<PreparedQuery<'_>>();
    // Preparation retains the plan and descriptor buffers, each with a 4,096-byte
    // allowance beyond its request. Temporary catalog/name-scope storage is gone.
    // Use this equation instead of calling the engine's reservation calculation.
    let allowance = retained_allocations * 4096 + usize::from(wrong_allowance);
    assert_eq!(
        query.accounted_memory_bytes(),
        (requested + inline + allowance) as u64,
        "prepared ownership attribution"
    );
    println!(
        "ownership prepared-{label} charge={} requested={requested} usable={usable} inline={inline} allowance={allowance} rounding={}",
        query.accounted_memory_bytes(),
        usable.checked_sub(requested).unwrap(),
    );
    assert!(
        usable as u64 <= query.accounted_memory_bytes(),
        "prepared usable allocations exceed admission"
    );
    Ok(query)
}
