//! Build the small databases shared by reclamation tests.
//!
//! `database` creates a one-column facts table, and `append` commits one value.
//! Query helpers collect all rows and require successful completion; they sort
//! values because these queries promise no row order. Individual cases supply
//! their expected values, protected object IDs and failure points.
//!
//! Child modules exercise reference traversal, scratch ownership, and cleanup
//! followed by reopen. The shared Directory guard removes their temporary files.

use crate::storage::catalog::ObjectId;
pub(super) use crate::test_support::Directory;
use crate::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Commit,
    DataType, Database,
};

pub(super) fn database(directory: &Directory) -> Database {
    let db = Database::create_empty(
        &directory.0.join("database"),
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
