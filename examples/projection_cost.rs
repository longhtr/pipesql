//! Compare the cost of six projection stages with one equivalent expression.
//!
//! Both queries add one six times to each amount and return the same count and
//! sum. One names every intermediate value in a separate SELECT; the other puts
//! all six additions in one SELECT. This isolates the effect of expressing the
//! work as several stages while keeping the input and arithmetic the same.
//!
//! Both prepared queries remain live. Warm-up executions precede ten sample
//! pairs, with the first query alternating between pairs. Each reported time is
//! the mean of 100 executions, including answer checks and result disposal.
//! Setup and preparation are reported separately. Additional memory samples
//! exclude the shared baseline and measure reservations, not process memory.
//! Run `cargo run --release --offline --locked --example projection_cost --
//! /absolute/new-sales`. The caller owns the remaining database. These are warm
//! repeated queries without cache eviction; compare all ten pairs. Computed
//! projection evaluation lives in `src/execution/computed.rs`.
//!
//! Pipeline fusion retains separate scalar programs and intermediate buffers.
//! The staged query needs six computed buffers; the single expression needs one.
//! Each holds 256 eight-byte values and four eight-byte validity words, so the
//! difference is 10,400 logical bytes, separate from other query owners. Six
//! program evaluations and result copies become one; shared scan and aggregate
//! work can outweigh that saving. Both forms preserve addition order on small
//! integers. This comparison does not justify substituting expressions across
//! filters or conditional branches, where demand and failures could change.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, PreparedQuery, QueryStep, Value,
};
use std::path::Path;
use std::time::{Duration, Instant};

const ROWS: usize = 32_768;
const BATCH_ROWS: usize = 256;
const SAMPLES: usize = 10;
const EXECUTIONS_PER_SAMPLE: u32 = 100;
const WARMUPS: usize = 20;
const MEMORY_BYTES: u64 = 8_000_000;
const TEMP_BYTES: u64 = 8_000_000;
const STAGED: &str = "FROM sales
|> SELECT amount + 1 AS a
|> SELECT a + 1 AS b
|> SELECT b + 1 AS c
|> SELECT c + 1 AS d
|> SELECT d + 1 AS e
|> SELECT e + 1 AS adjusted
|> AGGREGATE COUNT(*) AS n, SUM(adjusted) AS total";
const SINGLE: &str = "FROM sales
|> SELECT amount + 1 + 1 + 1 + 1 + 1 + 1 AS adjusted
|> AGGREGATE COUNT(*) AS n, SUM(adjusted) AS total";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("supply a fresh absolute database path")?;
    let path = Path::new(&path);
    if args.next().is_some() || !path.is_absolute() || path.try_exists()? {
        return Err("expected one fresh absolute database path".into());
    }
    let cancel = CancellationToken::new();
    let start = Instant::now();
    create_sales(path, &cancel)?;
    let db = Database::open(path, Config::new(MEMORY_BYTES, TEMP_BYTES)?)?;
    let setup = start.elapsed();
    let start = Instant::now();
    let queries = [db.prepare(STAGED)?, db.prepare(SINGLE)?];
    let preparation = start.elapsed();

    // Keep both prepared-query reservations in the baseline for every run.
    // Warm both paths before sampling so their first executions are not compared
    // with already-warmed executions.
    for _ in 0..WARMUPS {
        for query in &queries {
            execute_checked(&db, query, &cancel)?;
        }
    }
    let mut times = [[Duration::ZERO; 2]; SAMPLES];
    let mut peak_memory = [0; 2];
    for (sample, pair) in times.iter_mut().enumerate() {
        for variant in [sample % 2, 1 - sample % 2] {
            for _ in 0..EXECUTIONS_PER_SAMPLE {
                let (elapsed, memory) = execute_checked(&db, &queries[variant], &cancel)?;
                pair[variant] += elapsed;
                peak_memory[variant] = peak_memory[variant].max(memory);
            }
        }
    }
    drop(queries);
    db.close()?;

    println!(
        "verified rows=32768 total=1817536 samples={SAMPLES} executions_per_sample={EXECUTIONS_PER_SAMPLE} warmups={WARMUPS}"
    );
    println!("memory_limit={MEMORY_BYTES} temporary_limit={TEMP_BYTES}");
    println!(
        "setup and open seconds={:.6}, preparation seconds={:.6}",
        setup.as_secs_f64(),
        preparation.as_secs_f64()
    );
    for (sample, pair) in times.iter().enumerate() {
        println!(
            "sample={} staged_seconds={:.6} single_seconds={:.6}",
            sample + 1,
            (pair[0] / EXECUTIONS_PER_SAMPLE).as_secs_f64(),
            (pair[1] / EXECUTIONS_PER_SAMPLE).as_secs_f64()
        );
    }
    for (variant, name) in ["staged", "single"].iter().enumerate() {
        println!(
            "{name} sampled additional logical memory={} temporary=0",
            peak_memory[variant]
        );
    }
    Ok(())
}

/// Time one complete execution and its answer checks, including result disposal.
/// Return the elapsed time and sampled peak reservation above the starting baseline.
fn execute_checked(
    db: &Database,
    query: &PreparedQuery<'_>,
    cancel: &CancellationToken,
) -> Result<(Duration, u64), Box<dyn std::error::Error>> {
    let baseline = db.reserved_memory_bytes();
    let start = Instant::now();
    let mut result = db.execute(query, cancel)?;
    let mut peak = db.reserved_memory_bytes();
    let mut seen = false;
    loop {
        if db.reserved_temp_bytes() != 0 {
            return Err("unexpected temporary reservation in projection workload".into());
        }
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                // The input sums to 327*(0+...+99) + (0+...+67) = 1,620,928.
                // Adding six to each of 32,768 rows gives a total of 1,817,536.
                if seen
                    || batch.len() != 1
                    || batch.column_count() != 2
                    || batch.value(0, 0) != Some(Value::Int64(32_768))
                    || batch.value(0, 1) != Some(Value::Int64(1_817_536))
                {
                    return Err("projection result differs from the literal oracle".into());
                }
                seen = true;
            }
            QueryStep::Finished => break,
            QueryStep::Failed(_) => {
                return Err(result.into_error().ok_or("missing query error")?.into());
            }
        }
        peak = peak.max(db.reserved_memory_bytes());
    }
    if !seen {
        return Err("query finished without its aggregate row".into());
    }
    drop(result);
    let elapsed = start.elapsed();
    if db.reserved_memory_bytes() != baseline || db.reserved_temp_bytes() != 0 {
        return Err("query resources remain after result destruction".into());
    }
    Ok((elapsed, peak - baseline))
}

fn create_sales(path: &Path, cancel: &CancellationToken) -> Result<(), pipesql::Error> {
    let db = Database::create_empty(path, Config::new(MEMORY_BYTES, TEMP_BYTES)?)?;
    db.declare_table(
        "sales",
        &[ColumnDeclaration {
            name: "amount",
            data_type: DataType::Int64,
            nullable: false,
        }],
        cancel,
    )?;
    let mut append = db.begin_append(
        "sales",
        AppendLimits {
            batches: (ROWS / BATCH_ROWS) as u32,
            encoded_bytes: 1_000_000,
        },
        cancel,
    )?;
    for start in (0..ROWS).step_by(BATCH_ROWS) {
        let amounts: [i64; BATCH_ROWS] = std::array::from_fn(|row| ((start + row) % 100) as i64);
        append.write(
            &[ColumnInput {
                values: ColumnValues::Int64(&amounts),
                validity: &[255; BATCH_ROWS / 8],
            }],
            cancel,
        )?;
    }
    append.commit(cancel)?;
    db.close()
}
