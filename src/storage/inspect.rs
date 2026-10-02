//! Inspect a table's columns without running a query.
//!
//! `Database::inspect_table` finds a declared table and reads its schema: the
//! column names, types and rules for NULL values. It passes a `TableSchema` view
//! to the caller's function, which can print the columns or copy their details.
//!
//! The view borrows the read buffers. Using a callback keeps those buffers alive
//! while the caller reads them, without allocating a second copy of the schema.
//! Inspection also retains the database version it started with, so another
//! write cannot change the schema being inspected.

use crate::effects::Effects;
use crate::storage::catalog;
use crate::storage::schema;
use crate::{CancellationToken, ColumnDeclaration, Database, Error};

/// A table's column definitions, borrowed during [`Database::inspect_table`].
/// Names use their declared spelling, and columns appear in declaration order.
/// Copy any names you need to keep after the callback returns.
pub struct TableSchema<'a> {
    name: &'a str,
    generation: u64,
    schema: schema::Schema<'a>,
}

impl TableSchema<'_> {
    /// The table's declared name.
    pub fn name(&self) -> &str {
        self.name
    }

    /// The database version chosen when inspection began.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The number of columns, between 1 and 64 inclusive.
    pub fn column_count(&self) -> usize {
        self.schema.len()
    }

    /// Read a column's name, type and NULL rule at a zero-based position.
    /// Returns `None` if the position is outside the schema. This reads the
    /// already validated buffers without allocating memory or accessing files.
    pub fn column(&self, index: usize) -> Option<ColumnDeclaration<'_>> {
        self.schema.column(index).map(|column| ColumnDeclaration {
            name: column.name(),
            data_type: column.data_type(),
            nullable: column.nullable(),
        })
    }
}

impl Database {
    /// Read a declared table's schema and pass it to `inspect`.
    ///
    /// Names are matched without regard to ASCII case. An invalid name returns
    /// `Unsupported`; a missing table returns `NotFound`. Legacy databases do
    /// not support inspection. Resource limits, cancellation, file errors or
    /// corrupt data can prevent the schema from being read.
    ///
    /// After reading and validating the schema, call `inspect` exactly once.
    /// No registry mutex is held during the callback, so it can publish a write
    /// without changing this view. The borrowed view cannot outlive the call;
    /// any copies the callback makes use the caller's storage.
    ///
    /// Return the callback's error unchanged. Cancellation is checked before
    /// reads and immediately before the callback. The callback must check for
    /// cancellation during its own work if needed.
    pub fn inspect_table(
        &self,
        name: &str,
        cancel: &CancellationToken,
        inspect: impl FnOnce(TableSchema<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        self.inspect_table_with_effects(name, cancel, inspect, &mut Effects::default())
    }

    pub(crate) fn inspect_table_with_effects(
        &self,
        name: &str,
        cancel: &CancellationToken,
        inspect: impl FnOnce(TableSchema<'_>) -> Result<(), Error>,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        cancel.check()?;
        schema::validate_name(name.as_bytes())
            .map_err(|_| Error::Unsupported("invalid table name"))?;
        // Retaining the snapshot prevents its files from being reclaimed during
        // the callback. Local buffers and this pin drop even if the callback
        // returns an error or unwinds after a panic.
        let snapshot = self.catalog_snapshot()?;
        // Reading a file needs its full path while the units-directory path is
        // still alive. Reserve space for both, and keep that reservation until
        // both paths have been freed, including when a read fails.
        let _paths = self.reserve_memory(
            (2 * crate::path::MAX_PATH_BYTES) as u64,
            "schema inspection paths",
        )?;
        let mut scratch = catalog::Scratch::sized(
            &self.memory,
            catalog::MAX_BYTES + schema::MAX_BYTES,
            "schema inspection buffers",
        )?;
        let (catalog_bytes, schema_bytes) = scratch.bytes().split_at_mut(catalog::MAX_BYTES);
        let catalog = snapshot
            .read_catalog(catalog_bytes, cancel, effects)?
            .ok_or(Error::NotFound)?;
        let ordinal = (0..catalog.len())
            .find(|&index| {
                catalog
                    .table(index)
                    .is_some_and(|table| table.name().eq_ignore_ascii_case(name))
            })
            .ok_or(Error::NotFound)?;
        let table = catalog.table(ordinal).expect("matched catalog table");
        let objects = crate::path::joined_path(self.path(), crate::storage::recovery::UNITS_NAME)?;
        let schema = catalog.read_schema(&objects, ordinal, schema_bytes, cancel, effects)?;
        cancel.check()?;
        inspect(TableSchema {
            name: table.name(),
            generation: snapshot.generation(),
            schema,
        })
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;
    use crate::effects::Faults;
    use crate::{Config, DataType};
    use std::sync::Arc;

    #[test]
    fn inspection_releases_each_failed_read_and_refused_reservation() {
        let directory = crate::test_support::Directory::new();
        let db = Database::create_empty(
            &directory.0.join("db"),
            Config::new(1_000_000, 1_000_000).unwrap(),
        )
        .unwrap();
        let cancel = CancellationToken::new();
        db.declare_table(
            "events",
            &[ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            }],
            &cancel,
        )
        .unwrap();
        let resident = db.reserved_memory_bytes();
        let mut census = Effects::default();
        db.inspect_table_with_effects("events", &cancel, |_| Ok(()), &mut census)
            .unwrap();
        assert!(census.count() > 0);
        for cut in 0..census.count() {
            let mut effects = Effects::with_faults(Faults {
                fail_at: Some(cut),
                ..Faults::default()
            });
            let outcome = db.inspect_table_with_effects(
                "events",
                &cancel,
                |_| panic!("callback after refused read {cut}"),
                &mut effects,
            );
            assert!(
                matches!(outcome, Err(Error::Io { .. })),
                "cut {cut}: {outcome:?}"
            );
            assert_eq!(db.reserved_memory_bytes(), resident, "cut {cut}");

            let token = Arc::new(CancellationToken::new());
            let request = Arc::clone(&token);
            let mut effects = Effects::with_faults(Faults {
                action: Some(Box::new(move |index, _| {
                    if index == cut {
                        request.cancel();
                    }
                })),
                ..Faults::default()
            });
            assert!(matches!(
                db.inspect_table_with_effects(
                    "events",
                    &token,
                    |_| panic!("callback after cancellation at {cut}"),
                    &mut effects,
                ),
                Err(Error::Cancelled)
            ));
            assert_eq!(db.reserved_memory_bytes(), resident);
        }
        // Refuse the path reservation first, then the read-buffer reservation.
        // Neither refusal should touch files or call the user's callback.
        for spare in [0, (2 * crate::path::MAX_PATH_BYTES) as u64] {
            let held = db
                .reserve_memory(
                    db.config().memory_limit_bytes() - resident - spare,
                    "test peer",
                )
                .unwrap();
            let mut effects = Effects::default();
            assert!(matches!(
                db.inspect_table_with_effects(
                    "events",
                    &cancel,
                    |_| panic!("callback without admitted storage"),
                    &mut effects,
                ),
                Err(Error::Resource { .. })
            ));
            assert_eq!(effects.count(), 0);
            drop(held);
            assert_eq!(db.reserved_memory_bytes(), resident);
        }
        db.inspect_table("events", &cancel, |_| Ok(())).unwrap();
        assert_eq!(db.reserved_memory_bytes(), resident);
        assert_eq!(db.reserved_temp_bytes(), 0);
        db.close().unwrap();
    }
}
