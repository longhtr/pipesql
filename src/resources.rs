//! Keep live operations within the database's memory and temporary-file budgets.
//!
//! An operation first reserves space in a shared account, then allocates its
//! buffers or writes its files. Reserving space does not provide the memory or
//! disk space itself: allocation and I/O can still fail. The accounts limit what
//! the engine agrees to use; they do not measure the process's physical memory.
//!
//! A memory reservation holds part of the budget until it is dropped. The caller
//! must free the corresponding buffers first, so another operation cannot reuse
//! that budget while the old buffers are still live. Reservations can move with
//! their buffers without releasing and reacquiring space in the shared account.
//!
//! Temporary-file space needs a different lifetime. A failed operation can leave
//! files behind, so dropping its handle must not release their space. The owner
//! releases the charge when those bytes are discarded or durably published. Recovery
//! handles files left when the database closes; the in-memory account ends there.

use crate::Error;
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) const BUFFER_ALLOCATION_UNIT: usize = 16_384;

/// Choose a buffer capacity large enough for `bytes`, or return `None` on overflow.
/// Requests through 16 KiB keep their size. Larger requests round up to 32 bytes
/// below a multiple of 16 KiB, leaving room for allocator headers and alignment.
/// This avoids an extra mapped page for GNU libc allocations that would otherwise
/// end on that boundary. Cold Darwin allocations round to the 16-KiB class.
/// The caller must cover the chosen capacity in its charge. This layout does not
/// bound allocator metadata, cached free pages, stacks or process mappings.
pub(crate) const fn buffer_capacity(bytes: usize) -> Option<usize> {
    if bytes <= BUFFER_ALLOCATION_UNIT {
        return Some(bytes);
    }
    match bytes.checked_add(BUFFER_ALLOCATION_UNIT - 1 + 32) {
        Some(rounded) => Some((rounded & !(BUFFER_ALLOCATION_UNIT - 1)) - 32),
        None => None,
    }
}

/// Memory admission for an already chosen buffer capacity.
/// Darwin's large-allocation cache can return an entire earlier allocation,
/// provided its page-rounded size is less than twice the new page-rounded
/// request. Keep that space charged without exposing it as writable capacity.
/// The reviewed macOS small-allocation path covers requests through 32 KiB.
/// This is a stock-allocator premise, not a contract for custom allocators or RSS.
/// The reviewed implementation is Apple's libmalloc revision
/// c49dafa25f1efe8607701ae6014a663ad2ee437f, `src/magazine_large.c` (cache reuse)
/// and `src/thresholds.h` (the 32-KiB small-allocation boundary).
pub(crate) const fn buffer_charge(capacity: usize) -> Option<usize> {
    if cfg!(target_os = "macos") && capacity > 32_768 {
        match capacity.checked_add(BUFFER_ALLOCATION_UNIT - 1) {
            Some(rounded) => (rounded & !(BUFFER_ALLOCATION_UNIT - 1)).checked_mul(2),
            None => None,
        }
    } else {
        Some(capacity)
    }
}

const RESERVATION_ATTEMPTS: usize = 64;

/// The database-wide memory limit and the total currently reserved against it.
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

    /// Reserve bytes and return the handle that will release them.
    /// Concurrent requests update the same total atomically. After 64 failed
    /// attempts, return contention rather than retrying without a bound; this
    /// can happen even when the requested bytes would fit.
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

    /// Release bytes after freeing their allocations. The database uses this
    /// directly for buffers whose reservation cannot borrow its own account.
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

    // Reserve space only. The caller must separately obtain permission to write
    // and track which files need cleanup; the byte count cannot answer either.
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

    // Call only after temporary bytes are discarded or durably published. Automatic
    // release on drop would hide space still occupied after failed cleanup.
    pub(crate) fn release(&self, bytes: u64) {
        let previous = self.reserved.fetch_sub(bytes, Ordering::AcqRel);
        assert!(previous >= bytes, "temporary reservation underflow");
    }
}

/// Part of a memory account held until drop. This handle owns no allocation;
/// its caller is responsible for freeing buffers before dropping it.
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

    // Give part of this reservation to a new owner. The shared total stays
    // unchanged, so other operations cannot take those bytes during the move.
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

    // Move bytes to another reservation in the same account. Validate both
    // amounts before changing either, so a failed transfer leaves them intact.
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

    // Return unused bytes to the shared budget. The caller must keep enough
    // reserved to cover every allocation it still owns.
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

/// Allocate an empty vector with capacity for at least `requested` elements.
/// Both `requested` and `ceiling` count elements, not bytes. The caller must
/// reserve the ceiling's byte cost first and retain that charge with the vector.
/// Reject a request above the ceiling before allocating, and reject an allocator
/// result above it afterward: a valid request does not guarantee a valid capacity.
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
    fn buffer_charge_covers_reuse_and_checks_overflow() {
        for (capacity, macos_charge) in [
            (0, 0),
            (16_384, 16_384),
            (16_385, 16_385),
            (32_736, 32_736),
            (32_768, 32_768),
            (32_769, 98_304),
            (65_536, 131_072),
            (3_817_440, 7_634_944),
        ] {
            let expected = if cfg!(target_os = "macos") {
                macos_charge
            } else {
                capacity
            };
            assert_eq!(buffer_charge(capacity), Some(expected));
        }
        if cfg!(target_os = "macos") {
            assert_eq!(buffer_charge(usize::MAX), None);
            assert_eq!(buffer_charge(usize::MAX / 2), None);
        }
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
        use std::sync::mpsc::sync_channel;
        use std::time::Duration;
        let case = std::env::var(crate::test_subprocess::CASE).unwrap_or_default();
        let authority = TemporaryAuthority::new(100);
        let outcome = std::panic::catch_unwind(|| {
            std::thread::scope(|scope| {
                // The parent owns these endpoints inside the scope closure.
                // Unwinding drops them before joining any waiting worker.
                let workers: [_; 2] = std::array::from_fn(|index| {
                    if index == 1 {
                        assert_ne!(case, "temporary-partial-start", "temporary partial startup");
                    }
                    let (ready, entered) = sync_channel(1);
                    let (start, released) = sync_channel(1);
                    let authority = &authority;
                    let case = case.as_str();
                    let handle = scope.spawn(move || {
                        if index == 0 {
                            assert_ne!(case, "temporary-worker-panic", "temporary worker failure");
                            if case == "temporary-delayed-readiness" {
                                assert!(released.recv().is_err());
                                return Err(Error::Cancelled);
                            }
                        }
                        if ready.send(()).is_err() || released.recv().is_err() {
                            return Err(Error::Cancelled);
                        }
                        authority.reserve(70)
                    });
                    (handle, entered, start)
                });
                for (_, entered, _) in &workers {
                    entered
                        .recv_timeout(if case == "temporary-delayed-readiness" {
                            Duration::from_millis(20)
                        } else {
                            Duration::from_secs(5)
                        })
                        .expect("temporary worker readiness");
                }
                assert_ne!(
                    case, "temporary-coordinator-panic",
                    "temporary coordinator failure"
                );
                for (_, _, start) in &workers {
                    start.send(()).unwrap();
                }
                let outcomes = workers.map(|(handle, _, _)| handle.join().unwrap());
                assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
                assert!(
                    outcomes
                        .iter()
                        .any(|result| matches!(result, Err(Error::Resource { .. })))
                );
            });
        });
        if !case.is_empty() {
            let panic = outcome.expect_err("temporary control must panic");
            let expected = match case.as_str() {
                "temporary-partial-start" => "temporary partial startup",
                "temporary-worker-panic" => "temporary worker readiness: Disconnected",
                "temporary-delayed-readiness" => "temporary worker readiness: Timeout",
                "temporary-coordinator-panic" => "temporary coordinator failure",
                _ => panic!("unknown temporary failure control"),
            };
            assert!(panic.downcast_ref::<String>().unwrap().contains(expected));
            assert_eq!(authority.reserved(), 0);
            authority.reserve(100).unwrap();
            authority.release(100);
            assert_eq!(authority.reserved(), 0);
            println!("temporary workers exited and shared capacity was reusable");
            return;
        }
        outcome.unwrap();
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
    fn temporary_worker_failure_controls_terminate() {
        for case in [
            "temporary-partial-start",
            "temporary-worker-panic",
            "temporary-delayed-readiness",
            "temporary-coordinator-panic",
        ] {
            crate::test_subprocess::run(
                concat!(
                    module_path!(),
                    "::temporary_authority_preserves_shared_capacity_and_overflow"
                ),
                case,
                true,
                "temporary workers exited and shared capacity was reusable",
            );
        }
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
