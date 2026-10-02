//! Exercise CSV import with allocation refused after a chosen successful prefix.
//!
//! The supervising campaign varies the prefix across fresh processes. This driver
//! compares reservations with live allocations, including immediately before free,
//! and checks complete literal rows after resolving the early transaction token.
//! Token-output failure can coincide with refused cleanup; both causes must survive.
//! Closing with refusal still armed must release all heap owners. Reopen determines
//! whether rows committed before a retry, rather than assuming an error means abort.

use super::workload::{allocation_cause, arm, finish, format_error, live};
use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, CommitResolution, Config, CsvLimits,
    DataType, Database, Error, ImportLimits, QueryStep, Value,
};
use std::path::Path;

const INPUT: &[u8] = b"id,txt\n1,one\n2,\\N\n3,\"\"\n";
const LIMIT: usize = 1000;

pub(super) fn run(root: &Path, after: Option<usize>, receipt_failure: bool) {
    std::fs::create_dir(root).unwrap();
    let path = root.join("database");
    let config = Config::new(4_000_000, 4_000_000).unwrap();
    let cancel = CancellationToken::new();
    let limits = ImportLimits {
        csv: CsvLimits {
            input_bytes: 1000,
            rows: 10,
            record_bytes: 128,
            field_bytes: 64,
            batch_rows: 2,
            batch_text_bytes: 128,
        },
        append: AppendLimits {
            batches: 2,
            encoded_bytes: 100_000,
        },
    };
    let baseline = live();
    let db = Database::create_empty(&path, config).unwrap();
    db.declare_table(
        "facts",
        &[
            ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "txt",
                data_type: DataType::String,
                nullable: true,
            },
        ],
        &cancel,
    )
    .unwrap();
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
            db.import_csv("facts", INPUT, limits, &cancel, |issued| {
                token = Some(issued);
                if receipt_failure {
                    // Refuse cleanup allocation after the token sink fails.
                    super::ALLOW.store(
                        super::CALLS.load(std::sync::atomic::Ordering::Relaxed),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    super::DENY.store(true, std::sync::atomic::Ordering::Relaxed);
                    return Err(std::io::ErrorKind::BrokenPipe.into());
                }
                Ok(())
            })
        });
        (result, observer.samples())
    };
    assert!(format_error(result.as_ref().err()));
    db.close().unwrap();
    finish("csv-import", baseline);
    assert!(
        samples.requested_headroom >= 0 && samples.usable_headroom >= 0,
        "CSV import buffers exceeded their reservations: {samples:?}"
    );
    if after.is_none() {
        assert!(samples.allocations > 0 && samples.frees > 0);
    }
    println!("CSV import allocation/free ownership: {samples:?}");
    let calls = super::CALLS.load(std::sync::atomic::Ordering::Relaxed);
    assert!(calls > 0 && calls <= LIMIT);
    match &result {
        Ok(_) => println!("CSV import outcome=committed"),
        Err(Error::Resource { .. }) => println!("CSV import outcome=refused"),
        Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::OutOfMemory => {
            println!("CSV import outcome=refused")
        }
        Err(Error::CleanupRequired { primary, cleanup }) if receipt_failure => {
            assert!(
                matches!(primary.kind(), pipesql::CauseKind::Io { operation: "report CSV transaction", source } if source.kind() == std::io::ErrorKind::BrokenPipe)
            );
            assert!(allocation_cause(cleanup.kind()));
            println!("CSV import receipt cleanup passed");
        }
        Err(Error::CleanupRequired { primary, cleanup })
            if allocation_cause(primary.kind()) && allocation_cause(cleanup.kind()) =>
        {
            println!("CSV import outcome=cleanup")
        }
        Err(Error::RecoveryRequired { source, .. }) if allocation_cause(source.kind()) => {
            println!("CSV import outcome=recovery")
        }
        Err(Error::CommitAmbiguous {
            transaction,
            source,
        }) if allocation_cause(source.kind()) => {
            assert_eq!(Some(*transaction), token);
            println!("CSV import outcome=ambiguous");
        }
        other => panic!("unexpected import outcome: {other:?}"),
    }
    if receipt_failure {
        assert!(matches!(result, Err(Error::CleanupRequired { .. })));
    } else if after.is_none() {
        assert!(result.is_ok());
    }
    let db = Database::open(&path, config).unwrap();
    let durable = token.is_some_and(|token| {
        matches!(
            db.resolve_commit(token).unwrap(),
            CommitResolution::Durable(_)
        )
    });
    assert_eq!(db.generation(), if durable { 2 } else { 1 });
    assert_eq!(db.reserved_temp_bytes(), 0);
    check_rows(&db, durable);
    if !durable {
        db.import_csv("facts", INPUT, limits, &cancel, |_| Ok(()))
            .unwrap();
        check_rows(&db, true);
    }
    db.close().unwrap();
    println!("CSV import recovery and complete rows passed");
}

fn check_rows(db: &Database, populated: bool) {
    let query = db.prepare("FROM facts |> ORDER BY id").unwrap();
    let cancel = CancellationToken::new();
    let mut result = db.execute(&query, &cancel).unwrap();
    let mut rows = 0;
    for _ in 0..1000 {
        match result.step() {
            QueryStep::Rows(batch) => {
                assert!(populated);
                assert_eq!(batch.column_count(), 2);
                for row in 0..batch.len() {
                    assert!(rows < 3);
                    assert_eq!(batch.value(row, 0), Some(Value::Int64([1, 2, 3][rows])));
                    match rows {
                        0 => assert!(
                            matches!(batch.value(row, 1), Some(Value::String(value)) if value.as_str() == "one")
                        ),
                        1 => assert_eq!(batch.value(row, 1), Some(Value::Null)),
                        2 => assert!(
                            matches!(batch.value(row, 1), Some(Value::String(value)) if value.as_str().is_empty())
                        ),
                        _ => unreachable!(),
                    }
                    rows += 1;
                }
            }
            QueryStep::Progress => {}
            QueryStep::Finished => {
                assert_eq!(rows, if populated { 3 } else { 0 });
                return;
            }
            QueryStep::Failed(error) => panic!("{error:?}"),
        }
    }
    panic!("import result did not finish");
}
