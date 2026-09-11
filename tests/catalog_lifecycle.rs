#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "catalog_lifecycle/aggregates.rs"]
mod aggregates;

#[path = "catalog_lifecycle/append.rs"]
mod append;

#[path = "catalog_lifecycle/boolean.rs"]
mod boolean;

#[path = "catalog_lifecycle/computed.rs"]
mod computed;

#[path = "catalog_lifecycle/distinct.rs"]
mod distinct;

#[path = "catalog_lifecycle/grouping.rs"]
mod grouping;

#[path = "catalog_lifecycle/join_corpus.rs"]
mod join_corpus;

#[path = "catalog_lifecycle/joins.rs"]
mod joins;

#[path = "catalog_lifecycle/limit.rs"]
mod limit;

#[path = "catalog_lifecycle/null_predicate.rs"]
mod null_predicate;

#[path = "catalog_lifecycle/order.rs"]
mod order;

#[path = "catalog_lifecycle/snapshots.rs"]
mod snapshots;

#[path = "catalog_lifecycle/spooling.rs"]
mod spooling;

#[path = "catalog_lifecycle/text_filter.rs"]
mod text_filter;

#[path = "catalog_lifecycle/wide.rs"]
mod wide;

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues,
    CommitResolution, Config, DataType, Database, DateValue, Error, QueryResult, QueryStep, Value,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "pipesql-public-catalog-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn database(&self) -> PathBuf {
        self.0.join("db")
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            // Preserve the test failure during unwinding; report cleanup failures
            // when the scenario itself completed successfully.
            if error.kind() != std::io::ErrorKind::NotFound && !std::thread::panicking() {
                panic!("remove test directory {}: {error}", self.0.display());
            }
        }
    }
}

fn config() -> Config {
    Config::new(4_000_000, 2_000_000).unwrap()
}

fn limits() -> AppendLimits {
    AppendLimits {
        batches: 2,
        encoded_bytes: 100_000,
    }
}

fn declarations() -> [ColumnDeclaration<'static>; 4] {
    [
        ColumnDeclaration {
            name: "note",
            data_type: DataType::String,
            nullable: true,
        },
        ColumnDeclaration {
            name: "amount",
            data_type: DataType::Int64,
            nullable: false,
        },
        ColumnDeclaration {
            name: "number",
            data_type: DataType::Double,
            nullable: false,
        },
        ColumnDeclaration {
            name: "day",
            data_type: DataType::Date,
            nullable: false,
        },
    ]
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Cell {
    Null,
    Integer(i64),
    Number(u64),
    Text(String),
    Day(i32),
}

fn owned_cell(value: Value<'_>) -> Cell {
    match value {
        Value::Null => Cell::Null,
        Value::Int64(value) => Cell::Integer(value),
        Value::Double(value) => Cell::Number(value.to_bits()),
        Value::String(value) => Cell::Text(value.as_str().to_owned()),
        Value::Date(value) => Cell::Day(value.days_since_unix_epoch()),
    }
}

fn join_fixture() -> (Directory, Database) {
    let directory = Directory::new();
    let cancel = CancellationToken::new();
    let db = Database::create_empty(
        &directory.database(),
        Config::new(8_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let key = ColumnDeclaration {
        name: "k",
        data_type: DataType::Int64,
        nullable: true,
    };
    db.declare_table(
        "facts",
        &[
            key,
            ColumnDeclaration {
                name: "v",
                data_type: DataType::Int64,
                nullable: false,
            },
        ],
        &cancel,
    )
    .unwrap();
    db.declare_table(
        "dimensions",
        &[
            key,
            ColumnDeclaration {
                name: "label",
                data_type: DataType::String,
                nullable: false,
            },
        ],
        &cancel,
    )
    .unwrap();
    for (table, value) in [
        ("facts", ColumnValues::Int64(&[10, 20, 30, 40])),
        ("dimensions", ColumnValues::String(&["a", "b", "c", "null"])),
    ] {
        let mut writer = db.begin_append(table, limits(), &cancel).unwrap();
        writer
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&[1, 1, 2, 0]),
                        validity: &[0b0111],
                    },
                    ColumnInput {
                        values: value,
                        validity: &[0b1111],
                    },
                ],
                &cancel,
            )
            .unwrap();
        writer.commit(&cancel).unwrap();
    }
    (directory, db)
}

fn assert_query_rows(query: &str, mut expected: Vec<Vec<Cell>>) {
    let (_directory, db) = join_fixture();
    let cancel = CancellationToken::new();
    let resident = db.reserved_memory_bytes();
    let prepared = db.prepare(query).unwrap();
    let mut result = db.execute(&prepared, &cancel).unwrap();
    let mut actual = collect(&mut result);
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
    drop(result);
    drop(prepared);
    assert_eq!(db.reserved_memory_bytes(), resident);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close().unwrap();
}

fn collect(result: &mut QueryResult<'_, '_>) -> Vec<Vec<Cell>> {
    let mut rows = Vec::new();
    for _ in 0..1024 {
        match result.step() {
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    rows.push(
                        (0..batch.column_count())
                            .map(|column| owned_cell(batch.value(row, column).unwrap()))
                            .collect(),
                    );
                }
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                rows.sort_unstable();
                return rows;
            }
            QueryStep::Failed(error) => panic!("query failed: {error}"),
        }
    }
    panic!("small public fixture exceeded bounded progress allowance");
}

#[test]
fn fixture_cleanup_preserves_failure_context() {
    const CHILD: &str = "PIPESQL_PUBLIC_DIRECTORY_UNWIND";
    if std::env::var_os(CHILD).is_some() {
        let directory = Directory::new();
        let path = directory.0.clone();
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, b"not a directory").unwrap();
        let original = std::panic::catch_unwind(|| {
            let _directory = directory;
            panic!("original scenario failure");
        })
        .unwrap_err();
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            original.downcast_ref::<&str>(),
            Some(&"original scenario failure")
        );
        std::process::exit(74);
    }

    let directory = Directory::new();
    let path = directory.0.clone();
    drop(directory);
    assert!(!path.exists());

    let missing = Directory::new();
    std::fs::remove_dir(&missing.0).unwrap();
    drop(missing);

    let directory = Directory::new();
    let path = directory.0.clone();
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, b"not a directory").unwrap();
    let failure = std::panic::catch_unwind(|| drop(directory)).unwrap_err();
    std::fs::remove_file(path).unwrap();
    assert!(
        failure
            .downcast_ref::<String>()
            .unwrap()
            .contains("remove test directory")
    );

    // A cleanup panic during unwinding would abort the child, not this harness.
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "fixture_cleanup_preserves_failure_context"])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert_eq!(
        child.status.code(),
        Some(74),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
}
