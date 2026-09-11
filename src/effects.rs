//! Named filesystem effects and deterministic fault injection.
//!
//! Callers place `before` at the attempted effect and `after` at an explicit
//! completion point. The counter includes refused attempts. Test callbacks may
//! observe, cancel, or interrupt those points; the caller still owns recovery.
//! These schedules supplement native failure tests, not replace OS behavior.

use crate::error::io_error;
use crate::{Error, file_io};
use std::fs::File;
use std::io;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DirectoryKind {
    Database,
    Parent,
    Units,
    Private,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MetadataKind {
    CatalogObject,
    Control,
    RootA,
    RootB,
    Wal,
    IssuedWal,
    IssuedRootA,
    IssuedRootB,
    RootRepair,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QueryEffect {
    InspectUnit,
    OpenUnit,
    InspectOpenUnit,
    ReadHeader,
    ReadDescriptors,
    ReadPadding,
    ReadPayload,
    ResetScratch,
}

impl QueryEffect {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::InspectUnit => "inspect query unit",
            Self::OpenUnit => "open query unit",
            Self::InspectOpenUnit => "inspect open query unit",
            Self::ReadHeader => "read query unit header",
            Self::ReadDescriptors => "read query unit descriptors",
            Self::ReadPadding => "read query unit padding",
            Self::ReadPayload => "read demanded unit payload",
            Self::ResetScratch => "reset query scratch file",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LoadEffect {
    InspectInput,
    CanonicalizeInput,
    OpenInput,
    InspectOpenInput,
    ReadInput,
    InspectInputFinal,
    CreateStaging,
    WriteStaging,
    InspectStaging,
    OpenStaging,
    ReadStaging,
    RemoveStaging,
    CreatePrivateUnit,
    WriteUnitPrefix,
    WriteUnitPayload,
    WriteUnitDescriptors,
    WriteUnitHeader,
    InspectPrivateUnit,
    ReadPrivateUnitMetadata,
    ReadPrivateUnitPayload,
    SyncPrivateUnit,
    LinkUnit,
    RemovePrivateUnit,
    OpenWalPublication,
    WriteWal,
    WriteIssuedWal,
    RenameIssuedRootA,
    RenameIssuedRootB,
    InspectWalPublication,
    ReadWalPublication,
    RenameRootA,
    RenameRootB,
}

impl LoadEffect {
    fn name(self) -> &'static str {
        match self {
            Self::InspectInput => "inspect load input",
            Self::CanonicalizeInput => "canonicalize load input",
            Self::OpenInput => "open load input",
            Self::InspectOpenInput => "inspect open load input",
            Self::ReadInput => "read load input",
            Self::InspectInputFinal => "inspect load input after scan",
            Self::CreateStaging => "create staging file",
            Self::WriteStaging => "write staging file",
            Self::InspectStaging => "inspect staging file",
            Self::OpenStaging => "open staging file",
            Self::ReadStaging => "read staging file",
            Self::RemoveStaging => "remove staging file",
            Self::CreatePrivateUnit => "create private unit",
            Self::WriteUnitPrefix => "write unit metadata prefix",
            Self::WriteUnitPayload => "write unit payload",
            Self::WriteUnitDescriptors => "write unit descriptors",
            Self::WriteUnitHeader => "write unit header",
            Self::InspectPrivateUnit => "inspect private unit",
            Self::ReadPrivateUnitMetadata => "read private unit metadata",
            Self::ReadPrivateUnitPayload => "read private unit payload",
            Self::SyncPrivateUnit => "sync private unit",
            Self::LinkUnit => "link published unit",
            Self::RemovePrivateUnit => "remove private unit link",
            Self::OpenWalPublication => "open WAL for publication",
            Self::WriteWal => "write WAL",
            Self::WriteIssuedWal => "write issuance fence",
            Self::RenameIssuedRootA => "rename issued ROOT.A",
            Self::RenameIssuedRootB => "rename issued ROOT.B",
            Self::InspectWalPublication => "inspect WAL after publication write",
            Self::ReadWalPublication => "read WAL after publication write",
            Self::RenameRootA => "rename ROOT.A publication",
            Self::RenameRootB => "rename ROOT.B publication",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Effect {
    UnlockDatabase,
    CanonicalizeParent,
    CanonicalizeDatabase,
    OpenRandom,
    ReadIdentity,
    CreateDatabaseDirectory,
    CreateLock,
    OpenLock,
    InspectOpenLock,
    LockDatabase,
    SyncLock,
    CreateUnitsDirectory,
    CreatePrivateDirectory,
    OpenDirectory(DirectoryKind),
    SyncDirectory(DirectoryKind),
    CreateMetadata(MetadataKind),
    WriteMetadata(MetadataKind),
    SyncMetadata(MetadataKind),
    InspectDatabaseDirectory,
    ListDatabaseDirectory,
    ReadDatabaseEntry,
    InspectNamespaceEntry,
    OpenMetadata(MetadataKind),
    InspectOpenMetadata(MetadataKind),
    ReadMetadata(MetadataKind),
    InspectMetadata(MetadataKind),
    InspectSubdirectory,
    ListSubdirectory,
    ReadSubdirectoryEntry,
    InspectUnitEntry,
    OpenUnit,
    InspectOpenUnit,
    ReadUnitHeader,
    ReadUnitDescriptors,
    ReadUnitPadding,
    InspectCreatedLock,
    RemoveStaleRootNext,
    RemoveRecoveryFile,
    OpenWalForRecovery,
    InspectWalForRecovery,
    RenameRepairedRoot,
    RemoveCleanupFile,
    RemoveCleanupDirectory,
    Query(QueryEffect),
    Load(LoadEffect),
}

impl Effect {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::CreateMetadata(MetadataKind::CatalogObject) => "create catalog object",
            Self::WriteMetadata(MetadataKind::CatalogObject) => "write catalog object",
            Self::SyncMetadata(MetadataKind::CatalogObject) => "sync catalog object",
            Self::OpenMetadata(MetadataKind::CatalogObject) => "open catalog object",
            Self::InspectOpenMetadata(MetadataKind::CatalogObject) => {
                "inspect opened catalog object"
            }
            Self::ReadMetadata(MetadataKind::CatalogObject) => "read catalog object",
            Self::InspectMetadata(MetadataKind::CatalogObject) => "inspect catalog object",
            Self::UnlockDatabase => "unlock database",
            Self::CanonicalizeParent => "canonicalize database parent",
            Self::CanonicalizeDatabase => "canonicalize database path",
            Self::OpenRandom => "open random identity source",
            Self::ReadIdentity => "read database identity",
            Self::CreateDatabaseDirectory => "create database directory",
            Self::CreateLock => "create database lock",
            Self::OpenLock => "open database lock",
            Self::InspectOpenLock => "inspect open database lock",
            Self::LockDatabase => "lock database",
            Self::SyncLock => "sync database lock",
            Self::CreateUnitsDirectory => "create units directory",
            Self::CreatePrivateDirectory => "create private directory",
            Self::OpenDirectory(DirectoryKind::Database) => "open database directory",
            Self::OpenDirectory(DirectoryKind::Parent) => "open database parent",
            Self::OpenDirectory(DirectoryKind::Units) => "open units directory",
            Self::OpenDirectory(DirectoryKind::Private) => "open private directory",
            Self::SyncDirectory(DirectoryKind::Database) => "sync database directory",
            Self::SyncDirectory(DirectoryKind::Parent) => "sync database parent",
            Self::SyncDirectory(DirectoryKind::Units) => "sync units directory",
            Self::SyncDirectory(DirectoryKind::Private) => "sync private directory",
            Self::CreateMetadata(
                MetadataKind::IssuedRootA | MetadataKind::IssuedRootB | MetadataKind::IssuedWal,
            ) => "create issuance metadata",
            Self::WriteMetadata(
                MetadataKind::IssuedRootA | MetadataKind::IssuedRootB | MetadataKind::IssuedWal,
            ) => "write issuance metadata",
            Self::SyncMetadata(
                MetadataKind::IssuedRootA | MetadataKind::IssuedRootB | MetadataKind::IssuedWal,
            ) => "sync issuance metadata",
            Self::OpenMetadata(
                MetadataKind::IssuedRootA | MetadataKind::IssuedRootB | MetadataKind::IssuedWal,
            ) => "open issuance metadata",
            Self::InspectOpenMetadata(
                MetadataKind::IssuedRootA | MetadataKind::IssuedRootB | MetadataKind::IssuedWal,
            ) => "inspect opened issuance metadata",
            Self::ReadMetadata(
                MetadataKind::IssuedRootA | MetadataKind::IssuedRootB | MetadataKind::IssuedWal,
            ) => "read issuance metadata",
            Self::InspectMetadata(
                MetadataKind::IssuedRootA | MetadataKind::IssuedRootB | MetadataKind::IssuedWal,
            ) => "inspect issuance metadata",
            Self::CreateMetadata(MetadataKind::Control) => "create CONTROL",
            Self::CreateMetadata(MetadataKind::RootA) => "create ROOT.A",
            Self::CreateMetadata(MetadataKind::RootB) => "create ROOT.B",
            Self::CreateMetadata(MetadataKind::Wal) => "create WAL",
            Self::CreateMetadata(MetadataKind::RootRepair) => "create root repair",
            Self::WriteMetadata(MetadataKind::Control) => "write CONTROL",
            Self::WriteMetadata(MetadataKind::RootA) => "write ROOT.A",
            Self::WriteMetadata(MetadataKind::RootB) => "write ROOT.B",
            Self::WriteMetadata(MetadataKind::Wal) => "write WAL",
            Self::WriteMetadata(MetadataKind::RootRepair) => "write root repair",
            Self::SyncMetadata(MetadataKind::Control) => "sync CONTROL",
            Self::SyncMetadata(MetadataKind::RootA) => "sync ROOT.A",
            Self::SyncMetadata(MetadataKind::RootB) => "sync ROOT.B",
            Self::SyncMetadata(MetadataKind::Wal) => "sync WAL",
            Self::SyncMetadata(MetadataKind::RootRepair) => "sync root repair",
            Self::InspectDatabaseDirectory => "inspect database directory",
            Self::ListDatabaseDirectory => "list database directory",
            Self::ReadDatabaseEntry => "read database directory entry",
            Self::InspectNamespaceEntry => "inspect namespace entry",
            Self::OpenMetadata(MetadataKind::Control) => "open CONTROL",
            Self::OpenMetadata(MetadataKind::RootA) => "open ROOT.A",
            Self::OpenMetadata(MetadataKind::RootB) => "open ROOT.B",
            Self::OpenMetadata(MetadataKind::Wal) => "open WAL",
            Self::OpenMetadata(MetadataKind::RootRepair) => "open root repair",
            Self::InspectOpenMetadata(MetadataKind::Control) => "inspect open CONTROL",
            Self::InspectOpenMetadata(MetadataKind::RootA) => "inspect open ROOT.A",
            Self::InspectOpenMetadata(MetadataKind::RootB) => "inspect open ROOT.B",
            Self::InspectOpenMetadata(MetadataKind::Wal) => "inspect open WAL",
            Self::InspectOpenMetadata(MetadataKind::RootRepair) => "inspect open root repair",
            Self::ReadMetadata(MetadataKind::Control) => "read CONTROL",
            Self::ReadMetadata(MetadataKind::RootA) => "read ROOT.A",
            Self::ReadMetadata(MetadataKind::RootB) => "read ROOT.B",
            Self::ReadMetadata(MetadataKind::Wal) => "read WAL",
            Self::ReadMetadata(MetadataKind::RootRepair) => "read root repair",
            Self::InspectMetadata(MetadataKind::Control) => "inspect CONTROL",
            Self::InspectMetadata(MetadataKind::RootA) => "inspect ROOT.A",
            Self::InspectMetadata(MetadataKind::RootB) => "inspect ROOT.B",
            Self::InspectMetadata(MetadataKind::Wal) => "inspect WAL",
            Self::InspectMetadata(MetadataKind::RootRepair) => "inspect root repair",
            Self::InspectSubdirectory => "inspect database subdirectory",
            Self::ListSubdirectory => "list database subdirectory",
            Self::ReadSubdirectoryEntry => "read database subdirectory entry",
            Self::InspectUnitEntry => "inspect unit entry",
            Self::OpenUnit => "open unit",
            Self::InspectOpenUnit => "inspect open unit",
            Self::ReadUnitHeader => "read unit header",
            Self::ReadUnitDescriptors => "read unit descriptors",
            Self::ReadUnitPadding => "read unit metadata padding",
            Self::InspectCreatedLock => "inspect created database lock",
            Self::RemoveStaleRootNext => "remove stale root next",
            Self::RemoveRecoveryFile => "remove uncommitted recovery file",
            Self::OpenWalForRecovery => "open WAL for recovery",
            Self::InspectWalForRecovery => "inspect WAL for recovery",
            Self::RenameRepairedRoot => "rename repaired root",
            Self::RemoveCleanupFile => "remove private namespace file",
            Self::RemoveCleanupDirectory => "remove private namespace directory",
            Self::Query(effect) => effect.name(),
            Self::Load(effect) => effect.name(),
        }
    }
}

#[derive(Default)]
pub(crate) struct Effects {
    fail_at: Option<u64>,
    second_fail_at: Option<u64>,
    short_at: Option<u64>,
    count: u64,
    #[cfg(test)]
    action: Option<Box<dyn FnMut(u64, Effect)>>,
    #[cfg(test)]
    after_action: Option<Box<dyn FnMut(u64, Effect)>>,
}

/// An operation-local schedule. Indices count attempted effects, starting at zero.
/// Callbacks run before counting an attempt or after an explicit completion.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Faults {
    pub(crate) fail_at: Option<u64>,
    pub(crate) second_fail_at: Option<u64>,
    pub(crate) short_at: Option<u64>,
    pub(crate) action: Option<Box<dyn FnMut(u64, Effect)>>,
    pub(crate) after_action: Option<Box<dyn FnMut(u64, Effect)>>,
}

impl Effects {
    #[cfg(test)]
    pub(crate) fn with_faults(faults: Faults) -> Self {
        Self {
            fail_at: faults.fail_at,
            second_fail_at: faults.second_fail_at,
            short_at: faults.short_at,
            count: 0,
            action: faults.action,
            after_action: faults.after_action,
        }
    }

    #[cfg(test)]
    pub(crate) fn count(&self) -> u64 {
        self.count
    }

    /// Schedule a refusal without resetting the count of earlier attempts.
    #[cfg(test)]
    pub(crate) fn fail_at(&mut self, index: u64) {
        self.fail_at = Some(index);
    }

    /// Count an attempted effect or return its injected refusal. A true result
    /// requests short I/O from a byte-transfer caller; it does not perform I/O.
    pub(crate) fn before_io(&mut self, effect: Effect) -> Result<bool, io::Error> {
        let index = self.count;

        #[cfg(test)]
        if let Some(action) = self.action.as_mut() {
            action(index, effect);
        }
        self.count = self
            .count
            .checked_add(1)
            .ok_or_else(|| io::Error::other("filesystem effect count overflow"))?;
        if self.fail_at == Some(index) || self.second_fail_at == Some(index) {
            return Err(io::Error::other(format!(
                "injected {} failure at effect {index}",
                effect.name()
            )));
        }
        Ok(self.short_at == Some(index))
    }

    pub(crate) fn before(&mut self, effect: Effect) -> Result<bool, Error> {
        self.before_io(effect)
            .map_err(|source| io_error(effect.name(), source))
    }

    /// Notify the observer at a caller-owned completion point. No new attempt
    /// is counted, and no completion is inferred after a failed operation.
    pub(crate) fn after(&mut self, effect: Effect) {
        #[cfg(test)]
        if let Some(action) = self.after_action.as_mut() {
            let index = self
                .count
                .checked_sub(1)
                .expect("effect preceded completion");
            action(index, effect);
        }

        #[cfg(not(test))]
        let _ = effect;
    }

    pub(crate) fn write(
        &mut self,
        file: &mut File,
        bytes: &[u8],
        effect: Effect,
    ) -> Result<(), Error> {
        let operation = effect.name();
        let short = self.before(effect)?;
        if short {
            let length = bytes.len().saturating_sub(1);
            file_io::write_all(file, &bytes[..length])
                .map_err(|source| io_error(operation, source))?;
            return Err(io_error(
                operation,
                io::Error::from(io::ErrorKind::WriteZero),
            ));
        }
        file_io::write_all(file, bytes).map_err(|source| io_error(operation, source))
    }

    pub(crate) fn read(
        &mut self,
        file: &mut File,
        bytes: &mut [u8],
        effect: Effect,
    ) -> Result<(), Error> {
        let operation = effect.name();
        let short = self.before(effect)?;
        if short {
            let length = bytes.len().saturating_sub(1);
            file_io::read_exact(file, &mut bytes[..length])
                .map_err(|source| io_error(operation, source))?;
            return Err(io_error(
                operation,
                io::Error::from(io::ErrorKind::UnexpectedEof),
            ));
        }
        file_io::read_exact(file, bytes).map_err(|source| {
            if source.kind() == io::ErrorKind::UnexpectedEof {
                Error::Corrupt("fixed-size metadata record was short")
            } else {
                io_error(operation, source)
            }
        })
    }
}

// Positional transfers share fault counting with metadata effects, while retaining
// I/O errors for their callers to interpret. Short transfers require nonempty input.
pub(crate) fn write_all_at(
    file: &File,
    bytes: &[u8],
    offset: u64,
    effect: Effect,
    effects: &mut Effects,
) -> Result<(), Error> {
    let short = effects.before(effect)?;
    let length = if short {
        bytes
            .len()
            .checked_sub(1)
            .expect("fixed positional writes are nonempty")
    } else {
        bytes.len()
    };
    crate::file_io::write_all_at(file, &bytes[..length], offset)
        .map_err(|source| io_error(effect.name(), source))?;
    if short {
        return Err(io_error(
            effect.name(),
            io::Error::from(io::ErrorKind::WriteZero),
        ));
    }
    Ok(())
}

pub(crate) fn read_exact_at(
    file: &File,
    bytes: &mut [u8],
    offset: u64,
    effect: Effect,
    effects: &mut Effects,
) -> Result<(), Error> {
    let short = effects.before(effect)?;
    let length = if short {
        bytes
            .len()
            .checked_sub(1)
            .expect("fixed positional reads are nonempty")
    } else {
        bytes.len()
    };
    crate::file_io::read_exact_at(file, &mut bytes[..length], offset)
        .map_err(|source| io_error(effect.name(), source))?;
    if short {
        return Err(io_error(
            effect.name(),
            io::Error::from(io::ErrorKind::UnexpectedEof),
        ));
    }
    Ok(())
}

/// Write a nonempty construction record. Injected short writes preserve that
/// precondition; unlike general metadata writes, an empty input is an invariant
/// failure when shortened. The caller owns the record's extent and validation.
pub(crate) fn write_nonempty(
    file: &mut File,
    bytes: &[u8],
    effect: Effect,
    effects: &mut Effects,
) -> Result<(), Error> {
    let short = effects.before(effect)?;
    if short {
        let length = bytes
            .len()
            .checked_sub(1)
            .expect("load writes are nonempty");
        crate::file_io::write_all(file, &bytes[..length])
            .map_err(|source| io_error(effect.name(), source))?;
        return Err(io_error(
            effect.name(),
            io::Error::from(io::ErrorKind::WriteZero),
        ));
    }
    crate::file_io::write_all(file, bytes).map_err(|source| io_error(effect.name(), source))
}
