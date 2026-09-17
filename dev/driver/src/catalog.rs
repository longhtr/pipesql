//! Create and query databases that the Rust catalog campaign inspects and damages.
//!
//! Setup declares a populated table and an empty table, commits the same nine
//! rows twice, and leaves aborted attempts before and after those appends. After
//! reopening, public queries must return every row twice with its original NULLs,
//! floating-point bits, text and DATE endpoints. Transaction checks separately
//! distinguish the four successful attempts from the two aborted ones.
//!
//! The supervisor copies the database, changes stored bytes and decodes
//! the result independently. It calls this driver to require open or query failure,
//! or to check that a valid changed layout still returns the expected data. The
//! `hold` mode runs the inspector while a real database lease remains held,
//! checking its refusal of an active database. Report-specific
//! cases live in `catalog_report.rs`.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues,
    CommitResolution, Config, DataType, Database, DateValue, Error, QueryStep, TransactionId,
    Value,
};

fn create_graph(path: &std::path::Path, config: Config, cancel: &CancellationToken) {
    let db = Database::create_empty(path, config).unwrap();
    db.declare_table(
        "facts",
        &[
            ColumnDeclaration {
                name: "k",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "d",
                data_type: DataType::Double,
                nullable: true,
            },
            ColumnDeclaration {
                name: "s",
                data_type: DataType::String,
                nullable: true,
            },
            ColumnDeclaration {
                name: "day",
                data_type: DataType::Date,
                nullable: true,
            },
        ],
        cancel,
    )
    .unwrap();
    db.declare_table(
        "empty",
        &[ColumnDeclaration {
            name: "x",
            data_type: DataType::String,
            nullable: true,
        }],
        cancel,
    )
    .unwrap();
    db.begin_append(
        "facts",
        AppendLimits {
            batches: 1,
            encoded_bytes: 100_000,
        },
        cancel,
    )
    .unwrap()
    .abort()
    .unwrap();
    // Repeating the rows makes multiplicity part of the check: a decoder or
    // query that silently deduplicates rows from separate appends must fail.
    for _ in 0..2 {
        let mut writer = db
            .begin_append(
                "facts",
                AppendLimits {
                    batches: 2,
                    encoded_bytes: 100_000,
                },
                cancel,
            )
            .unwrap();
        let keys = [i64::MIN, -1, 0, 1, 2, 3, 4, 5];
        // Use bit checks after reopen to retain the signaling NaN payload and
        // signed zero. Floating-point equality cannot distinguish these cases.
        let doubles = [
            f64::from_bits(0x7ff0000000000001),
            -0.0,
            f64::INFINITY,
            f64::NEG_INFINITY,
            1.0,
            2.0,
            3.0,
            4.0,
        ];
        // The last supplied value is NULL in all three nullable columns. Its
        // backing bytes must not become a visible value.
        let text = ["雪\0", "", "é", "🙂", "same", "same", "a", "ignored"];
        let dates = [DateValue::from_days_since_unix_epoch(-719162).unwrap(); 8];
        writer
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&keys),
                        validity: &[255],
                    },
                    ColumnInput {
                        values: ColumnValues::Double(&doubles),
                        validity: &[127],
                    },
                    ColumnInput {
                        values: ColumnValues::String(&text),
                        validity: &[127],
                    },
                    ColumnInput {
                        values: ColumnValues::Date(&dates),
                        validity: &[127],
                    },
                ],
                cancel,
            )
            .unwrap();
        writer
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&[i64::MAX]),
                        validity: &[1],
                    },
                    ColumnInput {
                        values: ColumnValues::Double(&[f64::from_bits(0xfff8000000001234)]),
                        validity: &[1],
                    },
                    ColumnInput {
                        values: ColumnValues::String(&["end"]),
                        validity: &[1],
                    },
                    ColumnInput {
                        values: ColumnValues::Date(&[DateValue::from_days_since_unix_epoch(
                            2932896,
                        )
                        .unwrap()]),
                        validity: &[1],
                    },
                ],
                cancel,
            )
            .unwrap();
        writer.commit(cancel).unwrap();
    }
    db.begin_append(
        "facts",
        AppendLimits {
            batches: 1,
            encoded_bytes: 100_000,
        },
        cancel,
    )
    .unwrap()
    .abort()
    .unwrap();
    db.close().unwrap();
}

fn check_receipts(db: &Database) {
    // Declarations are successful transactions too: attempts 1 and 2 declare
    // tables; 4 and 5 append data. Aborts 3 and 6 consume IDs, not generations.
    assert_eq!(db.generation(), 4);
    for sequence in 1u64..=6 {
        let mut bytes = [0; 24];
        bytes[..16].copy_from_slice(db.database_identity().as_bytes());
        bytes[16..].copy_from_slice(&sequence.to_le_bytes());
        match db
            .resolve_commit(TransactionId::from_bytes(bytes).unwrap())
            .unwrap()
        {
            CommitResolution::Durable(commit) => assert_eq!(
                commit.generation(),
                match sequence {
                    1 => 1,
                    2 => 2,
                    4 => 3,
                    5 => 4,
                    _ => panic!("wrong successful attempt"),
                }
            ),
            CommitResolution::Aborted => assert!(sequence == 3 || sequence == 6),
        }
    }
}

fn check_rows(db: &Database, cancel: &CancellationToken) {
    let query = db.prepare("FROM facts").unwrap();
    let mut result = db.execute(&query, cancel).unwrap();
    let mut seen = [0_u8; 9];
    for _ in 0..10000 {
        match result.step() {
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let Some(Value::Int64(key)) = batch.value(row, 0) else {
                        panic!("typed key");
                    };
                    let slot = [i64::MIN, -1, 0, 1, 2, 3, 4, 5, i64::MAX]
                        .iter()
                        .position(|&value| value == key)
                        .expect("known row key");
                    let double = [
                        0x7ff0000000000001,
                        0x8000000000000000,
                        0x7ff0000000000000,
                        0xfff0000000000000,
                        0x3ff0000000000000,
                        0x4000000000000000,
                        0x4008000000000000,
                        0,
                        0xfff8000000001234,
                    ][slot];
                    if slot == 7 {
                        for col in 1..4 {
                            assert_eq!(batch.value(row, col), Some(Value::Null));
                        }
                    } else {
                        assert!(matches!(
                            batch.value(row, 1),
                            Some(Value::Double(value)) if value.to_bits() == double
                        ));
                        let text = ["雪\0", "", "é", "🙂", "same", "same", "a", "", "end"][slot];
                        assert!(matches!(
                            batch.value(row, 2),
                            Some(Value::String(value)) if value.as_str() == text
                        ));
                        let day = if slot == 8 { 2932896 } else { -719162 };
                        assert!(matches!(
                            batch.value(row, 3),
                            Some(Value::Date(value)) if value.days_since_unix_epoch() == day
                        ));
                    }
                    seen[slot] = seen[slot].checked_add(1).unwrap();
                }
            }
            QueryStep::Progress => (),
            QueryStep::Finished => break,
            QueryStep::Failed(e) => panic!("query failed: {e:?}"),
        }
    }
    assert_eq!(seen, [2; 9], "complete row multiplicities");
    assert!(matches!(result.step(), QueryStep::Finished));
    drop(result);
    drop(query);
}

#[path = "catalog_report.rs"]
mod report_corruption;

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 3);
    if args[2].starts_with("report-") {
        report_corruption::run(std::path::Path::new(&args[1]), &args[2]);
        return;
    }
    let config = Config::new(4_000_000, 8_000_000).unwrap();
    let cancel = CancellationToken::new();
    if args[2].starts_with("reject") {
        let error = match Database::open(std::path::Path::new(&args[1]), config) {
            Ok(_) => panic!("public open accepted malformed graph"),
            Err(error) => error,
        };
        let expected = match args[2].as_str() {
            "reject-version" => matches!(error, Error::UnsupportedVersion(8)),
            "reject-missing" => {
                matches!(&error, Error::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound)
            }
            "reject" => {
                matches!(error, Error::Corrupt(_))
                    || matches!(&error, Error::RecoveryRequired { source, .. } if matches!(source.kind(), pipesql::CauseKind::Corrupt(_)))
            }
            _ => panic!("unknown catalog rejection mode"),
        };
        assert!(expected, "unexpected public open failure: {error:?}");
        return;
    }
    if args[2] == "genesis" {
        Database::create_empty(std::path::Path::new(&args[1]), config)
            .unwrap()
            .close()
            .unwrap();
        return;
    }
    if args[2] == "setup" {
        create_graph(std::path::Path::new(&args[1]), config, &cancel);
    }
    let db = Database::open(std::path::Path::new(&args[1]), config).unwrap();
    if args[2] == "hold" {
        // The enclosing supervisor bounds this whole process group. The inspector
        // starts only after open acquired the lease, avoiding a readiness channel.
        let inspector = std::env::var_os("PIPESQL_CATALOG_INSPECTOR").unwrap();
        let status = std::process::Command::new(inspector)
            .arg("catalog-locked")
            .arg(&args[1])
            .status()
            .unwrap();
        assert!(
            status.success(),
            "inspector did not reject live engine lease"
        );
        db.close().unwrap();
        return;
    }
    if args[2] == "query-reject" {
        // Opening can succeed before a query reads the damaged payload. Allow
        // earlier rows, but require a terminal failure when the scan reaches it.
        let query = db.prepare("FROM facts").unwrap();
        let mut result = db.execute(&query, &cancel).unwrap();
        for _ in 0..10000 {
            match result.step() {
                QueryStep::Failed(error) => {
                    assert!(
                        matches!(error, Error::Corrupt(_)),
                        "unexpected query failure: {error:?}"
                    );
                    return;
                }
                QueryStep::Finished => panic!("public query accepted corrupt payload"),
                QueryStep::Rows(_) | QueryStep::Progress => (),
            }
        }
        panic!("failed query did not terminate");
    }
    check_receipts(&db);
    check_rows(&db, &cancel);
    db.close().unwrap();
    println!("stock catalog graph passed");
}
