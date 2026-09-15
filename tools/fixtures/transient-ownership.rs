//! Sample live allocation ownership inside one synchronous public call.
//!
//! A thread-local borrowed observer reads reservations after allocation and before
//! physical free; return-time samples alone would miss a temporarily uncharged
//! owner. A guard clears the pointer on return or unwind. Calibration deliberately
//! allocates uncharged bytes and separately observes a free, proving both edges.
//! Attribution assumes fixed caller storage on this thread and excludes foreign
//! malloc calls; it does not establish an RSS bound.

use super::{LIVE_REQUESTED, LIVE_USABLE};
use pipesql::Database;
use std::cell::Cell;
use std::sync::atomic::Ordering;

thread_local! {
    // Const, non-dropping TLS does not allocate or run a destructor. The erased
    // lifetime is restored only while during() keeps the observer borrowed.
    static ACTIVE: Cell<*const Observer<'static>> = const { Cell::new(std::ptr::null()) };
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Samples {
    pub allocations: usize,
    pub frees: usize,
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
        // The synchronous call cannot outlive this shared borrow. Its guard
        // clears the pointer on normal return and unwinding before self can move
        // or the database borrow can end. No guard or pointer escapes this call.
        let _armed = Armed;
        call()
    }

    pub fn samples(&self) -> Samples {
        self.samples.get()
    }
}

// Called after successful allocation and before physical deallocation. Sampling
// before free also exposes a reservation released while its heap owner is live.
// This workload has one thread and fixed caller storage while armed. Aggregate
// deltas do not attribute individual pointers or observe foreign malloc calls.
pub(super) fn sample(allocation: bool) {
    let _ = ACTIVE.try_with(|active| {
        let pointer = active.get();
        if pointer.is_null() {
            return;
        }
        // SAFETY: during() owns the only installation and clears it before the
        // borrowed observer/database can disappear. TLS prevents access from a
        // different thread. Cell and the public atomic memory load cannot
        // allocate, lock or unwind through GlobalAlloc.
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
        // Explicit allocation plus black_box keeps the deliberately uncharged
        // owner observable. Missing events fail the control even if optimized.
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
