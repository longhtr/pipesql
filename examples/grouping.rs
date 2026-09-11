//! Compare grouping under different memory budgets using independently known rows.
//! Supply a new absolute database path and a query memory limit in bytes.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, QueryStep, Value,
};
use std::path::Path;

const GROUPS: usize = 4096;
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
    if args.next().is_some() {
        return Err("expected only database path and memory limit".into());
    }
    let config = Config::new(memory, TEMP_BYTES)?;
    let cancel = CancellationToken::new();
    create_sales(Path::new(&path), &cancel)?;

    let db = Database::open(Path::new(&path), config)?;
    let query = db.prepare(QUERY)?;
    let baseline = db.reserved_memory_bytes();
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
                    // Every region appears once with amount 1 and once with 3.
                    // Ordered keys also detect missing, duplicated, or extra groups.
                    if (region, count, total, smallest, largest) != (groups as i64, 2, 4, 1, 3)
                        || groups >= GROUPS
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
    if groups != GROUPS {
        return Err("query finished without all expected groups".into());
    }
    drop(result);
    if db.reserved_memory_bytes() != baseline || db.reserved_temp_bytes() != 0 {
        return Err("query resources remain after dropping the result".into());
    }
    drop(query);
    db.close()?;
    println!("verified {groups} groups: region=0..4095, n=2, total=4, smallest=1, largest=3");
    println!("sampled logical bytes: memory={peak_memory}, temporary={peak_temp}");
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
            nullable: false,
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
