//! Observe a self-join, nullable aggregation and final ordering under a query budget.
//!
//! Two rows per key join to four pairs. Their known NULL patterns determine the
//! counts and sums independently of execution. Check every descending group, then
//! cancel a second execution after temporary storage appears and require release.
//! Setup uses its own budget. Timings include execution, validation and result
//! destruction; sampled reservations are not process memory measurements.
//! Supply a new absolute database path and query memory bytes; see docs/getting-started.md.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, QueryStep, Value,
};
use std::path::Path;
use std::time::Instant;

const GROUPS: usize = 4096;
const BATCH_ROWS: usize = 256;
const TEMP_BYTES: u64 = 8_000_000;
const QUERY: &str = "FROM sales AS s |> JOIN sales AS copies ON s.region = copies.region \
     |> AGGREGATE COUNT(*) AS n, COUNT(s.amount) AS present, SUM(s.amount) AS total \
     GROUP BY s.region |> ORDER BY region DESC";

fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    let cancel = CancellationToken::new();
    create_sales(Path::new(&path), &cancel)?;

    let db = Database::open(Path::new(&path), config)?;
    let query = db.prepare(QUERY)?;
    let baseline = db.reserved_memory_bytes();
    // Include admission, complete row validation, and result destruction. Setup,
    // preparation, the second cancellation exercise, and close stay outside.
    let start = Instant::now();
    let mut result = db.execute(&query, &cancel)?;
    let mut progress_steps = 0_u64;
    let mut row_steps = 0_u64;
    let mut peak_memory = db.reserved_memory_bytes();
    let mut peak_temp = db.reserved_temp_bytes();
    let mut groups = 0;
    loop {
        match result.step() {
            QueryStep::Rows(batch) => {
                row_steps += 1;
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
                    if groups >= GROUPS || region != (GROUPS - 1 - groups) as i64 {
                        return Err("missing, duplicated, extra, or unordered group".into());
                    }
                    // Two rows match two copies, so each amount occurs twice.
                    // Key classes contain no amounts, only 3, or both 1 and 3.
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
                    groups += 1;
                }
            }
            QueryStep::Progress => progress_steps += 1,
            QueryStep::Finished => break,
            QueryStep::Failed(_) => {
                return Err(result.into_error().ok_or("missing query error")?.into());
            }
        }
        peak_memory = peak_memory.max(db.reserved_memory_bytes());
        peak_temp = peak_temp.max(db.reserved_temp_bytes());
    }
    if groups != GROUPS {
        return Err("query finished without all expected groups".into());
    }
    drop(result);
    let elapsed = start.elapsed();
    if db.reserved_memory_bytes() != baseline || db.reserved_temp_bytes() != 0 {
        return Err("query resources remain after dropping the result".into());
    }
    // Cancellation has the same cleanup obligation as successful completion.
    let mut cancelled = db.execute(&query, &cancel)?;
    for _ in 0..100_000 {
        if !matches!(cancelled.step(), QueryStep::Progress) {
            return Err("expected intermediate work before joined output".into());
        }
        if db.reserved_temp_bytes() > 0 {
            break;
        }
    }
    if db.reserved_temp_bytes() == 0 {
        return Err("query did not reach temporary storage before cancellation".into());
    }
    cancel.cancel();
    if !matches!(
        cancelled.step(),
        QueryStep::Failed(pipesql::Error::Cancelled)
    ) {
        return Err("cancelled query did not report cancellation".into());
    }
    drop(cancelled);
    if db.reserved_memory_bytes() != baseline || db.reserved_temp_bytes() != 0 {
        return Err("cancelled query resources remain after dropping the result".into());
    }
    drop(query);
    db.close()?;
    println!(
        "verified {groups} descending groups: four joined pairs per key, nullable counts and sums"
    );
    println!("sampled logical bytes: memory={peak_memory}, temporary={peak_temp}");
    println!("successful query steps: progress={progress_steps}, rows={row_steps}, finished=1");
    println!(
        "execution and validation seconds={:.6}",
        elapsed.as_secs_f64()
    );
    Ok(())
}

fn create_sales(path: &Path, cancel: &CancellationToken) -> Result<(), pipesql::Error> {
    // Build with a separate budget so the experiment measures query admission.
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
