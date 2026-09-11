//! Typed batch admission, private unit construction, and one catalog publication.
use super::Writer;
use super::construction::{cleanup, create_object, object, sync_object};
use crate::catalog::{self, TableEntry};
use crate::catalog_schema::{self, TableId};
use crate::effects::{DirectoryKind, Effect, Effects, write_nonempty};
use crate::namespace::UNITS_NAME;
use crate::native_unit::{self, InputColumn, UnitRef, WriteBuffers};
use crate::path::joined_path;
use crate::resources::Reservation;
use crate::storage_format::{CatalogCommit, RootState};
use crate::{
    CancellationToken, Commit, Error, ErrorCause, TransactionId, success_index, table_data,
};

/// Positive bounds reserved before an append issues its transaction.
#[derive(Clone, Copy)]
pub struct AppendLimits {
    /// Maximum number of batches accepted by this append.
    pub batches: u32,
    /// Maximum total encoded unit bytes, including unit headers and validity.
    /// The database separately reserves index, catalog, history, and root space.
    pub encoded_bytes: u64,
}

/// One batch column, borrowed during `Append::write` in declaration order.
#[derive(Clone, Copy)]
pub struct ColumnInput<'a> {
    /// Values of the declared type, with the same row count as every other column.
    pub values: native_unit::InputValues<'a>,
    /// One bit per row, least significant bit first: 1 is present, 0 is NULL.
    /// Supply exactly `ceil(rows / 8)` bytes with unused trailing bits clear.
    /// Nonnullable columns require every row to be present.
    pub validity: &'a [u8],
}

enum Columns<'a> {
    #[cfg(test)]
    Identified(&'a [InputColumn<'a>]),
    Positional(&'a [ColumnInput<'a>]),
}

// Inputs are borrowed only during write. All file and buffer owners precede
// their accounts; an unsettled Writer marks admission unavailable on drop.
pub(crate) struct Append<'db> {
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
    pub(crate) fn transaction(&self) -> TransactionId {
        let issued = self.writer.issued.expect("append owns an issued writer");
        TransactionId::for_attempt(issued.database, issued.issued).expect("issued identity")
    }

    #[cfg(test)]
    pub(crate) fn write(
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
        let schema = catalog_schema::decode(
            &schema_bytes[..self.schema_len],
            self.writer.prior.database,
            self.table,
            self.schema_crc,
        )
        .map_err(|_| Error::Corrupt("retained append schema differs"))?;
        // Only borrowed slices are copied. Identity comes from the validated
        // retained schema, never from the caller's position as a numeric ID.
        let mut mapped = [InputColumn {
            id: schema.column(0).expect("nonempty schema").id(),
            values: native_unit::InputValues::Int64(&[]),
            validity: &[],
        }; catalog_schema::MAX_COLUMNS];
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
        let requirement = native_unit::requirements(&schema, columns, cancel)?;
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
        let needed =
            requirement.metadata_bytes + requirement.column_bytes + success_index::SCRATCH_BYTES;
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
        let unit = native_unit::write(
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

    pub(crate) fn abort(self, effects: &mut Effects) -> Result<(), Error> {
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

    fn abort_error(self, primary: Error, effects: &mut Effects) -> Error {
        match self.abort(effects) {
            Ok(()) => primary,
            Err(cleanup) => Error::CleanupRequired {
                primary: ErrorCause::from_error(primary),
                cleanup: ErrorCause::from_error(cleanup),
            },
        }
    }

    pub(crate) fn commit(
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
        // Publication validates independently, after construction buffers close.
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
        let schema = catalog_schema::decode(
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
        let data = table_data::write(
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
        let history = success_index::append(
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
        crate::namespace::sync_directory_cancellable(
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
