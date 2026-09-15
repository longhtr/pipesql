//! Share the database's memory and temporary-space budgets across live operations.
//!
//! Reserving bytes grants space in an account; it does not allocate a buffer.
//! `MemoryAuthority::reserve` adds a charge only if the combined total fits the
//! limit and returns a `Reservation` that owns its eventual release. Concurrent
//! updates have a finite retry bound and may return contention even below the limit.
//!
//! Owners can split or transfer a reservation without changing the total charge.
//! This prevents another operation from taking the budget while a buffer moves
//! between owners. The physical allocation must be dropped before its reservation;
//! otherwise the account could admit new memory while the old bytes remain live.
//!
//! `TemporaryAuthority` requires explicit release after files are removed or
//! durably published. A failed operation may leave file cleanup outstanding, so
//! dropping its Rust handle is insufficient to settle that charge. Closing the
//! database ends the in-memory account; persisted state still owns recovery debt.
//!
//! `allocate` performs fallible vector allocation inside a caller's admitted
//! ceiling and checks the actual capacity returned. It can fail after reservation.
//! Operators determine their required bytes and retain the matching charge; these
//! counters are not measurements of process memory or arbitrary allocator overhead.

use crate::Error;
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) const BUFFER_ALLOCATION_UNIT: usize = 16_384;

/// Physical byte capacity, separate from encoded lengths and logical row limits.
/// Leave 32 bytes before a native allocation boundary for headers and alignment.
/// A request ending on the boundary can force another mmap page on GNU libc;
/// Darwin rounds these requests back to the same 16-KiB allocation class.
/// Admission charges the actual requested capacity, not an extra allowance.
/// Native allocator qualification remains separate from this geometry.
pub(crate) const fn buffer_capacity(bytes: usize) -> Option<usize> {
    if bytes <= BUFFER_ALLOCATION_UNIT {
        return Some(bytes);
    }
    match bytes.checked_add(BUFFER_ALLOCATION_UNIT - 1 + 32) {
        Some(rounded) => Some((rounded & !(BUFFER_ALLOCATION_UNIT - 1)) - 32),
        None => None,
    }
}

const RESERVATION_ATTEMPTS: usize = 64;

/// One shared memory budget; a successful admission returns its release owner.
pub(crate) struct MemoryAuthority {
    limit: u64,
    reserved: AtomicU64,
}

impl MemoryAuthority {
    pub(crate) fn new(limit: u64) -> Self {
        Self {
            limit,
            reserved: AtomicU64::new(0),
        }
    }

    pub(crate) fn limit(&self) -> u64 {
        self.limit
    }

    pub(crate) fn reserved(&self) -> u64 {
        self.reserved.load(Ordering::Acquire)
    }

    pub(crate) fn reserve(
        &self,
        bytes: u64,
        owner: &'static str,
    ) -> Result<Reservation<'_>, Error> {
        let mut current = self.reserved.load(Ordering::Acquire);
        for _ in 0..RESERVATION_ATTEMPTS {
            let next = current.checked_add(bytes).ok_or(Error::Resource {
                owner,
                required: u64::MAX,
                limit: self.limit,
            })?;
            if next > self.limit {
                return Err(Error::Resource {
                    owner,
                    required: next,
                    limit: self.limit,
                });
            }
            match self.reserved.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(Reservation {
                        authority: self,
                        bytes,
                    });
                }
                Err(observed) => current = observed,
            }
        }
        Err(Error::Contention("database memory reservation"))
    }

    /// Release a charge only after its physical owner has gone away. Database
    /// resident allocations use this directly because they own the authority.
    #[track_caller]
    pub(crate) fn release(&self, bytes: u64, underflow: &'static str) {
        let previous = self.reserved.fetch_sub(bytes, Ordering::AcqRel);
        assert!(previous >= bytes, "{underflow}");
    }
}

/// Temporary files can outlive a failed operation, so release is explicit.
pub(crate) struct TemporaryAuthority {
    limit: u64,
    reserved: AtomicU64,
}

impl TemporaryAuthority {
    pub(crate) fn new(limit: u64) -> Self {
        Self {
            limit,
            reserved: AtomicU64::new(0),
        }
    }

    pub(crate) fn limit(&self) -> u64 {
        self.limit
    }

    pub(crate) fn reserved(&self) -> u64 {
        self.reserved.load(Ordering::Acquire)
    }

    // Shared database borrows can own a serialized writer. Accounting neither
    // grants publication authority nor interprets retained bytes as cleanup debt.
    pub(crate) fn reserve(&self, bytes: u64) -> Result<(), Error> {
        let mut current = self.reserved();
        for _ in 0..RESERVATION_ATTEMPTS {
            let required = current.checked_add(bytes).ok_or(Error::Resource {
                owner: "database temporary storage",
                required: u64::MAX,
                limit: self.limit,
            })?;
            if required > self.limit {
                return Err(Error::Resource {
                    owner: "database temporary storage",
                    required,
                    limit: self.limit,
                });
            }
            match self.reserved.compare_exchange_weak(
                current,
                required,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(observed) => current = observed,
            }
        }
        Err(Error::Contention("database temporary reservation"))
    }

    // Call only after owned temporary files are gone or durably published.
    // There is deliberately no drop-based release of unresolved file ownership.
    pub(crate) fn release(&self, bytes: u64) {
        let previous = self.reserved.fetch_sub(bytes, Ordering::AcqRel);
        assert!(previous >= bytes, "temporary reservation underflow");
    }
}

pub(crate) struct Reservation<'database> {
    authority: &'database MemoryAuthority,
    bytes: u64,
}

impl Reservation<'_> {
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    pub(crate) fn belongs_to(&self, authority: &MemoryAuthority) -> bool {
        std::ptr::eq(self.authority, authority)
    }

    // Transfer admitted bytes without releasing them to competing allocations.
    pub(crate) fn split(&mut self, bytes: u64) -> Result<Self, Error> {
        let remaining = self
            .bytes
            .checked_sub(bytes)
            .ok_or(Error::Corrupt("reservation split exceeds admitted bytes"))?;
        self.bytes = remaining;
        Ok(Self {
            authority: self.authority,
            bytes,
        })
    }

    // Move an existing charge to the owner that outlives its physical storage.
    // The authority's total never changes, so no competing admission sees a gap.
    pub(crate) fn transfer_to(&mut self, destination: &mut Self, bytes: u64) -> Result<(), Error> {
        if !std::ptr::eq(self.authority, destination.authority) {
            return Err(Error::Corrupt("reservation transfer crosses authorities"));
        }
        let remaining = self.bytes.checked_sub(bytes).ok_or(Error::Corrupt(
            "reservation transfer exceeds admitted bytes",
        ))?;
        let combined = destination
            .bytes
            .checked_add(bytes)
            .ok_or(Error::Corrupt("reservation transfer overflow"))?;
        self.bytes = remaining;
        destination.bytes = combined;
        Ok(())
    }

    // Release only unused capacity; the retained physical owner still has bytes.
    pub(crate) fn shrink_to(&mut self, bytes: u64) {
        let unused = self
            .bytes
            .checked_sub(bytes)
            .expect("reservation only shrinks");
        self.authority
            .release(unused, "memory reservation underflow");
        self.bytes = bytes;
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        self.authority
            .release(self.bytes, "memory reservation underflow");
    }
}

/// Allocate within an already admitted capacity ceiling. Reject excess demand
/// before allocation and check the returned capacity afterward. This does not
/// reserve memory; the caller must retain the matching charge.
pub(crate) fn allocate<T>(
    requested: usize,
    ceiling: usize,
    owner: &'static str,
    limit: u64,
) -> Result<Vec<T>, Error> {
    let bytes = ceiling
        .checked_mul(size_of::<T>())
        .ok_or(Error::Corrupt("allocation geometry overflow"))?;
    if requested > ceiling {
        return Err(Error::Resource {
            owner,
            required: requested
                .checked_mul(size_of::<T>())
                .and_then(|value| u64::try_from(value).ok())
                .unwrap_or(u64::MAX),
            limit: bytes as u64,
        });
    }
    let mut values = Vec::new();
    values
        .try_reserve_exact(requested)
        .map_err(|_| Error::Resource {
            owner,
            required: bytes as u64,
            limit,
        })?;
    if values.capacity() < requested || values.capacity() > ceiling {
        return Err(Error::Resource {
            owner,
            required: values
                .capacity()
                .checked_mul(size_of::<T>())
                .and_then(|value| u64::try_from(value).ok())
                .unwrap_or(u64::MAX),
            limit: bytes as u64,
        });
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_capacity_preserves_extent_at_allocation_boundaries() {
        for (required, expected) in [
            (0, 0),
            (16_384, 16_384),
            (16_385, 32_736),
            (32_736, 32_736),
            (32_737, 49_120),
            (524_288, 540_640),
        ] {
            assert_eq!(super::buffer_capacity(required), Some(expected));
        }
        for unit in 2..=515 {
            for required in (unit * 16_384 - 33)..=(unit * 16_384 + 1) {
                let capacity = super::buffer_capacity(required).unwrap();
                assert!(capacity >= required);
                assert!(capacity - required < 16_384);
                assert_eq!((capacity + 32) % 16_384, 0);
                assert_eq!(capacity % std::mem::size_of::<[usize; 2]>(), 0);
            }
        }
        assert_eq!(super::buffer_capacity(usize::MAX), None);
    }

    #[test]
    fn reservation_transfer_preserves_admission_and_retains_destination_charge() {
        let authority = MemoryAuthority::new(100);
        let other = MemoryAuthority::new(100);
        let mut source = authority.reserve(60, "source").unwrap();
        let mut destination = authority.reserve(40, "destination").unwrap();
        let mut foreign = other.reserve(0, "foreign").unwrap();
        assert!(source.transfer_to(&mut foreign, 1).is_err());
        assert!(source.transfer_to(&mut destination, 61).is_err());
        assert_eq!(
            (source.bytes(), destination.bytes(), foreign.bytes()),
            (60, 40, 0)
        );
        source.transfer_to(&mut destination, 60).unwrap();
        assert_eq!((source.bytes(), destination.bytes()), (0, 100));
        assert_eq!(authority.reserved(), 100);
        assert!(authority.reserve(1, "concurrent admission").is_err());
        drop(source);
        assert_eq!(authority.reserved(), 100);
        drop(destination);
        assert_eq!(authority.reserved(), 0);
    }

    #[test]
    fn temporary_authority_preserves_shared_capacity_and_overflow() {
        let authority = TemporaryAuthority::new(100);
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let reserve = || {
                barrier.wait();
                authority.reserve(70)
            };
            let first = scope.spawn(reserve);
            let second = scope.spawn(reserve);
            let outcomes = [first.join().unwrap(), second.join().unwrap()];
            assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
            assert!(
                outcomes
                    .iter()
                    .any(|result| matches!(result, Err(Error::Resource { .. })))
            );
        });
        assert_eq!(authority.reserved(), 70);
        authority.release(70);
        assert_eq!(authority.reserved(), 0);
        let authority = TemporaryAuthority::new(u64::MAX);
        authority.reserve(u64::MAX).unwrap();
        assert!(matches!(authority.reserve(1), Err(Error::Resource { .. })));
        assert_eq!(authority.reserved(), u64::MAX);
        authority.release(u64::MAX);
        assert_eq!(authority.reserved(), 0);
    }

    #[test]
    fn memory_authority_refuses_overflow_without_saturation() {
        let authority = MemoryAuthority::new(u64::MAX);
        let reservation = authority.reserve(u64::MAX, "test").unwrap();
        assert!(matches!(
            authority.reserve(1, "test"),
            Err(Error::Resource {
                required: u64::MAX,
                limit: u64::MAX,
                ..
            })
        ));
        assert_eq!(authority.reserved(), u64::MAX);
        drop(reservation);
        assert_eq!(authority.reserved(), 0);
    }
}
