//! Source identity and bounded scans. Pass one admits rows; pass two feeds staging.
use super::staging::Staging;
use crate::effects::{Effect, Effects, LoadEffect};
use crate::error::io_error;
use crate::load_input::{InputError, MAX_CHUNK_BYTES, MAX_INPUT_BYTES, Scan, Scanner};
use crate::path::validate_requested_path;
use crate::{CancellationToken, Error};
use pipesql_filesystem as filesystem;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
struct InputIdentity {
    file: pipesql_filesystem::FileIdentity,
    length: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl InputIdentity {
    fn from_metadata(metadata: &filesystem::Metadata) -> Self {
        Self {
            file: metadata.identity(),
            length: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        }
    }
}

pub(super) struct InputSource {
    path: PathBuf,
    identity: InputIdentity,
}

impl InputSource {
    pub(super) fn check_after_build(
        &self,
        byte_offset: u64,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        effects.before(Effect::Load(LoadEffect::InspectInputFinal))?;
        let final_metadata = filesystem::metadata(&self.path)
            .map_err(|source| io_error("inspect input after unit build", source))?;
        if InputIdentity::from_metadata(&final_metadata) != self.identity {
            return Err(Error::Input {
                message: "input changed during unit build",
                byte_offset,
            });
        }
        Ok(())
    }
}

pub(super) fn inspect_input(path: &Path, effects: &mut Effects) -> Result<InputSource, Error> {
    validate_requested_path(path).map_err(|_| Error::Input {
        message: "input path must be absolute,bounded,and contain no dot components",
        byte_offset: 0,
    })?;
    effects.before(Effect::Load(LoadEffect::InspectInput))?;
    let metadata = filesystem::symlink_metadata(path)
        .map_err(|source| io_error("inspect load input", source))?;
    if !metadata.file_type().is_file() {
        return Err(Error::Input {
            message: "input must be a regular non-symlink file",
            byte_offset: 0,
        });
    }
    if metadata.len() > MAX_INPUT_BYTES {
        return Err(Error::Input {
            message: "input exceeds 1 GiB",
            byte_offset: MAX_INPUT_BYTES,
        });
    }
    let expected = InputIdentity::from_metadata(&metadata);
    effects.before(Effect::Load(LoadEffect::CanonicalizeInput))?;
    let canonical = filesystem::canonicalize(path).map_err(|source| match source {
        filesystem::CanonicalizeError::Io(source) => io_error("canonicalize load input", source),
        filesystem::CanonicalizeError::WorkLimit => crate::path::native_path_work_limit(),
    })?;
    Ok(InputSource {
        path: canonical,
        identity: expected,
    })
}

fn open_input(source: &InputSource, effects: &mut Effects) -> Result<File, Error> {
    effects.before(Effect::Load(LoadEffect::OpenInput))?;
    let file = filesystem::open_read(&source.path)
        .map_err(|source| io_error("open load input", source))?;
    effects.before(Effect::Load(LoadEffect::InspectOpenInput))?;
    let metadata = filesystem::file_metadata(&file)
        .map_err(|source| io_error("inspect open load input", source))?;
    if InputIdentity::from_metadata(&metadata) != source.identity {
        return Err(Error::Input {
            message: "input changed before scan",
            byte_offset: 0,
        });
    }
    Ok(file)
}

pub(super) fn scan_pass(
    source: &InputSource,
    input_buffer: &mut [u8],
    cancellation: &CancellationToken,
    effects: &mut Effects,
    mut staging: Option<&mut Staging>,
) -> Result<Scan, Error> {
    assert_eq!(input_buffer.len(), MAX_CHUNK_BYTES);
    let mut file = open_input(source, effects)?;
    let mut scanner = Scanner::new();
    loop {
        cancellation.check()?;
        let short = effects.before(Effect::Load(LoadEffect::ReadInput))?;
        let request = if short {
            input_buffer
                .len()
                .checked_sub(1)
                .expect("input buffer is nonempty")
        } else {
            input_buffer.len()
        };
        let read = file
            .read(&mut input_buffer[..request])
            .map_err(|source| io_error("read load input", source))?;
        if read == 0 {
            break;
        }
        scanner
            .begin_chunk(&input_buffer[..read])
            .map_err(|error| input_error(error, scanner.byte_offset()))?;
        for byte in &input_buffer[..read] {
            if let Some(row) = scanner
                .consume(*byte)
                .map_err(|error| input_error(error, scanner.byte_offset()))?
                && let Some(writer) = staging.as_deref_mut()
            {
                writer.append(row, scanner.byte_offset(), cancellation, effects)?;
            }
        }
    }
    effects.before(Effect::Load(LoadEffect::InspectInputFinal))?;
    let metadata = filesystem::file_metadata(&file)
        .map_err(|source| io_error("inspect load input after scan", source))?;
    if InputIdentity::from_metadata(&metadata) != source.identity {
        return Err(Error::Input {
            message: "input changed during scan",
            byte_offset: scanner.byte_offset(),
        });
    }
    let scan = scanner
        .finish()
        .map_err(|error| input_error(error, source.identity.length))?;
    if scan.input_bytes != source.identity.length {
        return Err(Error::Input {
            message: "input length disagrees with bytes read",
            byte_offset: scan.input_bytes,
        });
    }
    Ok(scan)
}

fn input_error(error: InputError, byte_offset: u64) -> Error {
    Error::Input {
        message: error.message(),
        byte_offset,
    }
}
