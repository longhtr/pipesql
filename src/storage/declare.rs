//! Add an empty table by publishing its schema in a new catalog version.
//!
//! `Writer::declare_table` turns borrowed column definitions into persistent
//! schema entries. `create_table` checks the name and catalog capacity, then
//! reserves construction space before durably issuing the transaction number.
//! That number also identifies the new table, so aborted attempts never cause
//! a later table to reuse an issued identity.
//!
//! Construction creates three files: the schema, a catalog including the new
//! table, and updated transaction history. Existing tables keep their references;
//! an empty table needs no row-data file. The files and their directory entries
//! must be synchronized before publication.
//!
//! Construction errors trigger removal of the files created by this attempt.
//! If removal fails too, both failures survive in the returned error. After the
//! handoff to `Writer::commit_prepared`, recovery owns any unfinished cleanup:
//! the builder must not delete files that may have become committed.

use super::construction::{cleanup, create_object, object, sync_object};
use super::snapshot::{Writer, generation};
use crate::effects::{DirectoryKind, Effect, Effects, write_nonempty};
use crate::path::joined_path;
use crate::storage::catalog::{self, ObjectRef, TableEntry};
use crate::storage::format::{self, CatalogCommit, RootState};
use crate::storage::recovery::UNITS_NAME;
use crate::storage::schema::{self, ColumnSpec, TableId};
use crate::{CancellationToken, Commit, Error, ErrorCause};

use crate::ColumnDeclaration;

const CREATE_OBJECTS: u32 = 3;

impl Writer<'_> {
    pub(crate) fn declare_table(
        self,
        name: &str,
        columns: &[ColumnDeclaration<'_>],
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Commit, Error> {
        let declarations = (|| {
            cancel.check()?;
            if columns.is_empty() {
                return Err(Error::Unsupported("empty table declaration"));
            }
            if columns.len() > schema::MAX_COLUMNS {
                return Err(Error::Resource {
                    owner: "table declaration columns",
                    required: columns.len() as u64,
                    limit: schema::MAX_COLUMNS as u64,
                });
            }
            let initial = ColumnSpec::new(
                schema::ColumnId::new(1).expect("nonzero initial column identity"),
                "_",
                crate::DataType::Int64,
                false,
            )
            .expect("valid placeholder in bounded construction scratch");
            let mut specs = [initial; schema::MAX_COLUMNS];
            for (index, column) in columns.iter().enumerate() {
                let id = u32::try_from(index + 1).expect("bounded declaration ordinal");
                specs[index] = ColumnSpec::new(
                    schema::ColumnId::new(id).expect("nonzero declaration ordinal"),
                    column.name,
                    column.data_type,
                    column.nullable,
                )
                .map_err(|_| Error::Unsupported("invalid table declaration"))?;
            }
            Ok(specs)
        })();
        let specs = match declarations {
            Ok(specs) => specs,
            Err(error) => {
                self.abort_unbuilt()?;
                return Err(error);
            }
        };
        // Derive table identity from the attempt sequence. Once issued, a number
        // stays spent after abort; counting current tables would permit reuse.
        let attempt = self
            .prior
            .issued
            .checked_add(1)
            .expect("admitted writer sequence");
        let table = TableId::new(attempt).expect("nonzero admitted attempt");
        self.create_table(name, table, &specs[..columns.len()], cancel, effects)
    }

    pub(crate) fn create_table(
        mut self,
        name: &str,
        table: TableId,
        columns: &[ColumnSpec<'_>],
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Commit, Error> {
        let mut scratch =
            match catalog::Scratch::new(&self.database.memory, "catalog construction scratch") {
                Ok(value) => value,
                Err(error) => {
                    self.abort_unbuilt()?;
                    return Err(error);
                }
            };
        let attempt = self
            .prior
            .issued
            .checked_add(1)
            .expect("admitted writer sequence");
        // Encode the schema and replacement catalog before creating their files.
        // Old and new catalog buffers are separate because entry names borrow old.
        let prepared = (|| {
            // Path allocation is still preparation: refusal must release the
            // unissued writer through abort_unbuilt, without requiring recovery.
            let objects = joined_path(self.database.path(), UNITS_NAME)?;
            cancel.check()?;
            if self.issued.is_some() {
                return Err(Error::Unsupported("create requires an unissued writer"));
            }
            let (old_bytes, rest) = scratch.bytes().split_at_mut(catalog::MAX_BYTES);
            let (new_bytes, schema_bytes) = rest.split_at_mut(catalog::MAX_BYTES);
            let old = match self.prior.state {
                RootState::Catalog(Some(commit)) => Some(catalog::read(
                    &objects,
                    self.prior.database,
                    commit.catalog(),
                    old_bytes,
                    cancel,
                    effects,
                )?),
                RootState::Catalog(None) => None,
                _ => unreachable!("catalog writer"),
            };
            let count = old.as_ref().map_or(0, catalog::Catalog::len);
            if count == catalog::MAX_TABLES {
                return Err(Error::Resource {
                    owner: "catalog tables",
                    required: (count + 1) as u64,
                    limit: catalog::MAX_TABLES as u64,
                });
            }
            let schema_len = schema::encode(schema_bytes, self.prior.database, table, columns)
                .map_err(|_| Error::Unsupported("invalid table declaration"))?;
            let schema = ObjectRef::new(
                object(attempt, 1),
                schema_len as u32,
                format::crc32c(&schema_bytes[..schema_len]),
            )
            .expect("bounded encoded schema");
            let entry = TableEntry::new(table, name, schema, None, 0, 0)
                .map_err(|_| Error::Unsupported("invalid table declaration"))?;
            // Fixed bounded stack storage; names borrow the admitted input and
            // old catalog scratch until the replacement encoding is complete.
            let mut entries = [entry; catalog::MAX_TABLES];
            if let Some(old) = &old {
                for (index, slot) in entries[..count].iter_mut().enumerate() {
                    *slot = old.table(index).expect("bounded catalog ordinal");
                    if slot.id() == table || slot.name().eq_ignore_ascii_case(name) {
                        return Err(Error::Unsupported("table name or identity already exists"));
                    }
                }
            }
            entries[..count + 1].sort_unstable_by_key(|entry| entry.id().value());
            let catalog = catalog::encode(
                new_bytes,
                self.prior.database,
                object(attempt, 2),
                &entries[..count + 1],
            )
            .map_err(|_| Error::Corrupt("validated table insertion failed"))?;
            let history_bytes = generation(self.prior)
                .checked_add(1)
                .and_then(|count| count.checked_mul(8))
                .and_then(|bytes| bytes.checked_add(64))
                .expect("admitted success capacity");
            let temporary = (schema_len as u64)
                .checked_add(u64::from(catalog.bytes()))
                .and_then(|bytes| bytes.checked_add(history_bytes))
                // The publisher creates one replacement root at a time. Its
                // peak overlaps the complete new graph, including history.
                .and_then(|bytes| bytes.checked_add(format::ROOT_BYTES as u64))
                .expect("bounded construction extent");
            // Leave room for all three new names under the same directory limit
            // enforced by open. Readers cannot add persistent catalog objects.
            let identity = crate::storage::recovery::validate_directory(&objects, effects)?;
            crate::storage::recovery::inspect_catalog_objects(
                &objects,
                identity,
                self.prior.issued,
                crate::storage::recovery::MAX_CATALOG_OBJECTS - CREATE_OBJECTS as usize,
                effects,
            )?;
            Ok((objects, schema_len, catalog, temporary))
        })();
        let (objects, schema_len, catalog, temporary) = match prepared {
            Ok(value) => value,
            Err(error) => {
                self.abort_unbuilt()?;
                return Err(error);
            }
        };
        if let Err(error) = self.reserve_construction(temporary) {
            self.abort_unbuilt()?;
            return Err(error);
        }
        // Issuance failure may have touched roots. Keep its existing recovery
        // outcome and charge; no construction names have been created yet.
        let transaction = self.issue(cancel, effects)?;
        let mut created = 0;
        let built = (|| {
            let (_, rest) = scratch.bytes().split_at_mut(catalog::MAX_BYTES);
            let (catalog_bytes, schema_bytes) = rest.split_at_mut(catalog::MAX_BYTES);
            let mut file = create_object(&objects, attempt, &mut created, cancel, effects)?;
            write_nonempty(
                &mut file,
                &schema_bytes[..schema_len],
                Effect::WriteMetadata(crate::effects::MetadataKind::CatalogObject),
                effects,
            )?;
            sync_object(&file, cancel, effects)?;
            drop(file);
            let mut file = create_object(&objects, attempt, &mut created, cancel, effects)?;
            write_nonempty(
                &mut file,
                &catalog_bytes[..catalog.bytes() as usize],
                Effect::WriteMetadata(crate::effects::MetadataKind::CatalogObject),
                effects,
            )?;
            sync_object(&file, cancel, effects)?;
            drop(file);
            let file = create_object(&objects, attempt, &mut created, cancel, effects)?;
            let history = crate::storage::history::append(
                &file,
                &objects,
                self.issued.expect("issued writer"),
                object(attempt, 3),
                scratch.bytes(),
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
                self.prior.database,
                generation(self.prior) + 1,
                transaction,
                catalog,
                history,
            )
            .map_err(|_| Error::Corrupt("constructed catalog commit is invalid"))
        })();
        let commit = match built {
            Ok(commit) => commit,
            Err(primary) => {
                drop(scratch);
                if let Err(cleanup) = cleanup(&objects, attempt, created, effects) {
                    return Err(Error::CleanupRequired {
                        primary: ErrorCause::from_error(primary),
                        cleanup: ErrorCause::from_error(cleanup),
                    });
                }
                self.abort_unbuilt()?;
                return Err(primary);
            }
        };
        drop(scratch);
        // Once publication starts, these dependencies may be authoritative.
        // The publisher owns classification; no builder cleanup follows it.
        self.commit_prepared(commit, cancel, effects)
    }
}
