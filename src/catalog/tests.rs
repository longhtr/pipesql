use super::*;
use crate::catalog_schema::{ColumnId, ColumnSpec};
use crate::effects::Faults;
use crate::frontend::DataType;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn database() -> DatabaseId {
    DatabaseId::new([7; 16]).unwrap()
}

fn id(attempt: u64, ordinal: u32) -> ObjectId {
    ObjectId::new(attempt, ordinal).unwrap()
}

fn table_id(value: u64) -> TableId {
    TableId::new(value).unwrap()
}

fn reference(object: ObjectId, bytes: &[u8]) -> ObjectRef {
    ObjectRef::new(object, u32::try_from(bytes.len()).unwrap(), crc32c(bytes)).unwrap()
}

fn schema(table: TableId) -> Vec<u8> {
    let mut bytes = vec![0; catalog_schema::MAX_BYTES];
    let length = catalog_schema::encode(
        &mut bytes,
        database(),
        table,
        &[
            ColumnSpec::new(ColumnId::new(29).unwrap(), "note", DataType::String, true).unwrap(),
            ColumnSpec::new(ColumnId::new(3).unwrap(), "amount", DataType::Double, true).unwrap(),
        ],
    )
    .unwrap();
    bytes.truncate(length);
    bytes
}

fn empty_entry<'a>(table: u64, name: &'a str, schema: &[u8], ordinal: u32) -> TableEntry<'a> {
    TableEntry::new(
        table_id(table),
        name,
        reference(id(1, ordinal), schema),
        None,
        0,
        0,
    )
    .unwrap()
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pipesql-catalog-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, object: ObjectId) -> PathBuf {
        self.0.join(std::str::from_utf8(&object.name()).unwrap())
    }
    // Narrow trusted fixture construction; this is not a second publisher.
    fn put(&self, object: ObjectId, bytes: &[u8]) {
        std::fs::write(self.path(object), bytes).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn catalog_to_schema_real_file_read_preserves_logical_identity() {
    let fixture = Fixture::new();
    let first = schema(table_id(3));
    let second = schema(table_id(29));
    fixture.put(id(1, 1), &first);
    fixture.put(id(1, 2), &second);
    let mut bytes = [0; MAX_BYTES];
    let root = encode(
        &mut bytes,
        database(),
        id(4, 1),
        &[
            empty_entry(3, "renamed_orders", &first, 1),
            empty_entry(29, "facts", &second, 2),
        ],
    )
    .unwrap();
    fixture.put(root.id, &bytes[..root.bytes as usize]);
    let mut buffer = [0; MAX_BYTES];
    let mut schema_buffer = [0; catalog_schema::MAX_BYTES];
    let mut effects = Effects::default();
    let cancel = CancellationToken::new();
    let catalog = read(
        &fixture.0,
        database(),
        root,
        &mut buffer,
        &cancel,
        &mut effects,
    )
    .unwrap();
    assert_eq!(catalog.len(), 2);
    assert_eq!(catalog.find("RENAMED_ORDERS").unwrap().id(), table_id(3));
    assert_eq!(catalog.table(1).unwrap().name(), "facts");
    assert_eq!(catalog.table(1).unwrap().rows(), 0);
    assert_eq!(catalog.table(1).unwrap().data(), None);
    assert!(catalog.table(2).is_none());
    assert!(catalog.find("missing").is_none());
    let schema = catalog
        .read_schema(&fixture.0, 1, &mut schema_buffer, &cancel, &mut effects)
        .unwrap();
    assert_eq!(schema.table(), table_id(29));
    assert_eq!(schema.column(0).unwrap().id().value(), 29);
    assert_eq!(schema.column(1).unwrap().id().value(), 3);
    assert!(schema.column(0).unwrap().nullable());
    assert_eq!(effects.count(), 8);
}

#[test]
fn every_graph_read_effect_refuses_without_returning_a_schema() {
    let fixture = Fixture::new();
    let schema = schema(table_id(3));
    fixture.put(id(1, 1), &schema);
    let mut encoded = [0; MAX_BYTES];
    let root = encode(
        &mut encoded,
        database(),
        id(4, 1),
        &[empty_entry(3, "facts", &schema, 1)],
    )
    .unwrap();
    fixture.put(root.id, &encoded[..root.bytes as usize]);
    let cancel = CancellationToken::new();
    let mut buffer = [0; MAX_BYTES];
    let mut schema_buffer = [0; catalog_schema::MAX_BYTES];
    for cut in 0..8 {
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(cut),
            ..Faults::default()
        });
        let result = read(
            &fixture.0,
            database(),
            root,
            &mut buffer,
            &cancel,
            &mut effects,
        )
        .and_then(|catalog| {
            catalog.read_schema(&fixture.0, 0, &mut schema_buffer, &cancel, &mut effects)
        });
        assert!(matches!(result, Err(Error::Io { .. })), "cut {cut}");
        assert_eq!(effects.count(), cut + 1);
    }
    for cut in [3, 7] {
        let mut effects = Effects::with_faults(Faults {
            short_at: Some(cut),
            ..Faults::default()
        });
        let result = read(
            &fixture.0,
            database(),
            root,
            &mut buffer,
            &cancel,
            &mut effects,
        )
        .and_then(|catalog| {
            catalog.read_schema(&fixture.0, 0, &mut schema_buffer, &cancel, &mut effects)
        });
        assert!(matches!(result, Err(Error::Io { .. })), "short {cut}");
    }
    cancel.cancel();
    let mut effects = Effects::default();
    assert!(matches!(
        read(
            &fixture.0,
            database(),
            root,
            &mut buffer,
            &cancel,
            &mut effects
        ),
        Err(Error::Cancelled)
    ));
    assert_eq!(effects.count(), 0);
    assert_eq!(
        std::fs::read(fixture.path(root.id)).unwrap(),
        &encoded[..root.bytes as usize]
    );
    assert_eq!(std::fs::read(fixture.path(id(1, 1))).unwrap(), schema);
}

#[test]
fn schema_substitution_fails_even_when_parent_checksum_matches() {
    let fixture = Fixture::new();
    let foreign = schema(table_id(29));
    fixture.put(id(1, 1), &foreign);
    let mut bytes = [0; MAX_BYTES];
    // A forged parent has the replacement's correct CRC but lies about table ID.
    let root = encode(
        &mut bytes,
        database(),
        id(4, 1),
        &[empty_entry(3, "facts", &foreign, 1)],
    )
    .unwrap();
    let catalog = decode(&bytes[..root.bytes as usize], database(), root).unwrap();
    let mut buffer = [0; catalog_schema::MAX_BYTES];
    let cancel = CancellationToken::new();
    assert!(matches!(
        catalog.read_schema(&fixture.0, 0, &mut buffer, &cancel, &mut Effects::default()),
        Err(Error::Corrupt(_))
    ));
    let valid = schema(table_id(3));
    fixture.put(id(1, 1), &valid);
    // The correct table with a stale parent checksum also fails closed.
    assert!(matches!(
        catalog.read_schema(&fixture.0, 0, &mut buffer, &cancel, &mut Effects::default()),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn catalog_capacities_and_producer_refusal_preserve_output() {
    let schemas: Vec<_> = (1..=65).map(|table| schema(table_id(table))).collect();
    let names: Vec<_> = (1..=65).map(|table| format!("t{table:031}")).collect();
    let entries: Vec<_> = (0..65)
        .map(|i| {
            empty_entry(
                u64::try_from(i + 1).unwrap(),
                &names[i],
                &schemas[i],
                u32::try_from(i + 1).unwrap(),
            )
        })
        .collect();
    let mut bytes = [0xa5; MAX_BYTES + TABLE];
    let root = encode(&mut bytes, database(), id(4, 1), &entries[..64]).unwrap();
    assert_eq!(root.bytes as usize, MAX_BYTES);
    assert_eq!(
        decode(&bytes[..MAX_BYTES], database(), root).unwrap().len(),
        64
    );
    assert!(bytes[MAX_BYTES..].iter().all(|byte| *byte == 0xa5));
    let saved = bytes;
    assert!(encode(&mut bytes, database(), id(4, 1), &entries).is_err());
    assert_eq!(bytes, saved);
    assert!(
        encode(
            &mut bytes[..MAX_BYTES - 1],
            database(),
            id(4, 1),
            &entries[..64]
        )
        .is_err()
    );
    assert_eq!(bytes, saved);
    for invalid in [
        vec![entries[1], entries[0]],
        vec![entries[0], entries[0]],
        vec![
            entries[0],
            TableEntry {
                name: entries[0].name,
                ..entries[1]
            },
        ],
        vec![TableEntry {
            schema: ObjectRef {
                id: id(4, 1),
                ..entries[0].schema
            },
            ..entries[0]
        }],
        vec![TableEntry {
            schema: ObjectRef {
                id: id(5, 1),
                ..entries[0].schema
            },
            ..entries[0]
        }],
    ] {
        assert!(encode(&mut bytes, database(), id(4, 1), &invalid).is_err());
        assert_eq!(bytes, saved);
    }
    let root = encode(&mut bytes, database(), id(4, 1), &[]).unwrap();
    assert_eq!(root.bytes, HEADER as u32);
    assert_eq!(decode(&bytes[..HEADER], database(), root).unwrap().len(), 0);
}

#[test]
fn malformed_catalog_fields_fail_with_fresh_checksums() {
    let schema = schema(table_id(3));
    let mut bytes = [0; MAX_BYTES];
    let root = encode(
        &mut bytes,
        database(),
        id(4, 1),
        &[empty_entry(3, "facts", &schema, 1)],
    )
    .unwrap();
    let original = &bytes[..root.bytes as usize];
    // Independently vary identity, counts, reserved bytes, name bytes, reference
    // extent/ordinal, empty-data geometry and reference reserved tail.
    for (offset, value) in [
        (12, 65),
        (16, 8),
        (32, 5),
        (40, 2),
        (44, 1),
        (64, 0),
        (72, 33),
        (73, 1),
        (80, b'/'),
        (100, 1),
        (120, 0),
        (124, 0),
        (125, 255),
        (132, 1),
        (136, 1),
        (160, 1),
        (168, 1),
        (172, 1),
    ] {
        let mut bad = original.to_vec();
        bad[offset] = value;
        assert_ne!(bad, original, "mutation {offset}");
        let anchor = ObjectRef {
            crc: crc32c(&bad),
            ..root
        };
        assert!(decode(&bad, database(), anchor).is_err(), "offset {offset}");
    }
    for length in 0..original.len() {
        assert!(
            decode(&original[..length], database(), root).is_err(),
            "truncation {length}"
        );
    }
    let mut extra = original.to_vec();
    extra.push(0);
    assert!(decode(&extra, database(), reference(root.id, &extra)).is_err());
    let mut newer = original.to_vec();
    newer[8..12].copy_from_slice(&7u32.to_le_bytes());
    assert!(matches!(
        decode(&newer[..12], database(), root),
        Err(FormatError::Version)
    ));
    assert!(matches!(
        decode(
            original,
            database(),
            ObjectRef {
                crc: root.crc ^ 1,
                ..root
            }
        ),
        Err(FormatError::Checksum)
    ));
}

#[test]
fn data_reference_geometry_and_identity_are_bounded() {
    let schema = schema(table_id(3));
    let schema_ref = reference(id(1, 1), &schema);
    let data = ObjectRef::new(id(2, 1), DATA_HEADER + MAX_UNITS * DATA_ENTRY, 7).unwrap();
    let full = TableEntry::new(
        table_id(3),
        "facts",
        schema_ref,
        Some(data),
        u64::from(MAX_UNITS),
        MAX_UNITS,
    )
    .unwrap();
    assert_eq!(full.data(), Some(data));
    for (data, rows, units) in [
        (None, 1, 0),
        (None, 0, 1),
        (Some(data), 0, 0),
        (Some(data), 1, 2),
        (Some(data), u64::MAX, MAX_UNITS + 1),
        (Some(data), 1, 1),
        (Some(schema_ref), 2, 2),
    ] {
        assert!(TableEntry::new(table_id(3), "facts", schema_ref, data, rows, units).is_err());
    }
    assert!(ObjectId::new(0, 1).is_err());
    assert!(ObjectId::new(1, 0).is_err());
    assert_eq!(
        id(u64::MAX, u32::MAX).name(),
        *b"ffffffffffffffff-ffffffff.obj"
    );
    let next = TableEntry::new(
        table_id(29),
        "FACTS",
        reference(id(1, 2), &schema),
        None,
        0,
        0,
    )
    .unwrap();
    assert!(no_alias(full, next).is_err());
}

#[test]
fn independent_catalog_and_schema_graph_matches_both_codecs() {
    let expected = include_bytes!("../../tests/fixtures/catalog-schema/catalog.bin");
    let schema_bytes = include_bytes!("../../tests/fixtures/catalog-schema/columns.bin");
    let root = ObjectRef::new(id(4, 1), 192, 0xe19d_b89d).unwrap();
    let catalog = decode(expected, database(), root).unwrap();
    let entry = catalog.find("facts").unwrap();
    assert_eq!(entry.id().value(), 0x0102_0304_0506_0708);
    let mut encoded = [0; MAX_BYTES];
    assert_eq!(
        encode(&mut encoded, database(), root.id, &[entry]).unwrap(),
        root
    );
    assert_eq!(&encoded[..192], expected);
    let fixture = Fixture::new();
    fixture.put(root.id, expected);
    fixture.put(id(1, 1), schema_bytes);
    let mut catalog_buffer = [0; MAX_BYTES];
    let mut schema_buffer = [0; catalog_schema::MAX_BYTES];
    let mut effects = Effects::default();
    let cancel = CancellationToken::new();
    let catalog = read(
        &fixture.0,
        database(),
        root,
        &mut catalog_buffer,
        &cancel,
        &mut effects,
    )
    .unwrap();
    let schema = catalog
        .read_schema(&fixture.0, 0, &mut schema_buffer, &cancel, &mut effects)
        .unwrap();
    assert_eq!(schema.table(), entry.id());
    assert_eq!(schema.column(0).unwrap().name(), "note");
    assert_eq!(schema.column(1).unwrap().name(), "amount");
    assert!(!schema.column(1).unwrap().nullable());
}

#[test]
fn namespace_replacement_is_rejected_before_content_read() {
    let fixture = Fixture::new();
    let mut bytes = [0; MAX_BYTES];
    let root = encode(&mut bytes, database(), id(4, 1), &[]).unwrap();
    fixture.put(root.id, &bytes[..HEADER]);
    fixture.put(id(4, 2), &bytes[..HEADER]);
    let path = fixture.path(root.id);
    let replacement = fixture.path(id(4, 2));
    let held = std::fs::File::open(&path).unwrap(); // Prevent inode reuse in the test.
    let mut effects = Effects::with_faults(Faults {
        action: Some(Box::new(move |_, effect| {
            if effect == Effect::OpenMetadata(MetadataKind::CatalogObject) {
                std::fs::rename(&replacement, &path).unwrap();
            }
        })),
        ..Faults::default()
    });
    let mut buffer = [0; MAX_BYTES];
    assert!(matches!(
        read(
            &fixture.0,
            database(),
            root,
            &mut buffer,
            &CancellationToken::new(),
            &mut effects
        ),
        Err(Error::Corrupt(_))
    ));
    assert_eq!(effects.count(), 3);
    drop(held);
}

#[test]
fn read_admission_and_cancellation_bound_effects() {
    let fixture = Fixture::new();
    let mut bytes = [0; MAX_BYTES];
    let root = encode(&mut bytes, database(), id(4, 1), &[]).unwrap();
    fixture.put(root.id, &bytes[..HEADER]);
    let mut small = [0xa5; HEADER - 1];
    let mut effects = Effects::default();
    assert!(matches!(
        read(
            &fixture.0,
            database(),
            root,
            &mut small,
            &CancellationToken::new(),
            &mut effects
        ),
        Err(Error::Resource {
            required: 64,
            limit: 63,
            ..
        })
    ));
    assert_eq!(small, [0xa5; HEADER - 1]);
    assert_eq!(effects.count(), 0);
    let mut buffer = [0; MAX_BYTES];
    let huge = ObjectRef::new(root.id, u32::MAX, root.crc).unwrap();
    assert!(matches!(
        read(
            &fixture.0,
            database(),
            huge,
            &mut buffer,
            &CancellationToken::new(),
            &mut effects
        ),
        Err(Error::Corrupt(_))
    ));
    assert_eq!(effects.count(), 0);
    for cut in 0..4 {
        let cancel = std::sync::Arc::new(CancellationToken::new());
        let peer = cancel.clone();
        let mut effects = Effects::with_faults(Faults {
            action: Some(Box::new(move |index, _| {
                if index == cut {
                    peer.cancel();
                }
            })),
            ..Faults::default()
        });
        assert!(
            matches!(
                read(
                    &fixture.0,
                    database(),
                    root,
                    &mut buffer,
                    &cancel,
                    &mut effects
                ),
                Err(Error::Cancelled)
            ),
            "cut {cut}"
        );
        assert!(effects.count() <= 4);
    }
    for length in [HEADER - 1, HEADER + 1] {
        fixture.put(root.id, &vec![0; length]);
        let mut effects = Effects::default();
        assert!(matches!(
            read(
                &fixture.0,
                database(),
                root,
                &mut buffer,
                &CancellationToken::new(),
                &mut effects
            ),
            Err(Error::Corrupt(_))
        ));
        assert_eq!(effects.count(), 3);
    }
    std::fs::remove_file(fixture.path(root.id)).unwrap();
    std::os::unix::fs::symlink(fixture.path(id(4, 2)), fixture.path(root.id)).unwrap();
    let mut effects = Effects::default();
    assert!(matches!(
        read(
            &fixture.0,
            database(),
            root,
            &mut buffer,
            &CancellationToken::new(),
            &mut effects
        ),
        Err(Error::Corrupt(_))
    ));
    assert_eq!(effects.count(), 1);
}
