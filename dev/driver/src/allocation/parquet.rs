//! Observe Parquet import/export buffers during allocation and before release.
//!
//! The caller owns the fixed output array and input fixture. Neither grows during
//! observation. Each operation must cover requested and allocator-rounded bytes
//! with reservations, including receipt failure and incomplete export cleanup.
//!
//! Successful transfers, row or byte limits and failed receipt writes use the same
//! observer. Each observed call must actually allocate and free, then leave no
//! temporary-space charge; an empty observation is not a passing memory check.

use super::transient_ownership::Observer;
use super::workload::live;
use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, Error, ParquetExportLimits, ParquetImportLimits, ParquetReadLimits,
    QueryStep, Value,
};
use std::io::Cursor;
use std::path::Path;

fn observed<T>(db: &Database, call: impl FnOnce() -> T) -> T {
    let before = live();
    let reserved = db.reserved_memory_bytes();
    let observer = Observer::new(db, reserved, before.requested, before.usable);
    let result = observer.during(call);
    let samples = observer.samples();
    assert!(samples.allocations > 0 && samples.frees > 0);
    assert!(
        samples.requested_headroom >= 0 && samples.usable_headroom >= 0,
        "Parquet buffers exceeded their reservations: {samples:?}"
    );
    assert_eq!(db.reserved_temp_bytes(), 0);
    println!("Parquet allocation/free ownership: {samples:?}");
    result
}

fn wide_descriptors(root: &Path) {
    // Preparing a 64-column query needs its existing full native workspace even
    // for one row. This separate fixture budget does not change the small cases.
    let db = Database::create_empty(
        &root.join("wide-database"),
        Config::new(64_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    // A small caller-owned input can still admit the maximum descriptor array.
    // Construct its bytes outside observation; literal empty values below check
    // the result independently of the codec used to prepare this allocation case.
    let names: Vec<_> = (0..64).map(|i| format!("c{i}")).collect();
    let wide_schema: Vec<_> = names
        .iter()
        .map(|name| ColumnDeclaration {
            name,
            data_type: DataType::String,
            nullable: true,
        })
        .collect();
    db.declare_table("wide", &wide_schema, &cancel).unwrap();
    db.declare_table("wide_source", &wide_schema, &cancel)
        .unwrap();
    let mut append = db
        .begin_append(
            "wide_source",
            AppendLimits {
                batches: 1,
                encoded_bytes: 64_000,
            },
            &cancel,
        )
        .unwrap();
    append
        .write(
            &[ColumnInput {
                values: ColumnValues::String(&[""]),
                validity: &[1],
            }; 64],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    let query = db.prepare("FROM wide_source").unwrap();
    let mut wide = Vec::new();
    assert_eq!(
        db.export_parquet(
            &query,
            &mut wide,
            ParquetExportLimits {
                rows: 1,
                bytes: 64_000,
                row_group_rows: 1,
                row_group_text_bytes: 1,
                row_groups: 1,
                metadata_bytes: 16_384,
            },
            &cancel
        )
        .unwrap(),
        1
    );
    drop(query);
    super::cache_allocation(1_032_192);
    observed(&db, || {
        db.import_parquet(
            "wide",
            Cursor::new(&wide),
            ParquetImportLimits {
                parquet: ParquetReadLimits {
                    input_bytes: 64_000,
                    rows: 1,
                    metadata_bytes: 16_384,
                    row_groups: 1,
                    row_group_rows: 512,
                    row_group_bytes: 16_384,
                    page_bytes: 1024,
                },
                append: AppendLimits {
                    batches: 1,
                    encoded_bytes: 64_000,
                },
            },
            &cancel,
            |_| Ok(()),
        )
    })
    .unwrap();
    let query = db.prepare("FROM wide").unwrap();
    let mut result = db.execute(&query, &cancel).unwrap();
    let mut rows = 0;
    let mut finished = false;
    for _ in 0..1000 {
        match result.step() {
            QueryStep::Progress => {}
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 64);
                for row in 0..batch.len() {
                    rows += 1;
                    for column in 0..64 {
                        assert!(
                            matches!(batch.value(row, column), Some(Value::String(value)) if value.as_str().is_empty())
                        );
                    }
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
    assert_eq!(rows, 1);
    drop(result);
    drop(query);
    db.close().unwrap();
}

pub(super) fn run(root: &Path) {
    std::fs::create_dir(root).unwrap();
    let db = Database::create_empty(
        &root.join("database"),
        Config::new(32_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let schema = [
        ("id", DataType::Int64, false),
        ("amount", DataType::Int64, true),
        ("number", DataType::Double, true),
        ("day", DataType::Date, true),
        ("note", DataType::String, true),
    ]
    .map(|(name, data_type, nullable)| ColumnDeclaration {
        name,
        data_type,
        nullable,
    });
    db.declare_table("facts", &schema, &cancel).unwrap();
    super::transient_ownership::check_calibration(&db, false);
    let input = &include_bytes!("../../../../test/data/parquet/plain-v2.parquet")[..];
    let limits = ParquetImportLimits {
        parquet: ParquetReadLimits {
            input_bytes: 100_000,
            rows: 8,
            metadata_bytes: 16_384,
            row_groups: 3,
            row_group_rows: 3,
            row_group_bytes: 70_000,
            page_bytes: 70_000,
        },
        append: AppendLimits {
            batches: 8,
            encoded_bytes: 200_000,
        },
    };
    // Only the encoded-group bound changes. A large freed block can back that
    // buffer even though the actual fixture is small. Receipt failure must still
    // release it without publishing rows.
    super::cache_allocation(7_503_872);
    let cached = observed(&db, || {
        db.import_parquet(
            "facts",
            Cursor::new(input),
            ParquetImportLimits {
                parquet: ParquetReadLimits {
                    row_group_bytes: 3_817_440,
                    ..limits.parquet
                },
                ..limits
            },
            &cancel,
            |_| Err(std::io::ErrorKind::BrokenPipe.into()),
        )
    });
    assert!(
        matches!(cached, Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::BrokenPipe)
    );
    let failed = observed(&db, || {
        db.import_parquet("facts", Cursor::new(input), limits, &cancel, |_| {
            Err(std::io::ErrorKind::BrokenPipe.into())
        })
    });
    assert!(
        matches!(failed, Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::BrokenPipe)
    );
    observed(&db, || {
        db.import_parquet("facts", Cursor::new(input), limits, &cancel, |_| Ok(()))
    })
    .unwrap();
    // Exercise larger conversion vectors and borrowed text descriptors, not just
    // the small retained eight-row fixture used by export checks below.
    db.declare_table("boundary", &schema, &cancel).unwrap();
    let boundary = &include_bytes!("../../../../test/data/parquet/typed-boundaries-v2.parquet")[..];
    let bounds = ParquetImportLimits {
        parquet: ParquetReadLimits {
            input_bytes: 250_000,
            rows: 777,
            metadata_bytes: 8192,
            row_groups: 2,
            row_group_rows: 512,
            row_group_bytes: 120_000,
            page_bytes: 70_000,
        },
        append: AppendLimits {
            batches: 16,
            encoded_bytes: 500_000,
        },
    };
    let failed = observed(&db, || {
        db.import_parquet("boundary", Cursor::new(boundary), bounds, &cancel, |_| {
            Err(std::io::ErrorKind::BrokenPipe.into())
        })
    });
    assert!(
        matches!(failed, Err(Error::Io { operation: "report Parquet transaction", source })
        if source.kind() == std::io::ErrorKind::BrokenPipe)
    );
    observed(&db, || {
        db.import_parquet("boundary", Cursor::new(boundary), bounds, &cancel, |_| {
            Ok(())
        })
    })
    .unwrap();
    wide_descriptors(root);
    let query = db.prepare("FROM facts |> ORDER BY id").unwrap();
    let limits = ParquetExportLimits {
        rows: 8,
        bytes: 100_000,
        row_group_rows: 3,
        row_group_text_bytes: 65_536,
        row_groups: 8,
        metadata_bytes: 16_384,
    };
    let mut storage = [0_u8; 100_000];
    for case in 0..5 {
        let mut output = &mut storage[..if case == 3 { 10 } else { 100_000 }];
        if case == 4 {
            super::cache_allocation(7_503_872);
        }
        let bounds = match case {
            4 => ParquetExportLimits {
                metadata_bytes: 3_817_440,
                ..limits
            },
            1 => ParquetExportLimits { rows: 7, ..limits },
            2 => ParquetExportLimits {
                row_group_text_bytes: 65_535,
                ..limits
            },
            _ => limits,
        };
        let result = observed(&db, || {
            db.export_parquet(&query, &mut output, bounds, &cancel)
        });
        let remaining = output.len();
        match case {
            0 | 4 => {
                assert_eq!(result.unwrap(), 8);
                assert_eq!(
                    &storage[..100_000 - remaining],
                    include_bytes!("../../../../test/data/parquet/pipesql-v1.parquet")
                );
            }
            1 | 2 => assert!(matches!(result, Err(Error::Resource { .. }))),
            3 => assert!(
                matches!(result, Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::WriteZero)
            ),
            _ => unreachable!(),
        }
    }
    drop(query);
    db.close().unwrap();
    println!(
        "Parquet ownership passed: receipt failure, import, complete export and three output failures"
    );
}
