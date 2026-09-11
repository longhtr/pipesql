//! Shared on-disk catalog fixtures and value readers. Contract modules own their
//! expectations and fault schedules; these helpers construct and inspect inputs.
use super::Snapshot;
use crate::DatabaseId;
use crate::catalog::{self, ObjectId, ObjectRef, TableEntry};
use crate::catalog_schema::{self, Schema, TableId};
use crate::effects::Effects;
use crate::namespace::UNITS_NAME;
use crate::namespace::{ROOT_A_NAME, ROOT_B_NAME, WAL_NAME};
use crate::native_unit::{self, InputColumn, InputValues, UnitRef, WriteBuffers};
use crate::publication::publish_snapshot;
use crate::storage_format;
use crate::storage_format::{CatalogCommit, Replica, Root};
use crate::storage_format::{RootState, WalRecord};
use crate::{CancellationToken, Error, TransactionId};
use crate::{success_index, table_data};
use pipesql_filesystem as filesystem;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

fn database() -> DatabaseId {
    DatabaseId::new([7; 16]).unwrap()
}

fn object(attempt: u64, ordinal: u32) -> ObjectId {
    ObjectId::new(attempt, ordinal).unwrap()
}

fn token(sequence: u64) -> TransactionId {
    TransactionId::for_attempt(database(), sequence).unwrap()
}

fn schema() -> Schema<'static> {
    catalog_schema::decode(
        include_bytes!("../../tests/fixtures/catalog-schema/columns.bin"),
        database(),
        TableId::new(0x0102030405060708).unwrap(),
        0x666a62b7,
    )
    .unwrap()
}

fn genesis() -> WalRecord {
    WalRecord {
        database: database(),
        issued: 2,
        state: RootState::Catalog(None),
    }
}

fn issuance(prior: WalRecord) -> WalRecord {
    WalRecord {
        issued: prior.issued + 1,
        ..prior
    }
}

fn graph(snapshot: WalRecord) -> CatalogCommit {
    let RootState::Catalog(Some(graph)) = snapshot.state else {
        panic!("committed graph required")
    };
    graph
}

struct Fixture(PathBuf);

impl Fixture {
    fn directory() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "pipesql-catalog-publication-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn new() -> Self {
        let this = Self::directory();
        let root = &this.0;
        fs::create_dir(root.join(UNITS_NAME)).unwrap();
        fs::create_dir(root.join(crate::namespace::PRIVATE_NAME)).unwrap();
        this.put(&this.0.join(crate::namespace::LOCK_NAME), &[]);
        this.put(
            &this.0.join(crate::namespace::CONTROL_NAME),
            include_bytes!("../../tests/fixtures/catalog-roots/CONTROL"),
        );
        for (name, replica) in [(ROOT_A_NAME, Replica::A), (ROOT_B_NAME, Replica::B)] {
            this.put(
                &this.0.join(name),
                &storage_format::encode_root(Root {
                    database: database(),
                    issued: 2,
                    replica,
                    state: RootState::Catalog(None),
                })
                .unwrap(),
            );
        }
        this.put(
            &this.0.join(WAL_NAME),
            &storage_format::encode_wal(genesis()).unwrap(),
        );
        this.sync(&this.0);
        this
    }

    fn objects(&self) -> PathBuf {
        self.0.join(UNITS_NAME)
    }

    fn path(&self, id: ObjectId) -> PathBuf {
        self.objects()
            .join(std::str::from_utf8(&id.name()).unwrap())
    }

    fn put(&self, path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).unwrap();
        filesystem::sync_all(&filesystem::open_read(path).unwrap()).unwrap();
    }

    fn sync(&self, path: &Path) {
        filesystem::sync_all(&filesystem::open_read(path).unwrap()).unwrap();
    }

    fn create(&self, id: ObjectId) -> File {
        filesystem::create_new_read_write(self.path(id)).unwrap()
    }

    fn prepare(&self, prior: WalRecord, first: Option<UnitRef>) -> (WalRecord, UnitRef) {
        let attempt = prior.issued;
        let cancel = CancellationToken::new();
        let mut effects = Effects::default();
        let schema_object = object(3, 1);
        if first.is_none() {
            self.put(
                &self.path(schema_object),
                include_bytes!("../../tests/fixtures/catalog-schema/columns.bin"),
            );
        }
        let unit = if first.is_none() {
            let unit = UnitRef::new(object(3, 2), 4, 192, 0xcdd46886).unwrap();
            self.put(
                &self.path(unit.object()),
                include_bytes!("../../tests/fixtures/catalog-schema/native-unit.bin"),
            );
            unit
        } else {
            let file = self.create(object(attempt, 2));
            let mut metadata = [0; native_unit::MAX_METADATA_BYTES];
            let mut column = vec![0; native_unit::MAX_COLUMN_BYTES];
            let inputs = [
                InputColumn {
                    id: catalog_schema::ColumnId::new(3).unwrap(),
                    values: InputValues::Double(&[10., 20., 30., 40.]),
                    validity: &[15],
                },
                InputColumn {
                    id: catalog_schema::ColumnId::new(29).unwrap(),
                    values: InputValues::String(&["new", "", "雪", "unused"]),
                    validity: &[7],
                },
            ];
            let unit = native_unit::write(
                &file,
                object(attempt, 2),
                &schema(),
                &inputs,
                WriteBuffers {
                    metadata: &mut metadata,
                    column: &mut column,
                },
                &cancel,
                &mut effects,
            )
            .unwrap();
            filesystem::sync_all(&file).unwrap();
            unit
        };
        let units = if let Some(first) = first {
            vec![first, unit]
        } else {
            vec![unit]
        };
        let data_file = self.create(object(attempt, 3));
        let data = table_data::write(
            &data_file,
            object(attempt, 3),
            &schema(),
            &units,
            &mut [0; table_data::SCRATCH_BYTES],
            &cancel,
            &mut effects,
        )
        .unwrap();
        filesystem::sync_all(&data_file).unwrap();
        let entry = TableEntry::new(
            schema().table(),
            "facts",
            ObjectRef::new(schema_object, 160, 0x666a62b7).unwrap(),
            Some(data),
            4 * units.len() as u64,
            units.len() as u32,
        )
        .unwrap();
        let mut bytes = [0; catalog::MAX_BYTES];
        let catalog =
            catalog::encode(&mut bytes, database(), object(attempt, 4), &[entry]).unwrap();
        self.put(
            &self.path(catalog.object()),
            &bytes[..catalog.bytes() as usize],
        );
        let history_file = self.create(object(attempt, 5));
        let mut scratch = vec![0; success_index::SCRATCH_BYTES];
        let history = success_index::append(
            &history_file,
            &self.objects(),
            prior,
            object(attempt, 5),
            &mut scratch,
            &cancel,
            &mut effects,
        )
        .unwrap();
        filesystem::sync_all(&history_file).unwrap();
        self.sync(&self.objects());
        let generation = match prior.state {
            RootState::Catalog(Some(old)) => old.generation() + 1,
            RootState::Catalog(None) => 1,
            _ => panic!("catalog prior"),
        };
        let committed =
            CatalogCommit::new(database(), generation, token(attempt), catalog, history).unwrap();
        (
            WalRecord {
                state: RootState::Catalog(Some(committed)),
                ..prior
            },
            unit,
        )
    }

    fn selected(&self) -> (WalRecord, Option<Replica>) {
        let mut effects = Effects::default();
        let a_path = self.0.join(ROOT_A_NAME);
        let b_path = self.0.join(ROOT_B_NAME);
        let a = crate::namespace::read_root_file(
            &a_path,
            filesystem::symlink_metadata(&a_path).unwrap().identity(),
            database(),
            Replica::A,
            &mut effects,
            crate::effects::MetadataKind::RootA,
        )
        .unwrap();
        let b = crate::namespace::read_root_file(
            &b_path,
            filesystem::symlink_metadata(&b_path).unwrap().identity(),
            database(),
            Replica::B,
            &mut effects,
            crate::effects::MetadataKind::RootB,
        )
        .unwrap();
        let fence = crate::namespace::read_fence(
            &self.0,
            filesystem::symlink_metadata(self.0.join(WAL_NAME))
                .unwrap()
                .identity(),
            database(),
            &mut effects,
        )
        .unwrap();
        storage_format::select_snapshot(a, b, fence).unwrap()
    }

    fn heal_metadata(&self, effects: &mut Effects) -> Result<WalRecord, Error> {
        let (selected, repair) = self.selected();
        catalog::validate_snapshot(
            &self.objects(),
            selected,
            &mut vec![0; catalog::SNAPSHOT_SCRATCH_BYTES],
            &CancellationToken::new(),
            effects,
        )?;
        crate::namespace::reconcile_roots(
            &self.0,
            database(),
            selected.issued,
            selected.state,
            repair,
            [
                self.0.join("ROOT.A.next").exists(),
                self.0.join("ROOT.B.next").exists(),
            ],
            effects,
        )?;
        let metadata = filesystem::symlink_metadata(self.0.join(WAL_NAME)).unwrap();
        crate::namespace::reconcile_fence(
            &self.0,
            metadata.identity(),
            metadata.len(),
            &storage_format::encode_wal(selected).unwrap(),
            effects,
        )?;
        assert_eq!(self.selected(), (selected, None));
        Ok(selected)
    }

    fn assert_graph(&self, snapshot: WalRecord, rows: u64) {
        let graph = graph(snapshot);
        let cancel = CancellationToken::new();
        let mut effects = Effects::default();
        let mut catalog_bytes = [0; catalog::MAX_BYTES];
        let mut schema_bytes = [0; catalog_schema::MAX_BYTES];
        let mut payload = vec![0; native_unit::MAX_COLUMN_BYTES];
        let catalog = catalog::read(
            &self.objects(),
            database(),
            graph.catalog(),
            &mut catalog_bytes,
            &cancel,
            &mut effects,
        )
        .unwrap();
        let schema = catalog
            .read_schema(&self.objects(), 0, &mut schema_bytes, &cancel, &mut effects)
            .unwrap();
        let mut cursor = catalog
            .open_data(&self.objects(), 0, &cancel, &mut effects)
            .unwrap()
            .unwrap();
        let mut observed = 0;
        while let Some(reference) = cursor.next(&cancel, &mut effects).unwrap() {
            let unit = native_unit::read(
                &self.objects(),
                database(),
                reference,
                &schema,
                &cancel,
                &mut effects,
            )
            .unwrap();
            let text = unit
                .read_column(
                    catalog_schema::ColumnId::new(29).unwrap(),
                    &mut payload,
                    &cancel,
                    &mut effects,
                )
                .unwrap();
            for row in 0..reference.rows() as usize {
                text.string(row).unwrap();
            }
            observed += u64::from(reference.rows());
        }
        assert_eq!(observed, rows);
    }

    fn find(&self, snapshot: WalRecord, sequence: u64) -> Option<u64> {
        success_index::find(
            &self.objects(),
            snapshot,
            token(sequence),
            &mut vec![0; success_index::SCRATCH_BYTES],
            &CancellationToken::new(),
            &mut Effects::default(),
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn publish(fixture: &Fixture, old: WalRecord, new: WalRecord) {
    if let Err(failure) = publish_snapshot(
        &fixture.0,
        old,
        new,
        &CancellationToken::new(),
        &mut Effects::default(),
    ) {
        panic!("publication failed: {}", failure.error)
    }
}

fn first_commit(fixture: &Fixture) -> (WalRecord, UnitRef) {
    let issued = issuance(genesis());
    publish(fixture, genesis(), issued);
    let (first, unit) = fixture.prepare(issued, None);
    publish(fixture, issued, first);
    (first, unit)
}

fn second_admission(fixture: &Fixture, first: WalRecord) -> WalRecord {
    let abort = issuance(first);
    publish(fixture, first, abort);
    let issued = issuance(abort);
    publish(fixture, abort, issued);
    issued
}

fn snapshot_cells(snapshot: &Snapshot<'_>, objects: &Path) -> Vec<(Option<u64>, Option<String>)> {
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let mut catalog_bytes = [0; catalog::MAX_BYTES];
    let catalog = snapshot
        .read_catalog(&mut catalog_bytes, &cancel, &mut effects)
        .unwrap()
        .unwrap();
    let mut schema_bytes = [0; catalog_schema::MAX_BYTES];
    let schema = catalog
        .read_schema(objects, 0, &mut schema_bytes, &cancel, &mut effects)
        .unwrap();
    let mut cursor = catalog
        .open_data(objects, 0, &cancel, &mut effects)
        .unwrap()
        .unwrap();
    let mut payload = vec![0; native_unit::MAX_COLUMN_BYTES];
    let mut values = Vec::new();
    while let Some(reference) = cursor.next(&cancel, &mut effects).unwrap() {
        let unit = native_unit::read(
            objects,
            schema.database(),
            reference,
            &schema,
            &cancel,
            &mut effects,
        )
        .unwrap();
        let column = unit
            .read_column(
                catalog_schema::ColumnId::new(29).unwrap(),
                &mut payload,
                &cancel,
                &mut effects,
            )
            .unwrap();
        let start = values.len();
        for row in 0..reference.rows() as usize {
            values.push((None, column.string(row).unwrap().map(str::to_owned)));
        }
        let column = unit
            .read_column(
                catalog_schema::ColumnId::new(3).unwrap(),
                &mut payload,
                &cancel,
                &mut effects,
            )
            .unwrap();
        for (row, value) in values[start..].iter_mut().enumerate() {
            value.0 = column.double(row).unwrap().map(f64::to_bits);
        }
    }
    assert_eq!(values.len() as u64, catalog.table(0).unwrap().rows());
    values
}

fn snapshot_rows(snapshot: &Snapshot<'_>, objects: &Path) -> u64 {
    snapshot_cells(snapshot, objects).len() as u64
}

fn declarations() -> [catalog_schema::ColumnSpec<'static>; 2] {
    [schema().column(0).unwrap(), schema().column(1).unwrap()]
}

fn append_columns() -> [InputColumn<'static>; 2] {
    // Reordered inputs must bind by stable column identity, not physical position.
    [
        InputColumn {
            id: catalog_schema::ColumnId::new(29).unwrap(),
            values: InputValues::String(&["雪", "", "abc", "ignored"]),
            validity: &[7],
        },
        InputColumn {
            id: catalog_schema::ColumnId::new(3).unwrap(),
            values: InputValues::Double(&[1., -0., f64::NAN, 42.]),
            validity: &[15],
        },
    ]
}

mod append;
mod declarations;
mod namespace;
mod publication;
mod queries;
mod snapshots;
