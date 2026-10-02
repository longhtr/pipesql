//! Select an append's table and reserve its retained storage before issuing it.
//!
//! `Writer::begin_append_named` reads the current catalog, retains the selected
//! schema, and loads existing unit references. It reserves reference slots for
//! the promised batch count and disk space for new units and commit metadata.
//! Directory capacity is checked before the transaction number is made durable.
//!
//! Fallible setup lives in `admit_append_inputs`. On failure, its local buffers
//! and reservations drop before `admit_append` aborts the writer or preserves a
//! recovery error. On success, `AdmittedAppend` carries the completed inputs into
//! the live append without another allocation.
//!
//! These reservations do not guarantee that later writes succeed. Streaming
//! admission leaves encoding workspace unallocated until the first batch; the
//! parent module validates each batch and grows that workspace when needed.

use super::{ALLOCATION_ROUNDING_BYTES, Append, AppendLimits};
use crate::effects::Effects;
use crate::storage::catalog;
use crate::storage::recovery::UNITS_NAME;
use crate::storage::schema::{self, TableId};
use crate::storage::snapshot::Writer;

use crate::path::joined_path;
use crate::resources::Reservation;
use crate::storage::format::{self, RootState};
use crate::storage::table;
#[cfg(test)]
use crate::storage::unit::InputColumn;
use crate::storage::unit::{self, UnitRef};
use crate::{CancellationToken, Error};

#[cfg(test)]
use crate::Commit;
#[cfg(test)]
use crate::storage::history;

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

// Return completed inputs as one value. Its field order also frees buffers
// before reservations if the value is dropped instead of becoming an Append.
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
        if let Err(error) = append.write_identified(columns, cancel, effects) {
            return Err(append.abort_error(error, effects));
        }
        append.commit_with_effects(cancel, effects)
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
                // The input-building call has already dropped its locals.
                // Do not clear admission if issuance left an unsettled disk state.
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
            TableTarget::Name(name) => schema::validate_name(name.as_bytes())
                .map_err(|_| Error::Unsupported("invalid table name"))?,
            #[cfg(test)]
            TableTarget::Identity(_) => (),
        }
        let RootState::Catalog(Some(prior)) = self.prior.state else {
            return Err(Error::NotFound);
        };
        let database = self.database;
        // Three heap owners remain live: catalog/schema, references, and the
        // reusable encoding/commit workspace. Their reservations include native
        // allocation reuse; this owner adds a rounding allowance for each.
        // Growth drops the old workspace first, so no fourth buffer
        // overlaps. Reserve this before allocating or issuing the transaction.
        let owner = database.memory.reserve(
            (std::mem::size_of::<Append<'_>>()
                + 2 * crate::path::MAX_PATH_BYTES
                + 3 * ALLOCATION_ROUNDING_BYTES) as u64,
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
        let schema_crc = format::crc32c(schema.encoded_bytes());
        let (limits, workspace) = match admission {
            #[cfg(test)]
            Admission::Single(columns) => {
                let requirement = unit::requirements(&schema, columns, cancel)?;
                // Encoding and commit reuse the same bytes in separate phases.
                let bytes = (requirement.metadata_bytes + requirement.column_bytes)
                    .max(history::SCRATCH_BYTES);
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
        let max_bytes = u64::from(limits.batches) * unit::MAX_UNIT_BYTES as u64;
        if limits.encoded_bytes == 0 || limits.encoded_bytes > max_bytes {
            return Err(Error::Resource {
                owner: "append encoded byte limit",
                required: limits.encoded_bytes.max(1),
                limit: max_bytes,
            });
        }
        let reference_bytes = count as usize * std::mem::size_of::<UnitRef>();
        let reference_charge = crate::resources::buffer_charge(reference_bytes)
            .expect("bounded append reference storage");
        let references = database
            .memory
            .reserve(reference_charge as u64, "append unit references")?;
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
        // The replacement index includes old and new units, but only new unit
        // files consume this attempt's encoded-byte allowance. The catalog keeps
        // the same table count and names, so its encoded size does not grow.
        let data_bytes =
            u64::from(table::HEADER) + u64::from(count) * u64::from(table::ENTRY_BYTES);
        let temporary = limits
            .encoded_bytes
            .checked_add(data_bytes)
            .and_then(|n| n.checked_add(u64::from(prior.catalog().bytes())))
            .and_then(|n| n.checked_add(64 + 8 * (prior.generation() + 1)))
            .and_then(|n| n.checked_add(format::ROOT_BYTES as u64))
            .ok_or(Error::Corrupt("append reservation overflow"))?;
        let identity = crate::storage::recovery::validate_directory(&objects, effects)?;
        crate::storage::recovery::inspect_catalog_objects(
            &objects,
            identity,
            self.prior.issued,
            crate::storage::recovery::MAX_CATALOG_OBJECTS - (limits.batches as usize + 3),
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
