//! Write batches into new files, then publish them together as one append.
//!
//! An append retains the table's schema and existing unit references. Each
//! `write` borrows the caller's columns and writes one immutable unit;
//! it retains a file reference, not a copy of those input rows. Existing files
//! stay unchanged, and new rows become visible only when the append commits.
//!
//! The `admission` child selects the table, reserves reference capacity and
//! construction space, and issues a transaction number. Batch writes allocate
//! encoding workspace as needed within the database budget. A failed write
//! disables further writes and commit, so the caller cannot accidentally publish
//! an earlier successful prefix.
//!
//! `build_commit` writes an index for old and new units, a replacement catalog,
//! and updated transaction history. `commit` then frees construction buffers
//! and passes the synchronized files to `Writer::commit_prepared` for validation
//! and publication. Before that handoff, abort removes this attempt's files.
//! After it, publication classifies the outcome and recovery handles cleanup.

use super::construction::{cleanup, create_object, object, sync_object};
use super::snapshot::Writer;
use crate::ColumnInput;
use crate::effects::{DirectoryKind, Effect, Effects, write_nonempty};
use crate::path::joined_path;
use crate::resources::Reservation;
use crate::storage::catalog::{self, TableEntry};
use crate::storage::format::{CatalogCommit, RootState};
use crate::storage::history;
use crate::storage::recovery::UNITS_NAME;
use crate::storage::schema::{self, TableId};
use crate::storage::table;
use crate::storage::unit::{self, InputColumn, UnitRef, WriteBuffers};
use crate::{CancellationToken, Commit, Error, ErrorCause, TransactionId};

// Native ownership checks cover every workspace size (65,536..=526,400)
// and reference count (1..=4,096). Darwin's largest observed rounding is
// 16,383 bytes; this per-allocation ceiling also covers the tested GNU allocator.
// It is a qualified allocator premise, not a bound for arbitrary GlobalAllocs.
const ALLOCATION_ROUNDING_BYTES: usize = 16_384;

/// Positive bounds reserved before an append issues its transaction.
#[derive(Clone, Copy)]
pub struct AppendLimits {
    /// Maximum number of batches accepted by this append.
    pub batches: u32,
    /// Maximum total encoded unit bytes, including unit headers and validity.
    /// The database separately reserves index, catalog, history, and root space.
    pub encoded_bytes: u64,
}

enum Columns<'a> {
    #[cfg(test)]
    Identified(&'a [InputColumn<'a>]),
    Positional(&'a [ColumnInput<'a>]),
}

/// A write in progress, limited to the batches and bytes reserved at its start.
/// Commit or abort it explicitly. Dropping it unfinished leaves work for recovery,
/// so the database must be reopened before further use.
pub struct Append<'db> {
    // Field order frees buffers before releasing their reservations. An unfinished
    // Writer disables admission on drop; recovery must settle its files.
    workspace: Option<catalog::Scratch<'db>>,
    admitted: catalog::Scratch<'db>,
    units: Vec<UnitRef>,
    _references: Reservation<'db>,
    writer: Writer<'db>,
    _owner: Reservation<'db>,
    ordinal: usize,
    table: TableId,
    schema_len: usize,
    schema_crc: u32,
    old_units: usize,
    rows: u64,
    limits: AppendLimits,
    encoded_bytes: u64,
    created: u32,
    failed: bool,
}

impl Append<'_> {
    /// The identifier for this write attempt. Save it before committing so you
    /// can check the outcome later if the commit returns an error.
    pub fn transaction(&self) -> TransactionId {
        let issued = self.writer.issued.expect("append owns an issued writer");
        TransactionId::for_attempt(issued.database, issued.issued).expect("issued identity")
    }

    /// Add one batch of rows. Supply columns in the table's declaration order.
    ///
    /// Every column must have the same positive row count and match its declared
    /// type and the NULL rules in [`ColumnInput`]. Inputs are borrowed only during
    /// this call. The rows remain invisible to readers until [`Self::commit`].
    /// A batch has at most 32,768 rows; each encoded column has at most 524,288
    /// bytes and each present STRING has at most 65,536 UTF-8 bytes. The table
    /// may contain at most 4,096 batches across all committed appends.
    ///
    /// If a write fails, this append can no longer commit. Call [`Self::abort`]
    /// to clean up, or drop the append and reopen the database for recovery.
    pub fn write(
        &mut self,
        columns: &[ColumnInput<'_>],
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        self.write_columns(columns, cancel, &mut Effects::default())
    }

    /// Commit all batches together and return a receipt confirming durability.
    /// If the outcome is uncertain, reopen and check this append's transaction
    /// identifier with [`crate::Database::resolve_commit`] before retrying.
    pub fn commit(self, cancel: &CancellationToken) -> Result<Commit, Error> {
        self.commit_with_effects(cancel, &mut Effects::default())
    }

    /// Remove this append's uncommitted files and synchronize their removal.
    /// If cleanup fails, reopen the database for recovery. Until the handle is
    /// closed or recovery settles the files, their temporary space stays reserved.
    pub fn abort(self) -> Result<(), Error> {
        self.abort_with_effects(&mut Effects::default())
    }

    #[cfg(test)]
    pub(crate) fn write_identified(
        &mut self,
        columns: &[InputColumn<'_>],
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        self.write_input(Columns::Identified(columns), cancel, effects)
    }

    pub(crate) fn write_columns(
        &mut self,
        columns: &[ColumnInput<'_>],
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        self.write_input(Columns::Positional(columns), cancel, effects)
    }

    fn write_input(
        &mut self,
        columns: Columns<'_>,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        if self.failed {
            return Err(Error::Unsupported("failed append requires abort"));
        }
        let result = self.write_batch(columns, cancel, effects);
        if result.is_err() {
            // Even a failure before file creation makes the append abort-only.
            // A caller must not commit a prefix after overlooking a write error.
            self.failed = true;
        }
        result
    }

    fn write_batch(
        &mut self,
        input: Columns<'_>,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        cancel.check()?;
        let count = self.units.len() - self.old_units;
        if count == self.limits.batches as usize {
            return Err(Error::Resource {
                owner: "append batches",
                required: count as u64 + 1,
                limit: u64::from(self.limits.batches),
            });
        }
        let attempt = self.transaction().sequence();
        let (_, schema_bytes) = self.admitted.bytes().split_at_mut(catalog::MAX_BYTES);
        let schema = schema::decode(
            &schema_bytes[..self.schema_len],
            self.writer.prior.database,
            self.table,
            self.schema_crc,
        )
        .map_err(|_| Error::Corrupt("retained append schema differs"))?;
        // Map declaration positions to persistent column IDs from the schema.
        // Copy only slice descriptors; row values remain borrowed from the caller.
        let mut mapped = [InputColumn {
            id: schema.column(0).expect("nonempty schema").id(),
            values: crate::ColumnValues::Int64(&[]),
            validity: &[],
        }; schema::MAX_COLUMNS];
        let columns = match input {
            #[cfg(test)]
            Columns::Identified(columns) => columns,
            Columns::Positional(columns) => {
                if columns.len() != schema.len() {
                    return Err(Error::Unsupported(
                        "append column count differs from schema",
                    ));
                }
                for (index, column) in columns.iter().enumerate() {
                    mapped[index] = InputColumn {
                        id: schema.column(index).expect("bounded schema ordinal").id(),
                        values: column.values,
                        validity: column.validity,
                    };
                }
                &mapped[..columns.len()]
            }
        };
        let requirement = unit::requirements(&schema, columns, cancel)?;
        let next_bytes = self
            .encoded_bytes
            .checked_add(requirement.unit_bytes as u64)
            .ok_or(Error::Corrupt("append byte count overflow"))?;
        if next_bytes > self.limits.encoded_bytes {
            return Err(Error::Resource {
                owner: "append encoded bytes",
                required: next_bytes,
                limit: self.limits.encoded_bytes,
            });
        }
        let next_rows = self
            .rows
            .checked_add(requirement.rows as u64)
            .expect("bounded native table rows");
        // Commit reuses the encoding bytes after the last batch has been written.
        // These requirements overlap in storage, never in lifetime.
        let needed =
            (requirement.metadata_bytes + requirement.column_bytes).max(history::SCRATCH_BYTES);
        if self
            .workspace
            .as_mut()
            .is_none_or(|workspace| workspace.bytes().len() < needed)
        {
            // Release the old physical buffer and charge before growing. A failed
            // allocation leaves private units owned and the transaction abort-only.
            self.workspace = None;
            self.workspace = Some(catalog::Scratch::sized(
                &self.writer.database.memory,
                needed,
                "append encoding workspace",
            )?);
        }
        let workspace = self
            .workspace
            .as_mut()
            .expect("encoding workspace admitted")
            .bytes();
        let (metadata, rest) = workspace.split_at_mut(requirement.metadata_bytes);
        let (column, _) = rest.split_at_mut(requirement.column_bytes);
        let objects = joined_path(self.writer.database.path(), UNITS_NAME)?;
        let file = create_object(&objects, attempt, &mut self.created, cancel, effects)?;
        let unit = unit::write(
            &file,
            object(attempt, self.created),
            &schema,
            columns,
            WriteBuffers { metadata, column },
            cancel,
            effects,
        )?;
        sync_object(&file, cancel, effects)?;
        drop(file);
        self.units.push(unit);
        self.rows = next_rows;
        self.encoded_bytes = next_bytes;
        Ok(())
    }

    pub(crate) fn abort_with_effects(self, effects: &mut Effects) -> Result<(), Error> {
        let cleaned = (|| {
            let objects = joined_path(self.writer.database.path(), UNITS_NAME)?;
            cleanup(
                &objects,
                self.transaction().sequence(),
                self.created,
                effects,
            )
        })();
        if let Err(error) = cleaned {
            return Err(self.writer.fail(error));
        }
        self.writer.abort_unbuilt()
    }

    pub(crate) fn abort_error(self, primary: Error, effects: &mut Effects) -> Error {
        match self.abort_with_effects(effects) {
            Ok(()) => primary,
            Err(cleanup) => Error::CleanupRequired {
                primary: ErrorCause::from_error(primary),
                cleanup: ErrorCause::from_error(cleanup),
            },
        }
    }

    pub(crate) fn commit_with_effects(
        mut self,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Commit, Error> {
        if self.failed || self.units.len() == self.old_units {
            return Err(self.abort_error(
                Error::Unsupported("append requires successful nonempty batches"),
                effects,
            ));
        }
        let commit = match self.build_commit(cancel, effects) {
            Ok(commit) => commit,
            Err(error) => return Err(self.abort_error(error, effects)),
        };
        // Independent validation rereads the completed files. Release encoding
        // buffers first, then hand off ownership of the outcome to publication.
        drop(self.workspace);
        drop(self.admitted);
        drop(self.units);
        drop(self._references);
        self.writer.commit_prepared(commit, cancel, effects)
    }

    fn build_commit(
        &mut self,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<CatalogCommit, Error> {
        cancel.check()?;
        let RootState::Catalog(Some(prior)) = self.writer.prior.state else {
            unreachable!("append has a catalog");
        };
        let objects = joined_path(self.writer.database.path(), UNITS_NAME)?;
        let (old, schema_bytes) = self.admitted.bytes().split_at_mut(catalog::MAX_BYTES);
        let catalog = catalog::decode(
            &old[..prior.catalog().bytes() as usize],
            self.writer.prior.database,
            prior.catalog(),
        )
        .map_err(|_| Error::Corrupt("retained append catalog differs"))?;
        let schema = schema::decode(
            &schema_bytes[..self.schema_len],
            self.writer.prior.database,
            self.table,
            self.schema_crc,
        )
        .map_err(|_| Error::Corrupt("retained append schema differs"))?;
        let auxiliary = self
            .workspace
            .as_mut()
            .expect("successful append owns workspace")
            .bytes();
        let attempt = self.writer.issued.expect("issued writer").issued;
        let file = create_object(&objects, attempt, &mut self.created, cancel, effects)?;
        let data = table::write(
            &file,
            object(attempt, self.created),
            &schema,
            &self.units,
            auxiliary,
            cancel,
            effects,
        )?;
        sync_object(&file, cancel, effects)?;
        drop(file);
        let replacement = catalog
            .table(self.ordinal)
            .expect("selected table")
            .with_data(data, self.rows, self.units.len() as u32)
            .map_err(|_| Error::Corrupt("admitted append shape differs"))?;
        let mut entries: [TableEntry<'_>; catalog::MAX_TABLES] = [replacement; catalog::MAX_TABLES];
        for (index, entry) in entries[..catalog.len()].iter_mut().enumerate() {
            *entry = if index == self.ordinal {
                replacement
            } else {
                catalog.table(index).expect("bounded table")
            };
        }
        let next = catalog::encode(
            auxiliary,
            self.writer.prior.database,
            object(attempt, self.created + 1),
            &entries[..catalog.len()],
        )
        .map_err(|_| Error::Corrupt("validated append catalog differs"))?;
        let mut file = create_object(&objects, attempt, &mut self.created, cancel, effects)?;
        write_nonempty(
            &mut file,
            &auxiliary[..next.bytes() as usize],
            Effect::WriteMetadata(crate::effects::MetadataKind::CatalogObject),
            effects,
        )?;
        sync_object(&file, cancel, effects)?;
        drop(file);
        let file = create_object(&objects, attempt, &mut self.created, cancel, effects)?;
        let history = history::append(
            &file,
            &objects,
            self.writer.issued.expect("issued writer"),
            object(attempt, self.created),
            auxiliary,
            cancel,
            effects,
        )?;
        sync_object(&file, cancel, effects)?;
        drop(file);
        crate::storage::recovery::sync_directory_cancellable(
            &objects,
            DirectoryKind::Units,
            cancel,
            effects,
        )?;
        CatalogCommit::new(
            self.writer.prior.database,
            prior.generation() + 1,
            self.transaction(),
            next,
            history,
        )
        .map_err(|_| Error::Corrupt("constructed append commit differs"))
    }
}

mod admission;
