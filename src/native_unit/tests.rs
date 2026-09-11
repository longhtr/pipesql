use super::*;
use crate::catalog_schema::{ColumnSpec, TableId};
use crate::effects::Faults;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn database() -> DatabaseId {
    DatabaseId::new([7; 16]).unwrap()
}

fn object() -> ObjectId {
    ObjectId::new(3, 2).unwrap()
}

fn id(value: u32) -> ColumnId {
    ColumnId::new(value).unwrap()
}

fn table() -> TableId {
    TableId::new(29).unwrap()
}

fn specs() -> [ColumnSpec<'static>; 4] {
    [
        ColumnSpec::new(id(9), "note", DataType::String, true).unwrap(),
        ColumnSpec::new(id(2), "amount", DataType::Double, true).unwrap(),
        ColumnSpec::new(id(42), "count", DataType::Int64, false).unwrap(),
        ColumnSpec::new(id(7), "day", DataType::Date, true).unwrap(),
    ]
}

fn schema_bytes(columns: &[ColumnSpec<'_>]) -> Vec<u8> {
    let mut bytes = vec![0; catalog_schema::MAX_BYTES];
    let length = catalog_schema::encode(&mut bytes, database(), table(), columns).unwrap();
    bytes.truncate(length);
    bytes
}

fn schema(bytes: &[u8]) -> Schema<'_> {
    catalog_schema::decode(bytes, database(), table(), crc32c(bytes)).unwrap()
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pipesql-native-unit-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> PathBuf {
        self.0.join(std::str::from_utf8(&object().name()).unwrap())
    }

    fn create(&self) -> File {
        pipesql_filesystem::create_new_read_write(self.path()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn build(
    fixture: &Fixture,
    schema: &Schema<'_>,
    columns: &[InputColumn<'_>],
    effects: &mut Effects,
) -> UnitRef {
    let file = fixture.create();
    let mut metadata = [0; MAX_METADATA_BYTES];
    let mut scratch = vec![0; MAX_COLUMN_BYTES];
    write(
        &file,
        object(),
        schema,
        columns,
        WriteBuffers {
            metadata: &mut metadata,
            column: &mut scratch,
        },
        &CancellationToken::new(),
        effects,
    )
    .unwrap()
}

fn data<'a>(
    strings: &'a [&'a str],
    doubles: &'a [f64],
    ints: &'a [i64],
    dates: &'a [DateValue],
) -> [InputColumn<'a>; 4] {
    // Physical order deliberately differs from the declaration.
    [
        InputColumn {
            id: id(42),
            values: InputValues::Int64(ints),
            validity: &[0b1111],
        },
        InputColumn {
            id: id(9),
            values: InputValues::String(strings),
            validity: &[0b1101],
        },
        InputColumn {
            id: id(7),
            values: InputValues::Date(dates),
            validity: &[0b1011],
        },
        InputColumn {
            id: id(2),
            values: InputValues::Double(doubles),
            validity: &[0b1011],
        },
    ]
}

#[test]
fn real_unit_preserves_null_utf8_bits_and_identity_under_reordering() {
    let fixture = Fixture::new();
    let bytes = schema_bytes(&specs());
    let original = schema(&bytes);
    let strings = ["", "ignored NULL", "Tiếng Việt", "雪\0🙂"];
    let doubles = [
        -0.0,
        f64::from_bits(0x7ff0_0000_0000_0001),
        999.0,
        f64::NEG_INFINITY,
    ];
    let ints = [i64::MIN, 0, 1, i64::MAX];
    let dates = [
        DateValue::from_days(-719162).unwrap(),
        DateValue::from_days(2932896).unwrap(),
        DateValue::from_days(17).unwrap(),
        DateValue::from_days(0).unwrap(),
    ];
    let columns = data(&strings, &doubles, &ints, &dates);
    let reference = build(&fixture, &original, &columns, &mut Effects::default());
    let mut renamed = specs();
    renamed.reverse();
    renamed[2] = ColumnSpec::new(id(2), "renamed_amount", DataType::Double, true).unwrap();
    let reordered_bytes = schema_bytes(&renamed);
    let reordered = schema(&reordered_bytes);
    let mut scratch = vec![0; MAX_COLUMN_BYTES];
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let unit = read(
        &fixture.0,
        database(),
        reference,
        &reordered,
        &cancel,
        &mut effects,
    )
    .unwrap();
    assert_eq!(effects.count(), 4);
    let text = unit
        .read_column(id(9), &mut scratch, &cancel, &mut effects)
        .unwrap();
    assert_eq!(text.string(0).unwrap(), Some(""));
    assert_eq!(text.string(1).unwrap(), None);
    assert_eq!(text.string(2).unwrap(), Some(strings[2]));
    assert_eq!(text.string(3).unwrap(), Some(strings[3]));
    assert!(text.string(4).is_err());
    assert!(text.double(0).is_err());
    let numeric = unit
        .read_column(id(2), &mut scratch, &cancel, &mut effects)
        .unwrap();
    for row in [0, 1, 3] {
        assert_eq!(
            numeric.double(row).unwrap().unwrap().to_bits(),
            doubles[row].to_bits()
        );
    }
    assert_eq!(numeric.double(2).unwrap(), None);
    let numeric = unit
        .read_column(id(42), &mut scratch, &cancel, &mut effects)
        .unwrap();
    for (row, value) in ints.iter().enumerate() {
        assert_eq!(numeric.int64(row).unwrap(), Some(*value));
    }
    let temporal = unit
        .read_column(id(7), &mut scratch, &cancel, &mut effects)
        .unwrap();
    for row in [0, 1, 3] {
        assert_eq!(temporal.date(row).unwrap(), Some(dates[row]));
    }
    assert_eq!(temporal.date(2).unwrap(), None);
    assert_eq!(effects.count(), 8);
}

#[test]
fn demanded_corruption_fails_without_reading_other_columns() {
    let fixture = Fixture::new();
    let bytes = schema_bytes(&specs());
    let schema = schema(&bytes);
    let dates = [DateValue::from_days(0).unwrap(); 4];
    let columns = data(&["a", "b", "c", "d"], &[1.; 4], &[2; 4], &dates);
    let reference = build(&fixture, &schema, &columns, &mut Effects::default());
    let mut file = std::fs::read(fixture.path()).unwrap();
    let offset = u64_at(&file, HEADER + DESCRIPTOR + 8) as usize;
    file[offset] ^= 1;
    std::fs::write(fixture.path(), file).unwrap();
    let mut scratch = vec![0; MAX_COLUMN_BYTES];
    let mut effects = Effects::default();
    let cancel = CancellationToken::new();
    let unit = read(
        &fixture.0,
        database(),
        reference,
        &schema,
        &cancel,
        &mut effects,
    )
    .unwrap();
    assert_eq!(
        unit.read_column(id(42), &mut scratch, &cancel, &mut effects)
            .unwrap()
            .int64(0)
            .unwrap(),
        Some(2)
    );
    assert_eq!(effects.count(), 5);
    assert!(matches!(
        unit.read_column(id(9), &mut scratch, &cancel, &mut effects),
        Err(Error::Corrupt(_))
    ));
    assert_eq!(effects.count(), 6);
}

#[test]
fn invalid_input_and_capacity_refuse_before_file_effects() {
    let fixture = Fixture::new();
    let file = fixture.create();
    let bytes = schema_bytes(&[ColumnSpec::new(id(2), "n", DataType::Double, false).unwrap()]);
    let schema = schema(&bytes);
    let mut metadata = [0; MAX_METADATA_BYTES];
    let mut scratch = vec![0; MAX_COLUMN_BYTES];
    let cancel = CancellationToken::new();
    for (values, bits) in [
        (vec![], vec![]),
        (vec![1.], vec![0]),
        (vec![1.], vec![3]),
        (vec![1.], vec![]),
        (
            vec![1.; MAX_ROWS + 1],
            vec![255; bitmap_bytes(MAX_ROWS + 1)],
        ),
    ] {
        let input = [InputColumn {
            id: id(2),
            values: InputValues::Double(&values),
            validity: &bits,
        }];
        let mut effects = Effects::default();
        assert!(
            write(
                &file,
                object(),
                &schema,
                &input,
                WriteBuffers {
                    metadata: &mut metadata,
                    column: &mut scratch
                },
                &cancel,
                &mut effects
            )
            .is_err()
        );
        assert_eq!(effects.count(), 0);
        assert_eq!(file.metadata().unwrap().len(), 0);
    }
    let input = [InputColumn {
        id: id(2),
        values: InputValues::Double(&[1.]),
        validity: &[1],
    }];
    for (meta_length, column_length) in [
        (HEADER + DESCRIPTOR - 1, MAX_COLUMN_BYTES),
        (MAX_METADATA_BYTES, 8),
    ] {
        let mut effects = Effects::default();
        assert!(matches!(
            write(
                &file,
                object(),
                &schema,
                &input,
                WriteBuffers {
                    metadata: &mut metadata[..meta_length],
                    column: &mut scratch[..column_length]
                },
                &cancel,
                &mut effects
            ),
            Err(Error::Resource { .. })
        ));
        assert_eq!(effects.count(), 0);
        assert_eq!(file.metadata().unwrap().len(), 0);
    }
    std::fs::write(fixture.path(), b"must remain").unwrap();
    let mut effects = Effects::default();
    assert!(matches!(
        write(
            &file,
            object(),
            &schema,
            &input,
            WriteBuffers {
                metadata: &mut metadata,
                column: &mut scratch
            },
            &cancel,
            &mut effects
        ),
        Err(Error::Corrupt(_))
    ));
    assert_eq!(std::fs::read(fixture.path()).unwrap(), b"must remain");
}

#[test]
fn construction_failures_never_return_a_unit_reference() {
    let bytes = schema_bytes(&[ColumnSpec::new(id(2), "n", DataType::Double, true).unwrap()]);
    let schema = schema(&bytes);
    let input = [InputColumn {
        id: id(2),
        values: InputValues::Double(&[1., 2.]),
        validity: &[1],
    }];
    let control = Fixture::new();
    let mut baseline = Effects::default();
    build(&control, &schema, &input, &mut baseline);
    assert_eq!(baseline.count(), 6);
    for cut in 0..baseline.count() {
        let fixture = Fixture::new();
        let file = fixture.create();
        let mut metadata = [0; MAX_METADATA_BYTES];
        let mut scratch = [0; 32];
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(cut),
            ..Faults::default()
        });
        assert!(
            matches!(
                write(
                    &file,
                    object(),
                    &schema,
                    &input,
                    WriteBuffers {
                        metadata: &mut metadata,
                        column: &mut scratch
                    },
                    &CancellationToken::new(),
                    &mut effects
                ),
                Err(Error::Io { .. })
            ),
            "effect {cut}"
        );
        assert_eq!(effects.count(), cut + 1);
    }
}

#[test]
fn catalog_schema_drives_independent_native_bytes_and_reads() {
    let fixture = Fixture::new();
    let catalog_bytes = include_bytes!("../../tests/fixtures/catalog-schema/catalog.bin");
    let schema_bytes = include_bytes!("../../tests/fixtures/catalog-schema/columns.bin");
    let expected = include_bytes!("../../tests/fixtures/catalog-schema/native-unit.bin");
    for (object, bytes) in [
        (ObjectId::new(4, 1).unwrap(), catalog_bytes.as_slice()),
        (ObjectId::new(1, 1).unwrap(), schema_bytes.as_slice()),
    ] {
        std::fs::write(
            fixture.0.join(std::str::from_utf8(&object.name()).unwrap()),
            bytes,
        )
        .unwrap();
    }
    let cancel = CancellationToken::new();
    let mut effects = Effects::default();
    let mut catalog_buffer = [0; catalog::MAX_BYTES];
    let mut schema_buffer = [0; catalog_schema::MAX_BYTES];
    let catalog = catalog::read(
        &fixture.0,
        database(),
        catalog::ObjectRef::new(ObjectId::new(4, 1).unwrap(), 192, 0xe19d_b89d).unwrap(),
        &mut catalog_buffer,
        &cancel,
        &mut effects,
    )
    .unwrap();
    let schema = catalog
        .read_schema(&fixture.0, 0, &mut schema_buffer, &cancel, &mut effects)
        .unwrap();
    let numbers = [
        f64::from_bits(0x8000000000000000),
        f64::from_bits(0x7ff0000000000001),
        f64::INFINITY,
        -f64::MAX,
    ];
    let text = ["ignored", "", "雪", "é\0🙂"];
    let columns = [
        InputColumn {
            id: id(3),
            values: InputValues::Double(&numbers),
            validity: &[15],
        },
        InputColumn {
            id: id(29),
            values: InputValues::String(&text),
            validity: &[14],
        },
    ];
    let reference = build(&fixture, &schema, &columns, &mut effects);
    assert_eq!(
        reference,
        UnitRef::new(object(), 4, 192, 0xcdd4_6886).unwrap()
    );
    assert_eq!(std::fs::read(fixture.path()).unwrap(), expected);
    let mut payload = [0; 64];
    let unit = read(
        &fixture.0,
        database(),
        reference,
        &schema,
        &cancel,
        &mut effects,
    )
    .unwrap();
    let strings = unit
        .read_column(id(29), &mut payload, &cancel, &mut effects)
        .unwrap();
    assert_eq!(strings.string(0).unwrap(), None);
    for (row, text) in text.iter().enumerate().skip(1) {
        assert_eq!(strings.string(row).unwrap(), Some(*text));
    }
    let numeric = unit
        .read_column(id(3), &mut payload, &cancel, &mut effects)
        .unwrap();
    for (row, value) in numbers.iter().enumerate() {
        assert_eq!(
            numeric.double(row).unwrap().unwrap().to_bits(),
            value.to_bits()
        );
    }
}

#[test]
fn matching_checksums_do_not_authorize_bad_metadata_or_text() {
    let fixture = Fixture::new();
    let original = include_bytes!("../../tests/fixtures/catalog-schema/native-unit.bin");
    let schema_bytes = include_bytes!("../../tests/fixtures/catalog-schema/columns.bin");
    let schema = catalog_schema::decode(
        schema_bytes,
        database(),
        TableId::new(0x0102030405060708).unwrap(),
        0x666a62b7,
    )
    .unwrap();
    let mut payload = [0; 64];
    let cancel = CancellationToken::new();
    for (at, value) in [
        (12, 1),
        (16, 8),
        (32, 1),
        (40, 4),
        (48, 3),
        (52, 5),
        (56, 127),
        (60, 1),
        (64, 29),
        (68, 3),
        (69, 1),
        (70, 1),
        (72, 129),
        (80, 32),
        (88, 1),
        (104, 162),
        (112, 30),
    ] {
        let mut bad = *original;
        assert_ne!(bad[at], value);
        bad[at] = value;
        let reference = UnitRef::new(object(), 4, 192, crc32c(&bad[..128])).unwrap();
        std::fs::write(fixture.path(), bad).unwrap();
        assert!(
            read(
                &fixture.0,
                database(),
                reference,
                &schema,
                &cancel,
                &mut Effects::default()
            )
            .is_err(),
            "metadata {at}"
        );
    }
    for (at, value, column, descriptor, offset, length) in [
        (128, 7, 3, 64, 128, 33),
        (128, 143, 3, 64, 128, 33),
        (161, 10, 29, 96, 161, 31),
        (161, 142, 29, 96, 161, 31),
        (162, 1, 29, 96, 161, 31),
        (174, 2, 29, 96, 161, 31),
        (178, 9, 29, 96, 161, 31),
        (182, 255, 29, 96, 161, 31),
    ] {
        let mut bad = *original;
        assert_ne!(bad[at], value);
        bad[at] = value;
        let crc = crc32c(&bad[offset..offset + length]);
        put_u32(&mut bad, descriptor + 20, crc);
        let reference = UnitRef::new(object(), 4, 192, crc32c(&bad[..128])).unwrap();
        std::fs::write(fixture.path(), bad).unwrap();
        let unit = read(
            &fixture.0,
            database(),
            reference,
            &schema,
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
        assert!(
            unit.read_column(id(column), &mut payload, &cancel, &mut Effects::default())
                .is_err(),
            "payload {at}={value}"
        );
    }
    let reference = UnitRef::new(object(), 4, 192, 0xcdd46886).unwrap();
    for length in 0..original.len() {
        std::fs::write(fixture.path(), &original[..length]).unwrap();
        assert!(
            read(
                &fixture.0,
                database(),
                reference,
                &schema,
                &cancel,
                &mut Effects::default()
            )
            .is_err(),
            "truncation {length}"
        );
    }
    let mut extra = original.to_vec();
    extra.push(0);
    std::fs::write(fixture.path(), extra).unwrap();
    assert!(
        read(
            &fixture.0,
            database(),
            reference,
            &schema,
            &cancel,
            &mut Effects::default()
        )
        .is_err()
    );
    let mut newer = *original;
    put_u32(&mut newer, 8, 7);
    assert!(matches!(
        validate_metadata(&newer[..12], database(), reference, &schema),
        Err(FormatError::Version)
    ));
}

#[test]
fn canonical_null_numeric_bytes_and_date_domain_are_checked() {
    let bytes = schema_bytes(&specs());
    let schema = schema(&bytes);
    let cancel = CancellationToken::new();
    for (column, ordinal, row) in [(2, 3, 2), (7, 2, 0)] {
        let fixture = Fixture::new();
        let dates = [DateValue::from_days(0).unwrap(); 4];
        let input = data(&["a", "b", "c", "d"], &[1.; 4], &[2; 4], &dates);
        let reference = build(&fixture, &schema, &input, &mut Effects::default());
        let mut file = std::fs::read(fixture.path()).unwrap();
        let entry = HEADER + ordinal * DESCRIPTOR;
        let offset = u64_at(&file, entry + 8) as usize;
        let length = u32_at(&file, entry + 16) as usize;
        if column == 2 {
            file[offset + 1 + row * 8] = 1;
        } else {
            file[offset + 1..offset + 5].copy_from_slice(&i32::MAX.to_le_bytes());
        }
        let crc = crc32c(&file[offset..offset + length]);
        put_u32(&mut file, entry + 20, crc);
        let reference = UnitRef {
            metadata_crc: crc32c(&file[..HEADER + 4 * DESCRIPTOR]),
            ..reference
        };
        std::fs::write(fixture.path(), file).unwrap();
        let mut scratch = [0; 64];
        let unit = read(
            &fixture.0,
            database(),
            reference,
            &schema,
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
        assert!(matches!(
            unit.read_column(id(column), &mut scratch, &cancel, &mut Effects::default()),
            Err(Error::Corrupt(_))
        ));
    }
}

#[test]
fn exact_row_text_byte_and_column_limits_are_constructible() {
    let nullable_double = ColumnSpec::new(id(2), "n", DataType::Double, true).unwrap();
    let bytes = schema_bytes(&[nullable_double]);
    let schema = schema(&bytes);
    let values = vec![1.; MAX_ROWS];
    let valid = vec![255; bitmap_bytes(MAX_ROWS)];
    let fixture = Fixture::new();
    let reference = build(
        &fixture,
        &schema,
        &[InputColumn {
            id: id(2),
            values: InputValues::Double(&values),
            validity: &valid,
        }],
        &mut Effects::default(),
    );
    assert_eq!(reference.rows, MAX_ROWS as u32);
    let mut metadata = [0; MAX_METADATA_BYTES];
    let mut scratch = vec![0; MAX_COLUMN_BYTES];
    let cancel = CancellationToken::new();
    let unit = read(
        &fixture.0,
        database(),
        reference,
        &schema,
        &cancel,
        &mut Effects::default(),
    )
    .unwrap();
    assert_eq!(
        unit.read_column(id(2), &mut scratch, &cancel, &mut Effects::default())
            .unwrap()
            .double(MAX_ROWS - 1)
            .unwrap(),
        Some(1.)
    );
    let nullable_text = ColumnSpec::new(id(9), "text", DataType::String, true).unwrap();
    let text_bytes = schema_bytes(&[nullable_text]);
    let text_schema = self::schema(&text_bytes);
    let full = "x".repeat(MAX_TEXT_BYTES);
    let tail = "é".repeat((MAX_TEXT_BYTES - 38) / 2) + "x";
    let values = [
        full.as_str(),
        full.as_str(),
        full.as_str(),
        full.as_str(),
        full.as_str(),
        full.as_str(),
        full.as_str(),
        tail.as_str(),
    ];
    let fixture = Fixture::new();
    let input = [InputColumn {
        id: id(9),
        values: InputValues::String(&values),
        validity: &[255],
    }];
    let reference = build(&fixture, &text_schema, &input, &mut Effects::default());
    assert_eq!(
        reference.bytes as usize,
        HEADER + DESCRIPTOR + MAX_COLUMN_BYTES
    );
    let unit = read(
        &fixture.0,
        database(),
        reference,
        &text_schema,
        &cancel,
        &mut Effects::default(),
    )
    .unwrap();
    let column = unit
        .read_column(id(9), &mut scratch, &cancel, &mut Effects::default())
        .unwrap();
    assert_eq!(column.string(0).unwrap().unwrap().len(), MAX_TEXT_BYTES);
    assert_eq!(column.string(7).unwrap(), Some(tail.as_str()));
    let next = tail + "x";
    let excessive = "x".repeat(MAX_TEXT_BYTES + 1);
    let fixture = Fixture::new();
    let file = fixture.create();
    for strings in [
        vec![full.as_str(); 8],
        vec![
            full.as_str(),
            full.as_str(),
            full.as_str(),
            full.as_str(),
            full.as_str(),
            full.as_str(),
            full.as_str(),
            next.as_str(),
        ],
        vec![excessive.as_str()],
    ] {
        let bits = if strings.len() == 1 {
            &[1][..]
        } else {
            &[255][..]
        };
        let mut effects = Effects::default();
        let input = [InputColumn {
            id: id(9),
            values: InputValues::String(&strings),
            validity: bits,
        }];
        assert!(
            write(
                &file,
                object(),
                &text_schema,
                &input,
                WriteBuffers {
                    metadata: &mut metadata,
                    column: &mut scratch
                },
                &cancel,
                &mut effects
            )
            .is_err()
        );
        assert_eq!(effects.count(), 0);
        assert_eq!(file.metadata().unwrap().len(), 0);
    }
    let names: Vec<_> = (1..=64).map(|i| format!("c{i}")).collect();
    let columns: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            ColumnSpec::new(
                id(u32::try_from(i + 1).unwrap()),
                name,
                DataType::Int64,
                false,
            )
            .unwrap()
        })
        .collect();
    let bytes = schema_bytes(&columns);
    let schema = self::schema(&bytes);
    let inputs: Vec<_> = (1..=64)
        .rev()
        .map(|i| InputColumn {
            id: id(i),
            values: InputValues::Int64(&[i64::MAX]),
            validity: &[1],
        })
        .collect();
    let fixture = Fixture::new();
    let reference = build(&fixture, &schema, &inputs, &mut Effects::default());
    let unit = read(
        &fixture.0,
        database(),
        reference,
        &schema,
        &cancel,
        &mut Effects::default(),
    )
    .unwrap();
    for i in [1, 64] {
        assert_eq!(
            unit.read_column(id(i), &mut scratch, &cancel, &mut Effects::default())
                .unwrap()
                .int64(0)
                .unwrap(),
            Some(i64::MAX)
        );
    }
}

#[test]
fn reads_short_io_and_cancellation_preserve_private_file_ownership() {
    let bytes = schema_bytes(&[ColumnSpec::new(id(2), "n", DataType::Double, true).unwrap()]);
    let schema = schema(&bytes);
    let input = [InputColumn {
        id: id(2),
        values: InputValues::Double(&[1., 2.]),
        validity: &[1],
    }];
    let cancel = CancellationToken::new();
    let required = requirements(&schema, &input, &cancel).unwrap();
    assert_eq!(required.metadata_bytes, 96);
    assert_eq!(required.column_bytes, 17);
    assert_eq!(required.unit_bytes, 113);
    let mut metadata = [0; MAX_METADATA_BYTES];
    let mut scratch = [0; 32];
    for (short, cut) in (1..5)
        .map(|cut| (true, cut))
        .chain((0..6).map(|cut| (false, cut)))
    {
        let fixture = Fixture::new();
        let file = fixture.create();
        let cancel = std::sync::Arc::new(CancellationToken::new());
        let peer = cancel.clone();
        let mut effects = if short {
            Effects::with_faults(Faults {
                short_at: Some(cut),
                ..Faults::default()
            })
        } else {
            Effects::with_faults(Faults {
                action: Some(Box::new(move |index, _| {
                    if index == cut {
                        peer.cancel();
                    }
                })),
                ..Faults::default()
            })
        };
        let result = write(
            &file,
            object(),
            &schema,
            &input,
            WriteBuffers {
                metadata: &mut metadata,
                column: &mut scratch,
            },
            &cancel,
            &mut effects,
        );
        if short {
            assert!(matches!(result, Err(Error::Io { .. })));
        } else {
            assert!(
                matches!(result, Err(Error::Cancelled)),
                "write cancellation {cut}"
            );
        }
        assert!(fixture.path().exists());
        assert!(file.metadata().unwrap().len() <= required.unit_bytes as u64);
    }
    let fixture = Fixture::new();
    let reference = build(&fixture, &schema, &input, &mut Effects::default());
    for cut in 0..5 {
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(cut),
            ..Faults::default()
        });
        let result = read(
            &fixture.0,
            database(),
            reference,
            &schema,
            &cancel,
            &mut effects,
        )
        .and_then(|unit| unit.read_column(id(2), &mut scratch, &cancel, &mut effects));
        assert!(
            matches!(result, Err(Error::Io { .. })),
            "read failure {cut}"
        );
        assert_eq!(effects.count(), cut + 1);
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
        let result = read(
            &fixture.0,
            database(),
            reference,
            &schema,
            &cancel,
            &mut effects,
        )
        .and_then(|unit| unit.read_column(id(2), &mut scratch, &cancel, &mut effects));
        assert!(
            matches!(result, Err(Error::Cancelled)),
            "read cancellation {cut}"
        );
    }
    for cut in [3, 4] {
        let mut effects = Effects::with_faults(Faults {
            short_at: Some(cut),
            ..Faults::default()
        });
        let result = read(
            &fixture.0,
            database(),
            reference,
            &schema,
            &cancel,
            &mut effects,
        )
        .and_then(|unit| unit.read_column(id(2), &mut scratch, &cancel, &mut effects));
        assert!(matches!(result, Err(Error::Io { .. })));
    }
    assert_eq!(
        std::fs::metadata(fixture.path()).unwrap().len(),
        required.unit_bytes as u64
    );
}

#[test]
fn retained_payload_invalidates_before_failed_refill() {
    let fixture = Fixture::new();
    let bytes = schema_bytes(&specs());
    let schema = schema(&bytes);
    let dates = [DateValue::from_days(0).unwrap(); 4];
    let input = data(&["雪", "", "abc", "last"], &[1.; 4], &[2; 4], &dates);
    let reference = build(&fixture, &schema, &input, &mut Effects::default());
    let cancel = CancellationToken::new();
    let unit = read(
        &fixture.0,
        database(),
        reference,
        &schema,
        &cancel,
        &mut Effects::default(),
    )
    .unwrap();
    let mut payload = ColumnBuffer::new(vec![0; MAX_COLUMN_BYTES]).unwrap();
    let address = payload.bytes.as_ptr();
    let capacity = payload.bytes.capacity();
    assert!(payload.column().is_none());
    payload
        .read(&unit, id(9), &cancel, &mut Effects::default())
        .unwrap();
    let mut moved = payload;
    assert_eq!(moved.column().unwrap().string(0).unwrap(), Some("雪"));
    assert_eq!(moved.column().unwrap().string(1).unwrap(), None);
    for failure in 0..3 {
        let cancelled = CancellationToken::new();
        let mut effects = Effects::default();
        if failure == 0 {
            cancelled.cancel();
        }
        if failure == 1 {
            effects.fail_at(0);
        }
        let column = if failure == 2 { id(99) } else { id(42) };
        assert!(moved.read(&unit, column, &cancelled, &mut effects).is_err());
        assert!(moved.column().is_none());
        moved
            .read(&unit, id(42), &cancel, &mut Effects::default())
            .unwrap();
        assert_eq!(moved.column().unwrap().int64(0).unwrap(), Some(2));
        assert_eq!(moved.bytes.as_ptr(), address);
        assert_eq!(moved.bytes.capacity(), capacity);
    }
    // The same validator must reject damaged payload after an earlier valid read.
    let mut file = std::fs::read(fixture.path()).unwrap();
    let offset = u64_at(&file, HEADER + DESCRIPTOR + 8) as usize;
    file[offset] ^= 1;
    std::fs::write(fixture.path(), file).unwrap();
    assert!(matches!(
        moved.read(&unit, id(9), &cancel, &mut Effects::default()),
        Err(Error::Corrupt(_))
    ));
    assert!(moved.column().is_none());
    let mut small = ColumnBuffer::new(vec![0; 1]).unwrap();
    let mut effects = Effects::default();
    assert!(matches!(
        small.read(&unit, id(42), &cancel, &mut effects),
        Err(Error::Resource { .. })
    ));
    assert_eq!(effects.count(), 0);
    assert!(small.column().is_none());
    assert!(matches!(
        ColumnBuffer::new(vec![0; MAX_COLUMN_BYTES + 1]),
        Err(Error::Resource { .. })
    ));
}
