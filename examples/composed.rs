//! Measure a self-join followed by grouping and descending order.
//!
//! Each region has two input rows. Joining the table to itself produces four
//! pairs per region, so each original amount contributes twice to the sum.
//! Missing amounts create three cases: an all-NULL group, one present amount,
//! or two present amounts. The program checks every group against these answers.
//!
//! One checked warm-up precedes five samples. Each sample includes execution,
//! complete typed answer checks and result disposal. Preparation, file probes,
//! cancellation and its checked retry are outside the timed samples. Memory
//! samples are engine reservations, not process memory.
//!
//! Pass a new absolute database path and query memory limit in bytes. Run
//! `cargo run --release --offline --locked --example composed -- /absolute/new-sales
//! 2200000`. Compare with 12000000 on a fresh path. Both can spill because the
//! join and ORDER BY sort their input; totals cannot attribute spill to one
//! operator. Linux checks written scratch files and closed descriptors.

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

const GROUPS: usize = 4096;
const BATCH_ROWS: usize = 256;
const TEMP_BYTES: u64 = 8_000_000;
const QUERY: &str = "FROM sales AS s |> JOIN sales AS copies ON s.region = copies.region \
     |> AGGREGATE COUNT(*) AS n, COUNT(s.amount) AS present, SUM(s.amount) AS total \
     GROUP BY s.region |> ORDER BY region DESC";

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("supply a new absolute database path")?;
    let memory: u64 = args
        .next()
        .ok_or("supply a query memory limit in bytes")?
        .to_str()
        .ok_or("memory limit must be UTF-8")?
        .parse()?;
    if args.next().is_some() {
        return Err("expected only database path and memory limit".into());
    }
    let config = Config::new(memory, TEMP_BYTES)?;
    create_sales(Path::new(&path), &CancellationToken::new())?;

    let db = Database::open(Path::new(&path), config)?;
    let query = db.prepare(QUERY)?;
    let path = Path::new(&path);
    let warmup = execute(&db, &query, path, true)?;
    println!("input groups={GROUPS} rows=8192 memory_limit={memory} batch_rows={BATCH_ROWS}");
    println!(
        "warmup scratch_bytes={} file_probe={}",
        warmup.scratch,
        cfg!(target_os = "linux")
    );
    for index in 0..5 {
        let sample = execute(&db, &query, path, false)?;
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
    cancel(&db, &query, path)?;
    execute(&db, &query, path, false)?;
    drop(query);
    db.close()?;
    println!(
        "verified {GROUPS} descending groups: four joined pairs per key, nullable counts and sums"
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

fn execute(db: &Database, query: &PreparedQuery<'_>, path: &Path, observe: bool) -> Result<Sample> {
    let columns = [
        ("region", false),
        ("n", false),
        ("present", false),
        ("total", true),
    ];
    if query.result_column_count() != columns.len()
        || columns.iter().enumerate().any(|(i, (name, nullable))| {
            query.result_column(i).is_none_or(|column| {
                column.name != Some(*name)
                    || column.data_type != DataType::Int64
                    || column.nullable != *nullable
            })
        })
    {
        return Err("unexpected composed schema".into());
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
                if batch.column_count() != 4 {
                    return Err("unexpected composed width".into());
                }
                for row in 0..batch.len() {
                    let (
                        Some(Value::Int64(region)),
                        Some(Value::Int64(count)),
                        Some(Value::Int64(present)),
                        total,
                    ) = (
                        batch.value(row, 0),
                        batch.value(row, 1),
                        batch.value(row, 2),
                        batch.value(row, 3),
                    )
                    else {
                        return Err("unexpected result schema or value".into());
                    };
                    if sample.rows >= GROUPS || region != (GROUPS - 1 - sample.rows) as i64 {
                        return Err("missing, duplicated, extra, or unordered group".into());
                    }
                    // The join repeats each left amount twice. Region remainder 0
                    // has no amounts, remainder 1 has only 3, and the others
                    // have both 1 and 3: sums NULL, 6 and 8 respectively.
                    let (expected_present, expected_total) = match region % 4 {
                        0 => (0, None),
                        1 => (2, Some(6)),
                        _ => (4, Some(8)),
                    };
                    let total_matches = match (total, expected_total) {
                        (Some(Value::Null), None) => true,
                        (Some(Value::Int64(actual)), Some(expected)) => actual == expected,
                        _ => false,
                    };
                    if count != 4 || present != expected_present || !total_matches {
                        return Err("joined aggregate differs from input construction".into());
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
    if !finished || sample.rows != GROUPS {
        return Err("incomplete composed result".into());
    }
    drop(result);
    sample.ns = start.elapsed().as_nanos();
    released(db, baseline, path)?;
    if observe && (sample.temporary == 0 || (cfg!(target_os = "linux") && sample.scratch == 0)) {
        return Err("composed query did not write required spill".into());
    }
    Ok(sample)
}

fn create_sales(
    path: &Path,
    cancel: &CancellationToken,
) -> std::result::Result<(), pipesql::Error> {
    // Create the input with a fixed budget. Changing the command-line query
    // budget does not change the allowance used to build the input.
    let db = Database::create_empty(path, Config::new(32_000_000, TEMP_BYTES)?)?;
    db.declare_table(
        "sales",
        &["region", "amount"].map(|name| ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: name == "amount",
        }),
        cancel,
    )?;
    let mut append = db.begin_append(
        "sales",
        AppendLimits {
            batches: (2 * GROUPS / BATCH_ROWS) as u32,
            encoded_bytes: 1_000_000,
        },
        cancel,
    )?;
    for amount in [1, 3] {
        for start in (0..GROUPS).step_by(BATCH_ROWS) {
            let regions: [i64; BATCH_ROWS] =
                std::array::from_fn(|row| (GROUPS - 1 - start - row) as i64);
            let mut validity = [0_u8; BATCH_ROWS / 8];
            for (row, region) in regions.iter().enumerate() {
                if region % 4 >= 2 || (region % 4 == 1 && amount == 3) {
                    validity[row / 8] |= 1 << (row % 8);
                }
            }
            append.write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&regions),
                        validity: &[255; BATCH_ROWS / 8],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&[amount; BATCH_ROWS]),
                        validity: &validity,
                    },
                ],
                cancel,
            )?;
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
    fn complete_answers_spill_cancellation_and_wrong_results() {
        let directory = support::Directory::new();
        let path = directory.0.join("sales");
        create_sales(&path, &CancellationToken::new()).unwrap();
        for memory in [2_200_000, 12_000_000] {
            let db = Database::open(&path, Config::new(memory, TEMP_BYTES).unwrap()).unwrap();
            let query = db.prepare(QUERY).unwrap();
            execute(&db, &query, &path, true).unwrap();
            cancel(&db, &query, &path).unwrap();
            execute(&db, &query, &path, false).unwrap();
            for (suffix, expected) in [
                (" |> LIMIT 4095", "incomplete composed result"),
                (
                    " |> SELECT region, n, present, total + 1 AS total",
                    "joined aggregate differs from input construction",
                ),
                (
                    " |> SELECT region + 1 AS region, n, present, total",
                    "missing, duplicated, extra, or unordered group",
                ),
                (
                    " |> SELECT region, n, present, total, 1 AS extra",
                    "unexpected composed schema",
                ),
            ] {
                let wrong = db.prepare(&format!("{QUERY}{suffix}")).unwrap();
                let baseline = db.reserved_memory_bytes();
                let error = execute(&db, &wrong, &path, false)
                    .err()
                    .expect("wrong result accepted");
                assert_eq!(error.to_string(), expected);
                released(&db, baseline, &path).unwrap();
            }
            if memory == 12_000_000 {
                let extra = db
                    .prepare(&format!("{QUERY} |> UNION ALL ({QUERY} |> LIMIT 1)"))
                    .unwrap();
                let baseline = db.reserved_memory_bytes();
                let error = execute(&db, &extra, &path, false)
                    .err()
                    .expect("extra row accepted");
                assert_eq!(
                    error.to_string(),
                    "missing, duplicated, extra, or unordered group"
                );
                released(&db, baseline, &path).unwrap();
            }
            drop(query);
            db.close().unwrap();
        }
    }
}
