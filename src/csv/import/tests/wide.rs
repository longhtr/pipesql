//! Keep maximum-width conversion and late abort within the measured stack bound.
//!
//! Header, declaration and result orders differ. Each repeated type has distinct
//! values and nullable columns change independently across nine-row batches.

use super::*;
use std::fmt::Write as _;

fn object_bytes(db: &Database) -> u64 {
    std::fs::read_dir(db.path().join(crate::storage::recovery::UNITS_NAME))
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum()
}

#[test]
fn maximum_width_import_aborts_and_retries_on_the_measured_small_stack() {
    std::thread::Builder::new()
        .stack_size(pipesql_filesystem::TEST_SMALL_STACK_REQUEST_BYTES)
        .spawn(|| {
            pipesql_filesystem::test_assert_small_stack();
            check_import();
        })
        .unwrap()
        .join()
        .unwrap();
}

fn check_import() {
    let directory = Directory::new();
    let path = directory.0.join("db");
    let config = Config::new(32_000_000, 16_000_000).unwrap();
    let mut db = Database::create_empty(&path, config).unwrap();
    let cancel = CancellationToken::new();
    let names: Vec<_> = (0..64).map(|column| format!("c{column:02}")).collect();
    let schema: Vec<_> = (0..64)
        .rev()
        .map(|column| ColumnDeclaration {
            name: &names[column],
            data_type: [
                DataType::Int64,
                DataType::Double,
                DataType::Date,
                DataType::String,
            ][column % 4],
            nullable: column % 3 != 0,
        })
        .collect();
    db.declare_table("facts", &schema, &cancel).unwrap();
    let epoch = [DateValue::from_days_since_unix_epoch(0).unwrap()];
    let seed: Vec<_> = (0..64)
        .rev()
        .map(|column| crate::ColumnInput {
            values: match column % 4 {
                0 => crate::ColumnValues::Int64(&[-1]),
                1 => crate::ColumnValues::Double(&[0.0]),
                2 => crate::ColumnValues::Date(&epoch),
                _ => crate::ColumnValues::String(&["existing"]),
            },
            validity: &[1],
        })
        .collect();
    {
        let mut append = db
            .begin_append(
                "facts",
                AppendLimits {
                    batches: 1,
                    encoded_bytes: 16_384,
                },
                &cancel,
            )
            .unwrap();
        append.write(&seed, &cancel).unwrap();
        append.commit(&cancel).unwrap();
    }
    let order: Vec<_> = (0..64).map(|ordinal| (ordinal * 17 + 7) % 64).collect();
    let mut input = order
        .iter()
        .map(|&column| names[column].as_str())
        .collect::<Vec<_>>()
        .join(",");
    input.push('\n');
    for row in (0..65).rev() {
        for (ordinal, &column) in order.iter().enumerate() {
            if ordinal != 0 {
                input.push(',');
            }
            if column % 3 != 0 && (row + 3 * column) % 11 == 0 {
                input.push_str(r"\N");
                continue;
            }
            match column % 4 {
                0 => write!(
                    input,
                    "{}",
                    if column == 0 { row } else { row * 100 + column }
                )
                .unwrap(),
                1 if row % 7 == 0 => input.push_str("-0"),
                1 => write!(input, "{}.{:02}", column + row / 4, (row % 4) * 25).unwrap(),
                2 => write!(input, "2000-01-{:02}", (row + column) % 28 + 1).unwrap(),
                _ => write!(input, "\"雪 {column},\"\"{row}\"\"\n\"").unwrap(),
            }
        }
        input.push('\n');
    }
    let bounds = ImportLimits {
        csv: CsvLimits {
            input_bytes: input.len() as u64,
            rows: 65,
            record_bytes: 4096,
            field_bytes: 128,
            batch_rows: 9,
            batch_text_bytes: 8192,
        },
        append: AppendLimits {
            batches: 8,
            encoded_bytes: 1_000_000,
        },
    };
    struct FinalError<'a> {
        input: &'a [u8],
        db: &'a Database,
        written: &'a Cell<u64>,
        calls: &'a Cell<usize>,
    }
    impl Read for FinalError<'_> {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            if self.input.is_empty() {
                self.written.set(object_bytes(self.db));
                self.calls.set(self.calls.get() + 1);
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            let count = output.len().min(7);
            self.input.read(&mut output[..count])
        }
    }
    let baseline = db.reserved_memory_bytes();
    let generation = db.generation();
    let initial_bytes = object_bytes(&db);
    let written = Cell::new(0);
    let calls = Cell::new(0);
    let mut token = None;
    let error = db
        .import_csv(
            "facts",
            FinalError {
                input: input.as_bytes(),
                db: &db,
                written: &written,
                calls: &calls,
            },
            bounds,
            &cancel,
            |issued| {
                token = Some(issued);
                Ok(())
            },
        )
        .unwrap_err();
    assert!(
        matches!(error, Error::Io { source, .. } if source.kind() == std::io::ErrorKind::BrokenPipe)
    );
    assert_eq!(calls.get(), 1);
    assert!(
        written.get() > initial_bytes,
        "source failure must follow native writes"
    );
    assert_eq!(object_bytes(&db), initial_bytes);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert_eq!(db.generation(), generation);
    let token = token.unwrap();
    assert_eq!(db.resolve_commit(token).unwrap(), CommitResolution::Aborted);
    check_rows(&db, &names, false);
    db.close().unwrap();
    db = Database::open(&path, config).unwrap();
    assert_eq!(db.resolve_commit(token).unwrap(), CommitResolution::Aborted);
    check_rows(&db, &names, false);
    let baseline = db.reserved_memory_bytes();
    let commit = db
        .import_csv("facts", input.as_bytes(), bounds, &cancel, |_| Ok(()))
        .unwrap();
    assert_eq!(commit.generation(), generation + 1);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close().unwrap();
    let db = Database::open(&path, config).unwrap();
    assert_eq!(db.resolve_commit(token).unwrap(), CommitResolution::Aborted);
    assert_eq!(
        db.resolve_commit(commit.transaction()).unwrap(),
        CommitResolution::Durable(commit)
    );
    check_rows(&db, &names, true);
    db.close().unwrap();
}

fn check_rows(db: &Database, names: &[String], populated: bool) {
    let baseline = db.reserved_memory_bytes();
    let query = db
        .prepare(&format!(
            "FROM facts |> ORDER BY c00 |> SELECT {}",
            names.join(", ")
        ))
        .unwrap();
    let cancel = CancellationToken::new();
    let mut result = db.execute(&query, &cancel).unwrap();
    let mut seen = 0;
    let mut finished = false;
    for _ in 0..10_000 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 64);
                for offset in 0..batch.len() {
                    assert!(seen < if populated { 66 } else { 1 });
                    let row = seen as i64 - 1;
                    for column in 0..64 {
                        let text;
                        let wanted =
                            if row >= 0 && column % 3 != 0 && (row + column as i64 * 3) % 11 == 0 {
                                Value::Null
                            } else {
                                match column % 4 {
                                    0 => Value::Int64(if row < 0 || column == 0 {
                                        row
                                    } else {
                                        row * 100 + column as i64
                                    }),
                                    1 => Value::Double(if row < 0 {
                                        0.0
                                    } else if row % 7 == 0 {
                                        -0.0
                                    } else {
                                        column as f64 + row as f64 / 4.0
                                    }),
                                    2 => Value::Date(
                                        DateValue::from_days_since_unix_epoch(if row < 0 {
                                            0
                                        } else {
                                            10957 + (row as i32 + column as i32) % 28
                                        })
                                        .unwrap(),
                                    ),
                                    _ => {
                                        text = if row < 0 {
                                            "existing".into()
                                        } else {
                                            format!("雪 {column},\"{row}\"\n")
                                        };
                                        Value::String(StringValue::new(&text))
                                    }
                                }
                            };
                        if let Value::Double(wanted) = wanted {
                            assert!(
                                matches!(batch.value(offset, column), Some(Value::Double(actual)) if actual.to_bits() == wanted.to_bits()),
                                "row {row}, column {column}"
                            );
                        } else {
                            assert_eq!(
                                batch.value(offset, column),
                                Some(wanted),
                                "row {row}, column {column}"
                            );
                        }
                    }
                    seen += 1;
                }
            }
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("{error:?}"),
        }
    }
    assert!(finished);
    assert_eq!(seen, if populated { 66 } else { 1 });
    drop(result);
    drop(query);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
}
