//! Advance CRC32C and IEEE CRC-32 when the CPU supports AArch64 CRC instructions.
//!
//! The caller owns initial and final inversion. Native detection uses fixed
//! storage and falls back on failure; it must not allocate or read `/proc`.
//! Inputs remain bounds-checked slices. Only capability probes and the call
//! across the target-feature boundary need unsafe code.

use std::sync::atomic::{AtomicU8, Ordering};

// Zero means unobserved, one means unavailable and two means available. CPU
// capabilities do not change during this process. Concurrent first callers may
// repeat the bounded probe; publishing this scalar needs no other memory order.
static AVAILABLE: AtomicU8 = AtomicU8::new(0);

fn available() -> bool {
    if cfg!(target_feature = "crc") {
        return true;
    }
    match AVAILABLE.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let supported = detect();
            AVAILABLE.store(if supported { 2 } else { 1 }, Ordering::Relaxed);
            supported
        }
    }
}

#[cfg(target_os = "linux")]
fn detect() -> bool {
    // SAFETY: getauxval reads the process's immutable auxiliary vector and
    // takes no pointers. A missing key returns zero, selecting the fallback.
    unsafe { libc::getauxval(libc::AT_HWCAP) & libc::HWCAP_CRC32 != 0 }
}

#[cfg(target_os = "macos")]
fn detect() -> bool {
    let mut enabled = 0_i32;
    let mut length = size_of::<i32>();
    // SAFETY: the terminated name and aligned output remain live through the
    // synchronous call. There is no input value to set and no retained pointer.
    let result = unsafe {
        libc::sysctlbyname(
            c"hw.optional.armv8_crc32".as_ptr(),
            (&mut enabled as *mut i32).cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    result == 0 && length == size_of::<i32>() && enabled != 0
}

pub(super) fn update(state: u32, bytes: &[u8]) -> Option<u32> {
    if !available() {
        return None;
    }
    // SAFETY: either the compilation target requires CRC instructions or the
    // OS reported them available. The routine accesses only safe slices.
    Some(unsafe { accelerated(state, bytes) })
}

// Avoid a separate dispatch call for each page chunk; the target-feature call
// remains guarded by the observed CPU capability.
#[inline]
pub(super) fn update_ieee(state: u32, bytes: &[u8]) -> Option<u32> {
    if !available() {
        return None;
    }
    // SAFETY: the same CRC capability includes the IEEE instructions. The
    // compilation target or successful OS probe established that capability.
    Some(unsafe { accelerated_ieee(state, bytes) })
}

#[target_feature(enable = "crc")]
fn accelerated_ieee(mut state: u32, bytes: &[u8]) -> u32 {
    use std::arch::aarch64::{__crc32b, __crc32d, __crc32h, __crc32w};

    let (words, mut tail) = bytes.as_chunks::<8>();
    for word in words {
        state = __crc32d(state, u64::from_le_bytes(*word));
    }
    if tail.len() >= 4 {
        state = __crc32w(state, u32::from_le_bytes(tail[..4].try_into().unwrap()));
        tail = &tail[4..];
    }
    if tail.len() >= 2 {
        state = __crc32h(state, u16::from_le_bytes(tail[..2].try_into().unwrap()));
        tail = &tail[2..];
    }
    if let Some(&byte) = tail.first() {
        state = __crc32b(state, byte);
    }
    state
}

#[target_feature(enable = "crc")]
fn accelerated(mut state: u32, bytes: &[u8]) -> u32 {
    use std::arch::aarch64::{__crc32cb, __crc32cd, __crc32ch, __crc32cw};

    let (words, mut tail) = bytes.as_chunks::<8>();
    for word in words {
        state = __crc32cd(state, u64::from_le_bytes(*word));
    }
    if tail.len() >= 4 {
        state = __crc32cw(state, u32::from_le_bytes(tail[..4].try_into().unwrap()));
        tail = &tail[4..];
    }
    if tail.len() >= 2 {
        state = __crc32ch(state, u16::from_le_bytes(tail[..2].try_into().unwrap()));
        tail = &tail[2..];
    }
    if let Some(&byte) = tail.first() {
        state = __crc32cb(state, byte);
    }
    state
}
