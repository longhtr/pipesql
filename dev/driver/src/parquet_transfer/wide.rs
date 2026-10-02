//! Exercise wide output without using the engine's reader as its value oracle.
//!
//! Rows and source columns vary independently. The projection permutes columns;
//! PyArrow checks every value against separately authored expectations. Limits
//! fail after a written group, then the same prepared query must export again.

use super::{Result, descriptors};
use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, DateValue, Error, ParquetExportLimits, PreparedQuery,
};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;

const ROWS: usize = 513;
const COLUMNS: usize = 64;

struct ShortWriter {
    file: File,
    bytes: usize,
    flushes: usize,
    writes: usize,
    fragment: usize,
}

impl ShortWriter {
    fn new(path: &Path) -> io::Result<Self> {
        Ok(Self {
            file: File::create_new(path)?,
            bytes: 0,
            flushes: 0,
            writes: 0,
            fragment: 127,
        })
    }
}

impl Write for ShortWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = self.file.write(&bytes[..bytes.len().min(self.fragment)])?;
        self.bytes += count;
        self.writes += 1;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        self.file.flush()
    }
}

fn integer(row: usize, column: usize) -> i64 {
    match row % 7 {
        0 => i64::MIN + column as i64,
        1 => i64::MAX - column as i64,
        _ => (row * 1000 + column) as i64,
    }
}

fn number(row: usize, column: usize) -> f64 {
    let bits = match row % 8 {
        0 => 0,
        1 => 0x8000_0000_0000_0000,
        2 => 0x7ff0_0000_0000_0000,
        3 => 0xfff0_0000_0000_0000,
        4 => 0x7ff8_0000_0000_0100 | column as u64,
        5 => 1 + column as u64,
        6 => ((row * 64 + column) as f64).to_bits(),
        _ => (-((row * 64 + column) as f64)).to_bits(),
    };
    f64::from_bits(bits)
}

#[derive(Clone, Copy)]
struct Measurement {
    group_rows: u32,
    fragment: usize,
}

pub(super) fn run(root: &Path) -> Result<()> {
    produce(root, ROWS, None)
}

pub(super) fn measure(root: &Path, rows: usize, group_rows: u32, fragment: usize) -> Result<()> {
    if !matches!(rows, 513 | 8192)
        || !matches!(group_rows, 257 | 1024)
        || !matches!(fragment, 127 | 4096)
    {
        return Err("unsupported wide export measurement".into());
    }
    produce(
        root,
        rows,
        Some(Measurement {
            group_rows,
            fragment,
        }),
    )
}

fn produce(root: &Path, rows: usize, measurement: Option<Measurement>) -> Result<()> {
    if !root.is_absolute() {
        return Err("wide output requires a new absolute directory".into());
    }
    fs::create_dir(root)?;
    let database =
        Database::create_empty(&root.join("database"), Config::new(64_000_000, 8_000_000)?)?;
    let cancel = CancellationToken::new();
    let names: Vec<_> = (0..COLUMNS).map(|column| format!("c{column:02}")).collect();
    let schema: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(column, name)| ColumnDeclaration {
            name,
            data_type: [
                DataType::Int64,
                DataType::Double,
                DataType::Date,
                DataType::String,
            ][column % 4],
            nullable: column % 3 != 0,
        })
        .collect();
    database.declare_table("wide", &schema, &cancel)?;
    let integers: [Vec<_>; 16] =
        std::array::from_fn(|i| (0..rows).map(|row| integer(row, i * 4)).collect());
    let numbers: [Vec<_>; 16] =
        std::array::from_fn(|i| (0..rows).map(|row| number(row, i * 4 + 1)).collect());
    let dates: [Vec<_>; 16] = std::array::from_fn(|i| {
        (0..rows)
            .map(|row| {
                let column = (i * 4 + 2) as i32;
                let day = match row % 3 {
                    0 => -719_162 + column,
                    1 => 2_932_896 - column,
                    _ => row as i32 * 37 + column,
                };
                DateValue::from_days_since_unix_epoch(day).unwrap()
            })
            .collect()
    });
    let text: [Vec<_>; 16] = std::array::from_fn(|i| {
        (0..rows)
            .map(|row| {
                if row % 13 == 0 {
                    String::new()
                } else {
                    format!("c{:02}:r{row}:雪\0é🙂", i * 4 + 3)
                }
            })
            .collect()
    });
    let text_views: [Vec<_>; 16] =
        std::array::from_fn(|i| text[i].iter().map(String::as_str).collect());
    let validity: [Vec<u8>; COLUMNS] = std::array::from_fn(|column| {
        let mut bits = vec![0; rows.div_ceil(8)];
        for row in 0..rows {
            if column % 3 == 0 || (row + column * 7) % 11 != 0 {
                bits[row / 8] |= 1 << (row % 8);
            }
        }
        bits
    });
    let inputs: Vec<_> = (0..COLUMNS)
        .map(|column| ColumnInput {
            values: match column % 4 {
                0 => ColumnValues::Int64(&integers[column / 4]),
                1 => ColumnValues::Double(&numbers[column / 4]),
                2 => ColumnValues::Date(&dates[column / 4]),
                _ => ColumnValues::String(&text_views[column / 4]),
            },
            validity: &validity[column],
        })
        .collect();
    // At 8,192 rows, fixed values, NULL maps, four-byte text offsets and at
    // most 20 UTF-8 bytes per string fit below 6 MB, including unit metadata.
    // Keep the existing temporary budget and the 513-row qualification bound.
    let mut append = database.begin_append(
        "wide",
        AppendLimits {
            batches: 1,
            encoded_bytes: if rows == 513 { 2_000_000 } else { 6_000_000 },
        },
        &cancel,
    )?;
    append.write(&inputs, &cancel)?;
    append.commit(&cancel)?;
    let projection = (0..COLUMNS)
        .map(|ordinal| {
            let source = (ordinal * 17 + 7) % COLUMNS;
            if ordinal < 8 {
                format!("c{source:02} AS v{ordinal:02}")
            } else {
                format!("c{source:02}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let query = database.prepare(&format!("FROM wide |> SELECT {projection}"))?;
    // The large baseline has 32 groups of 64 chunks each, so measurements
    // admit a larger footer while retaining the same per-group text bound.
    let limits = ParquetExportLimits {
        rows: rows as u64,
        bytes: if rows == 513 { 2_000_000 } else { 16_000_000 },
        row_group_rows: 257,
        row_group_text_bytes: 1_000_000,
        row_groups: 3,
        metadata_bytes: if rows == 513 { 65_536 } else { 262_144 },
    };
    if let Some(measurement) = measurement {
        measure_output(root, &database, &query, limits, measurement, &cancel)?;
        drop(query);
        database.close()?;
        return Ok(());
    }
    // Query admission rejects 65 outputs before the exporter can receive them.
    // Aliasing only eight columns keeps both selections within the token budget.
    let too_wide = format!("FROM wide |> SELECT {projection}, c00 AS extra");
    let baseline = database.reserved_memory_bytes();
    assert!(matches!(
        database.prepare(&too_wide),
        Err(Error::Parse { message: "output limit exceeded", span })
            if too_wide.get(span.start()..span.end()) == Some("|>")
    ));
    assert_eq!(database.reserved_memory_bytes(), baseline);
    let handles = descriptors()?;
    for group_rows in [257, 255] {
        let mut output = ShortWriter::new(&root.join(format!("wide-{group_rows}.parquet")))?;
        assert_eq!(
            database.export_parquet(
                &query,
                &mut output,
                ParquetExportLimits {
                    row_group_rows: group_rows,
                    ..limits
                },
                &cancel
            )?,
            rows as u64
        );
        assert_eq!(output.flushes, 1);
        drop(output);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
        assert_eq!(descriptors()?, handles);
    }
    let complete = fs::read(root.join("wide-257.parquet"))?;
    let footer = u32::from_le_bytes(complete[complete.len() - 8..complete.len() - 4].try_into()?);
    for (name, limits, owner) in [
        (
            "rows",
            ParquetExportLimits {
                rows: 512,
                ..limits
            },
            "Parquet output rows",
        ),
        (
            "metadata",
            ParquetExportLimits {
                metadata_bytes: footer - 1,
                ..limits
            },
            "Parquet footer bytes",
        ),
    ] {
        let mut output = ShortWriter::new(&root.join(format!("failed-{name}.parquet")))?;
        assert!(
            matches!(database.export_parquet(&query, &mut output, limits, &cancel), Err(Error::Resource { owner: actual, .. }) if actual == owner)
        );
        assert!(output.bytes > 4);
        assert_eq!(output.flushes, 0);
        drop(output);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
        assert_eq!(descriptors()?, handles);
    }
    let mut retry = ShortWriter::new(&root.join("retry.parquet"))?;
    assert_eq!(
        database.export_parquet(&query, &mut retry, limits, &cancel)?,
        rows as u64
    );
    assert_eq!(retry.flushes, 1);
    drop(retry);
    assert_eq!(fs::read(root.join("retry.parquet"))?, complete);
    assert_eq!(database.reserved_memory_bytes(), baseline);
    assert_eq!(database.reserved_temp_bytes(), 0);
    assert_eq!(descriptors()?, handles);
    drop(query);
    database.close()?;
    println!(
        "wide rows=513 columns=64 groups=2,3 query_width_refused=true rows_refused=true metadata_refused=true released=true retry=true status=exported"
    );
    Ok(())
}

// Setup and file opening precede each interval. API timing includes query
// execution, short writes and final flush; handle and reservation checks follow.
fn measure_output(
    root: &Path,
    database: &Database,
    query: &PreparedQuery<'_>,
    limits: ParquetExportLimits,
    measurement: Measurement,
    cancel: &CancellationToken,
) -> Result<()> {
    let limits = ParquetExportLimits {
        row_group_rows: measurement.group_rows,
        row_groups: u32::try_from(limits.rows.div_ceil(u64::from(measurement.group_rows)))?,
        ..limits
    };
    let baseline = database.reserved_memory_bytes();
    let handles = descriptors()?;
    println!(
        "input rows={} columns=64 group_rows={} fragment={} memory_limit=64000000",
        limits.rows, measurement.group_rows, measurement.fragment
    );
    for sample in 0..6 {
        let mut output = ShortWriter::new(&root.join(format!("sample-{sample}.parquet")))?;
        output.fragment = measurement.fragment;
        let started = Instant::now();
        let outcome = database.export_parquet(query, &mut output, limits, cancel);
        let elapsed = started.elapsed().as_nanos();
        assert_eq!(outcome?, limits.rows);
        assert!(elapsed > 0 && output.bytes > 8);
        assert_eq!(output.flushes, 1);
        let bytes = output.bytes;
        let writes = output.writes;
        assert!(writes > 0 && writes <= bytes);
        drop(output);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
        assert_eq!(descriptors()?, handles);
        println!(
            "sample={sample} elapsed_ns={elapsed} rows={} bytes={bytes} writes={writes} flushes=1 released=true handles=true",
            limits.rows
        );
    }
    println!("status=measured");
    Ok(())
}
