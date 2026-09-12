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
        crate::test_cleanup::directory(&self.0);
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
