//! Capture caller output without introducing observed file writes.
//!
//! Reservations and actual unlinked Linux file lengths are separate observations.
//! The caller owns a fixed output allowance; overflow fails the check rather than
//! silently growing it. Saved files and answer checks run outside this sink.

use pipesql::{CancellationToken, Database, Error, ExportLimits, PreparedQuery};
use std::{io::Write, path::Path};

pub(super) struct Output<'a> {
    pub bytes: Vec<u8>,
    db: &'a Database,
    path: &'a Path,
    pub spill: u64,
    pub temporary: u64,
}

impl<'a> Output<'a> {
    pub fn new(db: &'a Database, path: &'a Path, bytes: u64) -> Self {
        Self {
            bytes: Vec::with_capacity(usize::try_from(bytes).unwrap()),
            db,
            path,
            spill: 0,
            temporary: 0,
        }
    }
}

impl Write for Output<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.temporary = self.temporary.max(self.db.reserved_temp_bytes());
        if cfg!(target_os = "linux") && self.spill == 0 && self.temporary > 0 {
            self.spill = written_spill(self.path);
        }
        if bytes.len() > self.bytes.capacity() - self.bytes.len() {
            return Err(std::io::Error::other("native I/O capture limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn written_spill(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|entry| {
            let descriptor = entry.unwrap().path();
            let target = match std::fs::read_link(&descriptor) {
                Ok(target) => target,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
                Err(error) => panic!("read native I/O spill descriptor: {error}"),
            };
            if !target.starts_with(path) {
                return None;
            }
            let metadata = std::fs::metadata(descriptor).unwrap();
            (metadata.is_file() && metadata.nlink() == 0).then_some(metadata.len())
        })
        .sum()
}
#[cfg(not(target_os = "linux"))]
fn written_spill(_: &Path) -> u64 {
    0
}

pub(super) fn descriptors() -> usize {
    std::fs::read_dir("/dev/fd").unwrap().count()
}

pub(super) fn released(db: &Database, memory: u64, files: usize) {
    assert_eq!(db.reserved_memory_bytes(), memory);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert_eq!(descriptors(), files);
}

pub(super) fn export<'a>(
    db: &'a Database,
    query: &PreparedQuery<'_>,
    path: &'a Path,
    limits: ExportLimits,
) -> (Output<'a>, Result<u64, Error>) {
    let mut output = Output::new(db, path, limits.bytes);
    let result = db.export_jsonl(query, &mut output, limits, &CancellationToken::new());
    (output, result)
}
