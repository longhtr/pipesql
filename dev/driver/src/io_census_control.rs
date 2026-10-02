//! Check observation against known native bytes, then exercise both call limits.

use super::io_observer::*;
use std::{fs::OpenOptions, os::fd::AsRawFd, path::Path};

#[cfg(target_os = "macos")]
unsafe fn errno() -> *mut i32 {
    unsafe { libc::__error() }
}
#[cfg(target_os = "linux")]
unsafe fn errno() -> *mut i32 {
    unsafe { libc::__errno_location() }
}

pub(super) fn run(root: &Path, mode: &str, outcome: &str) {
    assert!(matches!(mode, "probe" | "census"));
    assert!(matches!(outcome, "complete" | "overflow"));
    let path = root.join("native-bytes");
    std::fs::write(&path, b"abcdefgh").unwrap();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    let null = OpenOptions::new().read(true).open("/dev/null").unwrap();
    let mut bytes = [0xff_u8; 18];
    // SAFETY: retained descriptors and these guarded stack buffers remain live
    // throughout synchronous calls. Every symbol was loaded before observation.
    unsafe {
        // Compare errno with the ordinary OS result, rather than assuming that a
        // successful syscall leaves its incoming value untouched on every host.
        *errno() = libc::EINTR;
        assert_eq!(
            libc::pread(file.as_raw_fd(), bytes.as_mut_ptr().cast(), 16, 2),
            6
        );
        let expected_errno = *errno();
        bytes.fill(0xff);
        io_census_start(2);
        *errno() = libc::EINTR;
        assert_eq!(
            libc::pread(file.as_raw_fd(), bytes[1..].as_mut_ptr().cast(), 16, 2),
            6
        );
        assert_eq!(*errno(), expected_errno);
        assert_eq!(&bytes[1..7], b"cdefgh");
        assert!(
            bytes[..1]
                .iter()
                .chain(bytes[7..].iter())
                .all(|b| *b == 0xff)
        );
        assert_eq!(
            libc::pread(file.as_raw_fd(), bytes.as_mut_ptr().cast(), 8, 8),
            0
        );
        assert_eq!(
            libc::pread(file.as_raw_fd(), bytes.as_mut_ptr().cast(), 0, 0),
            0
        );
        // Unselected stream reads and a selected nonregular descriptor do not
        // enter the census, even though they still execute against the OS.
        assert_eq!(
            libc::read(file.as_raw_fd(), bytes.as_mut_ptr().cast(), 1),
            1
        );
        assert_eq!(bytes[0], b'a');
        assert_eq!(
            libc::pread(null.as_raw_fd(), bytes.as_mut_ptr().cast(), 1, 0),
            0
        );
        io_probe_stop();
        assert_eq!(
            (
                io_probe_calls(),
                io_probe_requested_bytes(),
                io_probe_transferred_bytes()
            ),
            (3, 24, 6)
        );
        assert_eq!((io_probe_refused(), io_probe_partial()), (0, 0));

        let limit = if mode == "probe" { 4096 } else { 65_536 };
        if mode == "probe" {
            io_probe_start(2, 0, 0, 0);
        } else {
            io_census_start(2);
        }
        assert_eq!(
            (
                io_probe_calls(),
                io_probe_requested_bytes(),
                io_probe_transferred_bytes()
            ),
            (0, 0, 0)
        );
        for _ in 0..limit {
            assert_eq!(
                libc::pread(file.as_raw_fd(), bytes.as_mut_ptr().cast(), 1, 0),
                1
            );
            assert_eq!(bytes[0], b'a');
        }
        assert_eq!(
            (
                io_probe_calls(),
                io_probe_requested_bytes(),
                io_probe_transferred_bytes()
            ),
            (limit, u64::from(limit), u64::from(limit))
        );
        println!("native census limit reached: {limit}");
        if outcome == "overflow" {
            libc::pread(file.as_raw_fd(), bytes.as_mut_ptr().cast(), 1, 0);
            panic!("native census accepted excess calls");
        }
        io_probe_stop();
        assert_eq!(
            libc::pread(file.as_raw_fd(), bytes.as_mut_ptr().cast(), 1, 0),
            1
        );
        assert_eq!(io_probe_calls(), limit, "stopped observation kept counting");
    }
    println!("native census control complete");
}
