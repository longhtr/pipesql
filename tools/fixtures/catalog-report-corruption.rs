//! Public demand and recovery checks over a quiescent mixed report graph.
use pipesql::{
    AppendLimits, CancellationToken, Config, Database, Error, PreparedQuery, QueryStep, Value,
};
use std::path::Path;
#[path = "../../examples/support/event_data.rs"]
mod event_data;

type Group = (Option<i64>, Option<&'static str>, i64, i64, Option<i64>);

// Literal answers, independent of fixture construction and production evaluation.
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

fn check_report(db: &Database, query: &PreparedQuery<'_>) {
    let expected = NEW;
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
                    let &(year, label, entries, present, total) =
                        expected.get(seen).expect("report group history: extra row");
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

pub(super) fn run(path: &Path, mode: &str) {
    assert!(matches!(
        mode,
        "report-setup"
            | "report-healthy"
            | "report-version-reject"
            | "report-corrupt-reject"
            | "report-recovery-reject"
            | "report-measurement-reject"
    ));
    let config = Config::new(16_000_000, 8_000_000).unwrap();
    let cancel = CancellationToken::new();
    if mode == "report-setup" {
        let db = Database::create_empty(path, config).unwrap();
        event_data::declare(&db, &cancel).unwrap();
        for (index, events) in event_data::EVENTS.chunks(8).enumerate() {
            let mut append = db
                .begin_append(
                    "events",
                    AppendLimits {
                        batches: 1,
                        encoded_bytes: 100_000,
                    },
                    &cancel,
                )
                .unwrap();
            if index == 1 {
                // The pre-publication root includes the newly issued attempt.
                // This is adjacent to the committed root, unlike pre-issuance state.
                std::fs::copy(path.join("ROOT.A"), path.with_extension("older-root")).unwrap();
            }
            event_data::write_events(&mut append, events, &cancel).unwrap();
            append.commit(&cancel).unwrap();
        }
        db.close().unwrap();
    }
    if mode == "report-version-reject" {
        assert!(matches!(
            Database::open(path, config),
            Err(Error::UnsupportedVersion(8))
        ));
        println!("report corruption check passed");
        return;
    }
    if mode == "report-corrupt-reject" {
        assert!(matches!(
            Database::open(path, config),
            Err(Error::Corrupt(_))
        ));
        println!("report corruption check passed");
        return;
    }
    if mode == "report-recovery-reject" {
        assert!(matches!(
            Database::open(path, config),
            Err(Error::RecoveryRequired { .. })
        ));
        println!("report corruption check passed");
        return;
    }
    let db = Database::open(path, config).unwrap();
    let resident = db.reserved_memory_bytes();
    // Metadata admission may succeed while an unused payload remains corrupt.
    let query = db
        .prepare(include_str!("../../examples/event_report.sql"))
        .unwrap();
    check_report(&db, &query);
    drop(query);
    assert_eq!(db.reserved_memory_bytes(), resident);
    if mode == "report-measurement-reject" {
        let query = db.prepare("FROM events |> SELECT measurement").unwrap();
        let before = db.reserved_memory_bytes();
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut failed = false;
        for _ in 0..1024 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Failed(Error::Corrupt(_)) => {
                    failed = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("unexpected error: {error}"),
                QueryStep::Rows(_) | QueryStep::Finished => panic!("accepted damaged measurement"),
            }
        }
        assert!(failed, "corrupt query did not terminate");
        drop(result);
        assert_eq!(db.reserved_memory_bytes(), before);
        assert_eq!(db.reserved_temp_bytes(), 0);
        drop(query);
        // A failed demand releases the cursor and permits a healthy query.
        let query = db
            .prepare(include_str!("../../examples/event_report.sql"))
            .unwrap();
        check_report(&db, &query);
        drop(query);
    }
    assert_eq!(db.reserved_memory_bytes(), resident);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close().unwrap();
    println!("report corruption check passed");
}
