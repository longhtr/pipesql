//! The root/fence publisher shared by legacy loads and catalog transactions.
//!
//! Validate one legal old/new snapshot transition, persist its fence, then replace
//! both root replicas in order. The first replacement separates cancellable work
//! from a possibly published outcome. Callers still own issuance, rollback,
//! registry visibility, and transaction resolution.

use crate::effects::{
    DirectoryKind, Effect, Effects, LoadEffect, MetadataKind, read_exact_at, write_all_at,
    write_nonempty,
};
use crate::error::io_error;
use crate::namespace::{ROOT_A_NAME, ROOT_B_NAME, WAL_NAME, sync_directory, validate_fence_owner};
use crate::path::joined_path;
use crate::storage_format::{self, PublicationKind, Replica, Root, WalRecord};
use crate::{CancellationToken, Error};
use pipesql_filesystem as filesystem;
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailureStage {
    /// The first root replacement has not been attempted. Private changes may
    /// still require cleanup before the caller can report a definite abort.
    BeforePublication,
    /// Replacement may have changed the authoritative snapshot. The caller must
    /// preserve uncertainty even when the immediate error came from injection.
    Uncertain,
}

pub(crate) struct PublicationFailure {
    pub(crate) error: Error,
    pub(crate) stage: FailureStage,
}

impl PublicationFailure {
    pub(crate) fn before_publication(error: Error) -> Self {
        Self {
            error,
            stage: FailureStage::BeforePublication,
        }
    }

    pub(crate) fn uncertain(error: Error) -> Self {
        Self {
            error,
            stage: FailureStage::Uncertain,
        }
    }
}

pub(crate) fn publish_snapshot(
    root: &Path,
    prior: WalRecord,
    record: WalRecord,
    cancellation: &CancellationToken,
    effects: &mut Effects,
) -> Result<(), PublicationFailure> {
    // Operation identity comes from the validated transition, not whether the
    // existing data graph happens to be empty. Selection uses this same rule.
    let publication = record.transition_from(prior).ok_or_else(|| {
        PublicationFailure::before_publication(Error::Corrupt("invalid publication transition"))
    })?;
    let issuance = publication == PublicationKind::Issuance;
    let rename_a = if issuance {
        LoadEffect::RenameIssuedRootA
    } else {
        LoadEffect::RenameRootA
    };
    let rename_b = if issuance {
        LoadEffect::RenameIssuedRootB
    } else {
        LoadEffect::RenameRootB
    };
    write_fence(root, record, publication, cancellation, effects)
        .map_err(PublicationFailure::before_publication)?;

    write_root_next(
        root,
        record,
        publication,
        Replica::A,
        Some(cancellation),
        effects,
    )
    .map_err(PublicationFailure::before_publication)?;
    cancellation
        .check()
        .map_err(PublicationFailure::before_publication)?;

    // Admit both paths before entering replacement.
    let next_a =
        joined_path(root, "ROOT.A.next").map_err(PublicationFailure::before_publication)?;
    let root_a = joined_path(root, ROOT_A_NAME).map_err(PublicationFailure::before_publication)?;
    // Entering the first replacement call is the point of no return. Even an
    // injected or host rename error cannot prove that authority stayed old.
    effects
        .before(Effect::Load(rename_a))
        .map_err(PublicationFailure::uncertain)?;
    filesystem::rename(next_a, root_a).map_err(|source| {
        PublicationFailure::uncertain(io_error("rename ROOT.A publication", source))
    })?;
    effects.after(Effect::Load(rename_a));
    sync_directory(root, effects, DirectoryKind::Database)
        .map_err(PublicationFailure::uncertain)?;
    effects.after(Effect::SyncDirectory(DirectoryKind::Database));

    write_root_next(root, record, publication, Replica::B, None, effects)
        .map_err(PublicationFailure::uncertain)?;
    let next_b = joined_path(root, "ROOT.B.next").map_err(PublicationFailure::uncertain)?;
    let root_b = joined_path(root, ROOT_B_NAME).map_err(PublicationFailure::uncertain)?;
    effects
        .before(Effect::Load(rename_b))
        .map_err(PublicationFailure::uncertain)?;
    filesystem::rename(next_b, root_b).map_err(|source| {
        PublicationFailure::uncertain(io_error("rename ROOT.B publication", source))
    })?;
    effects.after(Effect::Load(rename_b));
    sync_directory(root, effects, DirectoryKind::Database)
        .map_err(PublicationFailure::uncertain)?;
    effects.after(Effect::SyncDirectory(DirectoryKind::Database));
    Ok(())
}

fn write_fence(
    root: &Path,
    record: WalRecord,
    publication: PublicationKind,
    cancellation: &CancellationToken,
    effects: &mut Effects,
) -> Result<(), Error> {
    let issuance = publication == PublicationKind::Issuance;
    let wal_kind = if issuance {
        MetadataKind::IssuedWal
    } else {
        MetadataKind::Wal
    };
    let write_wal = if issuance {
        LoadEffect::WriteIssuedWal
    } else {
        LoadEffect::WriteWal
    };
    let wal_bytes =
        storage_format::encode_wal(record).map_err(|_| Error::Corrupt("WAL encoding failed"))?;
    cancellation.check()?;
    effects.before(Effect::Load(LoadEffect::OpenWalPublication))?;
    let wal = filesystem::open_read_write(joined_path(root, WAL_NAME)?)
        .map_err(|source| io_error("open WAL for publication", source))?;
    cancellation.check()?;
    effects.before(Effect::Load(LoadEffect::InspectWalPublication))?;
    let metadata = filesystem::file_metadata(&wal)
        .map_err(|source| io_error("inspect WAL before write", source))?;
    validate_fence_owner(metadata.file_type().is_file(), metadata.nlink())?;
    if metadata.len() != storage_format::WAL_BYTES as u64 {
        return Err(Error::Corrupt(
            "WAL fence has wrong extent before publication",
        ));
    }
    cancellation.check()?;
    write_all_at(&wal, &wal_bytes, 0, Effect::Load(write_wal), effects)?;
    cancellation.check()?;
    effects.before(Effect::Load(LoadEffect::InspectWalPublication))?;
    if filesystem::file_metadata(&wal)
        .map_err(|source| io_error("inspect WAL after write", source))?
        .len()
        != u64::try_from(storage_format::WAL_BYTES).expect("WAL bytes fit u64")
    {
        return Err(Error::Corrupt("WAL length after write is invalid"));
    }
    let mut observed_wal = [0_u8; storage_format::WAL_BYTES];
    cancellation.check()?;
    read_exact_at(
        &wal,
        &mut observed_wal,
        0,
        Effect::Load(LoadEffect::ReadWalPublication),
        effects,
    )?;
    if storage_format::decode_wal(&observed_wal) != Ok(record) {
        return Err(Error::Corrupt("WAL verification failed"));
    }
    cancellation.check()?;
    effects.before(Effect::SyncMetadata(wal_kind))?;
    filesystem::sync_all(&wal).map_err(|source| io_error("sync WAL", source))?;
    effects.after(Effect::SyncMetadata(wal_kind));
    drop(wal);

    Ok(())
}

fn write_root_next(
    root: &Path,
    record: WalRecord,
    publication: PublicationKind,
    replica: Replica,
    cancellation: Option<&CancellationToken>,
    effects: &mut Effects,
) -> Result<(), Error> {
    let (next_name, kind) = match replica {
        Replica::A => ("ROOT.A.next", MetadataKind::RootA),
        Replica::B => ("ROOT.B.next", MetadataKind::RootB),
    };
    let kind = if publication == PublicationKind::Issuance {
        match replica {
            Replica::A => MetadataKind::IssuedRootA,
            Replica::B => MetadataKind::IssuedRootB,
        }
    } else {
        kind
    };
    let bytes = storage_format::encode_root(Root {
        database: record.database,
        issued: record.issued,
        replica,
        state: record.state,
    })
    .map_err(|_| Error::Corrupt("root encoding failed"))?;
    let path = joined_path(root, next_name)?;
    let create = Effect::CreateMetadata(kind);
    check_optional_cancel(cancellation)?;
    effects.before(create)?;
    let mut file = filesystem::create_new_read_write(&path)
        .map_err(|source| io_error(create.name(), source))?;
    check_optional_cancel(cancellation)?;
    write_nonempty(&mut file, &bytes, Effect::WriteMetadata(kind), effects)?;
    check_optional_cancel(cancellation)?;
    effects.before(Effect::InspectOpenMetadata(kind))?;
    if filesystem::file_metadata(&file)
        .map_err(|source| io_error("inspect root next", source))?
        .len()
        != u64::try_from(storage_format::ROOT_BYTES).expect("root bytes fit u64")
    {
        return Err(Error::Corrupt("root next length is invalid"));
    }
    let mut observed = [0_u8; storage_format::ROOT_BYTES];
    check_optional_cancel(cancellation)?;
    read_exact_at(&file, &mut observed, 0, Effect::ReadMetadata(kind), effects)?;
    if storage_format::decode_root(&observed)
        != Ok(Root {
            database: record.database,
            issued: record.issued,
            replica,
            state: record.state,
        })
    {
        return Err(Error::Corrupt("root next verification failed"));
    }
    check_optional_cancel(cancellation)?;
    effects.before(Effect::SyncMetadata(kind))?;
    filesystem::sync_all(&file).map_err(|source| io_error("sync root next", source))?;
    effects.after(Effect::SyncMetadata(kind));
    Ok(())
}

fn check_optional_cancel(cancellation: Option<&CancellationToken>) -> Result<(), Error> {
    match cancellation {
        Some(token) => token.check(),
        None => Ok(()),
    }
}
