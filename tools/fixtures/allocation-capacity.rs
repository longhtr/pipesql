//! Observe the production helper's capacity preflight through the caller allocator.
use super::{ALLOW, CALLS, DENY, Error, REFUSED, TRACK};
use std::sync::atomic::Ordering;

// This direct boundary probe needs the private helper, not a public engine hook.
#[allow(dead_code)]
#[path = "../../src/resources.rs"]
mod resources;

enum Expected {
    Empty,
    PreflightRefusal(u64),
    AllocationRefusal,
    Allocated,
}

fn attempt(requested: usize, ceiling: usize, expected: Expected) {
    let authority = resources::MemoryAuthority::new(64);
    let bytes = (ceiling * 8) as u64;
    let (deny, calls, required) = match expected {
        Expected::Empty => (true, 0, None),
        Expected::PreflightRefusal(required) => (true, 0, Some(required)),
        Expected::AllocationRefusal => (true, 1, Some(bytes)),
        Expected::Allocated => (false, 1, None),
    };
    let reservation = authority.reserve(bytes, "capacity probe").unwrap();
    ALLOW.store(0, Ordering::Relaxed);
    CALLS.store(0, Ordering::Relaxed);
    REFUSED.store(0, Ordering::Relaxed);
    TRACK.store(true, Ordering::Relaxed);
    DENY.store(deny, Ordering::Relaxed);
    let result = resources::allocate::<u64>(requested, ceiling, "capacity probe", bytes);
    DENY.store(false, Ordering::Relaxed);
    TRACK.store(false, Ordering::Relaxed);
    assert_eq!(
        CALLS.load(Ordering::Relaxed),
        calls,
        "allocation ceiling reached allocator"
    );
    assert_eq!(
        REFUSED.load(Ordering::Relaxed),
        if deny { calls } else { 0 }
    );
    match (&result, required) {
        (Ok(values), None) => {
            assert!(values.is_empty());
            assert!((requested..=ceiling).contains(&values.capacity()));
        }
        (
            Err(Error::Resource {
                owner: "capacity probe",
                required: observed,
                limit,
            }),
            Some(expected),
        ) => assert_eq!((*observed, *limit), (expected, bytes)),
        other => panic!("unexpected allocation capacity outcome: {other:?}"),
    }
    drop(result);
    assert_eq!(authority.reserved(), bytes);
    drop(reservation);
    assert_eq!(authority.reserved(), 0);
}

pub(super) fn run() {
    attempt(0, 0, Expected::Empty);
    attempt(2, 1, Expected::PreflightRefusal(16));
    attempt(1, 0, Expected::PreflightRefusal(8));
    attempt(usize::MAX, 1, Expected::PreflightRefusal(u64::MAX));
    attempt(1, 1, Expected::AllocationRefusal);
    attempt(2, 2, Expected::Allocated);
    attempt(1, 2, Expected::Allocated);
    println!(
        "allocation capacity passed: 3 preflight refusals; empty, denied, exact and spare-capacity controls"
    );
}
