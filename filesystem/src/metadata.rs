//! Put native and Rust file metadata into one representation for comparison.
//!
//! A pathname is a name that can be replaced; an open descriptor refers to the
//! file that was opened. Comparing their device and inode numbers can detect
//! that they now refer to different files. `FileIdentity` holds that pair, while
//! `Metadata` also records size, link count, file type and timestamps.
//!
//! Both conversion paths use the same units and integer representation, including
//! Darwin's signed device identifier. Invalid native sizes or narrowing conversions
//! return an error. These are copied observations: they neither lock a file nor
//! prevent a later change to its name or contents.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    pub fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

pub struct Metadata {
    identity: FileIdentity,
    length: u64,
    links: u64,
    kind: FileType,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileType {
    File,
    Directory,
    Symlink,
    Other,
}

impl FileType {
    pub fn is_file(self) -> bool {
        self == Self::File
    }

    pub fn is_dir(self) -> bool {
        self == Self::Directory
    }

    pub fn is_symlink(self) -> bool {
        self == Self::Symlink
    }
}

impl Metadata {
    /// Copy metadata already obtained through the standard library into the
    /// representation used for native inspection. This performs no filesystem I/O.
    pub fn from_std(raw: &fs::Metadata) -> Self {
        let kind = if raw.file_type().is_file() {
            FileType::File
        } else if raw.file_type().is_dir() {
            FileType::Directory
        } else if raw.file_type().is_symlink() {
            FileType::Symlink
        } else {
            FileType::Other
        };
        Self {
            identity: FileIdentity::from_metadata(raw),
            length: raw.len(),
            links: raw.nlink(),
            kind,
            modified_seconds: raw.mtime(),
            modified_nanoseconds: raw.mtime_nsec(),
            changed_seconds: raw.ctime(),
            changed_nanoseconds: raw.ctime_nsec(),
        }
    }

    /// Last content-modification time in Unix seconds; `mtime_nsec` is its fraction.
    pub fn mtime(&self) -> i64 {
        self.modified_seconds
    }

    pub fn mtime_nsec(&self) -> i64 {
        self.modified_nanoseconds
    }

    /// Last metadata-change time in Unix seconds, not file creation time.
    pub fn ctime(&self) -> i64 {
        self.changed_seconds
    }

    pub fn ctime_nsec(&self) -> i64 {
        self.changed_nanoseconds
    }

    pub fn identity(&self) -> FileIdentity {
        self.identity
    }

    pub fn len(&self) -> u64 {
        self.length
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn nlink(&self) -> u64 {
        self.links
    }

    pub fn file_type(&self) -> FileType {
        self.kind
    }

    // Keep explicit conversions because inode and link-count widths differ by
    // target, even where a conversion is redundant on the current platform.
    #[allow(clippy::useless_conversion)]
    pub(super) fn from_native(raw: &libc::stat) -> io::Result<Self> {
        #[cfg(target_os = "macos")]
        // Match MetadataExt exactly: sign-extend Darwin's dev_t before preserving
        // its bits as u64. Treating it as an unsigned 32-bit number would make the
        // same device compare unequal through the native and std paths.
        let device = i64::from(raw.st_dev).cast_unsigned();

        #[cfg(target_os = "linux")]
        let device = u64::from(raw.st_dev);
        let kind = match raw.st_mode & libc::S_IFMT {
            libc::S_IFREG => FileType::File,
            libc::S_IFDIR => FileType::Directory,
            libc::S_IFLNK => FileType::Symlink,
            _ => FileType::Other,
        };
        Ok(Self {
            identity: FileIdentity {
                device,
                inode: u64::from(raw.st_ino),
            },
            length: u64::try_from(raw.st_size).map_err(|_| io::ErrorKind::InvalidData)?,
            links: u64::from(raw.st_nlink),
            kind,
            modified_seconds: i64::try_from(raw.st_mtime)
                .map_err(|_| io::ErrorKind::InvalidData)?,
            modified_nanoseconds: i64::try_from(raw.st_mtime_nsec)
                .map_err(|_| io::ErrorKind::InvalidData)?,
            changed_seconds: i64::try_from(raw.st_ctime).map_err(|_| io::ErrorKind::InvalidData)?,
            changed_nanoseconds: i64::try_from(raw.st_ctime_nsec)
                .map_err(|_| io::ErrorKind::InvalidData)?,
        })
    }
}
