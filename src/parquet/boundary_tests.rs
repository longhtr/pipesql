//! Independent typed pages cross unrelated column-page and row-group boundaries.
//!
//! PyArrow's retained inputs have unrelated NULL periods and different page cuts
//! for integers, dates and text. Expected cells below come from the fixture's
//! authored values, never from the production metadata, page or value decoder.

use super::{ParquetImportLimits, ParquetReadLimits, decoder::Decoder};
use crate::test_support::Directory;
use crate::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues,
    CommitResolution, Config, DataType, Database, DateValue, Error, QueryStep, Value,
};
use std::cell::Cell;
use std::io::Cursor;
use std::path::Path;

const INPUTS: [&[u8]; 2] = [
    include_bytes!("../../test/data/parquet/typed-boundaries-v1.parquet"),
    include_bytes!("../../test/data/parquet/typed-boundaries-v2.parquet"),
];
const SCHEMA: [ColumnDeclaration<'static>; 5] = [
    ColumnDeclaration {
        name: "note",
        data_type: DataType::String,
        nullable: true,
    },
    ColumnDeclaration {
        name: "day",
        data_type: DataType::Date,
        nullable: true,
    },
    ColumnDeclaration {
        name: "number",
        data_type: DataType::Double,
        nullable: true,
    },
    ColumnDeclaration {
        name: "amount",
        data_type: DataType::Int64,
        nullable: true,
    },
    ColumnDeclaration {
        name: "ID",
        data_type: DataType::Int64,
        nullable: false,
    },
];

fn limits() -> ParquetImportLimits {
    ParquetImportLimits {
        parquet: ParquetReadLimits {
            input_bytes: 250_000,
            metadata_bytes: 8192,
            row_groups: 2,
            row_group_rows: 389,
            row_group_bytes: 120_000,
            page_bytes: 70_000,
            rows: 777,
        },
        append: AppendLimits {
            batches: 16,
            encoded_bytes: 500_000,
        },
    }
}

fn check_row<'a>(id: usize, mut value: impl FnMut(usize) -> Option<Value<'a>>) {
    assert!(id < 777);
    assert_eq!(value(4), Some(Value::Int64(id as i64)));
    let integers = [i64::MIN, i64::MAX, -1, 0, 1, 42, -42];
    assert_eq!(
        value(3),
        Some(if id % 11 == 3 {
            Value::Null
        } else {
            Value::Int64(integers[id % 7])
        })
    );
    let bits = [
        0,
        0x8000000000000000,
        0x7ff0000000000000,
        0xfff0000000000000,
        0x7ff8000000001234,
        0xfff0000000000001,
        1,
    ];
    if id % 13 == 5 {
        assert_eq!(value(2), Some(Value::Null));
    } else {
        let Some(Value::Double(number)) = value(2) else {
            panic!("DOUBLE at {id}")
        };
        assert_eq!(number.to_bits(), bits[id % 7], "DOUBLE bits at {id}");
    }
    let days = [-719_162, 2_932_896, -1, 0, 1, 11_016, 18_262];
    assert_eq!(
        value(1),
        Some(if id % 19 == 7 {
            Value::Null
        } else {
            Value::Date(DateValue::from_days_since_unix_epoch(days[id % 7]).unwrap())
        })
    );
    if id % 7 == 2 {
        assert_eq!(value(0), Some(Value::Null));
    } else {
        let expected = if matches!(id, 255 | 645) {
            "x".repeat(65_536)
        } else if id % 17 == 4 {
            String::new()
        } else {
            format!("{id}:é🙂\n\0{}", "q".repeat(id % 101))
        };
        let Some(Value::String(text)) = value(0) else {
            panic!("STRING at {id}")
        };
        assert_eq!(text.as_str(), expected, "STRING at {id}");
    }
}

// Offsets refer to the last text page of group two in the independently retained
// files. V1 changes a text byte and its IEEE CRC (computed with Python zlib), so
// validation must reach UTF-8. V2 changes only declared NULLs from two to three;
// payload CRC stays valid and decoding must compare actual definition levels.
fn damaged(version: usize) -> (Vec<u8>, &'static str, u64) {
    let mut bytes = INPUTS[version].to_vec();
    fn patch(bytes: &mut [u8], start: usize, before: &[u8], after: &[u8]) {
        assert_eq!(&bytes[start..start + before.len()], before);
        assert_eq!(before.len(), after.len());
        bytes[start..start + after.len()].copy_from_slice(after);
    }
    if version == 0 {
        patch(
            &mut bytes,
            195_009,
            &[0xde, 0xf6, 0xc1, 0xab, 4],
            &[0xe5, 0xa9, 0xfc, 0xe8, 7],
        );
        patch(&mut bytes, 195_128, &[0x37], &[0xff]);
        (bytes, "Parquet STRING is not UTF-8", 195_192)
    } else {
        patch(&mut bytes, 195_107, &[4], &[6]);
        (
            bytes,
            "Parquet page value bytes or NULL count differs",
            196_362,
        )
    }
}

fn check_error(error: Error, message: &str, offset: u64) {
    match error {
        Error::Input {
            message: actual,
            byte_offset,
        } => {
            assert_eq!(actual, message);
            assert_eq!(byte_offset, offset);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn complete_typed_batches_and_late_page_errors() {
    let cancel = CancellationToken::new();
    for (version, input) in INPUTS.into_iter().enumerate() {
        let memory = Decoder::<Cursor<&[u8]>>::required_memory(&SCHEMA, limits().parquet).unwrap();
        let mut decoder = Decoder::new(
            Cursor::new(input),
            &SCHEMA,
            limits().parquet,
            memory,
            &cancel,
        )
        .unwrap();
        let mut seen = 0;
        for count in [389, 388] {
            let batch = decoder.next_batch(&cancel).unwrap().unwrap();
            assert_eq!(batch.column_count(), 5);
            assert_eq!(batch.row_count(), count);
            for row in 0..count {
                check_row(seen + row, |column| batch.value(row, column));
            }
            seen += count;
        }
        assert_eq!(seen, 777);
        assert!(decoder.next_batch(&cancel).unwrap().is_none());
        drop(decoder);
        let (bytes, message, offset) = damaged(version);
        let mut decoder = Decoder::new(
            Cursor::new(bytes.as_slice()),
            &SCHEMA,
            limits().parquet,
            memory,
            &cancel,
        )
        .unwrap();
        let mut seen = 0;
        for count in [389] {
            let batch = decoder.next_batch(&cancel).unwrap().unwrap();
            assert_eq!(batch.row_count(), count);
            for row in 0..count {
                check_row(seen + row, |column| batch.value(row, column));
            }
            seen += count;
        }
        let error = match decoder.next_batch(&cancel) {
            Err(error) => error,
            Ok(_) => panic!("damaged group exposed"),
        };
        check_error(error, message, offset);
        assert!(matches!(
            decoder.next_batch(&cancel),
            Err(Error::Unsupported("Parquet decoder already failed"))
        ));
    }
}

fn check_table(db: &Database, imported: bool) {
    let query = db.prepare("FROM facts |> ORDER BY ID").unwrap();
    let cancel = CancellationToken::new();
    let mut result = db.execute(&query, &cancel).unwrap();
    let mut seen = 0;
    for _ in 0..10_000 {
        match result.step() {
            QueryStep::Progress => {}
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 5);
                for row in 0..batch.len() {
                    if seen == 0 {
                        assert_eq!(batch.value(row, 4), Some(Value::Int64(-1)));
                        assert_eq!(
                            batch.value(row, 0),
                            Some(Value::String(crate::StringValue::new("existing")))
                        );
                        for column in 1..4 {
                            assert_eq!(batch.value(row, column), Some(Value::Null));
                        }
                    } else {
                        assert!(imported);
                        check_row(seen - 1, |column| batch.value(row, column));
                    }
                    seen += 1;
                }
            }
            QueryStep::Finished => {
                assert_eq!(seen, if imported { 778 } else { 1 });
                return;
            }
            QueryStep::Failed(error) => panic!("{error:?}"),
        }
    }
    panic!("typed Parquet query did not finish");
}

fn database_with_existing_row(path: &Path, cancel: &CancellationToken) -> Database {
    let db = Database::create_empty(path, Config::new(8_000_000, 8_000_000).unwrap()).unwrap();
    db.declare_table("facts", &SCHEMA, cancel).unwrap();
    let day = [DateValue::from_days_since_unix_epoch(0).unwrap()];
    let mut append = db.begin_append("facts", limits().append, cancel).unwrap();
    append
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::String(&["existing"]),
                    validity: &[1],
                },
                ColumnInput {
                    values: ColumnValues::Date(&day),
                    validity: &[0],
                },
                ColumnInput {
                    values: ColumnValues::Double(&[0.0]),
                    validity: &[0],
                },
                ColumnInput {
                    values: ColumnValues::Int64(&[0]),
                    validity: &[0],
                },
                ColumnInput {
                    values: ColumnValues::Int64(&[-1]),
                    validity: &[1],
                },
            ],
            cancel,
        )
        .unwrap();
    append.commit(cancel).unwrap();
    db
}

#[test]
fn inconsistent_footer_counts_fail_before_issuance() {
    let cancel = CancellationToken::new();
    // Literal compact integers in PyArrow's retained files. Each replacement
    // fits the configured limit and preserves the field's wire length.
    for (version, file_rows, group_rows, column_rows, footer_end) in [
        (0, 196_358, 197_237, 197_187, 197_780),
        (1, 196_448, 197_327, 197_277, 197_870),
    ] {
        for (offset, before_bytes, after_bytes, message) in [
            (
                file_rows,
                [0x92, 0x0c], // 777
                [0x90, 0x0c], // 776: differs from 389 + 388 rows.
                "Parquet file row count differs from its row groups",
            ),
            (
                group_rows,
                [0x88, 0x06], // 388
                [0x86, 0x06], // 387: columns still declare 388.
                "Parquet column count or extent differs from its row group",
            ),
            (
                column_rows,
                [0x88, 0x06], // 388
                [0x86, 0x06], // 387: group still declares 388.
                "Parquet column count or extent differs from its row group",
            ),
        ] {
            let input = INPUTS[version];
            let mut bytes = input.to_vec();
            assert_eq!(bytes[offset..offset + 2], before_bytes);
            bytes[offset..offset + 2].copy_from_slice(&after_bytes);
            let directory = Directory::new();
            let db = database_with_existing_row(&directory.0.join("db"), &cancel);
            let memory = db.reserved_memory_bytes();
            let generation = db.generation();
            let error = db
                .import_parquet("facts", Cursor::new(bytes), limits(), &cancel, |_| {
                    panic!("inconsistent footer must fail before issuance")
                })
                .unwrap_err();
            check_error(error, message, footer_end);
            assert_eq!(db.generation(), generation);
            assert_eq!(db.reserved_memory_bytes(), memory);
            assert_eq!(db.reserved_temp_bytes(), 0);
            check_table(&db, false);
            db.import_parquet("facts", Cursor::new(input), limits(), &cancel, |_| Ok(()))
                .unwrap();
            check_table(&db, true);
            assert_eq!(db.reserved_memory_bytes(), memory);
            assert_eq!(db.reserved_temp_bytes(), 0);
            db.close().unwrap();
        }
    }
}

#[test]
fn late_typed_page_failure_preserves_nonempty_table_and_retry() {
    let cancel = CancellationToken::new();
    for (version, input) in INPUTS.into_iter().enumerate() {
        let directory = Directory::new();
        let path = directory.0.join("db");
        let config = || Config::new(8_000_000, 8_000_000).unwrap();
        let db = database_with_existing_row(&path, &cancel);
        let before = db.reserved_memory_bytes();
        let generation = db.generation();
        let objects = || {
            let mut names = std::fs::read_dir(path.join(crate::storage::recovery::UNITS_NAME))
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>();
            names.sort();
            names
        };
        let before_objects = objects();
        assert!(!before_objects.is_empty());
        let issued = Cell::new(None);
        let (bytes, message, offset) = damaged(version);
        let error = db
            .import_parquet("facts", Cursor::new(bytes), limits(), &cancel, |token| {
                issued.set(Some(token));
                Ok(())
            })
            .unwrap_err();
        check_error(error, message, offset);
        assert_eq!(db.generation(), generation);
        assert_eq!(
            objects(),
            before_objects,
            "private import objects must be removed"
        );
        assert_eq!(db.reserved_memory_bytes(), before);
        assert_eq!(db.reserved_temp_bytes(), 0);
        check_table(&db, false);
        let failed = issued.get().unwrap();
        assert!(matches!(
            db.resolve_commit(failed).unwrap(),
            CommitResolution::Aborted
        ));
        let commit = db
            .import_parquet("facts", Cursor::new(input), limits(), &cancel, |token| {
                issued.set(Some(token));
                Ok(())
            })
            .unwrap();
        assert_eq!(db.reserved_memory_bytes(), before);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert!(
            matches!(db.resolve_commit(issued.get().unwrap()).unwrap(), CommitResolution::Durable(receipt) if receipt.generation() == commit.generation())
        );
        check_table(&db, true);
        db.close().unwrap();
        let db = Database::open(&path, config()).unwrap();
        check_table(&db, true);
        assert!(matches!(
            db.resolve_commit(failed).unwrap(),
            CommitResolution::Aborted
        ));
        db.close().unwrap();
    }
}

#[test]
fn typed_export_preserves_values_across_row_text_and_cursor_boundaries() {
    use crate::ParquetExportLimits;
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("db"),
        Config::new(8_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    db.declare_table("facts", &SCHEMA, &cancel).unwrap();
    db.import_parquet("facts", Cursor::new(INPUTS[1]), limits(), &cancel, |_| {
        Ok(())
    })
    .unwrap();
    // PyArrow independently checked the complete schema, values and raw DOUBLE
    // bits. Group sizes are 255, 1, 311, 78, 2, 130: text and row limits both cut.
    let expected = include_bytes!("../../test/data/parquet/pipesql-typed-boundaries.parquet");
    let exact = ParquetExportLimits {
        rows: 777,
        bytes: 194_444,
        row_group_rows: 311,
        row_group_text_bytes: 65_536,
        row_groups: 6,
        metadata_bytes: 1110,
    };
    assert_eq!(expected.len() as u64, exact.bytes);
    let mut shapes = Vec::new();
    for sql in [
        "FROM facts |> SELECT ID AS id, amount, number, day, note",
        "FROM facts |> ORDER BY ID |> SELECT ID AS id, amount, number, day, note",
    ] {
        let query = db.prepare(sql).unwrap();
        let before = db.reserved_memory_bytes();
        let mut shape = Vec::new();
        let mut seen = 0;
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut finished = false;
        for _ in 0..10_000 {
            match result.step() {
                QueryStep::Progress => {}
                QueryStep::Rows(batch) => {
                    assert_eq!(batch.column_count(), 5);
                    shape.push(batch.len());
                    for row in 0..batch.len() {
                        check_row(seen + row, |column| batch.value(row, 4 - column));
                    }
                    seen += batch.len();
                }
                QueryStep::Finished => {
                    finished = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("{error:?}"),
            }
        }
        assert!(finished);
        assert_eq!(seen, 777);
        drop(result);
        shapes.push(shape);
        let mut bytes = Vec::new();
        assert_eq!(
            db.export_parquet(&query, &mut bytes, exact, &cancel)
                .unwrap(),
            777
        );
        assert_eq!(bytes, expected);
        assert_eq!(db.reserved_memory_bytes(), before);
        assert_eq!(db.reserved_temp_bytes(), 0);

        // Prefix lengths come from independently inspected column-chunk offsets.
        // Text refusal occurs before the first group flush; row/group refusals
        // retain five complete groups. Footer refusal retains all six groups.
        for (bounds, owner, required, limit, prefix) in [
            (
                ParquetExportLimits {
                    bytes: exact.bytes - 1,
                    ..exact
                },
                "Parquet output bytes",
                194_444,
                194_443,
                194_440,
            ),
            (
                ParquetExportLimits { rows: 776, ..exact },
                "Parquet output rows",
                777,
                776,
                182_589,
            ),
            (
                ParquetExportLimits {
                    row_groups: 5,
                    ..exact
                },
                "Parquet output row groups",
                6,
                5,
                182_589,
            ),
            (
                ParquetExportLimits {
                    metadata_bytes: 1109,
                    ..exact
                },
                "Parquet footer bytes",
                1110,
                1109,
                193_326,
            ),
            (
                ParquetExportLimits {
                    row_group_text_bytes: 65_535,
                    ..exact
                },
                "Parquet row text bytes",
                65_536,
                65_535,
                4,
            ),
        ] {
            bytes.clear();
            let error = db
                .export_parquet(&query, &mut bytes, bounds, &cancel)
                .unwrap_err();
            match error {
                Error::Resource {
                    owner: actual,
                    required: actual_required,
                    limit: actual_limit,
                } => {
                    assert_eq!(actual, owner);
                    assert_eq!(actual_required, required);
                    assert_eq!(actual_limit, limit);
                }
                other => panic!("unexpected export failure: {other:?}"),
            }
            assert_eq!(bytes, expected[..prefix]);
            assert_eq!(db.reserved_memory_bytes(), before);
            assert_eq!(db.reserved_temp_bytes(), 0);
            bytes.clear();
            assert_eq!(
                db.export_parquet(&query, &mut bytes, exact, &cancel)
                    .unwrap(),
                777
            );
            assert_eq!(bytes, expected);
            assert_eq!(db.reserved_memory_bytes(), before);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
    }
    assert_ne!(
        shapes[0], shapes[1],
        "exercise different query batch boundaries"
    );
    db.close().unwrap();
}
