//! Compare grouping under different memory budgets using independently known rows.
//! Arguments: new absolute database path, memory bytes, optional groups (32 or
//! 4096), and optional distribution (even or skewed). Defaults retain 4096/even.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, QueryStep, Value,
};
use std::path::Path;
use std::time::Instant;

const ROWS_PER_PASS: usize = 4096;
const BATCH_ROWS: usize = 256;
const TEMP_BYTES: u64 = 8_000_000;
const QUERY: &str = "FROM sales |> AGGREGATE COUNT(*) AS n,SUM(amount) AS total,MIN(amount) AS smallest,MAX(amount) AS largest GROUP AND ORDER BY region";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("supply a new absolute database path")?;
    let memory: u64 = args
        .next()
        .ok_or("supply a query memory limit in bytes")?
        .to_str()
        .ok_or("memory limit must be UTF-8")?
        .parse()?;
    let group_count: usize = match args.next() {
        Some(value) => value.to_str().ok_or("group count must be UTF-8")?.parse()?,
        None => 4096,
    };
    let skewed = match args.next() {
        None => false,
        Some(value) => match value.to_str() {
            Some("even") => false,
            Some("skewed") => true,
            _ => return Err("distribution must be even or skewed".into()),
        },
    };
    if args.next().is_some() || !matches!(group_count, 32 | 4096) {
        return Err("expected path, memory bytes, groups (32 or 4096), and even or skewed".into());
    }
    let config = Config::new(memory, TEMP_BYTES)?;
    let cancel = CancellationToken::new();
    create_sales(Path::new(&path), group_count, skewed, &cancel)?;

    let db = Database::open(Path::new(&path), config)?;
    let query = db.prepare(QUERY)?;
    let baseline = db.reserved_memory_bytes();
    // Match the STRING example: exclude construction/open/prepare, but include
    // execution, independent row validation, completion, and result destruction.
    let start = Instant::now();
    let mut result = db.execute(&query, &cancel)?;
    let mut peak_memory = db.reserved_memory_bytes();
    let mut peak_temp = db.reserved_temp_bytes();
    let mut groups = 0;
    loop {
        match result.step() {
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let (
                        Some(Value::Int64(region)),
                        Some(Value::Int64(count)),
                        Some(Value::Int64(total)),
                        Some(Value::Int64(smallest)),
                        Some(Value::Int64(largest)),
                    ) = (
                        batch.value(row, 0),
                        batch.value(row, 1),
                        batch.value(row, 2),
                        batch.value(row, 3),
                        batch.value(row, 4),
                    )
                    else {
                        return Err("unexpected result schema or value".into());
                    };
                    // The first pass contributes amount 1 evenly. The second
                    // contributes amount 3 evenly, or entirely to region zero.
                    // Derive each complete expected row from these distributions.
                    let first = (ROWS_PER_PASS / group_count) as i64;
                    let second = if skewed {
                        if groups == 0 { ROWS_PER_PASS as i64 } else { 0 }
                    } else {
                        first
                    };
                    let expected = (
                        groups as i64,
                        first + second,
                        first + 3 * second,
                        1,
                        if second == 0 { 1 } else { 3 },
                    );
                    if (region, count, total, smallest, largest) != expected
                        || groups >= group_count
                    {
                        return Err("group result differs from the input construction".into());
                    }
                    groups += 1;
                }
            }
            QueryStep::Progress => (),
            QueryStep::Finished => break,
            QueryStep::Failed(_) => {
                return Err(result.into_error().ok_or("missing query error")?.into());
            }
        }
        peak_memory = peak_memory.max(db.reserved_memory_bytes());
        peak_temp = peak_temp.max(db.reserved_temp_bytes());
    }
    if groups != group_count {
        return Err("query finished without all expected groups".into());
    }
    drop(result);
    let elapsed = start.elapsed();
    if db.reserved_memory_bytes() != baseline || db.reserved_temp_bytes() != 0 {
        return Err("query resources remain after dropping the result".into());
    }
    drop(query);
    db.close()?;
    println!("verified groups={groups} rows=8192 skewed={skewed} memory_limit={memory}");
    println!("sampled logical bytes: memory={peak_memory}, temporary={peak_temp}");
    println!(
        "execution and validation seconds={:.6}",
        elapsed.as_secs_f64()
    );
    Ok(())
}

fn create_sales(
    path: &Path,
    groups: usize,
    skewed: bool,
    cancel: &CancellationToken,
) -> Result<(), pipesql::Error> {
    // Build with a separate budget so the experiment measures query admission.
    let db = Database::create_empty(path, Config::new(32_000_000, TEMP_BYTES)?)?;
    db.declare_table(
        "sales",
        &["region", "amount"].map(|name| ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: false,
        }),
        cancel,
    )?;
    let mut append = db.begin_append(
        "sales",
        AppendLimits {
            batches: (2 * ROWS_PER_PASS / BATCH_ROWS) as u32,
            encoded_bytes: 1_000_000,
        },
        cancel,
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
            append.write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&regions),
                        validity: &[255; BATCH_ROWS / 8],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&[amount; BATCH_ROWS]),
                        validity: &[255; BATCH_ROWS / 8],
                    },
                ],
                cancel,
            )?;
        }
    }
    append.commit(cancel)?;
    db.close()
}
