//! Build small physical row layouts and input tables for blocking-operator tests.
//!
//! `schema` bypasses relational planning so codec tests can challenge exact layouts.
//! The two-column fixture forces multiple runs with reversed input and duplicate
//! keys across run boundaries. Expected answers stay in each suite. Disposable
//! directories use the common test owner; platform guards belong to the callers.

use super::{KeyColumn, RowLayout};
use crate::frontend::{DataType, Direction, MAX_ROW_VALUES, NullPlacement};
pub(in crate::execution) use crate::test_support::Directory;
use crate::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config, Database,
};

pub(in crate::execution) fn schema(specs: &[(DataType, bool)]) -> RowLayout {
    let mut columns = [KeyColumn {
        input: 0,
        kind: DataType::Int64,
        nullable: false,
        direction: Direction::Ascending,
        nulls: NullPlacement::First,
    }; MAX_ROW_VALUES];
    let mut max_bytes = 0;
    for (index, (kind, nullable)) in specs.iter().copied().enumerate() {
        columns[index] = KeyColumn {
            input: index,
            kind,
            nullable,
            direction: Direction::Ascending,
            nulls: NullPlacement::First,
        };
        max_bytes += match kind {
            DataType::Int64 | DataType::Double => 9,
            DataType::Date => 5,
            DataType::String => 5 + crate::batch::MAX_TEXT_BYTES,
        };
    }
    // Narrow codec test boundary: no relational plan is admitted by this helper.
    RowLayout {
        columns,
        count: specs.len(),
        key_count: specs.len(),
        max_bytes,
        key_max_bytes: max_bytes,
        layout: 0x44a7_3201,
    }
}

pub(in crate::execution) fn two_column_database(directory: &Directory) -> Database {
    let db = Database::create_empty(
        &directory.0.join("db"),
        Config::new(8_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "facts",
        &["k", "v"].map(|name| ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: false,
        }),
        &cancel,
    )
    .unwrap();
    // 180 rows exceed the sorter's byte-limited first run. Adjacent equal keys
    // cross run boundaries; reverse input order cannot masquerade as a merge.
    for start in [0, 90] {
        let values: Vec<i64> = (start..start + 90).rev().collect();
        let keys: Vec<_> = values.iter().map(|v| v / 2).collect();
        let mut valid = [255; 12];
        valid[11] = 3;
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
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&keys),
                        validity: &valid,
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&values),
                        validity: &valid,
                    },
                ],
                &cancel,
            )
            .unwrap();
        append.commit(&cancel).unwrap();
    }
    db
}
