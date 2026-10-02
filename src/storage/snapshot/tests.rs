//! Construct catalog states for publication, recovery and snapshot tests.
//!
//! `Fixture::new` writes the control files and empty roots directly. `prepare`
//! builds a candidate catalog graph: schema, data units, data index, catalog and
//! success history. It syncs those objects without publishing the new root, so
//! child suites can interrupt publication or damage a selected object.
//!
//! The first schema and data unit come from retained fixture bytes. Later units,
//! indexes and roots use production encoders. Read helpers also use production
//! decoders; they expose cells and history for each suite to compare with its own
//! expectations. They are not independent checks of the entire file format.
//!
//! These helpers bypass normal creation and writer admission to reach states a
//! public call cannot produce. `heal_metadata` validates the selected graph before
//! repairing roots and the log, but does not perform the complete Database::open
//! procedure. The shared Directory owner removes files when the fixture drops.

use super::Snapshot;
use crate::ColumnValues;
use crate::DatabaseId;
use crate::effects::Effects;
use crate::storage::catalog::{self, ObjectId, ObjectRef, TableEntry};
use crate::storage::format;
use crate::storage::format::{CatalogCommit, Replica, Root};
use crate::storage::format::{RootState, WalRecord};
use crate::storage::history;
use crate::storage::publication::publish_snapshot;
use crate::storage::recovery::UNITS_NAME;
use crate::storage::recovery::{ROOT_A_NAME, ROOT_B_NAME, WAL_NAME};
use crate::storage::schema::{self, Schema, TableId};
use crate::storage::table;
use crate::storage::unit::{self, InputColumn, UnitRef, WriteBuffers};
use crate::test_support::Directory;
use crate::{CancellationToken, Error, TransactionId};

use pipesql_filesystem as filesystem;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

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
    schema::decode(
        include_bytes!("../../../test/data/catalog-schema/columns.bin"),
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

struct Fixture {
    directory: Directory,
}

impl Fixture {
    fn directory() -> Self {
        Self {
            directory: Directory::new(),
        }
    }

    fn root(&self) -> &Path {
        &self.directory.0
    }

    fn new() -> Self {
        let this = Self::directory();
        let root = this.root();
        fs::create_dir(root.join(UNITS_NAME)).unwrap();
        fs::create_dir(root.join(crate::storage::recovery::PRIVATE_NAME)).unwrap();
        this.put(&this.root().join(crate::storage::recovery::LOCK_NAME), &[]);
        this.put(
            &this.root().join(crate::storage::recovery::CONTROL_NAME),
            include_bytes!("../../../test/data/catalog-roots/CONTROL"),
        );
        for (name, replica) in [(ROOT_A_NAME, Replica::A), (ROOT_B_NAME, Replica::B)] {
            this.put(
                &this.root().join(name),
                &format::encode_root(Root {
                    database: database(),
                    issued: 2,
                    replica,
                    state: RootState::Catalog(None),
                })
                .unwrap(),
            );
        }
        this.put(
            &this.root().join(WAL_NAME),
            &format::encode_wal(genesis()).unwrap(),
        );
        this.sync(this.root());
        this
    }

    fn objects(&self) -> PathBuf {
        self.root().join(UNITS_NAME)
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

    // The caller has already published issuance for this attempt. With no
    // prior unit, install the retained four-row fixture; otherwise keep that
    // unit and add four new rows. Publication of the returned state is separate.
    fn prepare(&self, prior: WalRecord, first: Option<UnitRef>) -> (WalRecord, UnitRef) {
        let attempt = prior.issued;
        let cancel = CancellationToken::new();
        let mut effects = Effects::default();
        let schema_object = object(3, 1);
        if first.is_none() {
            self.put(
                &self.path(schema_object),
                include_bytes!("../../../test/data/catalog-schema/columns.bin"),
            );
        }
        let unit = if first.is_none() {
            let unit = UnitRef::new(object(3, 2), 4, 192, 0xcdd46886).unwrap();
            self.put(
                &self.path(unit.object()),
                include_bytes!("../../../test/data/catalog-schema/native-unit.bin"),
            );
            unit
        } else {
            let file = self.create(object(attempt, 2));
            let mut metadata = [0; unit::MAX_METADATA_BYTES];
            let mut column = vec![0; unit::MAX_COLUMN_BYTES];
            let inputs = [
                InputColumn {
                    id: schema::ColumnId::new(3).unwrap(),
                    values: ColumnValues::Double(&[10., 20., 30., 40.]),
                    validity: &[15],
                },
                InputColumn {
                    id: schema::ColumnId::new(29).unwrap(),
                    values: ColumnValues::String(&["new", "", "雪", "unused"]),
                    validity: &[7],
                },
            ];
            let unit = unit::write(
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
        let data = table::write(
            &data_file,
            object(attempt, 3),
            &schema(),
            &units,
            &mut [0; table::SCRATCH_BYTES],
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
        let mut scratch = vec![0; history::SCRATCH_BYTES];
        let history = history::append(
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
        let a_path = self.root().join(ROOT_A_NAME);
        let b_path = self.root().join(ROOT_B_NAME);
        let a = crate::storage::recovery::read_root_file(
            &a_path,
            filesystem::symlink_metadata(&a_path).unwrap().identity(),
            database(),
            Replica::A,
            &mut effects,
            crate::effects::MetadataKind::RootA,
        )
        .unwrap();
        let b = crate::storage::recovery::read_root_file(
            &b_path,
            filesystem::symlink_metadata(&b_path).unwrap().identity(),
            database(),
            Replica::B,
            &mut effects,
            crate::effects::MetadataKind::RootB,
        )
        .unwrap();
        let fence = crate::storage::recovery::read_fence(
            self.root(),
            filesystem::symlink_metadata(self.root().join(WAL_NAME))
                .unwrap()
                .identity(),
            database(),
            &mut effects,
        )
        .unwrap();
        format::select_snapshot(a, b, fence).unwrap()
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
        crate::storage::recovery::reconcile_roots(
            self.root(),
            database(),
            selected.issued,
            selected.state,
            repair,
            [
                self.root().join("ROOT.A.next").exists(),
                self.root().join("ROOT.B.next").exists(),
            ],
            effects,
        )?;
        let metadata = filesystem::symlink_metadata(self.root().join(WAL_NAME)).unwrap();
        crate::storage::recovery::reconcile_fence(
            self.root(),
            metadata.identity(),
            metadata.len(),
            &format::encode_wal(selected).unwrap(),
            effects,
        )?;
        assert_eq!(self.selected(), (selected, None));
        Ok(selected)
    }

    // Walk the catalog through its data index and decode the text column of
    // every unit. Check the total rows; expected cell values belong to callers.
    fn assert_graph(&self, snapshot: WalRecord, rows: u64) {
        let graph = graph(snapshot);
        let cancel = CancellationToken::new();
        let mut effects = Effects::default();
        let mut catalog_bytes = [0; catalog::MAX_BYTES];
        let mut schema_bytes = [0; schema::MAX_BYTES];
        let mut payload = vec![0; unit::MAX_COLUMN_BYTES];
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
            let unit = unit::read(
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
                    schema::ColumnId::new(29).unwrap(),
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
        history::find(
            &self.objects(),
            snapshot,
            token(sequence),
            &mut vec![0; history::SCRATCH_BYTES],
            &CancellationToken::new(),
            &mut Effects::default(),
        )
        .unwrap()
    }
}

fn publish(fixture: &Fixture, old: WalRecord, new: WalRecord) {
    if let Err(failure) = publish_snapshot(
        fixture.root(),
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

// Leave one issued attempt without a commit before issuing the next one. This
// gives history tests an aborted attempt between two successful publications.
fn second_admission(fixture: &Fixture, first: WalRecord) -> WalRecord {
    let abort = issuance(first);
    publish(fixture, first, abort);
    let issued = issuance(abort);
    publish(fixture, abort, issued);
    issued
}

// Copy text before reusing the payload buffer for DOUBLE. Retain floating-point
// bits so child tests can distinguish signed zeros and NaN representations.
fn snapshot_cells(snapshot: &Snapshot<'_>, objects: &Path) -> Vec<(Option<u64>, Option<String>)> {
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let mut catalog_bytes = [0; catalog::MAX_BYTES];
    let catalog = snapshot
        .read_catalog(&mut catalog_bytes, &cancel, &mut effects)
        .unwrap()
        .unwrap();
    let mut schema_bytes = [0; schema::MAX_BYTES];
    let schema = catalog
        .read_schema(objects, 0, &mut schema_bytes, &cancel, &mut effects)
        .unwrap();
    let mut cursor = catalog
        .open_data(objects, 0, &cancel, &mut effects)
        .unwrap()
        .unwrap();
    let mut payload = vec![0; unit::MAX_COLUMN_BYTES];
    let mut values = Vec::new();
    while let Some(reference) = cursor.next(&cancel, &mut effects).unwrap() {
        let unit = unit::read(
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
                schema::ColumnId::new(29).unwrap(),
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
                schema::ColumnId::new(3).unwrap(),
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

fn declarations() -> [schema::ColumnSpec<'static>; 2] {
    [schema().column(0).unwrap(), schema().column(1).unwrap()]
}

fn append_columns() -> [InputColumn<'static>; 2] {
    // Supply text before DOUBLE, opposite to schema order. Appending must map
    // these columns by their stable IDs, not by positions in this array.
    [
        InputColumn {
            id: schema::ColumnId::new(29).unwrap(),
            values: ColumnValues::String(&["雪", "", "abc", "ignored"]),
            validity: &[7],
        },
        InputColumn {
            id: schema::ColumnId::new(3).unwrap(),
            values: ColumnValues::Double(&[1., -0., f64::NAN, 42.]),
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
