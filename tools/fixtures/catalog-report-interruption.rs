//! Carry the event report through interrupted append, recovery and healthy retry.
//!
//! Two writes form one publication. A prepared old report must retain its original
//! rows after a successful append; reopened reports follow the settled attempt.
//! Local OLD/NEW tuples supply expected groups independently of event_data's input
//! construction. The parent caller owns tokens and observer setup; the Python
//! supervisor owns cut enumeration, raw graph checks and wrong-answer controls.

use super::{durable, interruption_start, interruption_stop, token};
use pipesql::{
    AppendLimits, CancellationToken, CommitResolution, Config, Database, Error, PreparedQuery,
    QueryStep, Value,
};
use std::os::fd::AsRawFd;
use std::path::Path;
#[path = "../../examples/support/event_data.rs"]
mod event_data;

type Group = (Option<i64>, Option<&'static str>, i64, i64, Option<i64>);

// Literal answers, independent of fixture construction and production evaluation.
const OLD: &[Group] = &[
    (None, Some(""), 1, 1, Some(5)),
    (None, Some("north"), 1, 1, Some(7)),
    (Some(1999), Some("north"), 1, 1, Some(10)),
    (Some(2000), None, 2, 1, Some(30)),
    (Some(2000), Some("south"), 2, 2, Some(15)),
    (Some(2000), Some("南"), 2, 2, Some(15)),
    (Some(2001), Some("north"), 1, 0, None),
];
const NEW: &[Group] = &[
    (None, None, 1, 1, Some(11)),
    (None, Some(""), 1, 1, Some(5)),
    (None, Some("north"), 1, 1, Some(7)),
    (None, Some("south"), 1, 0, None),
    (None, Some("南"), 1, 0, None),
    (Some(1999), Some("north"), 2, 2, Some(8)),
    (Some(2000), None, 3, 2, Some(33)),
    (Some(2000), Some(""), 2, 2, Some(9)),
    (Some(2000), Some("south"), 2, 2, Some(15)),
    (Some(2000), Some("南"), 2, 2, Some(15)),
    (Some(2001), Some("north"), 2, 1, Some(13)),
    (Some(2001), Some("south"), 1, 1, Some(40)),
    (Some(2001), Some("南"), 1, 1, Some(40)),
];

fn config() -> Config {
    Config::new(16_000_000, 8_000_000).unwrap()
}
fn append(db: &Database, events: &[event_data::Event], attempt: u64) {
    let cancel = CancellationToken::new();
    let mut writer = db
        .begin_append(
            "events",
            AppendLimits {
                batches: 2,
                encoded_bytes: 4096,
            },
            &cancel,
        )
        .unwrap();
    assert_eq!(writer.transaction(), token(db, attempt));
    // Two separate writes retain a publication boundary covering both batches.
    for rows in events.chunks(4) {
        event_data::write_events(&mut writer, rows, &cancel).unwrap();
    }
    let receipt = writer.commit(&cancel).unwrap();
    assert_eq!(receipt.transaction(), token(db, attempt));
}
fn check_report(db: &Database, query: &PreparedQuery<'_>, appended: bool, retried: bool) {
    let expected = if appended { NEW } else { OLD };
    assert_eq!(query.result_column_count(), 5);
    for (index, (name, kind, nullable)) in [
        ("calendar_year", pipesql::DataType::Int64, true),
        ("label", pipesql::DataType::String, true),
        ("entries", pipesql::DataType::Int64, false),
        ("present", pipesql::DataType::Int64, false),
        ("total", pipesql::DataType::Int64, true),
    ]
    .into_iter()
    .enumerate()
    {
        let column = query.result_column(index).unwrap();
        assert_eq!(
            (column.name, column.data_type, column.nullable),
            (Some(name), kind, nullable)
        );
    }
    let before = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let mut result = db.execute(query, &cancel).unwrap();
    let mut seen = 0;
    let mut finished = false;
    for _ in 0..1024 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let &(year, label, mut entries, mut present, mut total) =
                        expected.get(seen).expect("report group history: extra row");
                    if retried && year == Some(1999) && label == Some("north") {
                        // The healthy retry is event 8: one -2 contribution.
                        (entries, present, total) = if appended {
                            (3, 3, Some(6))
                        } else {
                            (2, 2, Some(8))
                        };
                    }
                    assert_eq!(
                        batch.value(row, 0),
                        Some(year.map_or(Value::Null, Value::Int64)),
                        "report group history"
                    );
                    match (batch.value(row, 1), label) {
                        (Some(Value::Null), None) => (),
                        (Some(Value::String(actual)), Some(label)) => {
                            assert_eq!(actual.as_str(), label, "report group history")
                        }
                        _ => panic!("report group history: label"),
                    }
                    for (column, expected) in [(2, Some(entries)), (3, Some(present)), (4, total)] {
                        assert_eq!(
                            batch.value(row, column),
                            Some(expected.map_or(Value::Null, Value::Int64)),
                            "report group history"
                        );
                    }
                    seen += 1;
                }
            }
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("report failed: {error}"),
        }
    }
    assert!(finished, "finite report progress");
    assert_eq!(seen, expected.len(), "report group history");
    drop(result);
    assert_eq!(db.reserved_memory_bytes(), before);
    assert_eq!(db.reserved_temp_bytes(), 0);
}
fn report(db: &Database, appended: bool, retried: bool) {
    let before = db.reserved_memory_bytes();
    let query = db
        .prepare(include_str!("../../examples/event_report.sql"))
        .unwrap();
    check_report(db, &query, appended, retried);
    drop(query);
    assert_eq!(db.reserved_memory_bytes(), before);
}
fn history(db: &Database, state: u32, retried: bool) {
    assert!(state <= 2);
    let generation = 4 + u64::from(state == 2) + u64::from(retried);
    assert_eq!(
        db.generation(),
        generation,
        "independent report generation history"
    );
    for attempt in 1..=4 {
        durable(db, attempt, attempt);
    }
    assert_eq!(
        db.resolve_commit(token(db, 5)).unwrap(),
        CommitResolution::Aborted
    );
    match state {
        0 if !retried => assert!(matches!(
            db.resolve_commit(token(db, 6)),
            Err(Error::NotFound)
        )),
        1 => assert_eq!(
            db.resolve_commit(token(db, 6)).unwrap(),
            CommitResolution::Aborted
        ),
        2 => durable(db, 6, 5),
        _ => (),
    }
    let next = if state == 0 { 6 } else { 7 };
    if retried {
        durable(db, next, generation);
    }
    assert!(matches!(
        db.resolve_commit(token(db, if retried { next + 1 } else { 7 })),
        Err(Error::NotFound)
    ));
    report(db, state == 2, retried);
}

pub(super) fn run(path: &Path, mode: &str, trace: &std::fs::File, cut: u32, state: u32) {
    match mode {
        "setup" => {
            let db = Database::create_empty(path, config()).unwrap();
            event_data::declare(&db, &CancellationToken::new()).unwrap();
            append(&db, &event_data::EVENTS[..8], 4);
            let aborted = db
                .begin_append(
                    "events",
                    AppendLimits {
                        batches: 1,
                        encoded_bytes: 4096,
                    },
                    &CancellationToken::new(),
                )
                .unwrap();
            assert_eq!(aborted.transaction(), token(&db, 5));
            aborted.abort().unwrap();
            history(&db, 0, false);
            db.close().unwrap();
        }
        "append" => {
            let db = Database::open(path, config()).unwrap();
            let old = db
                .prepare(include_str!("../../examples/event_report.sql"))
                .unwrap();
            check_report(&db, &old, false, false);
            // SAFETY: the single-threaded caller retains the trace descriptor;
            // the existing observer owns only scalar state and native cuts.
            unsafe {
                interruption_start(trace.as_raw_fd(), cut);
            }
            append(&db, &event_data::EVENTS[8..], 6);
            unsafe {
                interruption_stop();
            }
            check_report(&db, &old, false, false);
            drop(old);
            history(&db, 2, false);
            db.close().unwrap();
        }
        "recover" => {
            // SAFETY: same exclusive observer state and live descriptor as above.
            unsafe {
                interruption_start(trace.as_raw_fd(), cut);
            }
            let db = Database::open(path, config()).unwrap();
            unsafe {
                interruption_stop();
            }
            history(&db, state, false);
            db.close().unwrap();
        }
        "verify" => {
            let db = Database::open(path, config()).unwrap();
            history(&db, state, false);
            append(
                &db,
                &event_data::EVENTS[8..9],
                if state == 0 { 6 } else { 7 },
            );
            history(&db, state, true);
            db.close().unwrap();
            let db = Database::open(path, config()).unwrap();
            history(&db, state, true);
            db.close().unwrap();
        }
        "wrong-rows" => {
            let db = Database::open(path, config()).unwrap();
            report(&db, false, false);
        }
        "wrong-receipt" => {
            let db = Database::open(path, config()).unwrap();
            durable(&db, 5, 5);
        }
        _ => panic!("unknown report interruption mode"),
    }
}
