//! Measure string extrema and grouping by short or maximum-length text keys.
//!
//! The default extrema profile gives each integer key a string of a's and one of
//! z's. The keys profile writes each distinct text key twice, carrying its integer
//! identity both times. Literal extrema, counts and twice each identity define
//! complete expected answers independently of engine grouping.
//!
//! Pass a new absolute database path, group count, text width in bytes and query
//! memory limit in bytes. Optional arguments set batch size (default one) and
//! profile (`extrema` or `keys`, default `extrema`).
//! Run `cargo run --release --offline --locked --example string_grouping --
//! /absolute/new-words 256 8 4000000`. Compare 4 or 256 groups, widths 8 or 65536,
//! and budgets 4000000 or 80000000, using fresh paths. Batch sizes are 1, 4 or 256;
//! 256 requires eight-byte text. Changing batching changes input units and I/O,
//! so it does not isolate hash-table cost. An untimed warm-up observes written
//! spill on Linux; five samples include execution, answer checks and cursor
//! disposal. Spilled work must also cancel, release its owners and retry. Input
//! construction and expected strings precede timing. Reservations exclude those
//! caller strings, OS buffering and other process costs.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, PreparedQuery, QueryStep, Value,
};
use std::path::Path;
use std::time::Instant;

#[path = "support/query.rs"]
mod query_support;
use query_support::{cancel, released, scratch_usage};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const TEMP_BYTES: u64 = 128_000_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Profile {
    Extrema,
    Keys,
}

impl Profile {
    fn name(self) -> &'static str {
        match self {
            Self::Extrema => "extrema",
            Self::Keys => "keys",
        }
    }

    fn sql(self) -> &'static str {
        match self {
            Self::Extrema => {
                "FROM words |> AGGREGATE MIN(word) AS lo, MAX(word) AS hi, COUNT(*) AS n GROUP AND ORDER BY k"
            }
            Self::Keys => {
                "FROM words |> AGGREGATE SUM(k) AS total, COUNT(*) AS n GROUP AND ORDER BY word"
            }
        }
    }
}

struct Expected {
    groups: usize,
    low: String,
    high: String,
    keys: Vec<String>,
}

impl Expected {
    fn new(groups: usize, width: usize, profile: Profile) -> Self {
        Self {
            groups,
            low: "a".repeat(width),
            high: "z".repeat(width),
            // Equal prefixes put the distinguishing identity at the end of each
            // key. Fixed-width decimal suffixes also establish the expected order.
            keys: if profile == Profile::Keys {
                (0..groups)
                    .map(|key| format!("{}{key:08}", "x".repeat(width - 8)))
                    .collect()
            } else {
                Vec::new()
            },
        }
    }
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("supply a new absolute database path")?;
    let mut number = || -> Result<u64> {
        Ok(args
            .next()
            .ok_or("supply groups, text bytes, and memory bytes")?
            .to_str()
            .ok_or("numeric arguments must be UTF-8")?
            .parse()?)
    };
    let groups = number()?;
    let width = number()?;
    let memory = number()?;
    let batch_rows = match args.next() {
        Some(value) => value
            .to_str()
            .ok_or("batch rows must be UTF-8")?
            .parse::<u64>()?,
        None => 1,
    };
    let profile = match args.next() {
        None => Profile::Extrema,
        Some(value) => match value.to_str() {
            Some("extrema") => Profile::Extrema,
            Some("keys") => Profile::Keys,
            _ => return Err("choose extrema or keys profile".into()),
        },
    };
    if args.next().is_some()
        || !matches!(groups, 4 | 256)
        || !matches!(width, 8 | 65_536)
        || !matches!(batch_rows, 1 | 4 | 256)
        || (batch_rows == 256 && width != 8)
    {
        return Err(
            "expected path, groups (4 or 256), text bytes (8 or 65536), memory bytes, optional batch rows (1, 4, or 256; 256 requires eight-byte text), and optional profile (extrema or keys)".into(),
        );
    }
    let config = Config::new(memory, TEMP_BYTES)?;
    let expected = Expected::new(groups as usize, width as usize, profile);
    let token = CancellationToken::new();
    let path = Path::new(&path);
    create_words(path, batch_rows, profile, &expected, &token)?;
    let db = Database::open(path, config)?;
    let query = db.prepare(profile.sql())?;
    let warmup = execute(&db, &query, profile, &expected, path, true)?;
    println!(
        "input profile={} groups={groups} text_bytes={width} memory_limit={memory} batch_rows={batch_rows}",
        profile.name()
    );
    println!(
        "warmup scratch_bytes={} file_probe={}",
        warmup.scratch,
        cfg!(target_os = "linux")
    );
    for index in 0..5 {
        let sample = execute(&db, &query, profile, &expected, path, false)?;
        println!(
            "sample={index} elapsed_ns={} rows={} batches={} progress={} memory={} temporary={}",
            sample.ns,
            sample.rows,
            sample.batches,
            sample.progress,
            sample.memory,
            sample.temporary
        );
    }
    if warmup.temporary > 0 {
        cancel(&db, &query, path)?;
        execute(&db, &query, profile, &expected, path, false)?;
    }
    drop(query);
    db.close()?;
    println!(
        "verified profile={} groups={groups} text_bytes={width} memory_limit={memory} batch_rows={batch_rows}",
        profile.name()
    );
    println!("status=finished");
    Ok(())
}

struct Sample {
    ns: u128,
    rows: usize,
    batches: u64,
    progress: u64,
    memory: u64,
    temporary: u64,
    scratch: u64,
}

fn execute(
    db: &Database,
    query: &PreparedQuery<'_>,
    profile: Profile,
    expected: &Expected,
    path: &Path,
    observe: bool,
) -> Result<Sample> {
    let columns: &[(&str, DataType, bool)] = match profile {
        Profile::Extrema => &[
            ("k", DataType::Int64, false),
            ("lo", DataType::String, true),
            ("hi", DataType::String, true),
            ("n", DataType::Int64, false),
        ],
        Profile::Keys => &[
            ("word", DataType::String, false),
            ("total", DataType::Int64, true),
            ("n", DataType::Int64, false),
        ],
    };
    if query.result_column_count() != columns.len()
        || columns
            .iter()
            .enumerate()
            .any(|(i, &(name, kind, nullable))| {
                query.result_column(i).is_none_or(|c| {
                    c.name != Some(name) || c.data_type != kind || c.nullable != nullable
                })
            })
    {
        return Err("unexpected string grouping schema".into());
    }
    let baseline = db.reserved_memory_bytes();
    // Start after setup and preparation. Stop after the query finishes, every
    // returned group is checked and the result has been dropped.
    let start = Instant::now();
    let token = CancellationToken::new();
    let mut result = db.execute(query, &token)?;
    let mut sample = Sample {
        ns: 0,
        rows: 0,
        batches: 0,
        progress: 0,
        memory: db.reserved_memory_bytes(),
        temporary: db.reserved_temp_bytes(),
        scratch: 0,
    };
    let mut finished = false;
    for _ in 0..20_000_000 {
        match result.step() {
            QueryStep::Rows(batch) => {
                sample.batches += 1;
                if batch.column_count() != columns.len() {
                    return Err("unexpected string grouping width".into());
                }
                for row in 0..batch.len() {
                    if sample.rows >= expected.groups {
                        return Err("extra string group".into());
                    }
                    let correct = match profile {
                        Profile::Extrema => matches!((
                        batch.value(row, 0),
                        batch.value(row, 1),
                        batch.value(row, 2),
                        batch.value(row, 3),
                    ),
                        (
                            Some(Value::Int64(key)),
                            Some(Value::String(minimum)),
                            Some(Value::String(maximum)),
                            Some(Value::Int64(2)),
                        ) if key == sample.rows as i64
                            && minimum.as_str() == expected.low
                            && maximum.as_str() == expected.high),
                        Profile::Keys => {
                            matches!((batch.value(row, 0), batch.value(row, 1), batch.value(row, 2)), (Some(Value::String(key)), Some(Value::Int64(total)), Some(Value::Int64(2))) if key.as_str() == expected.keys[sample.rows] && total == 2 * sample.rows as i64)
                        }
                    };
                    if !correct {
                        return Err("incorrect string group".into());
                    }
                    sample.rows += 1;
                }
            }
            QueryStep::Progress => sample.progress += 1,
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(_) => {
                return Err(result.into_error().ok_or("missing query error")?.into());
            }
        }
        sample.memory = sample.memory.max(db.reserved_memory_bytes());
        sample.temporary = sample.temporary.max(db.reserved_temp_bytes());
        if observe && sample.temporary > 0 && sample.scratch == 0 {
            sample.scratch = scratch_usage(path)?.1;
        }
    }
    if !finished || sample.rows != expected.groups {
        return Err("incomplete string groups".into());
    }
    drop(result);
    sample.ns = start.elapsed().as_nanos();
    released(db, baseline, path)?;
    if observe && cfg!(target_os = "linux") && (sample.scratch > 0) != (sample.temporary > 0) {
        return Err("string grouping spill observation disagrees with its account".into());
    }
    Ok(sample)
}

fn create_words(
    path: &Path,
    batch_rows: u64,
    profile: Profile,
    expected: &Expected,
    cancel: &CancellationToken,
) -> std::result::Result<(), pipesql::Error> {
    let groups = expected.groups as u64;
    let db = Database::create_empty(path, Config::new(128_000_000, TEMP_BYTES)?)?;
    db.declare_table(
        "words",
        &[
            ColumnDeclaration {
                name: "k",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "word",
                data_type: DataType::String,
                nullable: false,
            },
        ],
        cancel,
    )?;
    let mut append = db.begin_append(
        "words",
        AppendLimits {
            batches: (2 * groups.div_ceil(batch_rows)) as u32,
            encoded_bytes: 64_000_000,
        },
        cancel,
    )?;
    // Changing batch size must not change row order: write all low values,
    // then all high values, with descending keys in each pass. Four maximum-
    // length strings fit the column payload limit; 256-row batches therefore
    // require the eight-byte strings checked above.
    let mut keys = [0_i64; 256];
    for word in [expected.low.as_str(), expected.high.as_str()] {
        let mut words = [word; 256];
        let mut remaining = groups;
        while remaining != 0 {
            let rows = remaining.min(batch_rows) as usize;
            for (offset, key) in keys[..rows].iter_mut().enumerate() {
                *key = (remaining - 1 - offset as u64) as i64;
                if profile == Profile::Keys {
                    words[offset] = &expected.keys[*key as usize];
                }
            }
            let mut validity = [0xff_u8; 32];
            let validity_bytes = rows.div_ceil(8);
            if !rows.is_multiple_of(8) {
                validity[validity_bytes - 1] = (1 << (rows % 8)) - 1;
            }
            append.write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&keys[..rows]),
                        validity: &validity[..validity_bytes],
                    },
                    ColumnInput {
                        values: ColumnValues::String(&words[..rows]),
                        validity: &validity[..validity_bytes],
                    },
                ],
                cancel,
            )?;
            remaining -= rows as u64;
        }
    }
    append.commit(cancel)?;
    db.close()
}

#[cfg(test)]
#[path = "../test/support/mod.rs"]
mod support;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_profiles_require_complete_answers_spill_and_release() {
        let directory = support::Directory::new();
        for profile in [Profile::Extrema, Profile::Keys] {
            for width in [8, 65_536] {
                let path = directory.0.join(format!("{}-{width}", profile.name()));
                let mut expected = Expected::new(256, width, profile);
                create_words(&path, 4, profile, &expected, &CancellationToken::new()).unwrap();
                for memory in [4_000_000, 80_000_000] {
                    let db =
                        Database::open(&path, Config::new(memory, TEMP_BYTES).unwrap()).unwrap();
                    let query = db.prepare(profile.sql()).unwrap();
                    let sample = execute(&db, &query, profile, &expected, &path, true).unwrap();
                    assert_eq!(sample.temporary > 0, width == 65_536 && memory == 4_000_000);
                    if sample.temporary > 0 {
                        cancel(&db, &query, &path).unwrap();
                        execute(&db, &query, profile, &expected, &path, false).unwrap();
                    } else {
                        let baseline = db.reserved_memory_bytes();
                        assert!(cancel(&db, &query, &path).is_err());
                        released(&db, baseline, &path).unwrap();
                    }
                    if memory == 80_000_000 {
                        let wrong_rows = db
                            .prepare(&format!("{} |> LIMIT 255", profile.sql()))
                            .unwrap();
                        let baseline = db.reserved_memory_bytes();
                        let error = execute(&db, &wrong_rows, profile, &expected, &path, false)
                            .err()
                            .expect("prefix rejected");
                        assert_eq!(error.to_string(), "incomplete string groups");
                        released(&db, baseline, &path).unwrap();
                        drop(wrong_rows);

                        let suffix = match profile {
                            Profile::Extrema => " |> SELECT k, lo, hi, n+1 AS n",
                            Profile::Keys => " |> SELECT word, total+1 AS total, n",
                        };
                        let wrong_value =
                            db.prepare(&format!("{}{suffix}", profile.sql())).unwrap();
                        let baseline = db.reserved_memory_bytes();
                        let error = execute(&db, &wrong_value, profile, &expected, &path, false)
                            .err()
                            .expect("wrong value rejected");
                        assert_eq!(error.to_string(), "incorrect string group");
                        released(&db, baseline, &path).unwrap();
                        drop(wrong_value);

                        // Change only the last byte of an expected full string,
                        // after creating the input. Prefix checks cannot catch it.
                        let text = match profile {
                            Profile::Extrema => &mut expected.high,
                            Profile::Keys => expected.keys.last_mut().unwrap(),
                        };
                        let original = text.pop().unwrap();
                        text.push('!');
                        let baseline = db.reserved_memory_bytes();
                        let error = execute(&db, &query, profile, &expected, &path, false)
                            .err()
                            .expect("wrong complete text rejected");
                        assert_eq!(error.to_string(), "incorrect string group");
                        released(&db, baseline, &path).unwrap();
                        let text = match profile {
                            Profile::Extrema => &mut expected.high,
                            Profile::Keys => expected.keys.last_mut().unwrap(),
                        };
                        text.pop();
                        text.push(original);
                        execute(&db, &query, profile, &expected, &path, false).unwrap();
                    }
                    drop(query);
                    db.close().unwrap();
                }
            }
        }
    }
}
