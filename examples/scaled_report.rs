//! Check the classified event report with larger inputs and a chosen memory budget.
//!
//! The small profile uses the sixteen tutorial events; even and skewed profiles
//! each generate 131,072 events, with the latter concentrating joins on ID 2.
//! CASE separates missing, negative and nonnegative amounts. Each label receives
//! its group count and a running total through every class in the current year.
//! An empty profile checks that no groups are invented. A separate row-by-row
//! model computes the expected join and totals using a literal date-to-year map.
//! It shares input generation, but uses no engine query or grouping code.
//!
//! The program creates the database, reopens it with the requested budget, and
//! checks every result group. Large profiles must use temporary storage, cancel
//! a second execution after that storage appears, and succeed on a third. These
//! runs test cleanup and reuse as well as answers; a budget that prevents spill
//! is unsuitable for this particular check.
//!
//! Pass a new absolute database path, memory bytes and a profile name. Optional
//! `--measure` also checks the report after reclamation and prints phase times
//! and sampled engine reservations. Samples can miss peaks and do not measure process
//! memory. The database remains for inspection and caller cleanup.
//!
//! Run `cargo run --release --offline --locked --example scaled_report --
//! /absolute/new-database 8000000 even --measure`. Compare 8,000,000 and 32,000,000
//! byte budgets using fresh paths and the same profile; both can spill. Require
//! a successful exit and `status=finished`. `event_report.rs` is the smaller
//! example with literal answers. Tests exercise all four profiles and refusals:
//! `cargo test --release --offline --locked --example scaled_report -- --test-threads=1`.
//!
//! Whole-process measurements also include input generation, independent answers,
//! cancellation/replay and measurement projections. They cannot be attributed to
//! one query. Compare matching compiler artifacts and record host load.

#[path = "support/event_data.rs"]
mod event_data;

use event_data::Event;
use pipesql::{
    AppendLimits, CancellationToken, Config, DataType, Database, PreparedQuery, QueryStep, Value,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
/// Groups are ordered by year, label and amount class, with NULL first.
type Key = (Option<i64>, Option<&'static str>, i64);

#[derive(Debug, Default, PartialEq)]
struct Totals {
    entries: i64,
    present: i64,
    sum: Option<i64>,
    label_groups: i64,
    through_year: Option<i64>,
}

#[derive(Clone, Copy)]
enum Profile {
    Small,
    Even,
    Skewed,
    Empty,
}

impl Profile {
    fn len(self) -> usize {
        match self {
            Self::Small => event_data::EVENTS.len(),
            Self::Empty => 0,
            Self::Even | Self::Skewed => 131_072,
        }
    }

    fn event(self, index: usize) -> Event {
        if matches!(self, Self::Small) {
            return event_data::EVENTS[index];
        }
        let dimension = if index.is_multiple_of(11) {
            None
        } else if matches!(self, Self::Skewed) && !index.is_multiple_of(8) {
            Some(2)
        } else {
            let key_index = if matches!(self, Self::Skewed) {
                index / 8
            } else {
                index
            };
            Some([1, 2, 3, 99][key_index % 4])
        };
        Event {
            id: index as i64,
            dimension,
            day: (!index.is_multiple_of(13))
                .then_some([10_956, 11_016, 11_322, 11_323][(index / 4) % 4]),
            amount: (!index.is_multiple_of(7)).then_some((index % 101) as i64 - 50),
            measurement: (!index.is_multiple_of(17)).then_some((index % 257) as f64 * 0.5),
        }
    }
}

// Try every dimension row for each event. ID 2 contributes twice; an unmatched
// event contributes once with a NULL label. Keep this simple model separate from
// the engine's join and calendar code so it checks their results by another route.
fn expected(profile: Profile) -> BTreeMap<Key, Totals> {
    let mut groups = BTreeMap::<Key, Totals>::new();
    for index in 0..profile.len() {
        let event = profile.event(index);
        let year = event.day.map(|day| match day {
            10_956 => 1999,
            11_016 | 11_322 => 2000,
            11_323 => 2001,
            _ => panic!("date outside the independently mapped fixture"),
        });
        let class = match event.amount {
            None => 0,
            Some(value) if value < 0 => -1,
            Some(_) => 1,
        };
        let mut matches = 0;
        for (key, label) in [(1, "north"), (2, "south"), (2, "南"), (3, "")] {
            if event.dimension == Some(key) {
                accumulate(
                    groups.entry((year, Some(label), class)).or_default(),
                    event.amount,
                );
                matches += 1;
            }
        }
        if matches == 0 {
            accumulate(groups.entry((year, None, class)).or_default(), event.amount);
        }
    }
    // Compute each frame directly from all grouped rows. This deliberately does
    // not reproduce the engine's sorted peer replay or cumulative state machine.
    let frames: Vec<_> = groups
        .keys()
        .map(|&(year, label, _)| {
            let count = groups.keys().filter(|key| key.1 == label).count() as i64;
            let values: Vec<_> = groups
                .iter()
                .filter(|(key, _)| key.1 == label && key.0 <= year)
                .filter_map(|(_, totals)| totals.sum)
                .collect();
            (count, (!values.is_empty()).then(|| values.iter().sum()))
        })
        .collect();
    for (totals, (count, sum)) in groups.values_mut().zip(frames) {
        totals.label_groups = count;
        totals.through_year = sum;
    }
    groups
}

// COUNT(*) includes missing amounts; COUNT(amount) and SUM(amount) do not.
// Leave the sum absent until the first present amount, including a present zero.
fn accumulate(total: &mut Totals, amount: Option<i64>) {
    total.entries += 1;
    if let Some(amount) = amount {
        total.present += 1;
        total.sum = Some(total.sum.unwrap_or(0) + amount);
    }
}

// Ingestion uses a fixed budget. Reopening below applies the requested budget
// to query work, without making data construction depend on that experiment.
fn create(path: &Path, profile: Profile) -> Result<()> {
    let cancel = CancellationToken::new();
    let db = Database::create_empty(path, Config::new(32_000_000, 64_000_000)?)?;
    event_data::declare(&db, &cancel)?;
    if profile.len() != 0 {
        let mut append = db.begin_append(
            "events",
            AppendLimits {
                batches: profile.len().div_ceil(256) as u32,
                encoded_bytes: 8_000_000,
            },
            &cancel,
        )?;
        for start in (0..profile.len()).step_by(256) {
            let rows = 256.min(profile.len() - start);
            let mut events = [profile.event(start); 256];
            for (row, event) in events[..rows].iter_mut().enumerate() {
                *event = profile.event(start + row);
            }
            event_data::write_events(&mut append, &events[..rows], &cancel)?;
        }
        append.commit(&cancel)?;
    }
    db.close()?;
    Ok(())
}

// Maxima sampled at cursor construction and between steps miss transient peaks
// within a step. Reservations count admitted capacity, not touched heap or RSS.
#[derive(Debug)]
struct ReportUsage {
    sampled_memory_bytes: u64,
    sampled_temp_bytes: u64,
}

fn report(
    db: &Database,
    query: &PreparedQuery<'_>,
    reference: &BTreeMap<Key, Totals>,
) -> Result<ReportUsage> {
    check_schema(
        query,
        &[
            ("calendar_year", DataType::Int64, true),
            ("label", DataType::String, true),
            ("amount_class", DataType::Int64, false),
            ("entries", DataType::Int64, false),
            ("present", DataType::Int64, false),
            ("total", DataType::Int64, true),
            ("label_groups", DataType::Int64, false),
            ("through_year", DataType::Int64, true),
        ],
    )?;
    let resident = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let mut result = db.execute(query, &cancel)?;
    let mut expected = reference.iter();
    let mut usage = ReportUsage {
        sampled_memory_bytes: db.reserved_memory_bytes(),
        sampled_temp_bytes: db.reserved_temp_bytes(),
    };
    for _ in 0..10_000_000 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let (&(year, label, class), totals) =
                        expected.next().ok_or("extra report group")?;
                    let actual_label = match batch.value(row, 1) {
                        Some(Value::Null) => None,
                        Some(Value::String(value)) => Some(value.as_str()),
                        _ => return Err("unexpected label type".into()),
                    };
                    if batch.value(row, 0) != Some(year.map_or(Value::Null, Value::Int64))
                        || actual_label != label
                        || batch.value(row, 2) != Some(Value::Int64(class))
                        || batch.value(row, 3) != Some(Value::Int64(totals.entries))
                        || batch.value(row, 4) != Some(Value::Int64(totals.present))
                        || batch.value(row, 5) != Some(totals.sum.map_or(Value::Null, Value::Int64))
                        || batch.value(row, 6) != Some(Value::Int64(totals.label_groups))
                        || batch.value(row, 7)
                            != Some(totals.through_year.map_or(Value::Null, Value::Int64))
                    {
                        return Err("report differs from independent row model".into());
                    }
                }
            }
            QueryStep::Finished => {
                if expected.next().is_some() {
                    return Err("missing report group".into());
                }
                drop(result);
                if db.reserved_memory_bytes() != resident || db.reserved_temp_bytes() != 0 {
                    return Err("report reservations remain live".into());
                }
                return Ok(usage);
            }
            QueryStep::Failed(_) => return Err(result.into_error().ok_or("missing error")?.into()),
        }
        // Observe reservations between steps. Allocations acquired and released
        // within a step are invisible to these samples.
        usage.sampled_memory_bytes = usage.sampled_memory_bytes.max(db.reserved_memory_bytes());
        usage.sampled_temp_bytes = usage.sampled_temp_bytes.max(db.reserved_temp_bytes());
    }
    Err("report exhausted its step bound".into())
}

fn check_schema(query: &PreparedQuery<'_>, expected: &[(&str, DataType, bool)]) -> Result<()> {
    if query.result_column_count() != expected.len() {
        return Err("unexpected result width".into());
    }
    for (index, &(name, data_type, nullable)) in expected.iter().enumerate() {
        let actual = query.result_column(index).ok_or("missing result column")?;
        if actual.name != Some(name) || actual.data_type != data_type || actual.nullable != nullable
        {
            return Err("unexpected result schema".into());
        }
    }
    Ok(())
}

fn released(db: &Database, baseline: u64) -> Result<()> {
    if db.reserved_memory_bytes() != baseline || db.reserved_temp_bytes() != 0 {
        return Err("query reservations remain live".into());
    }
    Ok(())
}

// Wait for temporary storage before cancelling: an execution rejected at
// admission would not exercise cleanup of an active spill.
fn cancel_after_spill(db: &Database, query: &PreparedQuery<'_>) -> Result<()> {
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let mut result = db.execute(query, &cancel)?;
    let mut spilled = false;
    for _ in 0..10_000_000 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Failed(_) => return Err(result.into_error().ok_or("missing error")?.into()),
            _ => return Err("report emitted before the spill cancellation point".into()),
        }
        if db.reserved_temp_bytes() > 0 {
            spilled = true;
            break;
        }
    }
    if !spilled {
        return Err("report never reached spill".into());
    }
    cancel.cancel();
    if !matches!(result.step(), QueryStep::Failed(pipesql::Error::Cancelled)) {
        return Err("spill cancellation did not report Cancelled".into());
    }
    drop(result);
    released(db, baseline)
}

// Check every stored measurement and its doubled value against generated input.
// This exercises projection across the scaled data; event_report.rs separately
// checks literal floating-point bits without deriving them from the input builder.
fn verify_measurements(db: &Database, profile: Profile) -> Result<()> {
    let baseline = db.reserved_memory_bytes();
    let query = db.prepare(
        "FROM events |> ORDER BY id |> SELECT id, measurement, measurement * 2 AS doubled",
    )?;
    check_schema(
        &query,
        &[
            ("id", DataType::Int64, false),
            ("measurement", DataType::Double, true),
            ("doubled", DataType::Double, true),
        ],
    )?;
    let cancel = CancellationToken::new();
    let mut result = db.execute(&query, &cancel)?;
    let mut seen = 0;
    for _ in 0..10_000_000 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    if seen >= profile.len()
                        || batch.value(row, 0) != Some(Value::Int64(seen as i64))
                    {
                        return Err("unexpected measurement identity".into());
                    }
                    let measurement = profile.event(seen).measurement;
                    for (column, expected) in [(1, measurement), (2, measurement.map(|n| n * 2.0))]
                    {
                        let actual = match batch.value(row, column) {
                            Some(Value::Null) => None,
                            Some(Value::Double(value)) => Some(value.to_bits()),
                            _ => return Err("unexpected measurement type".into()),
                        };
                        if actual != expected.map(f64::to_bits) {
                            return Err("incorrect measurement bits".into());
                        }
                    }
                    seen += 1;
                }
            }
            QueryStep::Finished => {
                if seen != profile.len() {
                    return Err("missing measurements".into());
                }
                drop(result);
                drop(query);
                return released(db, baseline);
            }
            QueryStep::Failed(_) => return Err(result.into_error().ok_or("missing error")?.into()),
        }
    }
    Err("measurement query exhausted its step bound".into())
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("supply a new absolute database path")?;
    let memory: u64 = args
        .next()
        .ok_or("supply query memory bytes")?
        .to_str()
        .ok_or("memory must be UTF-8")?
        .parse()?;
    let profile = match args.next().as_deref().and_then(|s| s.to_str()) {
        Some("small") => Profile::Small,
        Some("even") => Profile::Even,
        Some("skewed") => Profile::Skewed,
        Some("empty") => Profile::Empty,
        _ => return Err("choose small, even, skewed or empty".into()),
    };
    let measure = match args.next() {
        None => false,
        Some(value) if value == "--measure" => true,
        Some(_) => return Err("optional final argument must be --measure".into()),
    };
    let path = Path::new(&path);
    if args.next().is_some() || !path.is_absolute() || path.exists() {
        return Err("expected a new absolute path, memory bytes and profile".into());
    }
    let reference = expected(profile);
    // Wall-clock phases: ingestion includes creation, declaration, batch building,
    // writes, commits and close, but excludes the independent model above. Large
    // profiles write 512 units of 256 events; each write synchronizes its unit.
    // Changing the batch size changes this workload, including its I/O cost.
    let started = Instant::now();
    create(path, profile)?;
    let ingest = started.elapsed();
    let started = Instant::now();
    let db = Database::open(path, Config::new(memory, 64_000_000)?)?;
    let reopen = started.elapsed();
    let resident = db.reserved_memory_bytes();
    let started = Instant::now();
    let query = db.prepare(include_str!("analytics.sql"))?;
    let prepare = started.elapsed();
    let prepared_memory = db.reserved_memory_bytes();
    let started = Instant::now();
    // Execution includes schema/answer checks and cursor release. Later replay,
    // cancellation and measurement checks are outside this phase.
    let usage = report(&db, &query, &reference)?;
    let execute = started.elapsed();
    let peak_temp = usage.sampled_temp_bytes;
    if profile.len() == 131_072 {
        if peak_temp == 0 {
            return Err("scaled report did not spill".into());
        }
        cancel_after_spill(&db, &query)?;
        report(&db, &query, &reference)?;
    }
    drop(query);
    released(&db, resident)?;
    verify_measurements(&db, profile)?;
    if measure {
        let started = Instant::now();
        let removed = db.reclaim(&CancellationToken::new())?;
        let reclaim = started.elapsed();
        // No earlier plan retains a snapshot here. Run the report again to
        // detect reclamation that removed data still needed by a new query.
        let query = db.prepare(include_str!("analytics.sql"))?;
        report(&db, &query, &reference)?;
        drop(query);
        released(&db, resident)?;
        eprintln!("phase=ingest elapsed_ns={}", ingest.as_nanos());
        eprintln!(
            "phase=reopen elapsed_ns={} resident_bytes={resident}",
            reopen.as_nanos()
        );
        eprintln!(
            "phase=prepare elapsed_ns={} reserved_bytes={prepared_memory}",
            prepare.as_nanos()
        );
        eprintln!(
            "phase=execute elapsed_ns={} sampled_memory_bytes={} sampled_temp_bytes={peak_temp}",
            execute.as_nanos(),
            usage.sampled_memory_bytes
        );
        eprintln!(
            "phase=reclaim elapsed_ns={} removed_names={removed}",
            reclaim.as_nanos()
        );
    }
    db.close()?;
    println!(
        "events={} groups={} sampled_temp_bytes={peak_temp}",
        profile.len(),
        reference.len()
    );
    println!("status=finished");
    Ok(())
}

#[cfg(test)]
#[path = "../test/support/mod.rs"]
mod support;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_match_at_both_budgets_and_release_cancelled_work() {
        for profile in [
            Profile::Small,
            Profile::Even,
            Profile::Skewed,
            Profile::Empty,
        ] {
            let directory = support::Directory::new();
            let path = directory.0.join("database");
            create(&path, profile).unwrap();
            let reference = expected(profile);
            for memory in [32_000_000, 8_000_000] {
                let db = Database::open(&path, Config::new(memory, 64_000_000).unwrap()).unwrap();
                let resident = db.reserved_memory_bytes();
                let query = db.prepare(include_str!("analytics.sql")).unwrap();
                let peak = report(&db, &query, &reference).unwrap();
                if profile.len() == 131_072 {
                    assert!(
                        peak.sampled_temp_bytes > 0,
                        "large report must exhibit temporary storage"
                    );
                    cancel_after_spill(&db, &query).unwrap();
                    report(&db, &query, &reference).unwrap();
                }
                drop(query);
                released(&db, resident).unwrap();
                verify_measurements(&db, profile).unwrap();
                db.close().unwrap();
            }
        }
    }

    #[test]
    fn refusal_and_wrong_answers_release_ownership_before_reuse() {
        let directory = support::Directory::new();
        let path = directory.0.join("database");
        create(&path, Profile::Even).unwrap();
        let reference = expected(Profile::Even);
        for (memory, temp, expected_owner) in [
            (500_000, 64_000_000, "native query workspace"),
            (8_000_000, 1, "database temporary storage"),
        ] {
            let db = Database::open(&path, Config::new(memory, temp).unwrap()).unwrap();
            let resident = db.reserved_memory_bytes();
            let attempt = || -> Result<ReportUsage> {
                let query = db.prepare(include_str!("analytics.sql"))?;
                report(&db, &query, &reference)
            };
            let failure = attempt().unwrap_err();
            match failure.downcast_ref::<pipesql::Error>() {
                Some(pipesql::Error::Resource {
                    owner,
                    required,
                    limit,
                }) => {
                    assert_eq!(*owner, expected_owner);
                    assert!(required > limit);
                }
                _ => panic!("expected typed resource refusal, got {failure}"),
            }
            drop(failure);
            released(&db, resident).unwrap();
            db.close().unwrap();
        }
        let db = Database::open(&path, Config::new(8_000_000, 64_000_000).unwrap()).unwrap();
        let resident = db.reserved_memory_bytes();
        let query = db.prepare(include_str!("analytics.sql")).unwrap();
        let baseline = db.reserved_memory_bytes();
        let mut wrong = expected(Profile::Even);
        wrong.get_mut(&(Some(2000), None, 1)).unwrap().entries += 1;
        assert_eq!(
            report(&db, &query, &wrong).unwrap_err().to_string(),
            "report differs from independent row model"
        );
        released(&db, baseline).unwrap();
        report(&db, &query, &reference).unwrap();

        // Receiving a batch does not finish the query. Drop it at that point
        // to check early cleanup, then execute the same prepared query again.
        let cancel = CancellationToken::new();
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut emitted = false;
        for _ in 0..10_000_000 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Rows(batch) => {
                    assert!(!batch.is_empty());
                    emitted = true;
                    break;
                }
                _ => panic!("expected a report batch before termination"),
            }
        }
        assert!(emitted);
        drop(result);
        released(&db, baseline).unwrap();
        report(&db, &query, &reference).unwrap();
        drop(query);
        released(&db, resident).unwrap();
        db.close().unwrap();
    }
}
