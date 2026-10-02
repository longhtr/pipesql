//! Inspect an offline catalog graph using namespace v7 and object v6 contracts.
//!
//! Root selection distinguishes missing or damaged records from readable records
//! with conflicting authority. Limits bound traversal and decoding, not heap use.
//! The lease excludes cooperating database writers; arbitrary external mutation
//! is detected at file reads but is not a supported inspection workload.

use crate::Result;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

#[path = "catalog_objects.rs"]
mod objects;

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;

fn require(condition: bool, reason: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(reason.into())
    }
}
fn zero(bytes: &[u8]) -> Result<()> {
    require(bytes.iter().all(|b| *b == 0), "nonzero reserved bytes")
}
fn region(data: &[u8], start: usize, length: usize) -> Result<&[u8]> {
    data.get(start..start.checked_add(length).ok_or("byte extent overflow")?)
        .ok_or_else(|| "truncated bytes".into())
}
fn u32_at(data: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(region(data, at, 4)?.try_into()?))
}
fn u64_at(data: &[u8], at: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(region(data, at, 8)?.try_into()?))
}

pub(crate) fn crc32c(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 == 1 { 0x82f6_3b78 } else { 0 };
        }
    }
    !crc
}
fn protected(data: &[u8], at: usize) -> Result<bool> {
    let expected = u32_at(data, at)?;
    let mut copy = data.to_vec();
    copy[at..at + 4].fill(0);
    Ok(crc32c(&copy) == expected)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Reference {
    attempt: u64,
    ordinal: u32,
    size: u32,
    crc: u32,
}
impl Reference {
    fn name(&self) -> String {
        format!("{:016x}-{:08x}.obj", self.attempt, self.ordinal)
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() == 24, "reference extent")?;
        zero(&bytes[20..])?;
        let result = Self {
            attempt: u64_at(bytes, 0)?,
            ordinal: u32_at(bytes, 8)?,
            size: u32_at(bytes, 12)?,
            crc: u32_at(bytes, 16)?,
        };
        require(
            result.attempt > 0 && result.ordinal > 0 && result.size >= 64,
            "reference identity/extent",
        )?;
        Ok(result)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    issued: u64,
    generation: u64,
    last: u64,
    catalog: Option<Reference>,
    successes: Option<Reference>,
}
impl Snapshot {
    fn follows(&self, old: &Self) -> bool {
        let issuance = old.issued.checked_add(1) == Some(self.issued)
            && (self.generation, self.last, &self.catalog, &self.successes)
                == (old.generation, old.last, &old.catalog, &old.successes);
        let commit = self.issued == old.issued
            && old.generation.checked_add(1) == Some(self.generation)
            && self.last == self.issued
            && old.last < self.last;
        issuance || commit
    }

    fn decode(data: &[u8], database: &[u8], role: u8, wal: bool) -> Result<Option<Self>> {
        let size = if wal { 512 } else { 4096 };
        // An unsupported readable version is not an excuse to select older data.
        if data.len() >= 12 {
            require(u32_at(data, 8)? == 7, "unsupported authoritative version")?;
        }
        if data.len() != size || &data[..8] != if wal { b"PSQLWAL\0" } else { b"PSQLROOT" } {
            return Ok(None);
        }
        if !protected(data, 108)? {
            return Ok(None);
        }
        require(
            u32_at(data, 12)? as usize == size && data[32] == role,
            "snapshot extent/role",
        )?;
        require(&data[16..32] == database, "foreign snapshot identity")?;
        for (lo, hi) in [(33, 40), (48, 56), (80, 108), (120, 128), (176, size)] {
            zero(&data[lo..hi])?;
        }
        let generation = u64_at(data, 40)?;
        let issued = u64_at(data, 112)?;
        if generation == 0 {
            zero(&data[56..80])?;
            zero(&data[128..176])?;
            return Ok(Some(Self {
                issued,
                generation,
                last: 0,
                catalog: None,
                successes: None,
            }));
        }
        require(&data[56..72] == database, "foreign transaction identity")?;
        let last = u64_at(data, 72)?;
        let catalog = Reference::decode(&data[128..152])?;
        let successes = Reference::decode(&data[152..176])?;
        require(
            generation <= 1_048_576 && generation <= last && last <= issued,
            "success/issuance bounds",
        )?;
        require(
            catalog.attempt <= last
                && successes.attempt == last
                && catalog.name() != successes.name(),
            "root object identity",
        )?;
        require(
            catalog.size <= 8256 && u64::from(successes.size) == 64 + 8 * generation,
            "root reference extent",
        )?;
        Ok(Some(Self {
            issued,
            generation,
            last,
            catalog: Some(catalog),
            successes: Some(successes),
        }))
    }
}

struct Reader {
    path: PathBuf,
    database: Vec<u8>,
    objects: BTreeMap<String, Metadata>,
    reached: BTreeSet<String>,
    read_bytes: u64,
    values: usize,
    limits: Limits,
}

/// Traversal limits constrain the inspector's work, not the engine's memory use.
#[derive(Clone, Copy)]
pub struct Limits {
    pub objects: usize,
    pub read_bytes: u64,
    pub values: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            objects: 4096,
            read_bytes: 64 * 1024 * 1024,
            values: 1_000_000,
        }
    }
}

impl Limits {
    fn options(arguments: &[String]) -> Result<Self> {
        let mut limits = Self::default();
        let mut seen = [false; 3];
        require(
            arguments.len().is_multiple_of(2),
            "inspection options need values",
        )?;
        for pair in arguments.as_chunks::<2>().0 {
            let (index, maximum) = match pair[0].as_str() {
                "--max-objects" => (0, 1_048_576),
                "--max-read-bytes" => (1, 1_073_741_824),
                "--max-values" => (2, 10_000_000),
                _ => return Err(format!("unknown inspection option: {}", pair[0]).into()),
            };
            require(!seen[index], "duplicate inspection option")?;
            seen[index] = true;
            let value: u64 = pair[1].parse()?;
            require(
                value > 0 && value <= maximum,
                "inspection limit outside supported range",
            )?;
            match index {
                0 => limits.objects = usize::try_from(value)?,
                1 => limits.read_bytes = value,
                2 => limits.values = usize::try_from(value)?,
                _ => unreachable!(),
            }
        }
        Ok(limits)
    }
}

/// Validate the complete graph before emitting JSON. Output errors remain failures.
pub fn command(path: &Path, options: &[String]) -> Result<()> {
    use std::io::Write;
    require(
        path.is_absolute(),
        "inspection needs an absolute database path",
    )?;
    let limits = Limits::options(options)?;
    let graph = inspect_with_limits(path, limits)?;
    let mut output = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut output, &graph)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

impl Reader {
    fn read(&mut self, path: &Path, maximum: u64, exact: Option<u64>) -> Result<Vec<u8>> {
        let before = fs::symlink_metadata(path)?;
        require(before.is_file(), "not a regular file")?;
        require(
            before.nlink() == 1
                || ["ROOT.A", "ROOT.B", "CONTROL", "LOCK"]
                    .iter()
                    .any(|name| path.file_name().is_some_and(|v| v == *name)),
            "aliased mutable/object file",
        )?;
        require(
            before.len() <= maximum && exact.is_none_or(|size| size == before.len()),
            "file extent",
        )?;
        self.read_bytes = self
            .read_bytes
            .checked_add(before.len())
            .ok_or("read budget overflow")?;
        require(
            self.read_bytes <= self.limits.read_bytes,
            "read byte budget",
        )?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        let opened = file.metadata()?;
        require(
            (opened.dev(), opened.ino(), opened.len())
                == (before.dev(), before.ino(), before.len()),
            "file changed during open",
        )?;
        let mut data = vec![0; opened.len() as usize];
        file.read_exact(&mut data)?;
        let after = file.metadata()?;
        require(
            (opened.mtime(), opened.mtime_nsec(), opened.len())
                == (after.mtime(), after.mtime_nsec(), after.len()),
            "file changed during read",
        )?;
        Ok(data)
    }

    fn object(
        &mut self,
        reference: &Reference,
        maximum: u64,
        magic: &[u8; 8],
        metadata: Option<usize>,
    ) -> Result<Vec<u8>> {
        require(
            reference.attempt > 0 && reference.ordinal > 0 && reference.size >= 64,
            "object identity/extent",
        )?;
        let name = reference.name();
        require(
            self.objects.contains_key(&name),
            "missing referenced object",
        )?;
        require(self.reached.insert(name.clone()), "object alias or cycle")?;
        require(
            u64::from(reference.size) <= maximum,
            "role-specific object extent",
        )?;
        let data = self.read(
            &self.path.join("units").join(name),
            maximum,
            Some(u64::from(reference.size)),
        )?;
        require(
            &data[..8] == magic && u32_at(&data, 8)? == 6,
            "object magic/version",
        )?;
        require(data[16..32] == self.database, "foreign object identity")?;
        require(
            crc32c(region(&data, 0, metadata.unwrap_or(data.len()))?) == reference.crc,
            "object checksum",
        )?;
        Ok(data)
    }

    fn graph(&mut self) -> Result<Value> {
        let entries = inventory(&self.path, 9)?;
        let allowed = [
            "LOCK",
            "CONTROL",
            "ROOT.A",
            "ROOT.B",
            "WAL",
            "units",
            "private",
            "ROOT.A.next",
            "ROOT.B.next",
        ];
        require(
            entries.keys().all(|name| allowed.contains(&name.as_str()))
                && ["LOCK", "CONTROL", "WAL", "units", "private"]
                    .iter()
                    .all(|name| entries.contains_key(*name)),
            "namespace names",
        )?;
        self.read(&self.path.join("LOCK"), 0, Some(0))?;
        let control = self.read(&self.path.join("CONTROL"), 128, Some(128))?;
        require(
            &control[..8] == b"PIPESQL\0"
                && u32_at(&control, 8)? == 7
                && u32_at(&control, 12)? == 128
                && protected(&control, 36)?,
            "CONTROL header/checksum",
        )?;
        zero(&control[32..36])?;
        zero(&control[40..])?;
        self.database = control[16..32].to_vec();
        require(
            self.database.iter().any(|byte| *byte != 0),
            "zero database identity",
        )?;
        let mut snapshots = Vec::new();
        for (label, role) in [("ROOT.A", 0), ("ROOT.B", 1), ("WAL", 0)] {
            if !entries.contains_key(label) {
                snapshots.push(None);
                continue;
            }
            let size = if label == "WAL" { 512 } else { 4096 };
            let raw = self.read(
                &self.path.join(label),
                size,
                (label == "WAL").then_some(size),
            )?;
            snapshots.push(Snapshot::decode(
                &raw,
                &self.database,
                role,
                label == "WAL",
            )?);
        }
        let a = snapshots[0].as_ref();
        let b = snapshots[1].as_ref();
        let fence = snapshots[2].as_ref();
        let selected = match (a, b) {
            (Some(a), Some(b)) => {
                require(a == b || a.follows(b) || b.follows(a), "nonadjacent roots")?;
                if (a.issued, a.generation) >= (b.issued, b.generation) {
                    a
                } else {
                    b
                }
            }
            _ => {
                let selected = a.or(b).ok_or("insufficient root authority")?;
                require(fence == Some(selected), "insufficient root authority")?;
                selected
            }
        };
        require(
            fence.is_none_or(|fence| fence == selected || fence.follows(selected)),
            "inconsistent fence",
        )?;
        let private = inventory(&self.path.join("private"), 2)?;
        for label in private.keys() {
            require(
                matches!(label.as_str(), "SCRATCH.A" | "SCRATCH.B"),
                "unknown scratch name",
            )?;
            self.read(&self.path.join("private").join(label), 0, Some(0))?;
        }
        for label in ["ROOT.A.next", "ROOT.B.next"] {
            if entries.contains_key(label) {
                self.read(&self.path.join(label), 4096, None)?;
            }
        }
        self.objects = inventory(&self.path.join("units"), self.limits.objects)?;
        for (name, metadata) in &self.objects {
            let bytes = name.as_bytes();
            require(
                bytes.len() == 29
                    && bytes[16] == b'-'
                    && &bytes[25..] == b".obj"
                    && bytes[..16]
                        .iter()
                        .chain(&bytes[17..25])
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b)),
                "object filename",
            )?;
            let attempt = u64::from_str_radix(&name[..16], 16)?;
            let ordinal = u32::from_str_radix(&name[17..25], 16)?;
            require(
                attempt > 0 && ordinal > 0 && attempt <= selected.issued,
                "object beyond issued prefix",
            )?;
            require(
                metadata.is_file() && metadata.nlink() == 1 && metadata.len() <= 33_556_544,
                "object ownership/extent",
            )?;
        }
        let mut successes = Vec::new();
        let mut tables = Vec::new();
        if let Some(reference) = &selected.successes {
            let data = self.object(reference, 8_388_672, b"PSQLSUCC", None)?;
            require(
                u64_at(&data, 32)? == selected.generation
                    && u64_at(&data, 40)? == reference.attempt
                    && u32_at(&data, 48)? == reference.ordinal
                    && u64_at(&data, 56)? == selected.last,
                "history header",
            )?;
            zero(&data[12..16])?;
            zero(&data[52..56])?;
            let mut previous = 0;
            for at in (64..data.len()).step_by(8) {
                let attempt = u64_at(&data, at)?;
                require(
                    previous < attempt && attempt <= selected.last,
                    "history order",
                )?;
                successes.push(attempt);
                previous = attempt;
            }
            require(previous == selected.last, "history final receipt")?;
            tables = self.catalog(selected.catalog.as_ref().unwrap())?;
        }
        let unreferenced: Vec<_> = self
            .objects
            .keys()
            .filter(|name| !self.reached.contains(*name))
            .collect();
        Ok(
            json!({"issued": selected.issued, "generation": selected.generation, "successes": successes, "tables": tables, "roots_settled": a == b && a == fence && a.is_some(), "reachable": self.reached, "unreferenced": unreferenced, "read_bytes": self.read_bytes, "decoded_values": self.values}),
        )
    }
}

fn inventory(path: &Path, maximum: usize) -> Result<BTreeMap<String, Metadata>> {
    require(fs::symlink_metadata(path)?.is_dir(), "not a directory")?;
    let mut entries = BTreeMap::new();
    for entry in fs::read_dir(path)? {
        require(entries.len() < maximum, "directory entry budget")?;
        let entry = entry?;
        entries.insert(
            entry
                .file_name()
                .into_string()
                .map_err(|_| "non-UTF-8 namespace entry")?,
            fs::symlink_metadata(entry.path())?,
        );
    }
    Ok(entries)
}

pub fn inspect(path: &Path) -> Result<Value> {
    inspect_with_limits(path, Limits::default())
}

pub fn inspect_with_limits(path: &Path, limits: Limits) -> Result<Value> {
    require(
        fs::symlink_metadata(path)?.is_dir(),
        "database is not a directory",
    )?;
    let lease: File = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path.join("LOCK"))?;
    require(lease.metadata()?.is_file(), "LOCK is not regular")?;
    // SAFETY: flock retains no pointer; the file holds the lock through traversal.
    if unsafe { libc::flock(lease.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Reader {
        path: path.to_owned(),
        database: Vec::new(),
        objects: BTreeMap::new(),
        reached: BTreeSet::new(),
        read_bytes: 0,
        values: 0,
        limits,
    }
    .graph()
}
