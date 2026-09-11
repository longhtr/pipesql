//! Fallibly allocated, stationary pthread mutex storage. No lazy Rust allocation.
use std::cell::UnsafeCell;
use std::io;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, Ordering};

struct Inner<T> {
    native: UnsafeCell<MaybeUninit<libc::pthread_mutex_t>>,
    value: UnsafeCell<T>,
    poisoned: AtomicBool,
}

pub struct Mutex<T> {
    // Exactly one object; moving this owner never moves an initialized pthread mutex.
    inner: Option<Box<[Inner<T>; 1]>>,
}

#[derive(Debug)]
pub enum LockError {
    System(io::Error),
    Poisoned,
}

// SAFETY: the native NORMAL mutex serializes every access to T. Its allocation
// stays fixed across moves; guards cannot be sent to another thread. T must be
// Send because different lock holders may access it sequentially.
unsafe impl<T: Send> Send for Mutex<T> {}

unsafe impl<T: Send> Sync for Mutex<T> {}

// As with std::sync::Mutex, unwinding poisons the owner before releasing access.
// Later acquisitions refuse the value rather than exposing interrupted mutation.
impl<T> std::panic::UnwindSafe for Mutex<T> {}

impl<T> std::panic::RefUnwindSafe for Mutex<T> {}

impl<T> Mutex<T> {
    /// Heap bytes requested by this owner, excluding allocator/OS overhead.
    pub const fn allocation_bytes() -> usize {
        std::mem::size_of::<Inner<T>>()
    }

    pub fn new(value: T) -> io::Result<Self> {
        let mut storage = Vec::new();
        storage
            .try_reserve_exact(1)
            .map_err(|_| io::ErrorKind::OutOfMemory)?;
        if storage.capacity() != 1 {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        storage.push(Inner {
            native: UnsafeCell::new(MaybeUninit::uninit()),
            value: UnsafeCell::new(value),
            poisoned: AtomicBool::new(false),
        });
        // Capacity equals length, so conversion has no excess allocation to trim.
        let storage: Box<[Inner<T>; 1]> = storage
            .into_boxed_slice()
            .try_into()
            .unwrap_or_else(|_| unreachable!("one mutex storage object"));
        let mut attributes = MaybeUninit::<libc::pthread_mutexattr_t>::uninit();
        // SAFETY: attributes have native alignment and exclusive stack storage.
        let status = unsafe { libc::pthread_mutexattr_init(attributes.as_mut_ptr()) };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        // SAFETY: attributes are initialized. NORMAL excludes the recursive-lock
        // undefined behavior permitted by DEFAULT on some pthread implementations.
        let mut status = unsafe {
            libc::pthread_mutexattr_settype(attributes.as_mut_ptr(), libc::PTHREAD_MUTEX_NORMAL)
        };
        if status == 0 {
            // SAFETY: native storage is aligned, uninitialized, exclusive, and
            // already at its final heap address. Only a successful init publishes it.
            status = unsafe {
                libc::pthread_mutex_init(storage[0].native.get().cast(), attributes.as_ptr())
            };
        }
        // SAFETY: this initialized private attribute object has no remaining users.
        let destroyed = unsafe { libc::pthread_mutexattr_destroy(attributes.as_mut_ptr()) };
        assert_eq!(destroyed, 0, "valid mutex attribute destruction");
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        Ok(Self {
            inner: Some(storage),
        })
    }

    fn inner(&self) -> &Inner<T> {
        &self
            .inner
            .as_ref()
            .expect("live mutex owns initialized storage")[0]
    }

    /// One nonblocking native attempt; None means another holder owns the mutex.
    pub fn try_lock(&self) -> Result<Option<MutexGuard<'_, T>>, LockError> {
        // SAFETY: new initialized the stationary native object. No field is exposed.
        let status = unsafe { libc::pthread_mutex_trylock(self.inner().native.get().cast()) };
        match status {
            0 => self.acquired().map(Some),
            libc::EBUSY => Ok(None),
            code => Err(LockError::System(io::Error::from_raw_os_error(code))),
        }
    }

    /// Blocking acquisition. Callers must prevent reentrancy and must establish
    /// finite holder work and scheduling/acquisition fairness for progress.
    pub fn lock(&self) -> Result<MutexGuard<'_, T>, LockError> {
        // SAFETY: new initialized the stationary native object; NORMAL makes
        // recursive acquisition block rather than create aliasing mutable guards.
        let status = unsafe { libc::pthread_mutex_lock(self.inner().native.get().cast()) };
        if status != 0 {
            return Err(LockError::System(io::Error::from_raw_os_error(status)));
        }
        self.acquired()
    }

    fn acquired(&self) -> Result<MutexGuard<'_, T>, LockError> {
        let guard = MutexGuard {
            mutex: self,
            panicking_on_entry: std::thread::panicking(),
            _not_send_or_sync: PhantomData,
        };
        if self.inner().poisoned.load(Ordering::Acquire) {
            drop(guard);
            Err(LockError::Poisoned)
        } else {
            Ok(guard)
        }
    }
}

pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
    panicking_on_entry: bool,
    // pthread requires the acquiring thread to unlock. Unlike the mutex owner,
    // a guard cannot move to or be shared with another thread.
    _not_send_or_sync: PhantomData<*mut ()>,
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: this live guard owns the native lock, and its borrow prevents
        // destruction. Shared references exclude mutable access through this guard.
        unsafe { &*self.mutex.inner().value.get() }
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the exclusive native lock plus &mut guard excludes every other
        // reference to the protected value for the returned borrow's lifetime.
        unsafe { &mut *self.mutex.inner().value.get() }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        if !self.panicking_on_entry && std::thread::panicking() {
            self.mutex.inner().poisoned.store(true, Ordering::Release);
        }
        // SAFETY: only this thread can own/drop this guard. Storage stays live
        // through the borrow; exactly one successful acquisition is released.
        let status = unsafe { libc::pthread_mutex_unlock(self.mutex.inner().native.get().cast()) };
        assert_eq!(status, 0, "owned native mutex unlock");
    }
}

impl<T> Drop for Mutex<T> {
    fn drop(&mut self) {
        if let Some(storage) = self.release_storage() {
            // A safely forgotten guard keeps native storage and T alive.
            std::mem::forget(storage);
        }
    }
}

impl<T> Mutex<T> {
    // Separate native teardown from ownership disposal so tests can reclaim the
    // intentionally retained storage of a forgotten guard without stale pointers.
    fn release_storage(&mut self) -> Option<Box<[Inner<T>; 1]>> {
        let storage = self.inner.take().expect("mutex storage drops once");
        let pointer = storage[0].native.get().cast();
        // SAFETY: &mut self excludes live ordinary guards/waiters. A forgotten
        // guard is legal safe Rust, so detect it before native destruction.
        let status = unsafe { libc::pthread_mutex_trylock(pointer) };
        if status == libc::EBUSY {
            // pthread forbids destroying locked storage. Leaking a guard also
            // leaks its protected value/storage; engine guards never escape.
            return Some(storage);
        }
        assert_eq!(status, 0, "valid unowned mutex at destruction");
        // SAFETY: the successful trylock belongs to this thread; no references
        // can acquire after it is released, because destruction owns &mut self.
        let unlocked = unsafe { libc::pthread_mutex_unlock(pointer) };
        assert_eq!(unlocked, 0, "destruction probe unlock");
        // SAFETY: initialized, unlocked, stationary storage has no remaining users.
        let destroyed = unsafe { libc::pthread_mutex_destroy(pointer) };
        assert_eq!(destroyed, 0, "valid mutex destruction");
        // Drop T and release physical storage only after native destruction.
        drop(storage);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn stationary_owner_moves_and_nonblocking_reentry_refuses() {
        let first = Mutex::new(7).unwrap();
        {
            let mut guard = first.lock().unwrap();
            *guard += 1;
            assert!(first.try_lock().unwrap().is_none());
        }
        let moved = first;
        assert_eq!(*moved.try_lock().unwrap().unwrap(), 8);
    }

    #[test]
    fn native_mutex_serializes_real_threads_and_drops_value_once() {
        struct Counted(Arc<AtomicUsize>);

        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let value = Mutex::new((0, Counted(drops.clone()))).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..2000 {
                        value.lock().unwrap().0 += 1;
                    }
                });
            }
        });
        assert_eq!(value.lock().unwrap().0, 8000);
        drop(value);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn panic_poisoning_refuses_protected_state() {
        let value = Mutex::new(0).unwrap();
        let result = std::panic::catch_unwind(|| {
            let mut guard = value.lock().unwrap();
            *guard = 1;
            panic!("injected protected-state failure");
        });
        assert!(result.is_err());
        assert!(matches!(value.try_lock(), Err(LockError::Poisoned)));
        assert!(matches!(value.lock(), Err(LockError::Poisoned)));
    }

    #[test]
    fn forgotten_guard_keeps_native_storage_alive() {
        struct Counted<'a>(&'a AtomicUsize);

        impl Drop for Counted<'_> {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let drops = AtomicUsize::new(0);
        let mut value = std::mem::ManuallyDrop::new(Mutex::new(Counted(&drops)).unwrap());
        std::mem::forget(value.lock().unwrap());
        // The exact teardown used by Drop must return the locked owner intact.
        let storage = value
            .release_storage()
            .expect("forgotten guard retains storage");
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        // SAFETY: release this thread's forgotten acquisition. The returned Box
        // is the original live allocation; no stale raw pointer is reconstructed.
        let status = unsafe { libc::pthread_mutex_unlock(storage[0].native.get().cast()) };
        assert_eq!(status, 0);
        drop(Mutex {
            inner: Some(storage),
        });
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}
