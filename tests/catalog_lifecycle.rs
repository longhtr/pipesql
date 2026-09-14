#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "catalog_lifecycle/aggregates.rs"]
mod aggregates;

#[path = "catalog_lifecycle/append.rs"]
mod append;

#[path = "catalog_lifecycle/boolean.rs"]
mod boolean;

#[path = "catalog_lifecycle/byte_length.rs"]
mod byte_length;

#[path = "catalog_lifecycle/computed.rs"]
mod computed;

#[path = "catalog_lifecycle/constant_projection.rs"]
mod constant_projection;

#[path = "catalog_lifecycle/distinct.rs"]
mod distinct;

#[path = "catalog_lifecycle/except.rs"]
mod except;
#[path = "catalog_lifecycle/intersect.rs"]
mod intersect;

#[path = "catalog_lifecycle/grouping.rs"]
mod grouping;

#[path = "catalog_lifecycle/join_corpus.rs"]
mod join_corpus;

#[path = "catalog_lifecycle/joins.rs"]
mod joins;

#[path = "catalog_lifecycle/limit.rs"]
mod limit;

#[path = "catalog_lifecycle/membership.rs"]
mod membership;

#[path = "catalog_lifecycle/multiset.rs"]
mod multiset;

#[path = "catalog_lifecycle/null_predicate.rs"]
mod null_predicate;

#[path = "catalog_lifecycle/null_safe.rs"]
mod null_safe;

#[path = "catalog_lifecycle/nullif.rs"]
mod nullif;

#[path = "catalog_lifecycle/order.rs"]
mod order;

#[path = "catalog_lifecycle/snapshots.rs"]
mod snapshots;

#[path = "catalog_lifecycle/spooling.rs"]
mod spooling;

#[path = "catalog_lifecycle/text_filter.rs"]
mod text_filter;

#[path = "catalog_lifecycle/union.rs"]
mod union;

#[path = "catalog_lifecycle/window_count.rs"]
mod window_count;

#[path = "catalog_lifecycle/wide.rs"]
mod wide;

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues,
    CommitResolution, Config, DataType, Database, DateValue, Error, QueryResult, QueryStep, Value,
};
use std::path::PathBuf;

mod support;
use support::Directory;

impl Directory {
    fn database(&self) -> PathBuf {
        self.0.join("db")
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
