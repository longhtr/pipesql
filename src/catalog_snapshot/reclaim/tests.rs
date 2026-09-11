//! Protected graph traversal, cleanup barriers, and failure/reopen behavior.
use crate::catalog::ObjectId;
use crate::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Commit,
    DataType, Database,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) struct Directory(pub(super) PathBuf);

impl Directory {
    pub(super) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!(
            "pipesql-reachable-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            // A test can fail before creating its database. During unwinding,
            // preserve the original panic instead of aborting on a second one.
            if error.kind() != std::io::ErrorKind::NotFound && !std::thread::panicking() {
                panic!("remove test directory {}: {error}", self.0.display());
            }
        }
    }
}

pub(super) fn database(directory: &Directory) -> Database {
    let db = Database::create_empty(
        &directory.0,
        crate::Config::new(2_000_000, 2_000_000).unwrap(),
    )
    .unwrap();
    db.declare_table(
        "facts",
        &[ColumnDeclaration {
            name: "v",
            data_type: DataType::Int64,
            nullable: false,
        }],
        &CancellationToken::new(),
    )
    .unwrap();
    db
}

pub(super) fn append(db: &Database, value: i64) -> Commit {
    let cancel = CancellationToken::new();
    let mut append = db
        .begin_append(
            "facts",
            AppendLimits {
                batches: 1,
                encoded_bytes: 10_000,
            },
            &cancel,
        )
        .unwrap();
    append
        .write(
            &[ColumnInput {
                values: ColumnValues::Int64(&[value]),
                validity: &[1],
            }],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap()
}

pub(super) fn id(attempt: u64, ordinal: u32) -> ObjectId {
    ObjectId::new(attempt, ordinal).unwrap()
}

fn query_values(db: &Database, query: &crate::PreparedQuery<'_>) -> Vec<i64> {
    let cancel = CancellationToken::new();
    let mut result = db.execute(query, &cancel).unwrap();
    result_values(&mut result)
}

fn result_values(result: &mut crate::QueryResult<'_, '_>) -> Vec<i64> {
    let mut values = Vec::new();
    for _ in 0..100 {
        match result.step() {
            crate::QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let Some(crate::Value::Int64(value)) = batch.value(row, 0) else {
                        panic!("integer fixture")
                    };
                    values.push(value);
                }
            }
            crate::QueryStep::Progress => (),
            crate::QueryStep::Finished => {
                values.sort_unstable();
                return values;
            }
            crate::QueryStep::Failed(error) => panic!("{error:?}"),
        }
    }
    panic!("small query failed to finish");
}

mod cleanup;
mod scratch;
mod traversal;

#[test]
fn directory_cleanup_preserves_failure_context() {
    const CHILD: &str = "PIPESQL_TEST_DIRECTORY_UNWIND";
    if std::env::var_os(CHILD).is_some() {
        let directory = Directory::new();
        let path = directory.0.clone();
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

    let missing = Directory::new();
    drop(missing);
    let directory = Directory::new();
    let path = directory.0.clone();
    std::fs::create_dir(&path).unwrap();
    drop(directory);
    assert!(!path.exists());

    let directory = Directory::new();
    let path = directory.0.clone();
    std::fs::write(&path, b"not a directory").unwrap();
    let cleanup = std::panic::catch_unwind(|| drop(directory)).unwrap_err();
    std::fs::remove_file(path).unwrap();
    assert!(
        cleanup
            .downcast_ref::<String>()
            .unwrap()
            .contains("remove test directory")
    );

    // Isolate the regression: a second panic during unwinding aborts its process.
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "catalog_snapshot::reclaim::tests::directory_cleanup_preserves_failure_context",
        ])
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
