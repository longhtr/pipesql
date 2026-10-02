//! Compare Linux pathname and descriptor identities after repeated replacement.
//!
//! Each observation uses lstat and statx while enumerating the directory, closes
//! that directory, then opens the file and repeats both metadata families. Healthy
//! runs make no mutation between inspection and open. This preserves the native
//! sequence that exposed the shared-mount failure without calling database code.
//! The caller owns the new directory and must exclude other writers. Exit 1 means
//! identity mismatch, 2 means an operation/content failure, and 0 means this run
//! found no mismatch; success cannot clear an intermittent filesystem failure.

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        ffi::{CStr, CString},
        fs::{self, File, OpenOptions},
        io::{self, Write},
        mem::MaybeUninit,
        os::{
            fd::{AsRawFd, IntoRawFd},
            unix::{
                ffi::OsStrExt,
                fs::{DirBuilderExt, OpenOptionsExt},
            },
        },
        path::Path,
    };

    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
    const NAMES: [&str; 2] = ["ROOT.A", "ROOT.B"];
    const ITERATIONS: u32 = 200;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Identity {
        device: libc::dev_t,
        inode: libc::ino_t,
    }

    fn path(root: &Path, name: &str) -> Result<CString> {
        let joined = root.join(name);
        let bytes = joined.as_os_str().as_bytes();
        if bytes.len() >= 4096 {
            return Err("pathname exceeds witness bound".into());
        }
        Ok(CString::new(bytes)?)
    }

    fn status(result: libc::c_int) -> Result<()> {
        if result != 0 {
            Err(io::Error::last_os_error().into())
        } else {
            Ok(())
        }
    }

    fn close(file: File) -> Result<()> {
        // SAFETY: ownership leaves File exactly once; close is never retried.
        status(unsafe { libc::close(file.into_raw_fd()) })
    }

    fn ordinary(name: &CStr, file: Option<&File>) -> Result<Identity> {
        let mut value = MaybeUninit::<libc::stat>::uninit();
        // SAFETY: both calls synchronously initialize valid output storage.
        // The descriptor or terminated path stays live throughout the call.
        unsafe {
            status(match file {
                Some(file) => libc::fstat(file.as_raw_fd(), value.as_mut_ptr()),
                None => libc::lstat(name.as_ptr(), value.as_mut_ptr()),
            })?;
            let value = value.assume_init();
            Ok(Identity {
                device: value.st_dev,
                inode: value.st_ino,
            })
        }
    }

    fn extended(name: &CStr, file: Option<&File>) -> Result<Identity> {
        let mut value = MaybeUninit::<libc::statx>::uninit();
        let (fd, name, flags, mask) = match file {
            Some(file) => (file.as_raw_fd(), c"", libc::AT_EMPTY_PATH, libc::STATX_ALL),
            None => (
                libc::AT_FDCWD,
                name,
                libc::AT_SYMLINK_NOFOLLOW | libc::AT_NO_AUTOMOUNT,
                libc::STATX_BASIC_STATS,
            ),
        };
        // SAFETY: statx initializes the output on success; path and descriptor
        // remain valid and the call retains no pointers.
        unsafe {
            status(libc::statx(
                fd,
                name.as_ptr(),
                flags,
                mask,
                value.as_mut_ptr(),
            ))?;
            let value = value.assume_init();
            if value.stx_mask & libc::STATX_INO == 0 {
                return Err("statx omitted inode".into());
            }
            Ok(Identity {
                device: libc::makedev(value.stx_dev_major, value.stx_dev_minor),
                inode: value.stx_ino,
            })
        }
    }

    fn contents(file: &File, tag: u32, positional: bool) -> Result<()> {
        let mut bytes = [0_u8; 4096];
        // SAFETY: writable buffer spans the supplied length; the live File owns
        // the descriptor. A short read is an observation failure, not retried.
        let count = unsafe {
            if positional {
                libc::pread(file.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len(), 0)
            } else {
                libc::read(file.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len())
            }
        };
        if count < 0 {
            return Err(io::Error::last_os_error().into());
        }
        if count as usize != bytes.len() || bytes.iter().any(|byte| *byte != (tag & 255) as u8) {
            return Err("wrong opened contents or short read".into());
        }
        Ok(())
    }

    fn write_file(root: &Path, name: &str, tag: u32) -> Result<()> {
        let name = path(root, name)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NONBLOCK)
            .mode(0o666)
            .open(Path::new(std::ffi::OsStr::from_bytes(name.to_bytes())))?;
        let bytes = [(tag & 255) as u8; 4096];
        // SAFETY: bytes and descriptor remain live for this synchronous write.
        let count = unsafe { libc::write(file.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
        if count < 0 {
            return Err(io::Error::last_os_error().into());
        }
        if count as usize != bytes.len() {
            return Err("short write".into());
        }
        extended(&name, Some(&file))?;
        contents(&file, tag, true)?;
        file.sync_all()?;
        close(file)
    }

    fn sync(root: &Path) -> Result<()> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(root)?;
        file.sync_all()?;
        close(file)
    }

    struct Directory(*mut libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) {
            // SAFETY: this guard uniquely owns an open DIR unless explicitly closed.
            if !self.0.is_null() {
                unsafe {
                    libc::closedir(self.0);
                }
            }
        }
    }

    fn enumerate(root: &Path) -> Result<[[Identity; 2]; 2]> {
        let name = CString::new(root.as_os_str().as_bytes())?;
        // SAFETY: the path is terminated; a successful result is owned by the guard.
        let mut directory = Directory(unsafe { libc::opendir(name.as_ptr()) });
        if directory.0.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        let mut found = [None; 2];
        loop {
            // SAFETY: the directory stays open and each returned record is read
            // before another readdir call. errno is this thread's native cell.
            let entry = unsafe {
                *libc::__errno_location() = 0;
                let entry = libc::readdir(directory.0);
                if entry.is_null() {
                    if *libc::__errno_location() != 0 {
                        return Err(io::Error::last_os_error().into());
                    }
                    break;
                }
                CStr::from_ptr((*entry).d_name.as_ptr()).to_bytes()
            };
            if let Some(index) = NAMES.iter().position(|name| name.as_bytes() == entry) {
                if found[index].is_some() {
                    return Err("duplicate directory entry".into());
                }
                let name = path(root, NAMES[index])?;
                found[index] = Some([ordinary(&name, None)?, extended(&name, None)?]);
            }
        }
        let pointer = std::mem::replace(&mut directory.0, std::ptr::null_mut());
        // SAFETY: transfer the guard's live directory to this single close call.
        status(unsafe { libc::closedir(pointer) })?;
        Ok([
            found[0].ok_or("missing ROOT.A")?,
            found[1].ok_or("missing ROOT.B")?,
        ])
    }

    fn compare(root: &Path, iteration: u32, name: &str, before: [Identity; 2]) -> Result<bool> {
        let native = path(root, name)?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(root.join(name))?;
        let observations = [
            before[0],
            before[1],
            ordinary(&native, Some(&file))?,
            extended(&native, Some(&file))?,
            ordinary(&native, None)?,
            extended(&native, None)?,
        ];
        contents(&file, iteration, false)?;
        close(file)?;
        if observations.iter().any(|value| *value != observations[0]) {
            let mut output = io::stdout().lock();
            write!(output, "mismatch iteration={iteration} file={name}")?;
            for (label, value) in [
                "before",
                "path_statx",
                "fstat",
                "statx",
                "after",
                "after_statx",
            ]
            .into_iter()
            .zip(observations)
            {
                write!(output, " {label}={}:{}", value.device, value.inode)?;
            }
            writeln!(output, " content={}", iteration & 255)?;
            output.flush()?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn run() -> Result<i32> {
        let arguments: Vec<_> = std::env::args_os().skip(1).collect();
        if arguments.len() != 2 {
            return Err("expected /absolute/new-directory healthy|replace|missing".into());
        }
        let root = Path::new(&arguments[0]);
        let mode = arguments[1].to_str().ok_or("invalid mode")?;
        if !root.is_absolute() || !matches!(mode, "healthy" | "replace" | "missing") {
            return Err("invalid path or mode".into());
        }
        path(root, "ROOT.A.next")?;
        fs::DirBuilder::new().mode(0o700).create(root)?;
        for name in NAMES {
            write_file(root, name, 0)?;
        }
        sync(root)?;
        for iteration in 1..=ITERATIONS {
            for name in NAMES {
                let next = format!("{name}.next");
                write_file(root, &next, iteration)?;
                fs::rename(root.join(&next), root.join(name))?;
                sync(root)?;
            }
            let before = enumerate(root)?;
            // Controls deliberately violate the healthy run's no-mutation premise.
            // Identical content ensures replacement is caught by identity alone.
            if mode == "replace" {
                write_file(root, "ROOT.A.next", iteration)?;
                fs::rename(root.join("ROOT.A.next"), root.join("ROOT.A"))?;
                sync(root)?;
            } else if mode == "missing" {
                fs::remove_file(root.join("ROOT.A"))?;
            }
            for (index, name) in NAMES.into_iter().enumerate() {
                if compare(root, iteration, name, before[index])? {
                    return Ok(1);
                }
            }
        }
        let mut output = io::stdout().lock();
        writeln!(output, "400 file replacements checked")?;
        output.flush()?;
        Ok(0)
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    let result = linux::run();
    #[cfg(not(target_os = "linux"))]
    let result: Result<i32, Box<dyn std::error::Error>> =
        Err("identity witness requires GNU/Linux".into());
    match result {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("identity observation failed: {error}");
            std::process::exit(2);
        }
    }
}
