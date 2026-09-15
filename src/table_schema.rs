//! Read a declared table's schema without preparing or executing a query.
//!
//! Inspection pins the current catalog generation, finds the table by name and
//! validates its schema file. A callback borrows the resulting column names while
//! the read buffers remain alive. This keeps inspection independent of SQL name
//! syntax and avoids allocating a second copy of the schema.
//!
//! The snapshot pin prevents reclamation during the callback; it holds no registry
//! mutex and does not block publication. Read failures prevent the callback from
//! running. Returning an error or unwinding releases the buffers and snapshot just
//! as a successful return does. The callback owns any output and its failures.

use crate::effects::Effects;
use crate::{CancellationToken, ColumnDeclaration, Database, Error, catalog, catalog_schema};

/// A validated schema borrowed for one [`Database::inspect_table`] callback.
/// Names retain their declared spelling; column positions follow declaration order.
pub struct TableSchema<'a> {
    name: &'a str,
    generation: u64,
    schema: catalog_schema::Schema<'a>,
}

impl TableSchema<'_> {
    /// The table's declared name.
    pub fn name(&self) -> &str {
        self.name
    }

    /// The database generation pinned when inspection began.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The number of columns, between 1 and 64 inclusive.
    pub fn column_count(&self) -> usize {
        self.schema.len()
    }

    /// Column metadata at a zero-based position, or `None` outside the schema.
    /// This reads validated memory and performs no allocation or file I/O.
    pub fn column(&self, index: usize) -> Option<ColumnDeclaration<'_>> {
        self.schema.column(index).map(|column| ColumnDeclaration {
            name: column.name(),
            data_type: column.data_type(),
            nullable: column.nullable(),
        })
    }
}

impl Database {
    /// Inspect a declared table from one immutable catalog generation.
    ///
    /// Lookup ignores ASCII case. Invalid names return `Unsupported`; absent
    /// tables return `NotFound`. Legacy databases do not support this operation.
    /// Reads require bounded memory and may return resource, cancellation, I/O or
    /// corruption errors before invoking `inspect`.
    ///
    /// The callback runs once, without a registry mutex held. Its borrowed schema
    /// cannot escape the call; any copies it makes belong to the caller. Its error
    /// propagates unchanged. Cancellation is checked before reads and immediately
    /// before the callback; the callback controls its own work and cancellation.
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
        catalog_schema::validate_name(name.as_bytes())
            .map_err(|_| Error::Unsupported("invalid table name"))?;
        let snapshot = self.catalog_snapshot()?;
        // A units path overlaps one object path during each read. Keep their
        // charge until both paths have been freed, including on early return.
        let _paths = self.reserve_memory(
            (2 * crate::path::MAX_PATH_BYTES) as u64,
            "schema inspection paths",
        )?;
        let mut scratch = catalog::Scratch::sized(
            &self.memory,
            catalog::MAX_BYTES + catalog_schema::MAX_BYTES,
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
        let objects = crate::path::joined_path(self.path(), crate::namespace::UNITS_NAME)?;
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
        // Leave enough for the path charge in the second case, but no buffers.
        // Both refusals must happen before the first filesystem effect.
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
