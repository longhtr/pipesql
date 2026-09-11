//! Build a private legacy unit, independently read it back, then attach it through
//! the publisher. Only encoded metadata survives the assembly/readback boundary.
use super::input::{InputSource, scan_pass};
use super::staging::{BLOCK_BYTES, COLUMNS, STAGING_BUFFER_BYTES, Staging};
use crate::effects::{
    DirectoryKind, Effect, Effects, LoadEffect, read_exact_at, write_all_at, write_nonempty,
};
use crate::error::io_error;
use crate::load_input::{MAX_CHUNK_BYTES, Scan};
use crate::namespace::{
    PRIVATE_NAME, UNIT_NAME, UNITS_NAME, sync_directory, sync_directory_cancellable,
};
use crate::path::joined_path;
use crate::publication::{PublicationFailure, publish_snapshot};
use crate::storage_format::{self, BlockDescriptor, RootState, WalRecord};
use crate::{CancellationToken, Error, TransactionId};
use pipesql_filesystem as filesystem;
use std::fs::File;
use std::io;
use std::path::Path;

const PAYLOAD_OFFSET_BYTES: usize = 28_672;
const PRIVATE_UNIT_NAME: &str = "UNIT.next";

#[derive(Clone, Copy)]
pub(super) struct LoadContext<'load> {
    pub(super) root: &'load Path,
    pub(super) database_id: storage_format::DatabaseId,
    pub(super) transaction: TransactionId,
    pub(super) source: &'load InputSource,
    pub(super) first: Scan,
    pub(super) layout: storage_format::Layout,
    pub(super) cancellation: &'load CancellationToken,
}

struct BuiltUnit {
    rows: u64,
    bytes: u64,
    metadata_crc32c: u32,
}

pub(super) fn build_and_publish(
    context: &LoadContext<'_>,
    arena: &mut [u8],
    effects: &mut Effects,
) -> Result<(), PublicationFailure> {
    let result = build_private_unit(context, arena, effects)
        .map_err(PublicationFailure::before_publication)?;
    publish_unit(
        context.root,
        context.database_id,
        context.transaction,
        result,
        context.cancellation,
        effects,
    )
}

fn build_private_unit(
    context: &LoadContext<'_>,
    arena: &mut [u8],
    effects: &mut Effects,
) -> Result<BuiltUnit, Error> {
    let root = context.root;
    let source = context.source;
    let first = context.first;
    let layout = context.layout;
    let cancellation = context.cancellation;
    let (input_buffer, rest) = arena.split_at_mut(MAX_CHUNK_BYTES);
    let (stage_buffers, rest) = rest.split_at_mut(STAGING_BUFFER_BYTES);
    let (descriptor_buffer, copy_buffer) = rest.split_at_mut(storage_format::DESCRIPTOR_BYTES);
    assert_eq!(copy_buffer.len(), BLOCK_BYTES);

    let mut staging = Staging::new(root, stage_buffers, first.rows, effects)?;
    let second = scan_pass(
        source,
        input_buffer,
        cancellation,
        effects,
        Some(&mut staging),
    )?;
    let staging_checksums = staging.finish(second.rows, cancellation, effects)?;
    if second != first {
        return Err(Error::Input {
            message: "input passes disagree",
            byte_offset: second.input_bytes,
        });
    }

    let private_unit = joined_path(&joined_path(root, PRIVATE_NAME)?, PRIVATE_UNIT_NAME)?;
    effects.before(Effect::Load(LoadEffect::CreatePrivateUnit))?;
    let mut unit = filesystem::create_new_read_write(&private_unit)
        .map_err(|source| io_error("create private unit", source))?;
    copy_buffer[..PAYLOAD_OFFSET_BYTES].fill(0);
    write_nonempty(
        &mut unit,
        &copy_buffer[..PAYLOAD_OFFSET_BYTES],
        Effect::Load(LoadEffect::WriteUnitPrefix),
        effects,
    )?;

    // The input pass is complete. Its buffer and the reserved descriptor region
    // now own encoded metadata; neither needs another stack or heap allocation.
    let header: &mut [u8; storage_format::HEADER_BYTES] = (&mut input_buffer
        [..storage_format::HEADER_BYTES])
        .try_into()
        .expect("header fits input workspace");
    let descriptor_bytes: &mut [u8; storage_format::DESCRIPTOR_BYTES] = descriptor_buffer
        .try_into()
        .expect("exact descriptor workspace");
    let metadata_crc32c = assemble_unit(
        context,
        &mut unit,
        staging_checksums,
        header,
        descriptor_bytes,
        copy_buffer,
        effects,
    )?;
    verify_unit(
        &unit,
        (header, descriptor_bytes),
        metadata_crc32c,
        layout.unit_bytes,
        copy_buffer,
        cancellation,
        effects,
    )?;
    cancellation.check()?;
    effects.before(Effect::Load(LoadEffect::SyncPrivateUnit))?;
    filesystem::sync_all(&unit).map_err(|source| io_error("sync private unit", source))?;
    effects.after(Effect::Load(LoadEffect::SyncPrivateUnit));
    drop(unit);
    for column in COLUMNS {
        cancellation.check()?;
        effects.before(Effect::Load(LoadEffect::RemoveStaging))?;
        filesystem::remove_file(joined_path(&joined_path(root, PRIVATE_NAME)?, column.name)?)
            .map_err(|source| io_error("remove staging file", source))?;
    }
    cancellation.check()?;
    sync_directory(
        &joined_path(root, PRIVATE_NAME)?,
        effects,
        DirectoryKind::Private,
    )?;
    cancellation.check()?;
    source.check_after_build(second.input_bytes, effects)?;
    Ok(BuiltUnit {
        rows: second.rows,
        bytes: layout.unit_bytes,
        metadata_crc32c,
    })
}

// Assembly's typed descriptors die before independent readback. Only encoded
// metadata in the charged arena crosses this phase boundary.
fn assemble_unit(
    context: &LoadContext<'_>,
    unit: &mut File,
    staging_checksums: [u32; 7],
    header: &mut [u8; storage_format::HEADER_BYTES],
    descriptor_bytes: &mut [u8; storage_format::DESCRIPTOR_BYTES],
    copy_buffer: &mut [u8],
    effects: &mut Effects,
) -> Result<u32, Error> {
    let root = context.root;
    let database_id = context.database_id;
    let second = context.first;
    let layout = context.layout;
    let cancellation = context.cancellation;
    let mut descriptors = [BlockDescriptor::EMPTY; storage_format::DESCRIPTOR_CAPACITY];
    let mut descriptor_count = 0_usize;
    let mut offset = storage_format::PAYLOAD_OFFSET;
    for (column_index, column) in COLUMNS.into_iter().enumerate() {
        cancellation.check()?;
        effects.before(Effect::Load(LoadEffect::OpenStaging))?;
        let mut stage =
            filesystem::open_read(joined_path(&joined_path(root, PRIVATE_NAME)?, column.name)?)
                .map_err(|source| io_error("open staging file", source))?;
        let expected = second
            .rows
            .checked_mul(column.width)
            .ok_or(Error::Resource {
                owner: "staging extent",
                required: u64::MAX,
                limit: u64::MAX,
            })?;
        let mut copied = 0_u64;
        let mut staging_checksum = storage_format::Crc32c::new();
        while copied < expected {
            cancellation.check()?;
            let remaining = expected - copied;
            let bytes = usize::try_from(remaining.min(column.block_bytes as u64))
                .map_err(|_| Error::Corrupt("staging block length does not fit"))?;
            read_exact_effect(
                &mut stage,
                &mut copy_buffer[..bytes],
                Effect::Load(LoadEffect::ReadStaging),
                effects,
            )?;
            staging_checksum.update(&copy_buffer[..bytes]);
            write_nonempty(
                unit,
                &copy_buffer[..bytes],
                Effect::Load(LoadEffect::WriteUnitPayload),
                effects,
            )?;
            let bytes_u32 = u32::try_from(bytes)
                .map_err(|_| Error::Corrupt("unit block length does not fit"))?;
            descriptors[descriptor_count] = BlockDescriptor {
                offset,
                bytes: bytes_u32,
                crc32c: storage_format::crc32c(&copy_buffer[..bytes]),
            };
            descriptor_count += 1;
            let bytes_u64 = u64::try_from(bytes)
                .map_err(|_| Error::Corrupt("unit block length does not fit u64"))?;
            offset = offset
                .checked_add(bytes_u64)
                .ok_or(Error::Corrupt("unit extent overflow"))?;
            copied = copied
                .checked_add(bytes_u64)
                .ok_or(Error::Corrupt("staging copy extent overflow"))?;
        }
        if staging_checksum.finish() != staging_checksums[column_index] {
            return Err(Error::Corrupt("staging checksum mismatch"));
        }
    }
    if descriptor_count
        != usize::try_from(layout.descriptor_count)
            .map_err(|_| Error::Corrupt("descriptor count does not fit usize"))?
        || offset != layout.unit_bytes
    {
        return Err(Error::Corrupt(
            "assembled unit geometry disagrees with layout",
        ));
    }
    let metadata_crc32c = storage_format::encode_unit_metadata_into(
        database_id,
        second.rows,
        second.projected_crc32c,
        &descriptors,
        header,
        descriptor_bytes,
    )
    .map_err(|_| Error::Corrupt("unit metadata encoding failed"))?;
    write_all_at(
        unit,
        descriptor_bytes,
        u64::try_from(storage_format::HEADER_BYTES).expect("header bytes fit u64"),
        Effect::Load(LoadEffect::WriteUnitDescriptors),
        effects,
    )?;
    write_all_at(
        unit,
        header,
        0,
        Effect::Load(LoadEffect::WriteUnitHeader),
        effects,
    )?;
    effects.before(Effect::Load(LoadEffect::InspectPrivateUnit))?;
    if filesystem::file_metadata(unit)
        .map_err(|source| io_error("inspect private unit", source))?
        .len()
        != layout.unit_bytes
    {
        return Err(Error::Corrupt("private unit length mismatch"));
    }
    Ok(metadata_crc32c)
}

fn verify_unit(
    file: &File,
    expected: (
        &[u8; storage_format::HEADER_BYTES],
        &[u8; storage_format::DESCRIPTOR_BYTES],
    ),
    metadata_crc32c: u32,
    unit_bytes: u64,
    buffer: &mut [u8],
    cancellation: &CancellationToken,
    effects: &mut Effects,
) -> Result<(), Error> {
    let (header, rest) = buffer.split_at_mut(storage_format::HEADER_BYTES);
    let descriptors = &mut rest[..storage_format::DESCRIPTOR_BYTES];
    cancellation.check()?;
    read_exact_at(
        file,
        header,
        0,
        Effect::Load(LoadEffect::ReadPrivateUnitMetadata),
        effects,
    )?;
    cancellation.check()?;
    read_exact_at(
        file,
        descriptors,
        u64::try_from(storage_format::HEADER_BYTES).expect("header bytes fit u64"),
        Effect::Load(LoadEffect::ReadPrivateUnitMetadata),
        effects,
    )?;
    let mut decoded_slot = std::mem::MaybeUninit::uninit();
    let (decoded, checksum) =
        storage_format::decode_unit_metadata_into(header, descriptors, &mut decoded_slot)
            .map_err(|_| Error::Corrupt("private unit metadata verification failed"))?;
    if header != expected.0 || descriptors != expected.1 || checksum != metadata_crc32c {
        return Err(Error::Corrupt(
            "private unit metadata changed after encoding",
        ));
    }
    // Readers admit the entire metadata prefix, including the reserved padding.
    // Construction must establish that same invariant before publishing the unit.
    let padding = &mut buffer[..storage_format::UNIT_PADDING_BYTES];
    cancellation.check()?;
    read_exact_at(
        file,
        padding,
        u64::try_from(storage_format::HEADER_BYTES + storage_format::DESCRIPTOR_BYTES)
            .expect("metadata prefix fits u64"),
        Effect::Load(LoadEffect::ReadPrivateUnitMetadata),
        effects,
    )?;
    if padding.iter().any(|byte| *byte != 0) {
        return Err(Error::Corrupt("private unit padding is nonzero"));
    }
    for descriptor in decoded.descriptors[..decoded.descriptor_count()].iter() {
        cancellation.check()?;
        let bytes = usize::try_from(descriptor.bytes)
            .map_err(|_| Error::Corrupt("descriptor bytes do not fit usize"))?;
        read_exact_at(
            file,
            &mut buffer[..bytes],
            descriptor.offset,
            Effect::Load(LoadEffect::ReadPrivateUnitPayload),
            effects,
        )?;
        if storage_format::crc32c(&buffer[..bytes]) != descriptor.crc32c {
            return Err(Error::Corrupt("private unit payload verification failed"));
        }
    }
    effects.before(Effect::Load(LoadEffect::InspectPrivateUnit))?;
    if filesystem::file_metadata(file)
        .map_err(|source| io_error("inspect verified private unit", source))?
        .len()
        != unit_bytes
    {
        return Err(Error::Corrupt(
            "private unit length changed during verification",
        ));
    }
    Ok(())
}

fn publish_unit(
    root: &Path,
    database_id: storage_format::DatabaseId,
    transaction: TransactionId,
    unit: BuiltUnit,
    cancellation: &CancellationToken,
    effects: &mut Effects,
) -> Result<(), PublicationFailure> {
    cancellation
        .check()
        .map_err(PublicationFailure::before_publication)?;
    effects
        .before(Effect::Load(LoadEffect::LinkUnit))
        .map_err(PublicationFailure::before_publication)?;
    let private =
        joined_path(root, PRIVATE_NAME).map_err(PublicationFailure::before_publication)?;
    let units = joined_path(root, UNITS_NAME).map_err(PublicationFailure::before_publication)?;
    filesystem::hard_link(
        joined_path(&private, PRIVATE_UNIT_NAME).map_err(PublicationFailure::before_publication)?,
        joined_path(&units, UNIT_NAME).map_err(PublicationFailure::before_publication)?,
    )
    .map_err(|source| {
        PublicationFailure::before_publication(io_error("link published unit", source))
    })?;
    effects.after(Effect::Load(LoadEffect::LinkUnit));
    cancellation
        .check()
        .map_err(PublicationFailure::before_publication)?;
    sync_directory_cancellable(&units, DirectoryKind::Units, cancellation, effects)
        .map_err(PublicationFailure::before_publication)?;
    effects.after(Effect::SyncDirectory(DirectoryKind::Units));
    cancellation
        .check()
        .map_err(PublicationFailure::before_publication)?;
    effects
        .before(Effect::Load(LoadEffect::RemovePrivateUnit))
        .map_err(PublicationFailure::before_publication)?;
    filesystem::remove_file(
        joined_path(&private, PRIVATE_UNIT_NAME).map_err(PublicationFailure::before_publication)?,
    )
    .map_err(|source| {
        PublicationFailure::before_publication(io_error("remove private unit link", source))
    })?;
    effects.after(Effect::Load(LoadEffect::RemovePrivateUnit));
    cancellation
        .check()
        .map_err(PublicationFailure::before_publication)?;
    sync_directory_cancellable(&private, DirectoryKind::Private, cancellation, effects)
        .map_err(PublicationFailure::before_publication)?;
    effects.after(Effect::SyncDirectory(DirectoryKind::Private));

    drop(private);
    drop(units);
    let record = WalRecord {
        database: database_id,
        issued: transaction.sequence(),
        state: RootState::Data {
            transaction,
            rows: unit.rows,
            unit_bytes: unit.bytes,
            unit_metadata_crc32c: unit.metadata_crc32c,
        },
    };
    let prior = WalRecord {
        database: database_id,
        issued: transaction.sequence(),
        state: RootState::Empty,
    };
    publish_snapshot(root, prior, record, cancellation, effects)
}

fn read_exact_effect(
    file: &mut File,
    bytes: &mut [u8],
    effect: Effect,
    effects: &mut Effects,
) -> Result<(), Error> {
    let short = effects.before(effect)?;
    let length = if short {
        bytes
            .len()
            .checked_sub(1)
            .expect("fixed staging reads are nonempty")
    } else {
        bytes.len()
    };
    crate::file_io::read_exact(file, &mut bytes[..length])
        .map_err(|source| io_error(effect.name(), source))?;
    if short {
        return Err(io_error(
            effect.name(),
            io::Error::from(io::ErrorKind::UnexpectedEof),
        ));
    }
    Ok(())
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;
