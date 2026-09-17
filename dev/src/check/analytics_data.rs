//! Supply analytical inputs that distinguish joins, NULLs and ordered window peers.
//!
//! Fixed dimensions include duplicate keys, unmatched events and an empty label.
//! The small event set has literal report answers, including peers that must share
//! the same running sum. The larger set exercises the same semantics beyond memory.
//! CSV writing preserves the input distinctions; the report model lives in the
//! parent module and is checked against these small literal answers before use.

use super::{Result, Schema, integer};
use serde_json::{Value, json};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

pub const DIMENSIONS: [(i64, &str); 4] = [(1, "north"), (2, "south"), (2, "南"), (3, "")];
pub const DIMENSION_SCHEMA: Schema = &[("id", "int64", false), ("label", "string", false)];
pub const EVENT_SCHEMA: Schema = &[
    ("id", "int64", false),
    ("dimension_id", "int64", true),
    ("happened", "date", true),
    ("amount", "int64", true),
    ("measurement", "double", true),
];
pub const REPORT_SCHEMA: Schema = &[
    ("calendar_year", "int64", true),
    ("label", "string", true),
    ("amount_class", "int64", false),
    ("entries", "int64", false),
    ("present", "int64", false),
    ("total", "int64", true),
    ("label_groups", "int64", false),
    ("through_year", "int64", true),
];

#[derive(Clone, Copy)]
pub struct Event {
    pub id: i64,
    pub dimension: Option<i64>,
    pub day: Option<&'static str>,
    pub amount: Option<i64>,
    pub measurement: Option<f64>,
}

impl Event {
    pub fn cells(&self) -> Value {
        json!([
            self.id.to_string(),
            integer(self.dimension),
            self.day,
            integer(self.amount),
            self.measurement
                .map(|value| format!("{:016x}", value.to_bits()))
        ])
    }
}

pub fn events(scaled: bool) -> Vec<Event> {
    if scaled {
        return (0..131_072)
            .map(|i: i64| Event {
                id: i,
                dimension: (i % 11 != 0).then_some([1, 2, 3, 99][(i % 4) as usize]),
                day: (i % 13 != 0).then_some(
                    ["1999-12-31", "2000-02-29", "2000-12-31", "2001-01-01"]
                        [((i / 4) % 4) as usize],
                ),
                amount: (i % 7 != 0).then_some(i % 101 - 50),
                measurement: (i % 17 != 0).then_some((i % 257) as f64 * 0.5),
            })
            .collect();
    }
    let rows = [
        (Some(1), Some("1999-12-31"), Some(10), Some(0.5)),
        (Some(2), Some("2000-02-29"), Some(20), Some(1.5)),
        (None, Some("2000-02-29"), Some(30), None),
        (Some(99), Some("2000-02-29"), None, Some(-0.0)),
        (Some(3), None, Some(5), Some(2.0)),
        (Some(1), Some("2001-01-01"), None, Some(2.5)),
        (Some(2), Some("2000-12-31"), Some(-5), Some(3.0)),
        (Some(1), None, Some(7), Some(-1.0)),
        (Some(1), Some("1999-12-31"), Some(-2), Some(4.0)),
        (Some(2), Some("2001-01-01"), Some(40), Some(4.5)),
        (Some(99), Some("2000-02-29"), Some(3), Some(5.0)),
        (None, None, Some(11), None),
        (Some(3), Some("2000-02-29"), Some(0), Some(6.0)),
        (Some(1), Some("2001-01-01"), Some(13), Some(6.5)),
        (Some(2), None, None, Some(7.0)),
        (Some(3), Some("2000-12-31"), Some(9), Some(-2.0)),
    ];
    rows.into_iter()
        .enumerate()
        .map(|(id, (dimension, day, amount, measurement))| Event {
            id: id as i64,
            dimension,
            day,
            amount,
            measurement,
        })
        .collect()
}

pub fn write_events(path: &Path, events: &[Event]) -> Result<()> {
    let mut output = BufWriter::new(File::create(path)?);
    writeln!(output, "id,dimension_id,happened,amount,measurement")?;
    for event in events {
        // These fixed inputs contain no commas, quotes or newlines. Debug's
        // float spelling preserves negative zero for the independent bit check.
        writeln!(
            output,
            "{},{},{},{},{}",
            event.id,
            event.dimension.map_or("\\N".into(), |v| v.to_string()),
            event.day.unwrap_or("\\N"),
            event.amount.map_or("\\N".into(), |v| v.to_string()),
            event.measurement.map_or("\\N".into(), |v| format!("{v:?}"))
        )?;
    }
    output.flush()?;
    Ok(())
}

pub fn small_report() -> Vec<Value> {
    // These are literal answers, not output from model(). Two north/1999 classes
    // both end at 15: a row-by-row window accumulation would be wrong.
    serde_json::from_str(include_str!("../../../test/data/analytics-report.json")).unwrap()
}
