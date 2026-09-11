//! Measure STRING extrema after constructing independently known input.
//! Arguments: a new absolute database path, group count, text bytes, memory bytes.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, QueryStep, Value,
};
use std::path::Path;
use std::time::Instant;

const TEMP_BYTES: u64 = 128_000_000;
const QUERY: &str =
    "FROM words |> AGGREGATE MIN(word) AS lo,MAX(word) AS hi,COUNT(*) AS n GROUP AND ORDER BY k";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("supply a new absolute database path")?;
    let mut number = || -> Result<u64, Box<dyn std::error::Error>> {
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
    if args.next().is_some() || !matches!(groups, 4 | 256) || !matches!(width, 8 | 65_536) {
        return Err(
            "expected path, groups (4 or 256), text bytes (8 or 65536), and memory bytes".into(),
        );
    }
    let config = Config::new(memory, TEMP_BYTES)?;
    // These two caller-owned strings are outside the database's logical counters.
    let low = "a".repeat(width as usize);
    let high = "z".repeat(width as usize);
    let cancel = CancellationToken::new();
    create_words(Path::new(&path), groups, &low, &high, &cancel)?;
    let db = Database::open(Path::new(&path), config)?;
    let query = db.prepare(QUERY)?;
    let baseline = db.reserved_memory_bytes();
    // Exclude input construction, opening, and preparation. Include execution,
    // row validation, successful completion, and destruction of the result owner.
    let start = Instant::now();
    let mut result = db.execute(&query, &cancel)?;
    let mut memory_peak = db.reserved_memory_bytes();
    let mut temp_peak = db.reserved_temp_bytes();
    let mut seen = 0;
    loop {
        match result.step() {
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    match (
                        batch.value(row, 0),
                        batch.value(row, 1),
                        batch.value(row, 2),
                        batch.value(row, 3),
                    ) {
                        (
                            Some(Value::Int64(key)),
                            Some(Value::String(minimum)),
                            Some(Value::String(maximum)),
                            Some(Value::Int64(2)),
                        ) if key == seen as i64
                            && seen < groups
                            && minimum.as_str() == low
                            && maximum.as_str() == high => {}
                        _ => {
                            return Err(
                                "group differs from the independently constructed input".into()
                            );
                        }
                    }
                    seen += 1;
                }
            }
            QueryStep::Progress => (),
            QueryStep::Finished => break,
            QueryStep::Failed(_) => {
                return Err(result.into_error().ok_or("missing query error")?.into());
            }
        }
        memory_peak = memory_peak.max(db.reserved_memory_bytes());
        temp_peak = temp_peak.max(db.reserved_temp_bytes());
    }
    if seen != groups {
        return Err("query finished without all expected groups".into());
    }
    drop(result);
    let elapsed = start.elapsed();
    if db.reserved_memory_bytes() != baseline || db.reserved_temp_bytes() != 0 {
        return Err("query resources remain after dropping the result".into());
    }
    drop(query);
    db.close()?;
    println!("verified groups={groups} text_bytes={width} memory_limit={memory}");
    println!("sampled logical bytes: memory={memory_peak}, temporary={temp_peak}");
    println!(
        "execution and validation seconds={:.6}",
        elapsed.as_secs_f64()
    );
    Ok(())
}

fn create_words(
    path: &Path,
    groups: u64,
    low: &str,
    high: &str,
    cancel: &CancellationToken,
) -> Result<(), pipesql::Error> {
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
            batches: (2 * groups) as u32,
            encoded_bytes: 64_000_000,
        },
        cancel,
    )?;
    // One row per input unit permits maximum-width text without changing the
    // workload's batch shape between widths. Each key occurs with low and high.
    for word in [low, high] {
        for key in (0..groups).rev() {
            append.write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&[key as i64]),
                        validity: &[1],
                    },
                    ColumnInput {
                        values: ColumnValues::String(&[word]),
                        validity: &[1],
                    },
                ],
                cancel,
            )?;
        }
    }
    append.commit(cancel)?;
    db.close()
}
