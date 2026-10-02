//! Compare BYTE_LENGTH and CHAR_LENGTH on the same persisted text.
//!
//! UTF-8 uses different numbers of bytes for different characters. For example,
//! é uses two bytes, while e followed by a combining accent contains two Unicode
//! scalars and uses three bytes. The repeated input pattern keeps byte lengths
//! equal while varying scalar counts; it also distinguishes empty text from NULL.
//!
//! Both queries total their lengths and check literal answers on every execution.
//! After warm-up, ten sample pairs alternate which query runs first. Each printed
//! time averages 50 executions and includes result checks and disposal. Setup and
//! preparation are timed separately; memory samples count additional reservations,
//! not process memory. Run `cargo run --release --offline --locked --example
//! text_cost -- /absolute/new-texts`. The caller owns the remaining database.
//! Compare all ten pairs; these are warm repeated queries without cache eviction.
//! Text validation is shared work even when only byte length is requested.
//!
//! Reading a stored STRING validates its checksum, layout, offsets and UTF-8
//! before either measurement. BYTE_LENGTH reads the resulting string's length;
//! CHAR_LENGTH visits its scalars. Constant-time byte measurement therefore does
//! not make the complete stored-text query constant-time in input size. These
//! timings include validation and aggregation; they cannot isolate either kernel.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, PreparedQuery, QueryStep, Value,
};
use std::path::Path;
use std::time::{Duration, Instant};

const ROWS: usize = 4_096;
const BATCH_ROWS: usize = 256;
const SAMPLES: usize = 10;
const EXECUTIONS_PER_SAMPLE: u32 = 50;
const WARMUPS: usize = 10;
const MEMORY_BYTES: u64 = 8_000_000;
const TEMP_BYTES: u64 = 8_000_000;
const BYTE_QUERY: &str = "FROM texts
|> SELECT BYTE_LENGTH(t) AS width
|> AGGREGATE COUNT(*) AS entries, COUNT(width) AS present, SUM(width) AS total";
const SCALAR_QUERY: &str = "FROM texts
|> SELECT CHAR_LENGTH(t) AS width
|> AGGREGATE COUNT(*) AS entries, COUNT(width) AS present, SUM(width) AS total";
// Each eight-row cycle has six 128-byte strings, empty text and NULL.
// The six strings contain 128 + 64 + 44 + 86 + 32 + 128 = 482 scalars.
// There are 512 cycles: 512*768 bytes and 512*482 scalars in total.
const EXPECTED_TOTALS: [i64; 2] = [393_216, 246_784];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("supply a fresh absolute database path")?;
    let path = Path::new(&path);
    if args.next().is_some() || !path.is_absolute() || path.try_exists()? {
        return Err("expected one fresh absolute database path".into());
    }
    let cancel = CancellationToken::new();
    let start = Instant::now();
    create_texts(path, &cancel)?;
    let db = Database::open(path, Config::new(MEMORY_BYTES, TEMP_BYTES)?)?;
    let setup = start.elapsed();
    let start = Instant::now();
    let queries = [db.prepare(BYTE_QUERY)?, db.prepare(SCALAR_QUERY)?];
    let preparation = start.elapsed();
    for query in &queries {
        if query.result_column_count() != 3 {
            return Err("expected three aggregate columns".into());
        }
        for (index, nullable) in [false, false, true].into_iter().enumerate() {
            let column = query
                .result_column(index)
                .ok_or("missing aggregate column")?;
            if column.data_type != DataType::Int64 || column.nullable != nullable {
                return Err("unexpected aggregate schema".into());
            }
        }
    }

    // Retain both queries so their preparation reservations are identical for
    // either execution. Warm both paths before recording measurements.
    for _ in 0..WARMUPS {
        for (variant, query) in queries.iter().enumerate() {
            execute_checked(&db, query, &cancel, EXPECTED_TOTALS[variant])?;
        }
    }
    let mut times = [[Duration::ZERO; 2]; SAMPLES];
    let mut peak_memory = [0; 2];
    for (sample, pair) in times.iter_mut().enumerate() {
        // Alternate the first query to reduce systematic bias from run order.
        for variant in [sample % 2, 1 - sample % 2] {
            for _ in 0..EXECUTIONS_PER_SAMPLE {
                let (elapsed, memory) =
                    execute_checked(&db, &queries[variant], &cancel, EXPECTED_TOTALS[variant])?;
                pair[variant] += elapsed;
                peak_memory[variant] = peak_memory[variant].max(memory);
            }
        }
    }
    drop(queries);
    db.close()?;

    println!(
        "verified rows=4096 present=3584 byte_total=393216 scalar_total=246784 samples={SAMPLES} executions_per_sample={EXECUTIONS_PER_SAMPLE} warmups={WARMUPS}"
    );
    println!("memory_limit={MEMORY_BYTES} temporary_limit={TEMP_BYTES}");
    println!(
        "setup and open seconds={:.6}, preparation seconds={:.6}",
        setup.as_secs_f64(),
        preparation.as_secs_f64()
    );
    for (sample, pair) in times.iter().enumerate() {
        println!(
            "sample={} bytes_seconds={:.6} scalars_seconds={:.6}",
            sample + 1,
            (pair[0] / EXECUTIONS_PER_SAMPLE).as_secs_f64(),
            (pair[1] / EXECUTIONS_PER_SAMPLE).as_secs_f64()
        );
    }
    for (variant, name) in ["bytes", "scalars"].iter().enumerate() {
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
    expected_total: i64,
) -> Result<(Duration, u64), Box<dyn std::error::Error>> {
    let baseline = db.reserved_memory_bytes();
    let start = Instant::now();
    let mut result = db.execute(query, cancel)?;
    let mut peak = db.reserved_memory_bytes();
    let mut seen = false;
    let mut finished = false;
    for _ in 0..100_000 {
        if db.reserved_temp_bytes() != 0 {
            return Err("unexpected temporary reservation in text workload".into());
        }
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                if seen
                    || batch.len() != 1
                    || batch.column_count() != 3
                    || batch.value(0, 0) != Some(Value::Int64(4_096))
                    || batch.value(0, 1) != Some(Value::Int64(3_584))
                    || batch.value(0, 2) != Some(Value::Int64(expected_total))
                {
                    return Err("text result differs from the literal oracle".into());
                }
                seen = true;
            }
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(_) => {
                return Err(result.into_error().ok_or("missing query error")?.into());
            }
        }
        peak = peak.max(db.reserved_memory_bytes());
    }
    if !seen || !finished {
        return Err("text query did not finish with its aggregate row".into());
    }
    drop(result);
    let elapsed = start.elapsed();
    if db.reserved_memory_bytes() != baseline || db.reserved_temp_bytes() != 0 {
        return Err("query resources remain after result destruction".into());
    }
    Ok((elapsed, peak - baseline))
}

fn create_texts(path: &Path, cancel: &CancellationToken) -> Result<(), pipesql::Error> {
    let db = Database::create_empty(path, Config::new(MEMORY_BYTES, TEMP_BYTES)?)?;
    db.declare_table(
        "texts",
        &[ColumnDeclaration {
            name: "t",
            data_type: DataType::String,
            nullable: true,
        }],
        cancel,
    )?;
    let texts = [
        "a".repeat(128),
        "é".repeat(64),
        "雪".repeat(42) + "ab",
        "e\u{301}".repeat(42) + "ab",
        "😀".repeat(32),
        "\0".repeat(128),
        String::new(),
        String::new(),
    ];
    // Both final strings are empty, but the bitmap marks only the last as NULL.
    // COUNT(width) must therefore include seven of every eight rows.
    let values: [&str; BATCH_ROWS] = std::array::from_fn(|row| texts[row % 8].as_str());
    let mut append = db.begin_append(
        "texts",
        AppendLimits {
            batches: (ROWS / BATCH_ROWS) as u32,
            encoded_bytes: 500_000,
        },
        cancel,
    )?;
    for _ in 0..ROWS / BATCH_ROWS {
        append.write(
            &[ColumnInput {
                values: ColumnValues::String(&values),
                validity: &[127; BATCH_ROWS / 8],
            }],
            cancel,
        )?;
    }
    append.commit(cancel)?;
    db.close()
}
