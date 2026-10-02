//! Measure grouped accumulation on independently checkable inputs.
//!
//! Two passes write 8,192 sales: amount 1 is spread evenly, then amount 3 is
//! spread evenly or concentrated in region zero. Integer totals define every
//! count, sum, extremum and mean. The numeric profiles compare INT64 and DOUBLE
//! SUM/AVG using exactly representable inputs and sums, followed by one division.
//! The filtered profile keeps even keys and uses ABS(total)+1 in both filtering
//! and projection; its expected value is the positive integer total plus one.
//!
//! Pass a fresh absolute path, query memory bytes, groups (32 or 4096, default
//! 4096), distribution (`even` or `skewed`, default `even`), and optional profile
//! (`extrema`, `integer`, `double` or `filtered`, default `extrema`). Input construction and
//! expected answers precede timing. One checked warm-up precedes five query
//! samples including complete answer checks and cursor disposal. Linux observes
//! actual unlinked spill during warm-up, outside measured samples. Spilled cases
//! must also cancel and retry. Resource samples are not process-memory bounds.
//!
//! Run `cargo run --release --offline --locked -j 1 --example grouping --
//! /absolute/new-sales 1200000 4096 even double`. The caller owns the remaining
//! database. Require exit zero and the final completion line.

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
const ROWS_PER_PASS: usize = 4096;
const BATCH_ROWS: usize = 256;
const TEMP_BYTES: u64 = 8_000_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Profile {
    Extrema,
    Integer,
    Double,
    Filtered,
}

impl Profile {
    fn name(self) -> &'static str {
        match self {
            Self::Extrema => "extrema",
            Self::Integer => "integer",
            Self::Double => "double",
            Self::Filtered => "filtered",
        }
    }
    fn kind(self) -> DataType {
        if self == Self::Double {
            DataType::Double
        } else {
            DataType::Int64
        }
    }
    fn sql(self) -> &'static str {
        if self == Self::Extrema {
            "FROM sales |> AGGREGATE COUNT(*) AS n, SUM(amount) AS total, MIN(amount) AS smallest, MAX(amount) AS largest GROUP AND ORDER BY region"
        } else if self == Self::Filtered {
            "FROM sales |> AGGREGATE COUNT(*) AS n, SUM(amount) AS total GROUP AND ORDER BY region |> EXTEND ABS(total)+1 AS adjusted, MOD(region, 2) AS parity |> WHERE parity=0 AND adjusted>0 |> SELECT region, n, total, adjusted"
        } else {
            "FROM sales |> AGGREGATE COUNT(*) AS n, SUM(amount) AS total, AVG(amount) AS mean GROUP AND ORDER BY region"
        }
    }
}

struct Expected {
    region: i64,
    count: i64,
    total: i64,
    largest: i64,
    mean: f64,
}

fn expected(groups: usize, skewed: bool, profile: Profile) -> Vec<Expected> {
    (0..groups)
        // All totals are positive. The filtered query therefore keeps exactly
        // the even keys, independently of the engine's predicate evaluation.
        .filter(|region| profile != Profile::Filtered || region % 2 == 0)
        .map(|region| {
            let first = (ROWS_PER_PASS / groups) as i64;
            let second = if skewed {
                if region == 0 { ROWS_PER_PASS as i64 } else { 0 }
            } else {
                first
            };
            let count = first + second;
            let total = first + 3 * second;
            // Every input and intermediate sum is an exact small integer. A single
            // division therefore gives the expected DOUBLE mean without engine code.
            Expected {
                region: region as i64,
                count,
                total,
                largest: if second == 0 { 1 } else { 3 },
                mean: total as f64 / count as f64,
            }
        })
        .collect()
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
    expected: &[Expected],
    path: &Path,
    observe: bool,
) -> Result<Sample> {
    let mut columns = vec![
        ("region", DataType::Int64, false),
        ("n", DataType::Int64, false),
        ("total", profile.kind(), true),
    ];
    if profile == Profile::Extrema {
        columns.extend([
            ("smallest", DataType::Int64, true),
            ("largest", DataType::Int64, true),
        ]);
    } else if profile == Profile::Filtered {
        columns.push(("adjusted", DataType::Int64, true));
    } else {
        columns.push(("mean", DataType::Double, true));
    }
    if query.result_column_count() != columns.len()
        || columns
            .iter()
            .enumerate()
            .any(|(i, (name, kind, nullable))| {
                query.result_column(i).is_none_or(|c| {
                    c.name != Some(*name) || c.data_type != *kind || c.nullable != *nullable
                })
            })
    {
        return Err("unexpected grouping schema".into());
    }
    let baseline = db.reserved_memory_bytes();
    let token = CancellationToken::new();
    let start = Instant::now();
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
                    return Err("unexpected grouping width".into());
                }
                for row in 0..batch.len() {
                    let answer = expected.get(sample.rows).ok_or("extra grouped row")?;
                    let total = if profile == Profile::Double {
                        Some(Value::Double(answer.total as f64))
                    } else {
                        Some(Value::Int64(answer.total))
                    };
                    if batch.value(row, 0) != Some(Value::Int64(answer.region))
                        || batch.value(row, 1) != Some(Value::Int64(answer.count))
                        || batch.value(row, 2) != total
                    {
                        return Err("incorrect grouped row".into());
                    }
                    if profile == Profile::Extrema {
                        if batch.value(row, 3) != Some(Value::Int64(1))
                            || batch.value(row, 4) != Some(Value::Int64(answer.largest))
                        {
                            return Err("incorrect grouped extrema".into());
                        }
                    } else if profile == Profile::Filtered {
                        // Positive integer totals make ABS(total)+1 exactly total+1.
                        if batch.value(row, 3) != Some(Value::Int64(answer.total + 1)) {
                            return Err("incorrect grouped adjusted value".into());
                        }
                    } else if !matches!(batch.value(row, 3), Some(Value::Double(mean)) if mean.to_bits() == answer.mean.to_bits())
                    {
                        return Err("incorrect grouped mean".into());
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
    if !finished || sample.rows != expected.len() {
        return Err("incomplete grouped result".into());
    }
    drop(result);
    sample.ns = start.elapsed().as_nanos();
    released(db, baseline, path)?;
    if observe && cfg!(target_os = "linux") && (sample.scratch > 0) != (sample.temporary > 0) {
        return Err("grouping spill observation disagrees with its account".into());
    }
    Ok(sample)
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(2..=5).contains(&args.len()) {
        return Err(
            "expected fresh path, memory bytes, optional groups, distribution and profile".into(),
        );
    }
    let path = Path::new(&args[0]);
    let memory: u64 = args[1].parse()?;
    let groups = args.get(2).map_or(Ok(4096), |s| s.parse())?;
    let skewed = match args.get(3).map(String::as_str).unwrap_or("even") {
        "even" => false,
        "skewed" => true,
        _ => return Err("choose even or skewed".into()),
    };
    let profile = match args.get(4).map(String::as_str).unwrap_or("extrema") {
        "extrema" => Profile::Extrema,
        "integer" => Profile::Integer,
        "double" => Profile::Double,
        "filtered" => Profile::Filtered,
        _ => return Err("choose extrema, integer, double or filtered".into()),
    };
    if !matches!(groups, 32 | 4096) {
        return Err("groups must be 32 or 4096".into());
    }
    let config = Config::new(memory, TEMP_BYTES)?;
    create_sales(path, groups, skewed, profile)?;
    let expected = expected(groups, skewed, profile);
    let db = Database::open(path, config)?;
    let query = db.prepare(profile.sql())?;
    let warmup = execute(&db, &query, profile, &expected, path, true)?;
    println!(
        "input profile={} groups={groups} rows=8192 skewed={skewed} memory_limit={memory} batch_rows={BATCH_ROWS}",
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
        "verified profile={} groups={groups} rows=8192 skewed={skewed} memory_limit={memory}",
        profile.name()
    );
    println!("status=finished");
    Ok(())
}

fn create_sales(path: &Path, groups: usize, skewed: bool, profile: Profile) -> Result<()> {
    let db = Database::create_empty(path, Config::new(32_000_000, TEMP_BYTES)?)?;
    let cancel = CancellationToken::new();
    db.declare_table(
        "sales",
        &[
            ColumnDeclaration {
                name: "region",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "amount",
                data_type: profile.kind(),
                nullable: false,
            },
        ],
        &cancel,
    )?;
    let mut append = db.begin_append(
        "sales",
        AppendLimits {
            batches: (2 * ROWS_PER_PASS / BATCH_ROWS) as u32,
            encoded_bytes: 1_000_000,
        },
        &cancel,
    )?;
    for amount in [1, 3] {
        for start in (0..ROWS_PER_PASS).step_by(BATCH_ROWS) {
            let regions: [i64; BATCH_ROWS] = std::array::from_fn(|row| {
                if skewed && amount == 3 {
                    0
                } else {
                    ((ROWS_PER_PASS - 1 - start - row) % groups) as i64
                }
            });
            let integers = [amount; BATCH_ROWS];
            let doubles = [amount as f64; BATCH_ROWS];
            append.write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&regions),
                        validity: &[255; BATCH_ROWS / 8],
                    },
                    ColumnInput {
                        values: if profile == Profile::Double {
                            ColumnValues::Double(&doubles)
                        } else {
                            ColumnValues::Int64(&integers)
                        },
                        validity: &[255; BATCH_ROWS / 8],
                    },
                ],
                &cancel,
            )?;
        }
    }
    append.commit(&cancel)?;
    db.close()?;
    Ok(())
}

#[cfg(test)]
#[path = "../test/support/mod.rs"]
mod support;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_grouped_answers_spill_cancellation_and_wrong_results() {
        let directory = support::Directory::new();
        for profile in [
            Profile::Extrema,
            Profile::Integer,
            Profile::Double,
            Profile::Filtered,
        ] {
            for skewed in [false, true] {
                let path = directory.0.join(format!("{}-{skewed}", profile.name()));
                create_sales(&path, 4096, skewed, profile).unwrap();
                let answers = expected(4096, skewed, profile);
                for memory in [
                    if cfg!(target_os = "macos") {
                        1_800_000
                    } else {
                        1_200_000
                    },
                    12_000_000,
                ] {
                    let db =
                        Database::open(&path, Config::new(memory, TEMP_BYTES).unwrap()).unwrap();
                    let query = db.prepare(profile.sql()).unwrap();
                    let sample = execute(&db, &query, profile, &answers, &path, true).unwrap();
                    assert_eq!(sample.temporary > 0, memory != 12_000_000);
                    if sample.temporary > 0 {
                        cancel(&db, &query, &path).unwrap();
                        execute(&db, &query, profile, &answers, &path, false).unwrap();
                    } else {
                        let baseline = db.reserved_memory_bytes();
                        assert!(
                            cancel(&db, &query, &path).is_err(),
                            "completion without spill is not cancellation"
                        );
                        released(&db, baseline, &path).unwrap();
                        let controls = if profile == Profile::Extrema {
                            [
                                (" |> LIMIT 4095", "incomplete grouped result"),
                                (
                                    " |> SELECT region, n, total, smallest + 1 AS smallest, largest",
                                    "incorrect grouped extrema",
                                ),
                                (
                                    " |> SELECT region + 1 AS region, n, total, smallest, largest",
                                    "incorrect grouped row",
                                ),
                            ]
                        } else if profile == Profile::Filtered {
                            [
                                (" |> LIMIT 2047", "incomplete grouped result"),
                                (
                                    " |> SELECT region, n, total, adjusted + 1 AS adjusted",
                                    "incorrect grouped adjusted value",
                                ),
                                (
                                    " |> SELECT region + 1 AS region, n, total, adjusted",
                                    "incorrect grouped row",
                                ),
                            ]
                        } else {
                            [
                                (" |> LIMIT 4095", "incomplete grouped result"),
                                (
                                    " |> SELECT region, n, total, mean + 1 AS mean",
                                    "incorrect grouped mean",
                                ),
                                (
                                    " |> SELECT region + 1 AS region, n, total, mean",
                                    "incorrect grouped row",
                                ),
                            ]
                        };
                        let before = db.reserved_memory_bytes();
                        let extra = execute(
                            &db,
                            &query,
                            profile,
                            &answers[..answers.len() - 1],
                            &path,
                            false,
                        )
                        .err()
                        .expect("extra group rejected");
                        assert_eq!(extra.to_string(), "extra grouped row");
                        released(&db, before, &path).unwrap();
                        for (suffix, message) in controls {
                            let wrong = db.prepare(&format!("{}{suffix}", profile.sql())).unwrap();
                            let before = db.reserved_memory_bytes();
                            let error = execute(&db, &wrong, profile, &answers, &path, false)
                                .err()
                                .expect("wrong result rejected");
                            assert_eq!(error.to_string(), message);
                            released(&db, before, &path).unwrap();
                        }
                    }
                    drop(query);
                    db.close().unwrap();
                }
            }
        }
    }
}
