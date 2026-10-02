//! Measure complete numeric join pairs across duplicate groups and NULL keys.
//!
//! Left identities repeat keys; right identities distinguish one or eight rows
//! per key. Some keys cannot match, and each side has independently nullable
//! values. Both inner and left joins check every typed pair and its multiplicity.
//!
//! A checked warm-up observes written Linux spill. Five samples time execution,
//! answer validation and cursor disposal; setup, preparation, file probes and
//! later cancellation/retry are excluded. Memory samples are reservations.
//! Pass a new absolute database path, memory bytes, 32 or 4096 keys, 1 or 8 right
//! matches, inner or left, and optionally 8192 or 262144 left rows (default 8192).
//! Temporary space is capped at 16 MB or 64 MB respectively; execution is capped
//! at 20 million steps. For example:
//! `cargo run --release --example numeric_join -- /absolute/new-join 2600000 32 8 left`.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, PreparedQuery, QueryStep, Value,
};
use std::{path::Path, time::Instant};

#[path = "support/query.rs"]
mod query_support;
use query_support::{cancel, released, scratch_usage};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const BATCH: usize = 256;

struct Input {
    rows: usize,
    keys: usize,
    matches: usize,
    left: bool,
}

impl Input {
    fn temporary_limit(&self) -> u64 {
        if self.rows == 8192 {
            16_000_000
        } else {
            64_000_000
        }
    }

    fn kind(&self) -> &'static str {
        if self.left { "left" } else { "inner" }
    }

    fn sql(&self) -> String {
        format!(
            "FROM facts AS f |> {}JOIN dimensions AS d ON f.k = d.k |> SELECT f.id AS left_id, d.id AS right_id, f.v AS left_value, d.v AS right_value",
            if self.left { "LEFT " } else { "" }
        )
    }

    // This is the expected relation, independent of the input-writing loops.
    // A NULL left key or a key with only NULL right keys cannot match.
    fn has_matches(&self, left: usize) -> bool {
        !left.is_multiple_of(13) && !(left % self.keys).is_multiple_of(11)
    }

    fn expected_rows(&self) -> usize {
        (0..self.rows)
            .map(|id| {
                if self.has_matches(id) {
                    self.matches
                } else {
                    usize::from(self.left)
                }
            })
            .sum()
    }
}

fn create(path: &Path, input: &Input) -> Result<()> {
    let db = Database::create_empty(path, Config::new(64_000_000, input.temporary_limit())?)?;
    let token = CancellationToken::new();
    for (table, count) in [
        ("facts", input.rows),
        ("dimensions", input.keys * input.matches),
    ] {
        db.declare_table(
            table,
            &["id", "k", "v"].map(|name| ColumnDeclaration {
                name,
                data_type: DataType::Int64,
                nullable: name != "id",
            }),
            &token,
        )?;
        let mut append = db.begin_append(
            table,
            AppendLimits {
                batches: count.div_ceil(BATCH) as u32,
                encoded_bytes: if count <= 32768 {
                    4_000_000
                } else {
                    16_000_000
                },
            },
            &token,
        )?;
        for start in (0..count).step_by(BATCH) {
            let size = BATCH.min(count - start);
            let ids: Vec<_> = (start..start + size)
                .map(|i| {
                    if table == "facts" {
                        (count - 1 - i) as i64
                    } else {
                        i as i64
                    }
                })
                .collect();
            let keys: Vec<_> = ids
                .iter()
                .map(|id| {
                    if table == "facts" {
                        id % input.keys as i64
                    } else {
                        id / input.matches as i64
                    }
                })
                .collect();
            let values: Vec<_> = ids
                .iter()
                .map(|id| {
                    if table == "facts" {
                        id % 101 - 50
                    } else {
                        id % 17 - 8
                    }
                })
                .collect();
            let valid = vec![255; size.div_ceil(8)];
            let mut key_valid = vec![0; size.div_ceil(8)];
            let mut value_valid = vec![0; size.div_ceil(8)];
            for row in 0..size {
                let key_present = if table == "facts" {
                    ids[row] % 13 != 0
                } else {
                    keys[row] % 11 != 0
                };
                let value_present = ids[row] % if table == "facts" { 7 } else { 5 } != 0;
                if key_present {
                    key_valid[row / 8] |= 1 << (row % 8);
                }
                if value_present {
                    value_valid[row / 8] |= 1 << (row % 8);
                }
            }
            append.write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&ids),
                        validity: &valid,
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&keys),
                        validity: &key_valid,
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&values),
                        validity: &value_valid,
                    },
                ],
                &token,
            )?;
        }
        append.commit(&token)?;
    }
    db.close()?;
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
    input: &Input,
    path: &Path,
    observe: bool,
) -> Result<Sample> {
    let columns = [
        ("left_id", false),
        ("right_id", input.left),
        ("left_value", true),
        ("right_value", true),
    ];
    if query.result_column_count() != columns.len()
        || columns.iter().enumerate().any(|(i, (name, nullable))| {
            query.result_column(i).is_none_or(|c| {
                c.name != Some(*name) || c.data_type != DataType::Int64 || c.nullable != *nullable
            })
        })
    {
        return Err("unexpected numeric join schema".into());
    }
    let mut seen = vec![false; input.rows * (input.matches + 1)];
    let expected_rows = input.expected_rows();
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
    let spill_target = if input.rows > 8192 {
        db.config().memory_limit_bytes()
    } else {
        0
    };
    for step in 0_usize..20_000_000 {
        match result.step() {
            QueryStep::Rows(batch) => {
                sample.batches += 1;
                if batch.column_count() != 4 {
                    return Err("unexpected numeric join width".into());
                }
                for row in 0..batch.len() {
                    let Some(Value::Int64(left)) = batch.value(row, 0) else {
                        return Err("missing left identity".into());
                    };
                    let left = usize::try_from(left).map_err(|_| "negative left identity")?;
                    if left >= input.rows {
                        return Err("incorrect numeric join pair".into());
                    }
                    let matched = input.has_matches(left);
                    let (slot, right_value) = match batch.value(row, 1) {
                        Some(Value::Int64(right)) if matched => {
                            let right =
                                usize::try_from(right).map_err(|_| "negative right identity")?;
                            if right / input.matches != left % input.keys {
                                return Err("incorrect numeric join pair".into());
                            }
                            (
                                right % input.matches,
                                if right.is_multiple_of(5) {
                                    Value::Null
                                } else {
                                    Value::Int64(right as i64 % 17 - 8)
                                },
                            )
                        }
                        Some(Value::Null) if input.left && !matched => (input.matches, Value::Null),
                        _ => return Err("incorrect numeric join pair".into()),
                    };
                    let left_value = if left.is_multiple_of(7) {
                        Value::Null
                    } else {
                        Value::Int64(left as i64 % 101 - 50)
                    };
                    if batch.value(row, 2) != Some(left_value)
                        || batch.value(row, 3) != Some(right_value)
                    {
                        return Err("incorrect numeric join values".into());
                    }
                    if std::mem::replace(&mut seen[left * (input.matches + 1) + slot], true) {
                        return Err("duplicate numeric join pair".into());
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
        // File probes are outside timing; sample periodically until the required
        // written volume is observed instead of inspecting every execution step.
        if observe
            && sample.temporary > 0
            && sample.scratch <= spill_target
            && step.is_multiple_of(BATCH)
        {
            sample.scratch = sample.scratch.max(scratch_usage(path)?.1);
        }
    }
    if !finished || sample.rows != expected_rows {
        return Err("incomplete numeric join result".into());
    }
    // Every accepted identity names one expected pair, and no pair can repeat.
    // The exact count therefore proves the whole expected relation was visited.
    drop(result);
    sample.ns = start.elapsed().as_nanos();
    released(db, baseline, path)?;
    if observe && (sample.temporary == 0 || (cfg!(target_os = "linux") && sample.scratch == 0)) {
        return Err("numeric join did not write required spill".into());
    }
    Ok(sample)
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !matches!(args.len(), 5 | 6) {
        return Err(
            "supply path, memory bytes, keys, matches, inner or left and optional left rows".into(),
        );
    }
    let path = Path::new(&args[0]);
    let number = |i: usize| -> Result<usize> {
        Ok(args[i].to_str().ok_or("argument must be UTF-8")?.parse()?)
    };
    let memory = number(1)? as u64;
    let input = Input {
        rows: if args.len() == 6 { number(5)? } else { 8192 },
        keys: number(2)?,
        matches: number(3)?,
        left: match args[4].to_str() {
            Some("inner") => false,
            Some("left") => true,
            _ => return Err("choose inner or left".into()),
        },
    };
    if !matches!(input.rows, 8192 | 262144)
        || !matches!(input.keys, 32 | 4096)
        || !matches!(input.matches, 1 | 8)
    {
        return Err("choose 8192 or 262144 left rows, 32 or 4096 keys and 1 or 8 matches".into());
    }
    let config = Config::new(memory, input.temporary_limit())?;
    create(path, &input)?;
    let db = Database::open(path, config)?;
    let query = db.prepare(&input.sql())?;
    let warmup = execute(&db, &query, &input, path, true)?;
    println!(
        "input kind={} left_rows={} keys={} matches={} memory_limit={memory} batch_rows={BATCH}",
        input.kind(),
        input.rows,
        input.keys,
        input.matches
    );
    println!(
        "warmup scratch_bytes={} file_probe={}",
        warmup.scratch,
        cfg!(target_os = "linux")
    );
    for index in 0..5 {
        let s = execute(&db, &query, &input, path, false)?;
        println!(
            "sample={index} elapsed_ns={} rows={} batches={} progress={} memory={} temporary={}",
            s.ns, s.rows, s.batches, s.progress, s.memory, s.temporary
        );
    }
    cancel(&db, &query, path)?;
    execute(&db, &query, &input, path, false)?;
    drop(query);
    db.close()?;
    println!(
        "verified kind={} keys={} matches={} rows={} memory_limit={memory}",
        input.kind(),
        input.keys,
        input.matches,
        input.expected_rows()
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
    fn complete_pairs_nulls_spill_cancellation_and_wrong_answers() {
        let directory = support::Directory::new();
        for keys in [32, 4096] {
            for matches in [1, 8] {
                let path = directory.0.join(format!("{keys}-{matches}"));
                let mut input = Input {
                    rows: 8192,
                    keys,
                    matches,
                    left: false,
                };
                create(&path, &input).unwrap();
                for left in [false, true] {
                    input.left = left;
                    let expected = match (keys, matches, left) {
                        (32, 1, false) => 6852,
                        (32, 8, false) => 54816,
                        (32, 8, true) => 56156,
                        (4096, 1, false) => 6873,
                        (4096, 8, false) => 54984,
                        (4096, 8, true) => 56303,
                        (_, 1, true) => 8192,
                        _ => unreachable!(),
                    };
                    assert_eq!(input.expected_rows(), expected);
                    for memory in [2_600_000, 12_000_000] {
                        let db = Database::open(
                            &path,
                            Config::new(memory, input.temporary_limit()).unwrap(),
                        )
                        .unwrap();
                        let sql = input.sql();
                        let query = db.prepare(&sql).unwrap();
                        execute(&db, &query, &input, &path, true).unwrap();
                        cancel(&db, &query, &path).unwrap();
                        execute(&db, &query, &input, &path, false).unwrap();
                        if keys == 32 && matches == 8 && memory == 12_000_000 {
                            for (suffix, message) in [
                                (format!(" |> LIMIT {}", expected - 1), "incomplete numeric join result"),
                                (" |> SELECT left_id + 1 AS left_id, right_id, left_value, right_value".into(), "incorrect numeric join pair"),
                                (" |> SELECT left_id, right_id + 8 AS right_id, left_value, right_value".into(), "incorrect numeric join pair"),
                                (" |> SELECT left_id, right_id, left_value + 1 AS left_value, right_value".into(), "incorrect numeric join values"),
                                (" |> SELECT left_id, right_id, CASE WHEN left_value IS NULL THEN 0 ELSE left_value END AS left_value, right_value".into(), "incorrect numeric join values"),
                                (" |> SELECT left_id, right_id, left_value, right_value, 1 AS extra".into(), "unexpected numeric join schema"),
                                (format!(" |> LIMIT {} |> UNION ALL ({sql} |> LIMIT 1)", expected - 1), "duplicate numeric join pair"),
                                (format!(" |> UNION ALL ({sql} |> LIMIT 1)"), "duplicate numeric join pair"),
                                (" |> SELECT left_id, right_id, left_value, CASE WHEN right_value IS NULL THEN 0 ELSE right_value END AS right_value".into(), "incorrect numeric join values"),
                            ] {
                                let wrong = db.prepare(&format!("{sql}{suffix}")).unwrap();
                                let baseline = db.reserved_memory_bytes();
                                let error = execute(&db, &wrong, &input, &path, false).err().expect("wrong join accepted");
                                assert_eq!(error.to_string(), message, "{suffix}");
                                released(&db, baseline, &path).unwrap();
                            }
                        }
                        drop(query);
                        db.close().unwrap();
                    }
                }
            }
        }
    }
}
