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
    AppendLimits, CancellationToken, ColumnDeclaration, Config, DataType, Database, Error,
    ParquetExportLimits, ParquetImportLimits, ParquetReadLimits,
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
