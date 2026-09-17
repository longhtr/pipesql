//! Build shared input tables and copy query results for public catalog tests.
//!
//! Fixtures provide duplicate join keys, nullable columns and text edge cases.
//! Tests supply their own expected answers. Collected cells own their text and
//! preserve floating-point bits, so comparisons remain valid after the result's
//! borrowed batch is gone.
//!
//! `query` checks ordered rows and releases the result and prepared plan before
//! comparing reservations with the starting balance. `collect_unordered` sorts
//! copied rows for tests that permit any row order; its caller owns cleanup.
//! Collectors fail if execution errors or does not finish within a step limit.

use super::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, DateValue, Directory, Error, QueryResult, QueryStep, Value,
};
use std::path::PathBuf;

impl Directory {
    pub(super) fn database(&self) -> PathBuf {
        self.0.join("db")
    }
}

pub(super) fn config() -> Config {
    Config::new(4_000_000, 2_000_000).unwrap()
}

pub(super) fn limits() -> AppendLimits {
    AppendLimits {
        batches: 2,
        encoded_bytes: 100_000,
    }
}

pub(super) fn declarations() -> [ColumnDeclaration<'static>; 4] {
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

// Derived ordering is only a canonical order for comparing multisets. It is
// not SQL ordering: Number compares raw bits, keeping NaNs and signed zero exact.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Cell {
    Null,
    Integer(i64),
    Number(u64),
    Text(String),
    Day(i32),
}

pub(super) fn owned_cell(value: Value<'_>) -> Cell {
    match value {
        Value::Null => Cell::Null,
        Value::Int64(value) => Cell::Integer(value),
        Value::Double(value) => Cell::Number(value.to_bits()),
        Value::String(value) => Cell::Text(value.as_str().to_owned()),
        Value::Date(value) => Cell::Day(value.days_since_unix_epoch()),
    }
}

// Both tables have keys [1, 1, 2, NULL]. Duplicate keys expose join multiplicity;
// the last row distinguishes NULL equality from ordinary key equality.
pub(super) fn join_fixture() -> (Directory, Database) {
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

pub(super) fn assert_query_rows(query: &str, mut expected: Vec<Vec<Cell>>) {
    let (_directory, db) = join_fixture();
    let cancel = CancellationToken::new();
    let resident = db.reserved_memory_bytes();
    let prepared = db.prepare(query).unwrap();
    let mut result = db.execute(&prepared, &cancel).unwrap();
    let actual = collect_unordered(&mut result);
    expected.sort_unstable();
    assert_eq!(actual, expected, "{query}");
    drop(result);
    drop(prepared);
    assert_eq!(db.reserved_memory_bytes(), resident);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close().unwrap();
}

/// Copy and sort rows, preserving duplicates. The caller must drop the result
/// and check release; sorting deliberately discards the engine's output order.
pub(super) fn collect_unordered(result: &mut QueryResult<'_, '_>) -> Vec<Vec<Cell>> {
    let mut rows = collect_rows(result, 1024, "small public fixture");
    rows.sort_unstable();
    rows
}

// Copy each borrowed batch before advancing. A step limit includes both Progress
// and Rows, so a query that emits data forever cannot pass as a complete result.
pub(super) fn collect_rows(
    result: &mut QueryResult<'_, '_>,
    steps: usize,
    context: &str,
) -> Vec<Vec<Cell>> {
    let mut rows = Vec::new();
    for _ in 0..steps {
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
            QueryStep::Finished => return rows,
            QueryStep::Failed(error) => panic!("{context}: {error}"),
        }
    }
    panic!("bounded progress allowance exhausted: {context}");
}

/// Cancel after progress, cancel after rows, then abandon a result after rows.
/// The query must reach both stopping points within 512 steps. Cancellation must
/// stay terminal; every path must release execution reservations on drop.
pub(super) fn assert_cancel_and_drop_release(db: &Database, sql: &str) {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum StopAt {
        Progress,
        Rows,
        AbandonRows,
    }
    let resident = db.reserved_memory_bytes();
    let prepared = db
        .prepare(sql)
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
    let retained = db.reserved_memory_bytes();
    for stop in [StopAt::Progress, StopAt::Rows, StopAt::AbandonRows] {
        let cancel = CancellationToken::new();
        let mut result = db
            .execute(&prepared, &cancel)
            .unwrap_or_else(|error| panic!("{sql}, {stop:?}: {error}"));
        let mut stopped = false;
        for _ in 0..512 {
            match result.step() {
                QueryStep::Progress => {
                    if stop == StopAt::Progress {
                        cancel.cancel();
                    }
                }
                QueryStep::Rows(batch) => {
                    assert!(!batch.is_empty(), "{sql}, {stop:?}");
                    if stop == StopAt::AbandonRows {
                        stopped = true;
                        break;
                    }
                    assert_eq!(
                        stop,
                        StopAt::Rows,
                        "{sql}: progress cancellation must precede output"
                    );
                    cancel.cancel();
                }
                QueryStep::Failed(Error::Cancelled) => {
                    assert!(cancel.is_cancelled(), "{sql}, {stop:?}");
                    assert!(
                        matches!(result.step(), QueryStep::Failed(Error::Cancelled)),
                        "{sql}, {stop:?}"
                    );
                    stopped = true;
                    break;
                }
                QueryStep::Finished => {
                    panic!("{sql}, {stop:?}: expected cancellation or abandonment")
                }
                QueryStep::Failed(error) => panic!("{sql}, {stop:?}: {error}"),
            }
        }
        assert!(stopped, "{sql}, {stop:?}: progress allowance exhausted");
        drop(result);
        assert_eq!(db.reserved_memory_bytes(), retained, "{sql}, {stop:?}");
        assert_eq!(db.reserved_temp_bytes(), 0, "{sql}, {stop:?}");
    }
    drop(prepared);
    assert_eq!(db.reserved_memory_bytes(), resident, "{sql}");
}

pub(super) fn query(db: &Database, sql: &str, expected: Vec<Vec<Cell>>) {
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let prepared = db
        .prepare(sql)
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut result = db
        .execute(&prepared, &cancel)
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
    let rows = collect_rows(&mut result, 100_000, sql);
    assert_eq!(rows, expected, "{sql}");
    drop(result);
    drop(prepared);
    assert_eq!(db.reserved_memory_bytes(), baseline, "{sql}");
    assert_eq!(db.reserved_temp_bytes(), 0, "{sql}");
}

pub(super) fn integers(values: &[i64]) -> Vec<Vec<Cell>> {
    values
        .iter()
        .map(|value| vec![Cell::Integer(*value)])
        .collect()
}

// Put each nullable column's NULL on a different row. Other numeric rows include
// zero, NaN and infinity so NULL tests cannot infer validity from the payload.
pub(super) fn nullable_facts() -> Result<(Directory, Database), Error> {
    let directory = Directory::new();
    let db = Database::create_empty(&directory.database(), config())?;
    let cancel = CancellationToken::new();
    let columns = [
        ("id", DataType::Int64),
        ("n", DataType::Double),
        ("s", DataType::String),
        ("i", DataType::Int64),
        ("d", DataType::Date),
    ]
    .map(|(name, data_type)| ColumnDeclaration {
        name,
        data_type,
        nullable: name != "id",
    });
    db.declare_table("facts", &columns, &cancel)?;
    let day = DateValue::from_days_since_unix_epoch(0).unwrap();
    let mut append = db.begin_append(
        "facts",
        AppendLimits {
            batches: 1,
            encoded_bytes: 20_000,
        },
        &cancel,
    )?;
    append.write(
        &[
            ColumnInput {
                values: ColumnValues::Int64(&[0, 1, 2, 3]),
                validity: &[15],
            },
            ColumnInput {
                values: ColumnValues::Double(&[0., 0., f64::NAN, f64::INFINITY]),
                validity: &[14],
            },
            ColumnInput {
                values: ColumnValues::String(&["present", "ignored", "", "é"]),
                validity: &[13],
            },
            ColumnInput {
                values: ColumnValues::Int64(&[0, 7, 0, 9]),
                validity: &[11],
            },
            ColumnInput {
                values: ColumnValues::Date(&[day; 4]),
                validity: &[7],
            },
        ],
        &cancel,
    )?;
    append.commit(&cancel)?;
    Ok((directory, db))
}

// Include composed and decomposed accents, an actual newline and a literal
// backslash, an embedded NUL and a maximum-length value. ID 7 has SQL NULL text.
pub(super) fn text_fixture() -> (Directory, Database) {
    let directory = Directory::new();
    let db = Database::create_empty(&directory.database(), config()).unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "texts",
        &[
            ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "category",
                data_type: DataType::String,
                nullable: true,
            },
        ],
        &cancel,
    )
    .unwrap();
    let long = "a".repeat(65_536);
    let values = [
        "",
        "a",
        "é",
        "e\u{301}",
        "line\nnext",
        "line\\nnext",
        "it's",
        "ignored",
        "\0",
        &long,
    ];
    let mut append = db
        .begin_append(
            "texts",
            AppendLimits {
                batches: 1,
                encoded_bytes: 100_000,
            },
            &cancel,
        )
        .unwrap();
    append
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]),
                    validity: &[255, 3],
                },
                ColumnInput {
                    values: ColumnValues::String(&values),
                    validity: &[127, 3],
                },
            ],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    (directory, db)
}
