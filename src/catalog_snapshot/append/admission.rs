//! Reserve append owners and bounds, retain the selected table, then issue once.
use super::{Append, AppendLimits};
use crate::catalog;
use crate::catalog_schema::{self, TableId};
use crate::catalog_snapshot::Writer;
use crate::effects::Effects;
use crate::namespace::UNITS_NAME;

#[cfg(test)]
use crate::native_unit::InputColumn;
use crate::native_unit::{self, UnitRef};
use crate::path::joined_path;
use crate::resources::Reservation;
use crate::storage_format::{self, RootState};
use crate::{CancellationToken, Error, table_data};

#[cfg(test)]
use crate::{Commit, success_index};

enum TableTarget<'a> {
    #[cfg(test)]
    Identity(TableId),
    Name(&'a str),
}

enum Admission<'a> {
    #[cfg(test)]
    Single(&'a [InputColumn<'a>]),
    Stream(&'a AppendLimits),
}

// Admission retains the catalog/schema bytes and prior unit references while
// the writer reserves construction space and issues the attempt. A failure drops
// these physical owners before the caller classifies or aborts the writer.
struct AdmittedAppend<'db> {
    workspace: Option<catalog::Scratch<'db>>,
    admitted: catalog::Scratch<'db>,
    units: Vec<UnitRef>,
    references: Reservation<'db>,
    owner: Reservation<'db>,
    ordinal: usize,
    table: TableId,
    schema_len: usize,
    schema_crc: u32,
    old_units: usize,
    rows: u64,
    limits: AppendLimits,
}

impl<'db> Writer<'db> {
    pub(crate) fn begin_append_named(
        self,
        name: &str,
        limits: AppendLimits,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Append<'db>, Error> {
        self.admit_append(
            TableTarget::Name(name),
            Admission::Stream(&limits),
            cancel,
            effects,
        )
    }

    #[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
    pub(crate) fn begin_append(
        self,
        table: TableId,
        limits: AppendLimits,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Append<'db>, Error> {
        self.admit_append(
            TableTarget::Identity(table),
            Admission::Stream(&limits),
            cancel,
            effects,
        )
    }

    #[cfg(test)]
    pub(crate) fn append(
        self,
        table: TableId,
        columns: &[InputColumn<'_>],
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Commit, Error> {
        let mut append = self.admit_append(
            TableTarget::Identity(table),
            Admission::Single(columns),
            cancel,
            effects,
        )?;
        if let Err(error) = append.write(columns, cancel, effects) {
            return Err(append.abort_error(error, effects));
        }
        append.commit(cancel, effects)
    }

    fn admit_append(
        mut self,
        target: TableTarget<'_>,
        admission: Admission<'_>,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<Append<'db>, Error> {
        let prepared = self.admit_append_inputs(target, admission, cancel, effects);
        match prepared {
            Ok(inputs) => Ok(Append {
                workspace: inputs.workspace,
                admitted: inputs.admitted,
                units: inputs.units,
                _references: inputs.references,
                writer: self,
                _owner: inputs.owner,
                ordinal: inputs.ordinal,
                table: inputs.table,
                schema_len: inputs.schema_len,
                schema_crc: inputs.schema_crc,
                old_units: inputs.old_units,
                rows: inputs.rows,
                limits: inputs.limits,
                encoded_bytes: 0,
                created: 0,
                failed: false,
            }),
            Err(primary) => {
                if self.database.registry()?.unavailable() {
                    return Err(primary);
                }
                self.abort_unbuilt()?;
                Err(primary)
            }
        }
    }

    fn admit_append_inputs(
        &mut self,
        target: TableTarget<'_>,
        admission: Admission<'_>,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<AdmittedAppend<'db>, Error> {
        cancel.check()?;
        if self.issued.is_some() {
            return Err(Error::Unsupported("append requires an unissued writer"));
        }
        match target {
            TableTarget::Name(name) => catalog_schema::validate_name(name.as_bytes())
                .map_err(|_| Error::Unsupported("invalid table name"))?,
            #[cfg(test)]
            TableTarget::Identity(_) => (),
        }
        let RootState::Catalog(Some(prior)) = self.prior.state else {
            return Err(Error::NotFound);
        };
        let database = self.database;
        let owner = database.memory.reserve(
            (std::mem::size_of::<Append<'_>>() + 2 * crate::path::MAX_PATH_BYTES) as u64,
            "streaming append owner",
        )?;
        let objects = joined_path(database.path(), UNITS_NAME)?;
        let mut admitted = catalog::Scratch::new(&database.memory, "append catalog admission")?;
        let (catalog_bytes, schema_bytes) = admitted.bytes().split_at_mut(catalog::MAX_BYTES);
        let catalog = catalog::read(
            &objects,
            self.prior.database,
            prior.catalog(),
            catalog_bytes,
            cancel,
            effects,
        )?;
        let ordinal = (0..catalog.len())
            .find(|&index| {
                catalog.table(index).is_some_and(|entry| match target {
                    #[cfg(test)]
                    TableTarget::Identity(id) => entry.id() == id,
                    TableTarget::Name(name) => entry.name().eq_ignore_ascii_case(name),
                })
            })
            .ok_or(Error::NotFound)?;
        let previous = catalog.table(ordinal).expect("selected table ordinal");
        let table = previous.id();
        let schema = catalog.read_schema(&objects, ordinal, schema_bytes, cancel, effects)?;
        let schema_len = schema.encoded_bytes().len();
        let schema_crc = storage_format::crc32c(schema.encoded_bytes());
        let (limits, workspace) = match admission {
            #[cfg(test)]
            Admission::Single(columns) => {
                let requirement = native_unit::requirements(&schema, columns, cancel)?;
                let bytes = requirement.metadata_bytes
                    + requirement.column_bytes
                    + success_index::SCRATCH_BYTES;
                (
                    AppendLimits {
                        batches: 1,
                        encoded_bytes: requirement.unit_bytes as u64,
                    },
                    Some(catalog::Scratch::sized(
                        &database.memory,
                        bytes,
                        "append encoding workspace",
                    )?),
                )
            }
            Admission::Stream(limits) => (*limits, None),
        };
        let count = previous
            .units()
            .checked_add(limits.batches)
            .filter(|&n| limits.batches != 0 && n <= catalog::MAX_UNITS)
            .ok_or(Error::Resource {
                owner: "table native units",
                required: u64::from(previous.units()) + u64::from(limits.batches).max(1),
                limit: u64::from(catalog::MAX_UNITS),
            })?;
        let max_bytes = u64::from(limits.batches) * native_unit::MAX_UNIT_BYTES as u64;
        if limits.encoded_bytes == 0 || limits.encoded_bytes > max_bytes {
            return Err(Error::Resource {
                owner: "append encoded byte limit",
                required: limits.encoded_bytes.max(1),
                limit: max_bytes,
            });
        }
        let reference_bytes = count as usize * std::mem::size_of::<UnitRef>();
        let references = database
            .memory
            .reserve(reference_bytes as u64, "append unit references")?;
        let mut units = crate::resources::allocate(
            count as usize,
            count as usize,
            "append reference allocation",
            database.memory.limit(),
        )?;
        if let Some(mut cursor) = catalog.open_data(&objects, ordinal, cancel, effects)? {
            while let Some(unit) = cursor.next(cancel, effects)? {
                if units.len() == previous.units() as usize {
                    return Err(Error::Corrupt("table index exceeds admitted units"));
                }
                units.push(unit);
            }
        }
        if units.len() != previous.units() as usize {
            return Err(Error::Corrupt(
                "table index differs from catalog unit count",
            ));
        }
        let rows = previous.rows();
        let old_units = units.len();
        let data_bytes =
            u64::from(table_data::HEADER) + u64::from(count) * u64::from(table_data::ENTRY_BYTES);
        let temporary = limits
            .encoded_bytes
            .checked_add(data_bytes)
            .and_then(|n| n.checked_add(u64::from(prior.catalog().bytes())))
            .and_then(|n| n.checked_add(64 + 8 * (prior.generation() + 1)))
            .and_then(|n| n.checked_add(storage_format::ROOT_BYTES as u64))
            .ok_or(Error::Corrupt("append reservation overflow"))?;
        let identity = crate::namespace::validate_directory(&objects, effects)?;
        crate::namespace::inspect_catalog_objects(
            &objects,
            identity,
            self.prior.issued,
            crate::namespace::MAX_CATALOG_OBJECTS - (limits.batches as usize + 3),
            effects,
        )?;
        self.reserve_construction(temporary)?;
        self.issue(cancel, effects)?;
        Ok(AdmittedAppend {
            workspace,
            admitted,
            units,
            references,
            owner,
            ordinal,
            table,
            schema_len,
            schema_crc,
            old_units,
            rows,
            limits,
        })
    }
}
