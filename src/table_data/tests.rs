use super::*;
use crate::catalog_schema::{self, TableId};
use crate::effects::Faults;
use crate::native_unit::{self, InputColumn, InputValues, WriteBuffers};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn database() -> DatabaseId {
    DatabaseId::new([7; 16]).unwrap()
}

fn object(attempt: u64, ordinal: u32) -> ObjectId {
    ObjectId::new(attempt, ordinal).unwrap()
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

fn entry(reference: Option<ObjectRef>, rows: u64, units: u32) -> TableEntry<'static> {
    TableEntry::new(
        schema().table(),
        "facts",
        ObjectRef::new(object(1, 1), 160, 0x666a62b7).unwrap(),
        reference,
        rows,
        units,
    )
    .unwrap()
}

fn unit(ordinal: u32) -> UnitRef {
    UnitRef::new(object(2, ordinal), 1, 105, 7).unwrap()
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pipesql-table-data-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, object: ObjectId) -> PathBuf {
        self.0.join(std::str::from_utf8(&object.name()).unwrap())
    }

    fn create(&self, object: ObjectId) -> File {
        pipesql_filesystem::create_new_read_write(self.path(object)).unwrap()
    }

    fn index(&self, object: ObjectId, units: &[UnitRef]) -> ObjectRef {
        let file = self.create(object);
        write(
            &file,
            object,
            &schema(),
            units,
            &mut [0; SCRATCH_BYTES],
            &CancellationToken::new(),
            &mut Effects::default(),
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn catalog_schema_index_and_two_real_units_preserve_shared_data() {
    let fixture = Fixture::new();
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let first = UnitRef::new(object(3, 2), 4, 192, 0xcdd46886).unwrap();
    std::fs::write(
        fixture.path(first.object()),
        include_bytes!("../../tests/fixtures/catalog-schema/native-unit.bin"),
    )
    .unwrap();
    std::fs::write(
        fixture.path(object(1, 1)),
        include_bytes!("../../tests/fixtures/catalog-schema/columns.bin"),
    )
    .unwrap();
    let old = fixture.index(object(4, 1), &[first]);
    let mut retained = open(
        &fixture.0,
        database(),
        entry(Some(old), 4, 1),
        &cancel,
        &mut effects,
    )
    .unwrap()
    .unwrap();
    let mut metadata = [0; native_unit::MAX_METADATA_BYTES];
    let mut payload = vec![0; native_unit::MAX_COLUMN_BYTES];
    let columns = [
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
    let second = native_unit::write(
        &fixture.create(object(5, 1)),
        object(5, 1),
        &schema(),
        &columns,
        WriteBuffers {
            metadata: &mut metadata,
            column: &mut payload,
        },
        &cancel,
        &mut effects,
    )
    .unwrap();
    let current = fixture.index(object(6, 1), &[first, second]);
    let mut catalog_buffer = [0; catalog::MAX_BYTES];
    let catalog_ref = catalog::encode(
        &mut catalog_buffer,
        database(),
        object(7, 1),
        &[entry(Some(current), 8, 2)],
    )
    .unwrap();
    std::fs::write(
        fixture.path(catalog_ref.object()),
        &catalog_buffer[..catalog_ref.bytes() as usize],
    )
    .unwrap();
    let catalog = catalog::read(
        &fixture.0,
        database(),
        catalog_ref,
        &mut catalog_buffer,
        &cancel,
        &mut effects,
    )
    .unwrap();
    let mut schema_buffer = [0; catalog_schema::MAX_BYTES];
    let actual_schema = catalog
        .read_schema(&fixture.0, 0, &mut schema_buffer, &cancel, &mut effects)
        .unwrap();
    let mut cursor = catalog
        .open_data(&fixture.0, 0, &cancel, &mut effects)
        .unwrap()
        .unwrap();
    assert_eq!(cursor.next(&cancel, &mut effects).unwrap(), Some(first));
    assert_eq!(cursor.next(&cancel, &mut effects).unwrap(), Some(second));
    assert!(cursor.next(&cancel, &mut effects).unwrap().is_none());
    assert_eq!(retained.next(&cancel, &mut effects).unwrap(), Some(first));
    assert!(retained.next(&cancel, &mut effects).unwrap().is_none());
    for (reference, expected) in [(first, -0.0_f64), (second, 10.0_f64)] {
        let unit = native_unit::read(
            &fixture.0,
            database(),
            reference,
            &actual_schema,
            &cancel,
            &mut effects,
        )
        .unwrap();
        let value = unit
            .read_column(
                catalog_schema::ColumnId::new(3).unwrap(),
                &mut payload,
                &cancel,
                &mut effects,
            )
            .unwrap()
            .double(0)
            .unwrap()
            .unwrap();
        assert_eq!(value.to_bits(), expected.to_bits());
    }
    assert_eq!(
        std::fs::read(fixture.path(first.object())).unwrap(),
        include_bytes!("../../tests/fixtures/catalog-schema/native-unit.bin")
    );
}

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn rewind_rechecks_pages_and_never_revives_a_failed_cursor() {
    let fixture = Fixture::new();
    let cancel = CancellationToken::new();
    let units = [unit(1), unit(2), unit(3)];
    let reference = fixture.index(object(5, 1), &units);
    let mut effects = Effects::default();
    let mut cursor = open(
        &fixture.0,
        database(),
        entry(Some(reference), 3, 3),
        &cancel,
        &mut effects,
    )
    .unwrap()
    .unwrap();
    for expected in units {
        assert_eq!(cursor.next(&cancel, &mut effects).unwrap(), Some(expected));
    }
    assert!(cursor.next(&cancel, &mut effects).unwrap().is_none());
    cursor.rewind().unwrap();
    assert_eq!(cursor.next(&cancel, &mut effects).unwrap(), Some(units[0]));
    assert_eq!(cursor.loaded_page, Some(0));
    let path = fixture.path(reference.object());
    let mut changed = std::fs::read(&path).unwrap();
    changed[HEADER as usize + 3] ^= 1;
    std::fs::write(&path, changed).unwrap();
    cursor.rewind().unwrap();
    assert_eq!(cursor.loaded_page, None);
    assert!(matches!(
        cursor.next(&cancel, &mut effects),
        Err(Error::Corrupt("table-data page changed after admission"))
    ));
    assert!(cursor.rewind().is_err());
    let at = effects.count();
    assert!(cursor.next(&cancel, &mut effects).is_err());
    assert_eq!(effects.count(), at);
}

#[test]
fn exact_index_capacity_streams_with_fixed_scratch() {
    let fixture = Fixture::new();
    let cancel = CancellationToken::new();
    let units: Vec<_> = (1..=MAX_UNITS).map(unit).collect();
    let reference = fixture.index(object(5, 1), &units);
    assert_eq!(reference.bytes(), HEADER + MAX_UNITS * ENTRY_BYTES);
    let mut buffer = [0; SCRATCH_BYTES];
    let mut effects = Effects::default();
    let mut cursor = open(
        &fixture.0,
        database(),
        entry(Some(reference), u64::from(MAX_UNITS), MAX_UNITS),
        &cancel,
        &mut effects,
    )
    .unwrap()
    .unwrap();
    assert_eq!(effects.count(), 4 + MAX_PAGES as u64);
    for expected in &units {
        assert_eq!(cursor.next(&cancel, &mut effects).unwrap(), Some(*expected));
    }
    assert!(cursor.next(&cancel, &mut effects).unwrap().is_none());
    let count = effects.count();
    assert!(cursor.next(&cancel, &mut effects).unwrap().is_none());
    assert_eq!(effects.count(), count);
    assert_eq!(count, 4 + 2 * MAX_PAGES as u64);
    let file = fixture.create(object(6, 1));
    let mut extra = units;
    extra.push(unit(MAX_UNITS + 1));
    let mut effects = Effects::default();
    assert!(matches!(
        write(
            &file,
            object(6, 1),
            &schema(),
            &extra,
            &mut buffer,
            &cancel,
            &mut effects
        ),
        Err(Error::Resource {
            required: 4097,
            limit: 4096,
            ..
        })
    ));
    assert_eq!(effects.count(), 0);
    assert_eq!(file.metadata().unwrap().len(), 0);
}

#[test]
fn empty_catalog_table_needs_no_index_or_scratch() {
    let mut effects = Effects::default();
    let cancel = CancellationToken::new();
    assert!(
        open(
            Path::new("/missing"),
            database(),
            entry(None, 0, 0),
            &cancel,
            &mut effects
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(effects.count(), 0);
}

#[test]
fn admission_and_late_corruption_fail_closed() {
    let fixture = Fixture::new();
    let reference = fixture.index(object(5, 1), &[unit(1), unit(2)]);
    let original = std::fs::read(fixture.path(reference.object())).unwrap();
    let cancel = CancellationToken::new();
    for (at, value) in [
        (12, 1),
        (16, 8),
        (32, 1),
        (40, 6),
        (48, 2),
        (52, 1),
        (56, 3),
        (64, 6),
        (72, 0),
        (76, 0),
        (80, 0),
        (88, 1),
        (96, 1),
        (112, 1),
        (136, 0),
    ] {
        let mut bad = original.clone();
        assert_ne!(bad[at], value, "mutation {at}");
        bad[at] = value;
        let forged = ObjectRef::new(reference.object(), reference.bytes(), crc32c(&bad)).unwrap();
        std::fs::write(fixture.path(reference.object()), bad).unwrap();
        assert!(
            open(
                &fixture.0,
                database(),
                entry(Some(forged), 2, 2),
                &cancel,
                &mut Effects::default()
            )
            .is_err(),
            "mutation {at}"
        );
    }
    std::fs::write(fixture.path(reference.object()), &original).unwrap();
    let mut cursor = open(
        &fixture.0,
        database(),
        entry(Some(reference), 2, 2),
        &cancel,
        &mut Effects::default(),
    )
    .unwrap()
    .unwrap();
    let mut changed = original.clone();
    changed[84] ^= 1;
    std::fs::write(fixture.path(reference.object()), changed).unwrap();
    assert!(matches!(
        cursor.next(&cancel, &mut Effects::default()),
        Err(Error::Corrupt(_))
    ));
    let mut effects = Effects::default();
    assert!(cursor.next(&cancel, &mut effects).is_err());
    assert_eq!(effects.count(), 0);
    drop(cursor);
    for length in 0..original.len() {
        std::fs::write(fixture.path(reference.object()), &original[..length]).unwrap();
        assert!(
            open(
                &fixture.0,
                database(),
                entry(Some(reference), 2, 2),
                &cancel,
                &mut Effects::default()
            )
            .is_err()
        );
    }
}

#[test]
fn invalid_constructor_input_leaves_private_file_empty() {
    let fixture = Fixture::new();
    let file = fixture.create(object(5, 1));
    let cancel = CancellationToken::new();
    let mut buffer = [0; SCRATCH_BYTES];
    for units in [
        vec![unit(1), unit(1)],
        vec![unit(2), unit(1)],
        vec![UnitRef::new(object(5, 1), 1, 105, 1).unwrap()],
        vec![UnitRef::new(object(6, 1), 1, 105, 1).unwrap()],
    ] {
        let mut effects = Effects::default();
        assert!(
            write(
                &file,
                object(5, 1),
                &schema(),
                &units,
                &mut buffer,
                &cancel,
                &mut effects
            )
            .is_err()
        );
        assert_eq!(effects.count(), 0);
        assert_eq!(file.metadata().unwrap().len(), 0);
    }
    let mut effects = Effects::default();
    assert!(matches!(
        write(
            &file,
            object(5, 1),
            &schema(),
            &[unit(1)],
            &mut buffer[..SCRATCH_BYTES - 1],
            &cancel,
            &mut effects
        ),
        Err(Error::Resource { .. })
    ));
    assert_eq!(effects.count(), 0);
}

#[test]
fn independent_complete_graph_matches_index_writer_and_readers() {
    let fixture = Fixture::new();
    let expected = include_bytes!("../../tests/fixtures/catalog-schema/table-data.bin");
    let catalog_bytes = include_bytes!("../../tests/fixtures/catalog-schema/data-catalog.bin");
    let first = UnitRef::new(object(3, 2), 4, 192, 0xcdd46886).unwrap();
    let reference = fixture.index(object(4, 2), &[first]);
    assert_eq!(reference.checksum(), 0x9cc59473);
    assert_eq!(
        std::fs::read(fixture.path(reference.object())).unwrap(),
        expected
    );
    for (object, bytes) in [
        (
            object(1, 1),
            include_bytes!("../../tests/fixtures/catalog-schema/columns.bin").as_slice(),
        ),
        (
            object(3, 2),
            include_bytes!("../../tests/fixtures/catalog-schema/native-unit.bin").as_slice(),
        ),
        (object(5, 1), catalog_bytes.as_slice()),
    ] {
        std::fs::write(fixture.path(object), bytes).unwrap();
    }
    let mut catalog_buffer = [0; catalog::MAX_BYTES];
    let mut schema_buffer = [0; catalog_schema::MAX_BYTES];
    let mut payload = [0; 64];
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let encoded = catalog::encode(
        &mut catalog_buffer,
        database(),
        object(5, 1),
        &[entry(Some(reference), 4, 1)],
    )
    .unwrap();
    assert_eq!(encoded.checksum(), 0x99e285f4);
    assert_eq!(&catalog_buffer[..encoded.bytes() as usize], catalog_bytes);
    let catalog = catalog::read(
        &fixture.0,
        database(),
        encoded,
        &mut catalog_buffer,
        &cancel,
        &mut effects,
    )
    .unwrap();
    let schema = catalog
        .read_schema(&fixture.0, 0, &mut schema_buffer, &cancel, &mut effects)
        .unwrap();
    let mut cursor = catalog
        .open_data(&fixture.0, 0, &cancel, &mut effects)
        .unwrap()
        .unwrap();
    let reference = cursor.next(&cancel, &mut effects).unwrap().unwrap();
    assert_eq!(reference, first);
    let unit = native_unit::read(
        &fixture.0,
        database(),
        reference,
        &schema,
        &cancel,
        &mut effects,
    )
    .unwrap();
    let strings = unit
        .read_column(
            catalog_schema::ColumnId::new(29).unwrap(),
            &mut payload,
            &cancel,
            &mut effects,
        )
        .unwrap();
    assert_eq!(strings.string(0).unwrap(), None);
    assert_eq!(strings.string(1).unwrap(), Some(""));
    assert_eq!(strings.string(3).unwrap(), Some("é\0🙂"));
    assert!(cursor.next(&cancel, &mut effects).unwrap().is_none());
}

#[test]
fn failed_and_cancelled_io_never_becomes_successful_completion() {
    let cancel = CancellationToken::new();
    let units = [unit(1)];
    for (mode, cut) in (0..6)
        .map(|cut| (0, cut))
        .chain((1..5).map(|cut| (1, cut)))
        .chain((0..6).map(|cut| (2, cut)))
    {
        let fixture = Fixture::new();
        let file = fixture.create(object(5, 1));
        let mut buffer = [0; SCRATCH_BYTES];
        let token = std::sync::Arc::new(CancellationToken::new());
        let peer = token.clone();
        let mut effects = match mode {
            0 => Effects::with_faults(Faults {
                fail_at: Some(cut),
                ..Faults::default()
            }),
            1 => Effects::with_faults(Faults {
                short_at: Some(cut),
                ..Faults::default()
            }),
            _ => Effects::with_faults(Faults {
                action: Some(Box::new(move |index, _| {
                    if index == cut {
                        peer.cancel();
                    }
                })),
                ..Faults::default()
            }),
        };
        let result = write(
            &file,
            object(5, 1),
            &schema(),
            &units,
            &mut buffer,
            &token,
            &mut effects,
        );
        if mode == 2 {
            assert!(
                matches!(result, Err(Error::Cancelled)),
                "write cancellation {cut}"
            );
        } else {
            assert!(
                matches!(result, Err(Error::Io { .. })),
                "write I/O {mode}/{cut}"
            );
        }
        assert!(fixture.path(object(5, 1)).exists());
        assert!(file.metadata().unwrap().len() <= u64::from(HEADER + ENTRY_BYTES));
    }
    let fixture = Fixture::new();
    let reference = fixture.index(object(5, 1), &units);
    for (mode, cut) in (0..6)
        .map(|cut| (0, cut))
        .chain((3..6).map(|cut| (1, cut)))
        .chain((0..6).map(|cut| (2, cut)))
    {
        let token = std::sync::Arc::new(CancellationToken::new());
        let peer = token.clone();
        let mut effects = match mode {
            0 => Effects::with_faults(Faults {
                fail_at: Some(cut),
                ..Faults::default()
            }),
            1 => Effects::with_faults(Faults {
                short_at: Some(cut),
                ..Faults::default()
            }),
            _ => Effects::with_faults(Faults {
                action: Some(Box::new(move |index, _| {
                    if index == cut {
                        peer.cancel();
                    }
                })),
                ..Faults::default()
            }),
        };
        let result = open(
            &fixture.0,
            database(),
            entry(Some(reference), 1, 1),
            &token,
            &mut effects,
        )
        .and_then(|cursor| {
            let mut cursor = cursor.unwrap();
            let result = cursor.next(&token, &mut effects);
            if result.is_err() {
                let before = effects.count();
                assert!(cursor.next(&cancel, &mut effects).is_err());
                assert_eq!(effects.count(), before);
            }
            result
        });
        if mode == 2 {
            assert!(
                matches!(result, Err(Error::Cancelled)),
                "read cancellation {cut}"
            );
        } else {
            assert!(
                matches!(result, Err(Error::Io { .. })),
                "read I/O {mode}/{cut}"
            );
        }
    }
    assert!(
        TableEntry::new(
            schema().table(),
            "facts",
            ObjectRef::new(object(1, 1), 160, 7).unwrap(),
            Some(reference),
            native_unit::MAX_ROWS as u64 + 1,
            1
        )
        .is_err()
    );
}
