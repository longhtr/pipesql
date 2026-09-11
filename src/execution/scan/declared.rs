//! Native source for the shared query controller.
use super::{AdmittedScan, ScanCursor, ScanPhase, Source};
use crate::Database;
use crate::batch::{Batch, OwnedBatch};
use crate::effects::Effects;

#[cfg(test)]
use crate::execution::Aggregation;
use crate::execution::COMPUTE_ROWS;
use crate::execution::computed::{BatchLayout, BatchScratch};
use crate::execution::planning::{PhysicalPlan, Pipeline};
use crate::execution::predicate::BranchScratch;
use crate::frontend::{DataType, MAX_COLUMNS, PreparedQuery};
use crate::namespace::UNITS_NAME;
use crate::resources::allocate;
use crate::storage_format;
use crate::{CancellationToken, DatabaseId, Error, StringValue, Value};
use crate::{catalog, catalog_schema, native_unit, table_data};
use std::mem::size_of;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScanState {
    Ready,
    Replayed,
    Failed,
}

const _: () = assert!(MAX_COLUMNS <= u64::BITS as usize);
// Runtime nodes own the inline cursor and batch handles. This account owns only
// the native reader allocation, paths, selection and payload allocations.
const FIXED_BYTES: usize =
    size_of::<Scan>() + 2 * crate::path::MAX_PATH_BYTES + COMPUTE_ROWS * size_of::<u32>();
// Includes the catalog scratch that overlaps retained scan ownership at admission.
pub(in crate::execution) const MAX_WORKSPACE_BYTES: u64 = FIXED_BYTES as u64
    + (MAX_COLUMNS * native_unit::MAX_COLUMN_BYTES) as u64
    + 2 * crate::batch::MAX_BYTES_WITH_TEXT
    + catalog::MAX_BYTES as u64
    + BranchScratch::MAX_BYTES;

pub(in crate::execution) struct Scan {
    state: ScanState,
    unit: Option<native_unit::Unit>,
    cursor: Option<table_data::Cursor>,
    payloads: [Option<native_unit::ColumnBuffer>; MAX_COLUMNS],
    identities: [Option<catalog_schema::ColumnId>; MAX_COLUMNS],
    kinds: [DataType; MAX_COLUMNS],
    schema: [u8; catalog_schema::MAX_BYTES],
    schema_len: usize,
    schema_crc: u32,
    table: catalog_schema::TableId,
    objects: PathBuf,
    loaded: u64,
    rows: usize,
    database_id: DatabaseId,
}

impl Scan {
    pub(in crate::execution) fn rows(&self) -> usize {
        self.rows
    }

    pub(in crate::execution) fn has_unit(&self) -> bool {
        self.unit.is_some()
    }

    pub(in crate::execution) fn finish_unit(&mut self) {
        self.unit = None;
        self.loaded = 0;
        self.rows = 0;
    }

    pub(in crate::execution) fn restart_once(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        let state = std::mem::replace(&mut self.state, ScanState::Failed);
        cancel.check()?;
        match state {
            ScanState::Failed => return Err(Error::Corrupt("failed native scan cannot restart")),
            ScanState::Replayed => {
                return Err(Error::Resource {
                    owner: "query source restarts",
                    required: 2,
                    limit: 1,
                });
            }
            ScanState::Ready => {}
        }
        if let Some(cursor) = &mut self.cursor {
            cursor.rewind()?;
        }
        self.finish_unit();
        self.state = ScanState::Replayed;
        Ok(())
    }

    pub(in crate::execution) fn next_unit(
        &mut self,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<bool, Error> {
        let state = std::mem::replace(&mut self.state, ScanState::Failed);
        if state == ScanState::Failed {
            return Err(Error::Corrupt("native scan has failed"));
        }
        assert!(
            self.unit.is_none(),
            "finish the current native unit before advancing"
        );
        let Some(cursor) = &mut self.cursor else {
            self.state = state;
            return Ok(false);
        };
        let Some(reference) = cursor.next(cancel, effects)? else {
            self.state = state;
            return Ok(false);
        };
        let bytes = &self.schema[..self.schema_len];
        let schema = catalog_schema::decode(bytes, self.database_id, self.table, self.schema_crc)
            .map_err(|_| Error::Corrupt("retained scan schema differs"))?;
        let unit = native_unit::read(
            &self.objects,
            self.database_id,
            reference,
            &schema,
            cancel,
            effects,
        )?;
        self.rows = reference.rows() as usize;
        self.unit = Some(unit);
        self.state = state;
        Ok(true)
    }

    pub(in crate::execution) fn load(
        &mut self,
        column: u8,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<bool, Error> {
        let state = std::mem::replace(&mut self.state, ScanState::Failed);
        if state == ScanState::Failed {
            return Err(Error::Corrupt("native scan has failed"));
        }
        let index = usize::from(column);
        let id = self
            .identities
            .get(index)
            .copied()
            .flatten()
            .ok_or(Error::Corrupt("native source identity absent"))?;
        let mask = 1_u64 << index;
        if self.loaded & mask != 0 {
            self.state = state;
            return Ok(false);
        }
        self.payloads[index]
            .as_mut()
            .ok_or(Error::Corrupt("native column not admitted"))?
            .read(
                self.unit
                    .as_ref()
                    .ok_or(Error::Corrupt("native unit absent"))?,
                id,
                cancel,
                effects,
            )?;
        self.loaded |= mask;
        self.state = state;
        Ok(true)
    }

    pub(in crate::execution) fn value(&self, column: u8, row: usize) -> Result<Value<'_>, Error> {
        let index = usize::from(column);
        if self.state == ScanState::Failed
            || index >= MAX_COLUMNS
            || self.loaded & (1 << index) == 0
        {
            return Err(Error::Corrupt("native column not loaded"));
        }
        let column = self.payloads[index]
            .as_ref()
            .and_then(|buffer| buffer.column())
            .ok_or(Error::Corrupt("native column invalidated"))?;
        let result = match self.kinds[index] {
            DataType::Int64 => column
                .int64(row)
                .map(|v| v.map_or(Value::Null, Value::Int64)),
            DataType::Double => column
                .double(row)
                .map(|v| v.map_or(Value::Null, Value::Double)),
            DataType::Date => column.date(row).map(|v| v.map_or(Value::Null, Value::Date)),
            DataType::String => column
                .string(row)
                .map(|v| v.map_or(Value::Null, |s| Value::String(StringValue::new(s)))),
        };
        result.map_err(|_| Error::Corrupt("native value disagrees with physical plan"))
    }
}

// Retained scan owners are admitted without catalog I/O. A graph can admit
// every source before optional operator growth; no uninitialized scan escapes.
pub(in crate::execution) struct Admission<'db> {
    workspace: AdmittedScan<'db>,
    query: &'db PreparedQuery<'db>,
    occurrence: u8,
}

pub(in crate::execution) fn admit<'db>(
    database: &'db Database,
    query: &'db PreparedQuery<'_>,
    plan: &PhysicalPlan<'_>,
    source: &Pipeline,
    output: Option<&Pipeline>,
) -> Result<Admission<'db>, Error> {
    let snapshot = query
        .snapshot
        .as_ref()
        .ok_or(Error::Corrupt("catalog query pin absent"))?;
    if !snapshot.belongs_to(database)
        || snapshot.generation() != plan.generation
        || snapshot.state() != plan.root
    {
        return Err(Error::Corrupt("catalog query snapshot differs"));
    }
    let occurrence = source.source()?;
    let table = query
        .plan
        .occurrence(occurrence)?
        .table
        .ok_or(Error::Corrupt("catalog query table absent"))?;
    let demand = source.raw_demand()?;
    let mut types = [DataType::Double; MAX_COLUMNS];
    let mut text = [None; MAX_COLUMNS];
    for (index, column) in source.output_columns(&query.plan).enumerate() {
        types[index] = column.data_type();
        if types[index] == DataType::String {
            text[index] = Some(crate::batch::MAX_TEXT_BYTES);
        }
    }
    let count = source.column_count;
    let mut output_types = [DataType::Double; MAX_COLUMNS];
    let mut output_text = [None; MAX_COLUMNS];
    let mut output_count = 0;
    if let Some(output) = output {
        for column in output.output_columns(&query.plan) {
            output_types[output_count] = column.data_type();
            if column.data_type() == DataType::String {
                output_text[output_count] = Some(crate::batch::MAX_TEXT_BYTES);
            }
            output_count += 1;
        }
    }
    let batch_bytes = Batch::required_bytes_with_text(&types[..count], &text[..count])?
        .checked_add(Batch::required_bytes_with_text(
            &output_types[..output_count],
            &output_text[..output_count],
        )?)
        .ok_or(Error::Corrupt("native batch bytes"))?;
    // Includes retained path and one transient bounded object path. Payloads are
    // allocated once per demanded column, never per unit or row.
    let mut capacities = [0; MAX_COLUMNS];
    for column in query.plan.occurrence_columns(occurrence)?.iter().copied() {
        let index = usize::from(column.storage_slot());
        if index >= MAX_COLUMNS {
            return Err(Error::Corrupt("native source slot bound"));
        }
        if demand & (1 << index) != 0 {
            capacities[index] = native_unit::column_capacity(column.data_type());
        }
    }
    let payload_bytes = capacities
        .iter()
        .try_fold(0_usize, |total, &capacity| total.checked_add(capacity))
        .ok_or(Error::Corrupt("native payload geometry"))?;
    let computation = BatchLayout::new(source)?;
    let branch_rows = BranchScratch::rows(source);
    let bytes = (FIXED_BYTES as u64)
        .checked_add(payload_bytes as u64)
        .and_then(|n| n.checked_add(batch_bytes))
        .and_then(|n| n.checked_add(computation.bytes()))
        .and_then(|n| n.checked_add(BranchScratch::bytes(branch_rows)))
        .ok_or(Error::Corrupt("native scan reservation overflow"))?;
    let mut reservation = database.reserve_memory(bytes, "native query workspace")?;
    let mut owner = allocate(1, 1, "native scan owner", bytes)?;
    let objects = crate::path::joined_path(database.path(), UNITS_NAME)?;
    owner.push(Scan {
        state: ScanState::Ready,
        unit: None,
        cursor: None,
        payloads: std::array::from_fn(|_| None),
        identities: [None; MAX_COLUMNS],
        kinds: [DataType::Double; MAX_COLUMNS],
        schema: [0; catalog_schema::MAX_BYTES],
        schema_len: 0,
        schema_crc: 0,
        table,
        objects,
        loaded: 0,
        rows: 0,
        database_id: database.database_identity(),
    });
    let scan = &mut owner[0];
    for (index, capacity) in capacities.iter().enumerate() {
        if demand & (1 << index) != 0 {
            let mut payload = allocate(*capacity, *capacity, "native query payload", bytes)?;
            payload.resize(*capacity, 0);
            scan.payloads[index] = Some(native_unit::ColumnBuffer::new(payload)?);
        }
    }
    Ok(Admission {
        query,
        workspace: AdmittedScan {
            input: OwnedBatch::new_with_text(&types[..count], &text[..count], &mut reservation)?,
            output: OwnedBatch::new_with_text(
                &output_types[..output_count],
                &output_text[..output_count],
                &mut reservation,
            )?,
            scan: ScanCursor {
                computation: BatchScratch::new(computation)?,
                source: Source::Declared(owner),
                selection: allocate(COMPUTE_ROWS, COMPUTE_ROWS, "native query selection", bytes)?,
                branches: BranchScratch::new(branch_rows, bytes)?,
                start: 0,
                end: 0,
                phase: ScanPhase::Begin,
                reservation,
            },
        },
        occurrence,
    })
}

impl<'db> Admission<'db> {
    pub(in crate::execution) fn open(
        mut self,
        scratch: &mut catalog::Scratch<'_>,
        cancel: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<AdmittedScan<'db>, Error> {
        let query = self.query;
        let occurrence = self.occurrence;
        let Source::Declared(owner) = &mut self.workspace.scan.source else {
            return Err(Error::Corrupt("native admission source kind"));
        };
        let scan = &mut owner[0];
        let table = scan.table;
        let snapshot = query
            .snapshot
            .as_ref()
            .ok_or(Error::Corrupt("catalog query pin absent"))?;
        let catalog = snapshot
            .read_catalog(scratch.bytes(), cancel, effects)?
            .ok_or(Error::Corrupt("pinned query catalog absent"))?;
        let ordinal = (0..catalog.len())
            .find(|&index| {
                catalog
                    .table(index)
                    .is_some_and(|entry| entry.id() == table)
            })
            .ok_or(Error::Corrupt("pinned query table absent"))?;
        let schema =
            catalog.read_schema(&scan.objects, ordinal, &mut scan.schema, cancel, effects)?;
        let mut sources = query.plan.occurrence_columns(occurrence)?.iter().copied();
        for index in 0..schema.len() {
            let declaration = schema.column(index).expect("validated schema ordinal");
            if index >= MAX_COLUMNS
                || !sources.next().is_some_and(|column| {
                    query
                        .plan
                        .matches_source_declaration(occurrence, column, declaration, index)
                })
            {
                return Err(Error::Corrupt("bound source differs from pinned schema"));
            }
            scan.identities[index] = Some(declaration.id());
            scan.kinds[index] = declaration.data_type();
        }
        if sources.next().is_some() {
            return Err(Error::Corrupt("bound source has extra columns"));
        }
        scan.schema_len = schema.encoded_bytes().len();
        scan.schema_crc = storage_format::crc32c(&scan.schema[..scan.schema_len]);
        scan.cursor = catalog.open_data(&scan.objects, ordinal, cancel, effects)?;
        Ok(self.workspace)
    }
}

#[cfg(test)]
pub(in crate::execution) fn open<'db>(
    database: &'db Database,
    query: &'db PreparedQuery<'_>,
    plan: &PhysicalPlan,
    cancel: &CancellationToken,
    effects: &mut Effects,
) -> Result<(AdmittedScan<'db>, Option<Aggregation<'db>>), Error> {
    let admitted = admit(
        database,
        query,
        plan,
        plan.scan(),
        plan.aggregate_columns().map(|_| plan.output()),
    )?;
    let mut scratch = catalog::Scratch::sized(
        &database.memory,
        catalog::MAX_BYTES,
        "native scan catalog admission",
    )?;
    let groups = query
        .plan
        .aggregates
        .first()
        .map(|aggregate| {
            Aggregation::open(
                database,
                true,
                aggregate,
                query.plan.aggregate_demand(0),
                plan.scan().output_columns(&query.plan),
                plan.output().output_columns(&query.plan),
            )
        })
        .transpose()?;
    Ok((admitted.open(&mut scratch, cancel, effects)?, groups))
}

#[cfg(test)]
mod tests {
    use crate::effects::{Effect, Effects, Faults};
    use crate::execution::{QueryResult, QueryStep};
    use crate::frontend::{self, DataType};
    use crate::{CancellationToken, Database, Error, Value, catalog, catalog_schema, native_unit};
    use std::path::PathBuf;

    #[test]
    fn native_query_read_faults_and_truncation_release_workspace() {
        struct Fixture(PathBuf);

        impl Drop for Fixture {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }

        fn drain(result: &mut QueryResult<'_, '_>, effects: &mut Effects) -> Result<Vec<i64>, ()> {
            let mut rows = Vec::new();
            for _ in 0..64 {
                match result.step_with_effects(effects) {
                    QueryStep::Progress => {}
                    QueryStep::Rows(batch) => {
                        for row in 0..batch.len() {
                            let Some(Value::Int64(value)) = batch.value(row, 0) else {
                                panic!("INT64");
                            };
                            rows.push(value);
                        }
                    }
                    QueryStep::Finished => {
                        rows.sort();
                        return Ok(rows);
                    }
                    QueryStep::Failed(_) => return Err(()),
                }
            }
            panic!("native query exceeded finite step budget");
        }
        let path = std::env::temp_dir().join(format!(
            "pipesql-native-query-faults-{}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        let fixture = Fixture(path);
        let db = Database::create_catalog_with_effects(
            &fixture.0.join("db"),
            crate::Config::new(2_000_000, 1_000_000).unwrap(),
            &mut Effects::default(),
        )
        .unwrap();
        let table = catalog_schema::TableId::new(19).unwrap();
        let id = catalog_schema::ColumnId::new(71).unwrap();
        let cancel = CancellationToken::new();
        let columns = [catalog_schema::ColumnSpec::new(id, "n", DataType::Int64, true).unwrap()];
        db.catalog_writer()
            .unwrap()
            .create_table("facts", table, &columns, &cancel, &mut Effects::default())
            .unwrap();
        db.catalog_writer()
            .unwrap()
            .append(
                table,
                &[native_unit::InputColumn {
                    id,
                    values: native_unit::InputValues::Int64(&[1, 2, 3]),
                    validity: &[7],
                }],
                &cancel,
                &mut Effects::default(),
            )
            .unwrap();
        for (sql, expected) in [
            ("FROM facts |> WHERE n > 1", &[2, 3][..]),
            ("FROM facts |> AGGREGATE SUM(n) AS total", &[6][..]),
        ] {
            let query = frontend::prepare_catalog(&db, sql).unwrap();
            let before = db.reserved_memory_bytes();
            let trace = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let captured = trace.clone();
            let mut baseline = Effects::with_faults(Faults {
                action: Some(Box::new(move |_, effect| {
                    captured.lock().unwrap().push(effect)
                })),
                ..Faults::default()
            });
            let mut result = db
                .execute_with_effects(&query, &cancel, &mut baseline)
                .unwrap();
            let minimum_peak = result.first_aggregate().map(|storage| {
                let groups = storage.dense();
                db.reserved_memory_bytes() - before + catalog::MAX_BYTES as u64
                    - groups.extra_expression_bytes()
            });
            assert_eq!(drain(&mut result, &mut baseline).unwrap(), expected);
            drop(result);
            assert_eq!(db.reserved_memory_bytes(), before);
            if let Some(minimum_peak) = minimum_peak {
                for extra in [0, 1] {
                    let pressure = db
                        .reserve_memory(
                            db.config().memory_limit_bytes() - before - minimum_peak + extra,
                            "test pressure",
                        )
                        .unwrap();
                    let mut effects = Effects::default();
                    let outcome = db.execute_with_effects(&query, &cancel, &mut effects);
                    if extra == 0 {
                        let mut result = outcome.unwrap();
                        assert_eq!(
                            result.first_aggregate().unwrap().dense().expression_lanes(),
                            1
                        );
                        assert_eq!(drain(&mut result, &mut effects).unwrap(), expected);
                    } else {
                        assert!(matches!(outcome, Err(Error::Resource { .. })));
                        assert_eq!(effects.count(), 0, "admission refusal precedes query I/O");
                    }
                    drop(pressure);
                    assert_eq!(db.reserved_memory_bytes(), before);
                }
            }
            assert!(baseline.count() > 0 && baseline.count() < 32);
            for cut in 0..baseline.count() {
                let mut effects = Effects::with_faults(Faults {
                    fail_at: Some(cut),
                    ..Faults::default()
                });
                match db.execute_with_effects(&query, &cancel, &mut effects) {
                    Ok(mut result) => {
                        assert!(drain(&mut result, &mut effects).is_err(), "fault cut {cut}");
                        let after = effects.count();
                        assert!(matches!(
                            result.step_with_effects(&mut effects),
                            QueryStep::Failed(Error::Io { .. })
                        ));
                        assert_eq!(effects.count(), after, "failed query is terminal");
                    }
                    Err(error) => assert!(
                        matches!(error, Error::Io { .. }),
                        "fault cut {cut}: {error:?}"
                    ),
                }
                assert!(effects.count() > cut, "injected fault was reached: {cut}");
                assert_eq!(db.reserved_memory_bytes(), before, "fault cut {cut}");
                if trace.lock().unwrap()[cut as usize]
                    == Effect::ReadMetadata(crate::effects::MetadataKind::CatalogObject)
                {
                    let mut effects = Effects::with_faults(Faults {
                        short_at: Some(cut),
                        ..Faults::default()
                    });
                    match db.execute_with_effects(&query, &cancel, &mut effects) {
                        Ok(mut result) => {
                            assert!(
                                drain(&mut result, &mut effects).is_err(),
                                "truncated read {cut}"
                            );
                            assert!(matches!(result.into_error(), Some(Error::Io { .. })));
                        }
                        Err(error) => assert!(
                            matches!(error, Error::Io { .. }),
                            "truncated read {cut}: {error:?}"
                        ),
                    }
                    assert!(effects.count() > cut);
                    assert_eq!(db.reserved_memory_bytes(), before, "truncated read {cut}");
                }
                let mut result = db.execute(&query, &cancel).unwrap();
                assert_eq!(
                    drain(&mut result, &mut Effects::default()).unwrap(),
                    expected
                );
                drop(result);
                assert_eq!(db.reserved_memory_bytes(), before, "healed read {cut}");
            }
        }
    }
}
