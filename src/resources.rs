//! Database-wide resource admission and release.
//!
//! Memory reservations release their charge on drop, after their physical owner
//! has been destroyed. Temporary charges survive failed cleanup and ambiguous
//! publication; only the owner that settles those files may release them.
use crate::Error;
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering};

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

/// Allocate within an already admitted capacity ceiling. This checks the allocator
/// result but does not reserve memory; the caller must retain the matching charge.
pub(crate) fn allocate<T>(
    requested: usize,
    ceiling: usize,
    owner: &'static str,
    limit: u64,
) -> Result<Vec<T>, Error> {
    let bytes = ceiling
        .checked_mul(size_of::<T>())
        .ok_or(Error::Corrupt("allocation geometry overflow"))?;
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
