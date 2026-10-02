//! Call the original OS functions from wrappers that observe or alter their use.
//!
//! Linux resolves RTLD_NEXT symbols in an initializer before main, while observation
//! is disarmed. Wrappers then use cached addresses without recursive symbol lookup
//! or loader allocations in the measured interval. A missing address terminates the
//! process instead of silently bypassing instrumentation. Darwin's interposition
//! uses the libc functions directly. The errno helper exposes the calling thread's
//! error slot so observers can preserve or deliberately replace native failures.

#[cfg(target_os = "macos")]
pub use libc::{
    pread as real_pread, pwrite as real_pwrite, read as real_read, rename as real_rename,
    unlink as real_unlink, write as real_write,
};

pub unsafe fn errno() -> *mut i32 {
    #[cfg(target_os = "macos")]
    unsafe {
        libc::__error()
    }
    #[cfg(target_os = "linux")]
    unsafe {
        libc::__errno_location()
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use libc::{c_char, c_void, off_t};
    use std::sync::atomic::{AtomicPtr, Ordering};
    macro_rules! bind {
        ($slot:ident, $call:ident, $name:literal, ($($argument:ident: $ty:ty),*) -> $result:ty) => {
            static $slot: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
            pub unsafe fn $call($($argument: $ty),*) -> $result {
                let address = $slot.load(Ordering::Relaxed);
                if address.is_null() { super::super::stop(98); }
                // POSIX supplies a callable function address for this exact ABI.
                let function: unsafe extern "C" fn($($ty),*) -> $result = unsafe { std::mem::transmute(address) };
                unsafe { function($($argument),*) }
            }
        };
    }
    bind!(WRITE, real_write, "write", (fd: i32, bytes: *const c_void, count: usize) -> isize);
    bind!(READ, real_read, "read", (fd: i32, bytes: *mut c_void, count: usize) -> isize);
    bind!(PREAD, real_pread, "pread", (fd: i32, bytes: *mut c_void, count: usize, offset: off_t) -> isize);
    bind!(PWRITE, real_pwrite, "pwrite", (fd: i32, bytes: *const c_void, count: usize, offset: off_t) -> isize);
    bind!(RENAME, real_rename, "rename", (source: *const c_char, destination: *const c_char) -> i32);
    bind!(UNLINK, real_unlink, "unlink", (path: *const c_char) -> i32);
    bind!(FSYNC, real_fsync, "fsync", (fd: i32) -> i32);
    bind!(FDATASYNC, real_fdatasync, "fdatasync", (fd: i32) -> i32);
    bind!(LSTAT, real_lstat, "lstat", (path: *const c_char, output: *mut libc::stat) -> i32);
    bind!(READLINK, real_readlink, "readlink", (path: *const c_char, output: *mut c_char, capacity: usize) -> isize);
    bind!(REALPATH, real_realpath, "realpath", (path: *const c_char, output: *mut c_char) -> *mut c_char);

    unsafe extern "C" fn resolve() {
        for (slot, name) in [
            (&WRITE, c"write"),
            (&READ, c"read"),
            (&PREAD, c"pread"),
            (&PWRITE, c"pwrite"),
            (&RENAME, c"rename"),
            (&UNLINK, c"unlink"),
            (&FSYNC, c"fsync"),
            (&FDATASYNC, c"fdatasync"),
            (&LSTAT, c"lstat"),
            (&READLINK, c"readlink"),
            (&REALPATH, c"realpath"),
        ] {
            unsafe {
                libc::dlerror();
                let symbol = libc::dlsym(libc::RTLD_NEXT, name.as_ptr());
                if symbol.is_null() || !libc::dlerror().is_null() {
                    super::super::stop(98);
                }
                slot.store(symbol, Ordering::Relaxed);
            }
        }
    }

    #[used]
    #[unsafe(link_section = ".init_array")]
    static INITIALIZE: unsafe extern "C" fn() = resolve;
}

#[cfg(target_os = "linux")]
pub use linux::*;
