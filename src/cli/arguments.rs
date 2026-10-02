//! Capture native process arguments with explicit count, byte and allocation bounds.
//!
//! macOS reads the runtime's argc/argv pointers. Linux reads NUL-separated bytes
//! from `/proc/self/cmdline`. Both produce owned `OsString` values and leave the
//! program-name slot empty for the command parser to skip. Paths may contain
//! non-UTF-8 bytes; capture does not interpret their contents.
//!
//! Fixed scanning buffers bound each argument before a fallible copy. Linux also
//! bounds the complete stream, including the ignored program name. Invalid or
//! unreadable input fails capture. Native pointers never escape this module;
//! the macOS path requires argv to remain unchanged throughout startup capture.

use super::command::{ArgumentError, MAX_ARGUMENT_BYTES};
use pipesql::Error;
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;

// The longest valid command has a program name, an operation and fifteen option
// pairs. Allow one extra pair so parsing can report duplicates or wrong options.
const MAX_ARGUMENTS: usize = 34;

pub(super) struct Arguments {
    values: [OsString; MAX_ARGUMENTS],
    count: usize,
}

impl Arguments {
    fn empty() -> Self {
        Self {
            values: std::array::from_fn(|_| OsString::new()),
            count: 0,
        }
    }

    pub(super) fn into_iter(self) -> impl Iterator<Item = OsString> {
        self.values.into_iter().take(self.count)
    }
}

fn copy(bytes: &[u8]) -> Result<OsString, ArgumentError> {
    if bytes.len() > MAX_ARGUMENT_BYTES {
        return Err("argument exceeds 4096 bytes".into());
    }
    let mut value = OsString::new();
    value
        .try_reserve_exact(bytes.len())
        .map_err(|_| failure("allocate CLI argument", io::ErrorKind::OutOfMemory.into()))?;
    value.push(OsStr::from_bytes(bytes));
    Ok(value)
}

fn failure(operation: &'static str, source: io::Error) -> ArgumentError {
    ArgumentError::Library(Error::Io { operation, source })
}

#[cfg(target_os = "macos")]
pub(super) fn capture() -> Result<Arguments, ArgumentError> {
    // SAFETY: the macOS runtime supplies live argc/argv storage. Capture runs
    // before this executable starts threads or enters the engine, and no code
    // here mutates argv. Foreign code must not change it during this copy.
    let count = unsafe { libc::_NSGetArgc().read() };
    let count =
        usize::try_from(count).map_err(|_| ArgumentError::from("invalid native argument count"))?;
    // SAFETY: argv and its C strings stay readable and unchanged until
    // copy_native returns. The returned Arguments owns copies of their bytes.
    let argv = unsafe { libc::_NSGetArgv().read() };
    unsafe { copy_native(count, argv.cast()) }
}

// SAFETY: if count <= MAX_ARGUMENTS, non-null argv must hold count readable
// pointers. Each non-null string after argv[0] must remain immutable and readable
// through its first NUL, or for MAX_ARGUMENT_BYTES + 1 bytes if no NUL occurs.
// Null vectors and entries return errors before dereferencing; argv[0] is ignored.
#[cfg(any(target_os = "macos", test))]
unsafe fn copy_native(count: usize, argv: *const *const u8) -> Result<Arguments, ArgumentError> {
    if count > MAX_ARGUMENTS {
        return Err("argument count exceeds 34".into());
    }
    let mut result = Arguments::empty();
    result.count = count;
    if count > 0 && argv.is_null() {
        return Err("native argument vector is null".into());
    }
    for index in 1..count {
        // SAFETY: index is within the validated native argument vector.
        let pointer = unsafe { argv.add(index).read() }.cast::<u8>();
        if pointer.is_null() {
            return Err("native argument is null".into());
        }
        let mut length = 0;
        loop {
            // SAFETY: the caller supplies readable bytes through NUL or byte
            // 4097. Stop at NUL, or reject before attempting byte 4098.
            if unsafe { pointer.add(length).read() } == 0 {
                break;
            }
            if length == MAX_ARGUMENT_BYTES {
                return Err("argument exceeds 4096 bytes".into());
            }
            length += 1;
        }
        // SAFETY: the scan found this readable extent in one string, which the
        // caller keeps immutable until copy finishes.
        result.values[index] = copy(unsafe { std::slice::from_raw_parts(pointer, length) })?;
    }
    Ok(result)
}

#[cfg(target_os = "linux")]
pub(super) fn capture() -> Result<Arguments, ArgumentError> {
    let mut file = pipesql_filesystem::open_read("/proc/self/cmdline")
        .map_err(|source| failure("open CLI arguments", source))?;
    read_arguments(&mut file)
}

#[cfg(any(target_os = "linux", test))]
fn read_arguments(input: &mut impl io::Read) -> Result<Arguments, ArgumentError> {
    // The program name is discarded, but still consumes read work. Bound the
    // complete stream separately so an oversized ignored name cannot bypass the
    // per-argument limit. A read error ends capture without retry or fallback.
    const MAX_NATIVE_BYTES: usize = 4 * 1_048_576;
    let mut result = Arguments::empty();
    let mut chunk = [0; 4096];
    let mut argument = [0; MAX_ARGUMENT_BYTES];
    let mut length = 0;
    let mut total = 0_usize;
    let mut terminated = true;
    loop {
        let read = input
            .read(&mut chunk)
            .map_err(|source| failure("read CLI arguments", source))?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read)
            .filter(|total| *total <= MAX_NATIVE_BYTES)
            .ok_or_else(|| ArgumentError::from("native arguments exceed 4 MiB"))?;
        for &byte in &chunk[..read] {
            if result.count == MAX_ARGUMENTS {
                return Err("argument count exceeds 34".into());
            }
            terminated = byte == 0;
            if terminated {
                if result.count != 0 {
                    result.values[result.count] = copy(&argument[..length])?;
                }
                result.count += 1;
                length = 0;
            } else if result.count != 0 {
                if length == argument.len() {
                    return Err("argument exceeds 4096 bytes".into());
                }
                argument[length] = byte;
                length += 1;
            }
        }
    }
    if !terminated {
        return Err("native argument lacks NUL terminator".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    //! Check both capture paths with bounded memory and synthetic byte streams.
    //! Empty and non-UTF-8 arguments must survive; bad extents and reads must fail.

    use super::{
        ArgumentError, MAX_ARGUMENT_BYTES, MAX_ARGUMENTS, copy, copy_native, read_arguments,
    };
    use pipesql::Error;
    use std::ffi::OsStr;
    use std::io;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn capture_matches_the_native_process_arguments() {
        let captured: Vec<_> = super::capture().unwrap().into_iter().collect();
        let mut expected: Vec<_> = std::env::args_os().collect();
        expected[0].clear();
        assert_eq!(captured, expected);
    }

    #[test]
    fn native_copy_checks_extents_and_preserves_empty_raw_arguments() {
        for length in [0, 1, MAX_ARGUMENT_BYTES, MAX_ARGUMENT_BYTES + 1] {
            let mut bytes = vec![0xff; length];
            bytes.push(0);
            let pointers = [std::ptr::null(), c"".as_ptr().cast(), bytes.as_ptr()];
            // SAFETY: these local pointers and strings outlive the copy and
            // remain unchanged. The null program-name pointer is never read.
            let result = unsafe { copy_native(pointers.len(), pointers.as_ptr()) };
            if length <= MAX_ARGUMENT_BYTES {
                let values: Vec<_> = result.unwrap().into_iter().collect();
                assert!(values[0].is_empty());
                assert!(values[1].is_empty());
                assert_eq!(values[2].as_encoded_bytes(), &bytes[..length]);
            } else {
                assert!(result.is_err());
            }
        }
        let unterminated = [0xff; MAX_ARGUMENT_BYTES + 1];
        let pointers = [std::ptr::null(), unterminated.as_ptr()];
        // SAFETY: the array supplies all 4097 readable bytes required when no
        // NUL occurs. Reading farther would cross the array's allocation.
        assert!(unsafe { copy_native(pointers.len(), pointers.as_ptr()) }.is_err());
        let nulls = [std::ptr::null(); MAX_ARGUMENTS];
        // SAFETY: the zero-count case reads nothing. The other cases reject
        // the null vector, null string or excessive count before reading bytes.
        unsafe {
            assert!(copy_native(0, std::ptr::null()).is_ok());
            assert!(copy_native(1, std::ptr::null()).is_err());
            assert!(copy_native(2, nulls.as_ptr()).is_err());
            assert!(copy_native(MAX_ARGUMENTS + 1, std::ptr::null()).is_err());
        }
    }

    #[test]
    fn stream_preserves_empty_and_non_utf8_arguments() {
        let args = read_arguments(&mut &b"\0\0\xff\0last\0"[..]).unwrap();
        let values: Vec<_> = args.into_iter().collect();
        assert_eq!(
            values,
            [
                OsStr::new(""),
                OsStr::new(""),
                OsStr::from_bytes(b"\xff"),
                OsStr::new("last")
            ]
        );
    }

    #[test]
    fn stream_bounds_even_the_ignored_program_name() {
        use io::Read;
        let mut exact = io::repeat(b'x').take(4 * 1_048_576 - 1).chain(&b"\0"[..]);
        assert!(read_arguments(&mut exact).is_ok());
        let mut next = io::repeat(b'x').take(4 * 1_048_576).chain(&b"\0"[..]);
        assert!(read_arguments(&mut next).is_err());
    }

    #[test]
    fn stream_checks_count_lengths_terminators_and_io() {
        let exact = [b'x'; MAX_ARGUMENT_BYTES];
        assert!(copy(&exact).is_ok());
        assert!(copy(&[b'x'; MAX_ARGUMENT_BYTES + 1]).is_err());
        assert!(read_arguments(&mut &[0_u8; MAX_ARGUMENTS][..]).is_ok());
        assert!(read_arguments(&mut &[0_u8; MAX_ARGUMENTS + 1][..]).is_err());
        assert!(read_arguments(&mut &b"ignored\0unterminated"[..]).is_err());

        struct Broken;

        impl io::Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::Interrupted.into())
            }
        }
        assert!(
            matches!(read_arguments(&mut Broken), Err(ArgumentError::Library(Error::Io { source, .. })) if source.kind() == io::ErrorKind::Interrupted)
        );
    }
}
