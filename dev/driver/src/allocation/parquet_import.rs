//! Refuse each Parquet import allocation prefix and resolve publication after reopen.
//!
//! The stored external fixture includes three row groups, every scalar type and
//! a maximum-length string. A preexisting row must survive every refusal. Denial
//! remains armed through error formatting and close; recovery decides whether a
//! retry is needed. Literal typed answers do not use the engine's Parquet encoder.

use super::workload::{allocation_cause, arm, finish, format_error, live};
use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues,
    CommitResolution, Config, DataType, Database, DateValue, Error, ParquetImportLimits,
    ParquetReadLimits, QueryStep, Value,
};
use std::{io::Cursor, path::Path, sync::atomic::Ordering};

const INPUT: &[u8] = include_bytes!("../../../../test/data/parquet/plain-v2.parquet");
const LIMIT: usize = 1000;

pub(super) fn run(root: &Path, after: Option<usize>, receipt_failure: bool) {
    std::fs::create_dir(root).unwrap();
    let path = root.join("database");
    let config = Config::new(32_000_000, 8_000_000).unwrap();
    let cancel = CancellationToken::new();
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
            batches: 3,
            encoded_bytes: 200_000,
        },
    };
    let baseline = live();
    let db = Database::create_empty(&path, config).unwrap();
    let schema = [
        ("note", DataType::String, true),
        ("day", DataType::Date, true),
        ("number", DataType::Double, true),
        ("amount", DataType::Int64, true),
        ("id", DataType::Int64, false),
    ]
    .map(|(name, data_type, nullable)| ColumnDeclaration {
        name,
        data_type,
        nullable,
    });
    db.declare_table("facts", &schema, &cancel).unwrap();
    {
        let dates = [DateValue::from_days_since_unix_epoch(-1).unwrap()];
        let values = [
            ColumnValues::String(&["existing"]),
            ColumnValues::Date(&dates),
            ColumnValues::Double(&[1.25]),
            ColumnValues::Int64(&[99]),
            ColumnValues::Int64(&[0]),
        ]
        .map(|values| ColumnInput {
            values,
            validity: &[1],
        });
        let mut append = db
            .begin_append(
                "facts",
                AppendLimits {
                    batches: 1,
                    encoded_bytes: 4096,
                },
                &cancel,
            )
            .unwrap();
        append.write(&values, &cancel).unwrap();
        append.commit(&cancel).unwrap();
    }
    let mut token = None;
    arm(after, LIMIT);
    let import_live = live();
    let (result, samples) = {
        let observer = super::transient_ownership::Observer::new(
            &db,
            db.reserved_memory_bytes(),
            import_live.requested,
            import_live.usable,
        );
        let result = observer.during(|| {
            db.import_parquet("facts", Cursor::new(INPUT), limits, &cancel, |issued| {
                token = Some(issued);
                if receipt_failure {
                    super::ALLOW.store(super::CALLS.load(Ordering::Relaxed), Ordering::Relaxed);
                    super::DENY.store(true, Ordering::Relaxed);
                    return Err(std::io::ErrorKind::BrokenPipe.into());
                }
                Ok(())
            })
        });
        (result, observer.samples())
    };
    assert!(format_error(result.as_ref().err()));
    db.close().unwrap();
    finish("parquet-import", baseline);
    assert!(
        samples.requested_headroom >= 0 && samples.usable_headroom >= 0,
        "Parquet import buffers exceeded reservations: {samples:?}"
    );
    if after.is_none() {
        assert!(samples.allocations > 0 && samples.frees > 0);
    }
    println!("Parquet import allocation/free ownership: {samples:?}");
    let calls = super::CALLS.load(Ordering::Relaxed);
    assert!(calls > 0 && calls <= LIMIT);
    match &result {
        Ok(_) => println!("Parquet import outcome=committed"),
        Err(Error::Resource { .. }) => println!("Parquet import outcome=refused"),
        Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::OutOfMemory => {
            println!("Parquet import outcome=refused")
        }
        Err(Error::CleanupRequired { primary, cleanup }) if receipt_failure => {
            assert!(
                matches!(primary.kind(), pipesql::CauseKind::Io { operation: "report Parquet transaction", source } if source.kind() == std::io::ErrorKind::BrokenPipe)
            );
            assert!(allocation_cause(cleanup.kind()));
            println!("Parquet import receipt cleanup passed");
        }
        Err(Error::CleanupRequired { primary, cleanup })
            if allocation_cause(primary.kind()) && allocation_cause(cleanup.kind()) =>
        {
            println!("Parquet import outcome=cleanup")
        }
        Err(Error::RecoveryRequired { source, .. }) if allocation_cause(source.kind()) => {
            println!("Parquet import outcome=recovery")
        }
        Err(Error::CommitAmbiguous {
            transaction,
            source,
        }) if allocation_cause(source.kind()) => {
            assert_eq!(Some(*transaction), token);
            println!("Parquet import outcome=ambiguous");
        }
        other => panic!("unexpected Parquet import outcome: {other:?}"),
    }
    if receipt_failure {
        assert!(matches!(result, Err(Error::CleanupRequired { .. })));
    } else if after.is_none() {
        assert!(result.is_ok());
    }
    let mut db = Database::open(&path, config).unwrap();
    let durable = token.is_some_and(|token| {
        matches!(
            db.resolve_commit(token).unwrap(),
            CommitResolution::Durable(_)
        )
    });
    match &result {
        Ok(commit) => assert_eq!(
            db.resolve_commit(commit.transaction()).unwrap(),
            CommitResolution::Durable(*commit)
        ),
        Err(Error::CommitAmbiguous { .. }) => (),
        Err(_) => assert!(!durable, "definite import failure published rows"),
    }
    assert_eq!(db.generation(), if durable { 3 } else { 2 });
    assert_eq!(db.reserved_temp_bytes(), 0);
    check_rows(&db, durable);
    if !durable {
        let commit = db
            .import_parquet("facts", Cursor::new(INPUT), limits, &cancel, |_| Ok(()))
            .unwrap();
        db.close().unwrap();
        db = Database::open(&path, config).unwrap();
        assert_eq!(
            db.resolve_commit(commit.transaction()).unwrap(),
            CommitResolution::Durable(commit)
        );
        if let Some(token) = token {
            assert_eq!(db.resolve_commit(token).unwrap(), CommitResolution::Aborted);
        }
        check_rows(&db, true);
    }
    db.close().unwrap();
    println!("Parquet import recovery and complete rows passed");
}

fn check_rows(db: &Database, populated: bool) {
    let memory = db.reserved_memory_bytes();
    let query = db
        .prepare("FROM facts |> ORDER BY id |> SELECT id, amount, number, day, note")
        .unwrap();
    let cancel = CancellationToken::new();
    let mut result = db.execute(&query, &cancel).unwrap();
    let integers = [
        Some(i64::MIN),
        Some(i64::MAX),
        None,
        Some(-1),
        Some(0),
        Some(1),
        Some(42),
        Some(-42),
    ];
    let numbers = [
        Some(0_u64),
        Some(0x8000000000000000),
        Some(0x7ff0000000000000),
        Some(0xfff0000000000000),
        Some(0x7ff8000000001234),
        Some(0xfff0000000000001),
        Some(1),
        None,
    ];
    let dates = [
        Some(-719162),
        Some(2932896),
        None,
        Some(-1),
        Some(0),
        Some(1),
        Some(11016),
        Some(18262),
    ];
    let large = "x".repeat(65_536);
    let texts = [
        Some(""),
        None,
        Some("é🙂"),
        Some("a,\n\"\\\0"),
        Some(r"\N"),
        Some(large.as_str()),
        Some("tail"),
        Some("end"),
    ];
    let mut rows = 0;
    let mut finished = false;
    for _ in 0..1000 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 5);
                for row in 0..batch.len() {
                    assert!(rows < if populated { 9 } else { 1 });
                    let expected = if rows == 0 {
                        [
                            Value::Int64(0),
                            Value::Int64(99),
                            Value::Double(1.25),
                            Value::Date(DateValue::from_days_since_unix_epoch(-1).unwrap()),
                        ]
                    } else {
                        let i = rows - 1;
                        [
                            Value::Int64(rows as i64),
                            integers[i].map_or(Value::Null, Value::Int64),
                            numbers[i]
                                .map_or(Value::Null, |bits| Value::Double(f64::from_bits(bits))),
                            dates[i].map_or(Value::Null, |days| {
                                Value::Date(DateValue::from_days_since_unix_epoch(days).unwrap())
                            }),
                        ]
                    };
                    for (column, wanted) in expected.into_iter().enumerate() {
                        if let Value::Double(wanted) = wanted {
                            assert!(
                                matches!(batch.value(row, column), Some(Value::Double(actual)) if actual.to_bits() == wanted.to_bits())
                            );
                        } else {
                            assert_eq!(batch.value(row, column), Some(wanted));
                        }
                    }
                    let wanted = if rows == 0 {
                        Some("existing")
                    } else {
                        texts[rows - 1]
                    };
                    match (batch.value(row, 4), wanted) {
                        (Some(Value::Null), None) => (),
                        (Some(Value::String(actual)), Some(wanted)) => {
                            assert_eq!(actual.as_str(), wanted)
                        }
                        other => panic!("Parquet import text: {other:?}"),
                    }
                    rows += 1;
                }
            }
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("Parquet import result: {error:?}"),
        }
    }
    assert!(finished);
    assert_eq!(rows, if populated { 9 } else { 1 });
    drop(result);
    drop(query);
    assert_eq!(db.reserved_memory_bytes(), memory);
    assert_eq!(db.reserved_temp_bytes(), 0);
}
