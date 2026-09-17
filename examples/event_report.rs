//! Build a yearly event report and show how a prepared query survives an append.
//!
//! The program creates events and their labels, appends eight events, and prepares
//! a report. It then appends eight more. The old query must still return its seven
//! groups; a new query returns thirteen. Reopening must reproduce the new report.
//! This connects stored rows, a join, grouping and snapshot lifetime in one flow.
//!
//! Expected groups are written out below, independently of the input builder.
//! A separate query checks dates and DOUBLE bits so correct totals cannot hide
//! altered input values. The tests run the full flow and deliberately change an
//! expected total to verify that the checker rejects it and releases the result.
//! Pass one new absolute database path, which remains available afterward.
//!
//! Run `cargo run --release --offline --locked --example event_report --
//! /absolute/new-database`. Require a successful exit and `status=finished`;
//! the database remains for inspection and caller cleanup.
//!
//! Dimension ID 2 has two labels, so sixteen events produce twenty joined rows.
//! Missing matches and NULL keys receive a NULL label, distinct from the empty
//! label at ID 3. In year 2000, events 2 and 3 initially form the NULL-label
//! group: two entries, one present amount and total 30. Event 10 adds amount 3,
//! changing that group to three entries, two present amounts and total 33.
//! A group with no present amounts has a NULL sum. See `event_report.sql` and
//! `support/event_data.rs` for the query and input; `scaled_report.rs` adds spill.

#[path = "support/event_data.rs"]
mod event_data;
#[cfg(test)]
#[path = "../test/support/mod.rs"]
mod support;

use pipesql::{
    AppendLimits, CancellationToken, Config, DataType, Database, PreparedQuery, QueryStep, Value,
};
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
/// Report columns: year, label, joined row count, present amount count and sum.
type Group = (Option<i64>, Option<&'static str>, i64, i64, Option<i64>);

// Keep these answers separate from event_data: a mistake in its row builder
// must not automatically change what the report checker accepts.
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

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("supply a new absolute database path")?;
    let path = Path::new(&path);
    if args.next().is_some() || !path.is_absolute() || path.exists() {
        return Err("supply exactly one new absolute database path".into());
    }
    run(path)?;
    // Print the report only after all query checks and database closes succeed.
    println!("old groups=7 joined_rows=10; pinned report unchanged");
    println!("new groups=13 joined_rows=20; reopened report unchanged");
    for (year, label, entries, present, total) in NEW {
        println!(
            "year={year:?} label={label:?} entries={entries} present={present} total={total:?}"
        );
    }
    println!("measurements=16; original and doubled bits verified");
    println!("status=finished");
    Ok(())
}

fn append(db: &Database, events: &[event_data::Event], cancel: &CancellationToken) -> Result<()> {
    let mut append = db.begin_append(
        "events",
        AppendLimits {
            batches: 1,
            encoded_bytes: 4096,
        },
        cancel,
    )?;
    event_data::write_events(&mut append, events, cancel)?;
    append.commit(cancel)?;
    Ok(())
}

fn prepare_report(db: &Database) -> Result<PreparedQuery<'_>> {
    // The local SQL string is dropped on return. The prepared query must keep
    // its resolved names and snapshot without borrowing that string.
    let source = String::from(include_str!("event_report.sql"));
    Ok(db.prepare(&source)?)
}

fn run(path: &Path) -> Result<()> {
    let config = Config::new(16_000_000, 8_000_000)?;
    let cancel = CancellationToken::new();
    let db = Database::create_empty(path, config)?;
    event_data::declare(&db, &cancel)?;
    append(&db, &event_data::EVENTS[..8], &cancel)?;
    let old = prepare_report(&db)?;
    verify_report(&db, &old, OLD)?;
    append(&db, &event_data::EVENTS[8..], &cancel)?;
    let resident = db.reserved_memory_bytes() - old.accounted_memory_bytes();
    verify_report(&db, &old, OLD)?;
    let new = prepare_report(&db)?;
    verify_report(&db, &new, NEW)?;
    drop(new);
    drop(old);
    released(&db, resident)?;
    verify_measurements(&db)?;
    db.close()?;

    let db = Database::open(path, config)?;
    let resident = db.reserved_memory_bytes();
    let reopened = prepare_report(&db)?;
    verify_report(&db, &reopened, NEW)?;
    drop(reopened);
    released(&db, resident)?;
    verify_measurements(&db)?;
    db.close()?;
    Ok(())
}

fn released(db: &Database, baseline: u64) -> Result<()> {
    if db.reserved_memory_bytes() != baseline || db.reserved_temp_bytes() != 0 {
        return Err("query reservations remain live".into());
    }
    Ok(())
}

fn verify_report(db: &Database, query: &PreparedQuery, expected: &[Group]) -> Result<()> {
    let schema = [
        ("calendar_year", DataType::Int64, true),
        ("label", DataType::String, true),
        ("entries", DataType::Int64, false),
        ("present", DataType::Int64, false),
        ("total", DataType::Int64, true),
    ];
    if query.result_column_count() != schema.len() {
        return Err("unexpected report width".into());
    }
    for (index, (name, data_type, nullable)) in schema.into_iter().enumerate() {
        let column = query.result_column(index).ok_or("missing report column")?;
        if column.name != Some(name) || column.data_type != data_type || column.nullable != nullable
        {
            return Err("unexpected report schema".into());
        }
    }
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let mut result = db.execute(query, &cancel)?;
    let mut seen = 0;
    for _ in 0..100_000 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let &(year, label, entries, present, total) =
                        expected.get(seen).ok_or("too many report groups")?;
                    let actual_label = match batch.value(row, 1) {
                        Some(Value::Null) => None,
                        Some(Value::String(value)) => Some(value.as_str()),
                        _ => return Err("unexpected report label type".into()),
                    };
                    if batch.value(row, 0) != Some(year.map_or(Value::Null, Value::Int64))
                        || actual_label != label
                        || batch.value(row, 2) != Some(Value::Int64(entries))
                        || batch.value(row, 3) != Some(Value::Int64(present))
                        || batch.value(row, 4) != Some(total.map_or(Value::Null, Value::Int64))
                    {
                        return Err(format!("unexpected report group {seen}").into());
                    }
                    seen += 1;
                }
            }
            QueryStep::Finished => {
                if seen != expected.len() {
                    return Err("missing report groups".into());
                }
                drop(result);
                return released(db, baseline);
            }
            QueryStep::Failed(_) => {
                return Err(result.into_error().ok_or("missing query error")?.into());
            }
        }
    }
    Err("report did not finish within the bounded step count".into())
}

fn verify_measurements(db: &Database) -> Result<()> {
    // Compare original and doubled values with literal bit patterns, including
    // negative zero. This check is independent of the report's integer totals.
    const BITS: &[(Option<u64>, Option<u64>)] = &[
        (Some(0x3fe0000000000000), Some(0x3ff0000000000000)),
        (Some(0x3ff8000000000000), Some(0x4008000000000000)),
        (None, None),
        (Some(0x8000000000000000), Some(0x8000000000000000)),
        (Some(0x4000000000000000), Some(0x4010000000000000)),
        (Some(0x4004000000000000), Some(0x4014000000000000)),
        (Some(0x4008000000000000), Some(0x4018000000000000)),
        (Some(0xbff0000000000000), Some(0xc000000000000000)),
        (Some(0x4010000000000000), Some(0x4020000000000000)),
        (Some(0x4012000000000000), Some(0x4022000000000000)),
        (Some(0x4014000000000000), Some(0x4024000000000000)),
        (None, None),
        (Some(0x4018000000000000), Some(0x4028000000000000)),
        (Some(0x401a000000000000), Some(0x402a000000000000)),
        (Some(0x401c000000000000), Some(0x402c000000000000)),
        (Some(0xc000000000000000), Some(0xc010000000000000)),
    ];
    const DAYS: &[Option<i32>] = &[
        Some(10_956),
        Some(11_016),
        Some(11_016),
        Some(11_016),
        None,
        Some(11_323),
        Some(11_322),
        None,
        Some(10_956),
        Some(11_323),
        Some(11_016),
        None,
        Some(11_016),
        Some(11_323),
        None,
        Some(11_322),
    ];
    let baseline = db.reserved_memory_bytes();
    let query = db.prepare(
        "FROM events |> ORDER BY id |> SELECT id, happened, measurement, measurement * 2 AS doubled",
    )?;
    let schema = [
        ("id", DataType::Int64, false),
        ("happened", DataType::Date, true),
        ("measurement", DataType::Double, true),
        ("doubled", DataType::Double, true),
    ];
    if query.result_column_count() != schema.len() {
        return Err("unexpected measurement width".into());
    }
    for (index, (name, data_type, nullable)) in schema.into_iter().enumerate() {
        let column = query
            .result_column(index)
            .ok_or("missing measurement column")?;
        if column.name != Some(name) || column.data_type != data_type || column.nullable != nullable
        {
            return Err("unexpected measurement schema".into());
        }
    }
    let cancel = CancellationToken::new();
    let mut result = db.execute(&query, &cancel)?;
    let mut seen = 0;
    for _ in 0..100_000 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let &(original, doubled) = BITS.get(seen).ok_or("too many measurements")?;
                    if batch.value(row, 0) != Some(Value::Int64(seen as i64)) {
                        return Err("unexpected event identity".into());
                    }
                    let day = match batch.value(row, 1) {
                        Some(Value::Null) => None,
                        Some(Value::Date(value)) => Some(value.days_since_unix_epoch()),
                        _ => return Err("unexpected DATE type".into()),
                    };
                    if day != DAYS[seen] {
                        return Err("unexpected stored DATE".into());
                    }
                    for (column, expected) in [(2, original), (3, doubled)] {
                        let actual = match batch.value(row, column) {
                            Some(Value::Null) => None,
                            Some(Value::Double(value)) => Some(value.to_bits()),
                            _ => return Err("unexpected measurement type".into()),
                        };
                        if actual != expected {
                            return Err("unexpected measurement bits".into());
                        }
                    }
                    seen += 1;
                }
            }
            QueryStep::Finished => {
                if seen != BITS.len() {
                    return Err("missing measurements".into());
                }
                drop(result);
                drop(query);
                return released(db, baseline);
            }
            QueryStep::Failed(_) => {
                return Err(result.into_error().ok_or("missing query error")?.into());
            }
        }
    }
    Err("measurements did not finish within the bounded step count".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use support::Directory;

    #[test]
    fn complete_report_survives_append_and_reopen() {
        let directory = Directory::new();
        run(&directory.0.join("database")).unwrap();
    }

    #[test]
    fn altered_group_is_rejected_and_releases_the_result() {
        let directory = Directory::new();
        let db = Database::create_empty(
            &directory.0.join("database"),
            Config::new(16_000_000, 8_000_000).unwrap(),
        )
        .unwrap();
        let cancel = CancellationToken::new();
        event_data::declare(&db, &cancel).unwrap();
        append(&db, &event_data::EVENTS[..8], &cancel).unwrap();
        let resident = db.reserved_memory_bytes();
        let query = prepare_report(&db).unwrap();
        let baseline = db.reserved_memory_bytes();
        let mut wrong = OLD.to_vec();
        wrong[3].4 = Some(31);
        let error = verify_report(&db, &query, &wrong).unwrap_err();
        assert_eq!(error.to_string(), "unexpected report group 3");
        drop(error);
        released(&db, baseline).unwrap();
        verify_report(&db, &query, OLD).unwrap();
        drop(query);
        released(&db, resident).unwrap();
        db.close().unwrap();
    }
}

#[cfg(test)]
#[path = "support/analytics_tests.rs"]
mod analytics_tests;
