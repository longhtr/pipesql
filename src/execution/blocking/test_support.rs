//! Supply row layouts and a shared table fixture for blocking-operator tests.
//!
//! `schema` constructs an all-key layout directly for record and sorter tests;
//! it does not exercise the parser, binder or plan validation. The database helper
//! uses the public creation and append APIs to store two reversed chunks of rows
//! with repeated keys. Each test defines its own expected results and checks the
//! sort transitions it needs. `Directory` removes the fixture when dropped.

use super::{KeyColumn, RowLayout};
use crate::query::{Direction, MAX_ROW_VALUES, NullPlacement};
pub(in crate::execution) use crate::test_support::Directory;
use crate::value::DataType;
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
    // A fixed layout tag lets record tests choose field types without building
    // a query. Tests of production layout tags must use RowLayout constructors.
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
    // Store v from 89 down to 0, then from 179 down to 90, with k=v/2. Each key
    // occurs twice. Separate commits create two source units, so a scan must
    // continue across a unit boundary.
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

// Expected memory for an observed sort-buffer capacity, independent of production
// sizing. Darwin reserves two 16-KiB pages for every page needed by a large buffer.
// These account checks complement the public caller's actual allocator extents.
pub(in crate::execution) fn expected_buffer_charge(capacity: usize) -> usize {
    if cfg!(target_os = "macos") && capacity > 32_768 {
        capacity.div_ceil(16_384) * 32_768
    } else {
        capacity
    }
}
