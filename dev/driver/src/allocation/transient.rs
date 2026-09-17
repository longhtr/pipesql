//! Check memory reservations while a database operation allocates and frees buffers.
//!
//! Measuring only before and after a call can miss a buffer that briefly exists
//! without a reservation. The allocator in `allocation.rs` therefore
//! calls this observer after allocating and just before freeing each buffer.
//! Callers use `Observer::during` to limit observation to one synchronous call.
//!
//! Each sample subtracts live heap growth from reservation growth, relative to
//! caller-supplied baselines. Negative headroom means the buffers exceed their
//! combined reservation. Calibration deliberately creates that condition, then
//! checks that observing only the free still detects the live buffer.
//!
//! This comparison requires one thread and no changes to caller-owned heap
//! storage while observing. It counts Rust allocations through the fixture's
//! allocator, not foreign allocations or total process memory.

use super::{LIVE_REQUESTED, LIVE_USABLE};
use pipesql::Database;
use std::cell::Cell;
use std::sync::atomic::Ordering;

thread_local! {
    // Allocator callbacks must not allocate to find their observer. Const TLS
    // with no destructor avoids that recursion; during() bounds the pointer's use.
    static ACTIVE: Cell<*const Observer<'static>> = const { Cell::new(std::ptr::null()) };
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Samples {
    pub allocations: usize,
    pub frees: usize,
    // Minima across events, in bytes. Usable size includes the allocator's size
    // rounding; requested size is what the Rust allocation asked for.
    pub requested_headroom: i128,
    pub usable_headroom: i128,
}

pub(super) struct Observer<'db> {
    database: &'db Database,
    resident: u64,
    requested: usize,
    usable: usize,
    samples: Cell<Samples>,
}

impl<'db> Observer<'db> {
    pub fn new(database: &'db Database, resident: u64, requested: usize, usable: usize) -> Self {
        Self {
            database,
            resident,
            requested,
            usable,
            samples: Cell::new(Samples {
                allocations: 0,
                frees: 0,
                requested_headroom: i128::MAX,
                usable_headroom: i128::MAX,
            }),
        }
    }

    pub fn during<T>(&self, call: impl FnOnce() -> T) -> T {
        struct Armed;
        impl Drop for Armed {
            fn drop(&mut self) {
                ACTIVE.with(|active| active.set(std::ptr::null()));
            }
        }
        ACTIVE.with(|active| {
            assert!(active.get().is_null(), "nested transient observation");
            active.set(std::ptr::from_ref(self).cast());
        });
        // Clear the erased pointer before this borrow ends, including on panic.
        // The guard stays here so the caller cannot forget or retain it.
        let _armed = Armed;
        call()
    }

    pub fn samples(&self) -> Samples {
        self.samples.get()
    }
}

// Sample before free to catch a reservation released too early. Aggregate totals
// can expose a shortage but cannot assign an individual pointer to an account.
pub(super) fn sample(allocation: bool) {
    let _ = ACTIVE.try_with(|active| {
        let pointer = active.get();
        if pointer.is_null() {
            return;
        }
        // SAFETY: during() keeps the observer and database borrowed until it
        // clears this pointer. TLS restricts access to that same thread. Sampling
        // uses only Cell and atomic loads, so it cannot allocate, lock or unwind
        // through GlobalAlloc.
        let observer = unsafe { &*pointer };
        let charge =
            i128::from(observer.database.reserved_memory_bytes()) - i128::from(observer.resident);
        let requested = LIVE_REQUESTED.load(Ordering::Relaxed) as i128 - observer.requested as i128;
        let usable = LIVE_USABLE.load(Ordering::Relaxed) as i128 - observer.usable as i128;
        let mut samples = observer.samples.get();
        if allocation {
            samples.allocations = samples.allocations.saturating_add(1);
        } else {
            samples.frees = samples.frees.saturating_add(1);
        }
        samples.requested_headroom = samples.requested_headroom.min(charge - requested);
        samples.usable_headroom = samples.usable_headroom.min(charge - usable);
        observer.samples.set(samples);
    });
}

pub(super) fn check_calibration(database: &Database, disabled: bool) {
    let requested = LIVE_REQUESTED.load(Ordering::Relaxed);
    let usable = LIVE_USABLE.load(Ordering::Relaxed);
    let observer = Observer::new(
        database,
        database.reserved_memory_bytes(),
        requested,
        usable,
    );
    let transient = || {
        // Make the uncharged buffer observable to the optimizer. Requiring both
        // events below also rejects a build that eliminates the allocation.
        let bytes = std::hint::black_box(vec![0_u8; 65_536]);
        std::hint::black_box(&bytes);
        drop(bytes);
    };
    if disabled {
        transient();
    } else {
        observer.during(transient);
    }
    assert_eq!(LIVE_REQUESTED.load(Ordering::Relaxed), requested);
    assert_eq!(LIVE_USABLE.load(Ordering::Relaxed), usable);
    let samples = observer.samples();
    assert!(
        samples.allocations > 0
            && samples.frees > 0
            && samples.requested_headroom <= -65_536
            && samples.usable_headroom <= -65_536,
        "transient ownership calibration missed uncharged allocation: {samples:?}"
    );
    // A second observer sees only the free. If sampling moves after physical
    // release and counter subtraction, the negative headroom disappears.
    let free_observer = Observer::new(
        database,
        database.reserved_memory_bytes(),
        requested,
        usable,
    );
    let bytes = std::hint::black_box(vec![0_u8; 65_536]);
    free_observer.during(|| drop(bytes));
    assert_eq!(LIVE_REQUESTED.load(Ordering::Relaxed), requested);
    assert_eq!(LIVE_USABLE.load(Ordering::Relaxed), usable);
    let free_samples = free_observer.samples();
    assert!(
        free_samples.allocations == 0
            && free_samples.frees > 0
            && free_samples.requested_headroom <= -65_536
            && free_samples.usable_headroom <= -65_536,
        "transient ownership calibration missed live owner before free: {free_samples:?}"
    );
    println!(
        "transient ownership calibration passed: hidden allocation detected; entry/exit agree"
    );
}
