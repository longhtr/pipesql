//! Produce timed typed exports for the independent development supervisor.
//!
//! Arguments are a fresh absolute directory, scan/order, plain8/plain1024/escaped1024,
//! input batch rows (64/256), writer fragment bytes (127/1024), and rows
//! (0/257/8192). Append parquet, group rows (128/311) and group text bytes
//! (4096/65536/131072) for Parquet profiles. The supervisor
//! parses every schema, value and completion record with an independent reader;
//! this producer checks API outcomes, flushes, spill, cancellation and release.
//!
//! Samples time the export API through its final flush, including writer calls and
//! reservation observations. File creation, setup, warm-up, failure controls and
//! independent validation are outside those intervals. Whole-process measurements
//! include them. Files belong to the caller and remain available after return.
//! Writer observations can miss reservation peaks before output starts; caller
//! and OS output buffering are not engine reservations. Times include query
//! execution and do not isolate encoder CPU time.

#[path = "support/query.rs"]
mod query;

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, DateValue, Error, ExportLimits, ParquetExportLimits, PreparedQuery,
};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const MEMORY: u64 = 4_000_000;
const TEMP: u64 = 128_000_000;
const OUTPUT: u64 = 64_000_000;
const SELECT: &str = "SELECT id, amount AS duplicate, number AS duplicate, day, label";
const LATE: &str = "FROM facts |> ORDER BY id |> SELECT id + 9223372036854767617 AS overflow";

#[derive(Clone, Copy)]
enum Bounds {
    Jsonl(ExportLimits),
    Parquet(ParquetExportLimits),
}

impl Bounds {
    fn with_rows(mut self, rows: u64) -> Self {
        match &mut self {
            Self::Jsonl(limits) => limits.rows = rows,
            Self::Parquet(limits) => limits.rows = rows,
        }
        self
    }

    fn with_bytes(mut self, bytes: u64) -> Self {
        match &mut self {
            Self::Jsonl(limits) => limits.bytes = bytes,
            Self::Parquet(limits) => limits.bytes = bytes,
        }
        self
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Jsonl(_) => "jsonl",
            Self::Parquet(_) => "parquet",
        }
    }

    fn owner(self, failure: &str) -> &'static str {
        match (self, failure) {
            (Self::Jsonl(_), "rows") => "result export rows",
            (Self::Jsonl(_), "bytes") => "result export bytes",
            (Self::Parquet(_), "rows") => "Parquet output rows",
            (Self::Parquet(_), "bytes") => "Parquet output bytes",
            (Self::Parquet(_), "groups") => "Parquet output row groups",
            (Self::Parquet(_), "text") => "Parquet row text bytes",
            (Self::Parquet(_), "metadata") => "Parquet footer bytes",
            _ => unreachable!(),
        }
    }
}

fn create(path: &Path, rows: usize, batch_rows: usize, profile: &str) -> Result<()> {
    let db = Database::create_empty(path, Config::new(32_000_000, TEMP)?)?;
    let token = CancellationToken::new();
    db.declare_table(
        "facts",
        &[
            ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "amount",
                data_type: DataType::Int64,
                nullable: true,
            },
            ColumnDeclaration {
                name: "number",
                data_type: DataType::Double,
                nullable: true,
            },
            ColumnDeclaration {
                name: "day",
                data_type: DataType::Date,
                nullable: true,
            },
            ColumnDeclaration {
                name: "label",
                data_type: DataType::String,
                nullable: true,
            },
        ],
        &token,
    )?;
    if rows != 0 {
        let mut append = db.begin_append(
            "facts",
            AppendLimits {
                batches: rows.div_ceil(batch_rows) as u32,
                encoded_bytes: OUTPUT,
            },
            &token,
        )?;
        let amounts = [
            i64::MIN,
            i64::MAX,
            -1,
            0,
            1,
            -9_007_199_254_740_993,
            9_007_199_254_740_993,
        ];
        let bits = [
            0,
            0x8000_0000_0000_0000,
            0x3ff0_0000_0000_0000,
            0xbff0_0000_0000_0000,
            1,
            0x7fef_ffff_ffff_ffff,
            0x7ff0_0000_0000_0000,
            0xfff0_0000_0000_0000,
            0x7ff8_0000_0000_0123,
            0xfff8_0000_0000_0042,
        ];
        let days = [-719_162, -1, 0, 11_016, -25_508, 2_932_896];
        for start in (0..rows).step_by(batch_rows) {
            let ids: Vec<_> = (start..(start + batch_rows).min(rows))
                .map(|index| (rows - 1 - index) as i64)
                .collect();
            let amounts: Vec<_> = ids
                .iter()
                .map(|id| amounts[*id as usize % amounts.len()])
                .collect();
            let numbers: Vec<_> = ids
                .iter()
                .map(|id| f64::from_bits(bits[*id as usize % bits.len()]))
                .collect();
            let days: Vec<_> = ids
                .iter()
                .map(|id| {
                    DateValue::from_days_since_unix_epoch(days[*id as usize % days.len()]).unwrap()
                })
                .collect();
            let labels: Vec<_> = ids
                .iter()
                .map(|id| {
                    if id % 17 == 0 {
                        return String::new();
                    }
                    let width = if profile == "plain8" { 8 } else { 1024 };
                    let pattern = if profile == "escaped1024" {
                        "\0\n\t\"\\é🙂"
                    } else {
                        "x"
                    };
                    let mut text = pattern.repeat((width - 8) / pattern.len());
                    text.push_str(&"x".repeat(width - 8 - text.len()));
                    text.push_str(&format!("{id:08}"));
                    text
                })
                .collect();
            let views: Vec<_> = labels.iter().map(String::as_str).collect();
            let validity: Vec<Vec<u8>> = [0, 5, 7, 11, 13]
                .into_iter()
                .map(|divisor| {
                    let mut bytes = vec![0; ids.len().div_ceil(8)];
                    for (index, id) in ids.iter().enumerate() {
                        if divisor == 0 || id % divisor != 0 {
                            bytes[index / 8] |= 1 << (index % 8);
                        }
                    }
                    bytes
                })
                .collect();
            append.write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&ids),
                        validity: &validity[0],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&amounts),
                        validity: &validity[1],
                    },
                    ColumnInput {
                        values: ColumnValues::Double(&numbers),
                        validity: &validity[2],
                    },
                    ColumnInput {
                        values: ColumnValues::Date(&days),
                        validity: &validity[3],
                    },
                    ColumnInput {
                        values: ColumnValues::String(&views),
                        validity: &validity[4],
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fault {
    None,
    Write,
    Flush,
    Cancel,
}

#[derive(Clone, Copy, Default)]
struct Observed {
    bytes: u64,
    writes: u64,
    flushes: u64,
    memory: u64,
    temporary: u64,
    scratch: u64,
}

struct Output<'a> {
    file: File,
    database: &'a Database,
    token: &'a CancellationToken,
    fragment: usize,
    fault: Fault,
    probe: bool,
    probe_after: u64,
    observed: Observed,
}

impl Write for Output<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.observed.writes += 1;
        self.observed.memory = self
            .observed
            .memory
            .max(self.database.reserved_memory_bytes());
        self.observed.temporary = self
            .observed
            .temporary
            .max(self.database.reserved_temp_bytes());
        if self.probe && self.observed.bytes >= self.probe_after {
            self.observed.scratch = query::scratch_usage(self.database.path())
                .map_err(|error| io::Error::other(error.to_string()))?
                .1;
            self.probe = false;
        }
        if self.fault == Fault::Write && self.observed.bytes >= 1024 {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let count = self.file.write(&bytes[..bytes.len().min(self.fragment)])?;
        self.observed.bytes += count as u64;
        if self.fault == Fault::Cancel && self.observed.bytes >= 1024 {
            self.token.cancel();
        }
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.observed.flushes += 1;
        self.file.flush()?;
        if self.fault == Fault::Flush {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        Ok(())
    }
}

fn descriptors() -> io::Result<usize> {
    fs::read_dir(if cfg!(target_os = "linux") {
        "/proc/self/fd"
    } else {
        "/dev/fd"
    })?
    .try_fold(0, |count, entry| entry.map(|_| count + 1))
}

fn export(
    db: &Database,
    query: &PreparedQuery<'_>,
    path: &Path,
    fragment: usize,
    fault: Fault,
    probe: bool,
    limits: Bounds,
) -> Result<(std::result::Result<u64, Error>, Observed, u128)> {
    let baseline = db.reserved_memory_bytes();
    let handles = descriptors()?;
    let token = CancellationToken::new();
    let mut output = Output {
        file: OpenOptions::new().write(true).create_new(true).open(path)?,
        database: db,
        token: &token,
        fragment,
        fault,
        probe,
        // Parquet's magic precedes execution. The selected ordered profiles
        // write their first group while the cursor still owns its spill files.
        probe_after: if matches!(limits, Bounds::Parquet(_)) {
            4
        } else {
            0
        },
        observed: Observed::default(),
    };
    let start = Instant::now();
    let outcome = match limits {
        Bounds::Jsonl(limits) => db.export_jsonl(query, &mut output, limits, &token),
        Bounds::Parquet(limits) => db.export_parquet(query, &mut output, limits, &token),
    };
    let elapsed = start.elapsed().as_nanos();
    let observed = output.observed;
    drop(output);
    query::released(db, baseline, db.path())?;
    if descriptors()? != handles {
        return Err("export retained a descriptor".into());
    }
    if fs::metadata(path)?.len() != observed.bytes {
        return Err("writer byte count differs from file".into());
    }
    Ok((outcome, observed, elapsed))
}

fn report(name: &str, rows: u64, observed: Observed, elapsed: u128) {
    println!(
        "{name} elapsed_ns={elapsed} rows={rows} bytes={} writes={} flushes={} memory={} temporary={} scratch={}",
        observed.bytes,
        observed.writes,
        observed.flushes,
        observed.memory,
        observed.temporary,
        observed.scratch
    );
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let skip_flush_fault = match std::env::var_os("PIPESQL_EXPORT_CONTROL") {
        None => false,
        Some(value) if value == "skip-flush-fault" => true,
        Some(_) => return Err("unknown export control".into()),
    };
    if !matches!(args.len(), 6 | 9) {
        return Err(
            "expected directory, scan/order, text profile, batch rows, writer fragment, rows, and optional parquet/group-rows/group-text"
                .into(),
        );
    }
    let root = Path::new(&args[0]);
    let ordered = match args[1].as_str() {
        "scan" => false,
        "order" => true,
        _ => return Err("unknown export query".into()),
    };
    let profile = &args[2];
    let batch: usize = args[3].parse()?;
    let fragment: usize = args[4].parse()?;
    let rows: usize = args[5].parse()?;
    if !matches!(profile.as_str(), "plain8" | "plain1024" | "escaped1024")
        || !matches!(batch, 64 | 256)
        || !matches!(fragment, 127 | 1024)
        || !matches!(rows, 0 | 257 | 8192)
    {
        return Err("unsupported export input shape".into());
    }
    let limits = if args.len() == 6 {
        Bounds::Jsonl(ExportLimits {
            rows: rows as u64,
            bytes: OUTPUT,
        })
    } else {
        let group_rows = args[7].parse()?;
        let group_text = args[8].parse()?;
        if args[6] != "parquet"
            || !matches!(group_rows, 128 | 311)
            || !matches!(group_text, 4096 | 65536 | 131072)
        {
            return Err("unsupported Parquet output shape".into());
        }
        Bounds::Parquet(ParquetExportLimits {
            rows: rows as u64,
            bytes: OUTPUT,
            row_group_rows: group_rows,
            row_group_text_bytes: group_text,
            row_groups: 256,
            metadata_bytes: 65_536,
        })
    };
    let extension = limits.extension();
    fs::create_dir(root)?;
    create(&root.join("database"), rows, batch, profile)?;
    let db = Database::open(&root.join("database"), Config::new(MEMORY, TEMP)?)?;
    let select = if matches!(limits, Bounds::Parquet(_)) {
        "SELECT id, amount, number, day, label"
    } else {
        SELECT
    };
    let sql = format!(
        "FROM facts {} |> {select}",
        if ordered { "|> ORDER BY id" } else { "" }
    );
    let prepared = db.prepare(&sql)?;
    println!(
        "input query={} profile={profile} batch_rows={batch} fragment={fragment} rows={rows} memory_limit={MEMORY}",
        args[1]
    );
    if let Bounds::Parquet(limits) = limits {
        println!(
            "format=parquet group_rows={} group_text={}",
            limits.row_group_rows, limits.row_group_text_bytes
        );
    }
    let (outcome, warm, elapsed) = export(
        &db,
        &prepared,
        &root.join(format!("warmup.{extension}")),
        fragment,
        Fault::None,
        true,
        limits,
    )?;
    assert_eq!(outcome?, rows as u64);
    assert_eq!(warm.flushes, 1);
    assert_eq!(warm.temporary > 0, ordered && rows > 0);
    if cfg!(target_os = "linux") {
        assert_eq!(warm.scratch > 0, ordered && rows > 0);
    }
    report("warmup", rows as u64, warm, elapsed);
    for sample in 0..5 {
        let (outcome, observation, elapsed) = export(
            &db,
            &prepared,
            &root.join(format!("sample-{sample}.{extension}")),
            fragment,
            Fault::None,
            false,
            limits,
        )?;
        assert_eq!(outcome?, rows as u64);
        assert_eq!(observation.flushes, 1);
        report(
            &format!("sample={sample}"),
            rows as u64,
            observation,
            elapsed,
        );
    }
    if rows != 0 {
        let mut failures = vec![
            ("write", Fault::Write, limits),
            ("flush", Fault::Flush, limits),
            ("cancel", Fault::Cancel, limits),
            ("rows", Fault::None, limits.with_rows(rows as u64 - 1)),
            (
                "bytes",
                Fault::None,
                limits.with_bytes(if matches!(limits, Bounds::Parquet(_)) {
                    1024
                } else {
                    8192
                }),
            ),
        ];
        if let Bounds::Parquet(p) = limits {
            failures.extend([
                (
                    "groups",
                    Fault::None,
                    Bounds::Parquet(ParquetExportLimits { row_groups: 1, ..p }),
                ),
                (
                    "text",
                    Fault::None,
                    Bounds::Parquet(ParquetExportLimits {
                        row_group_text_bytes: 1,
                        ..p
                    }),
                ),
                (
                    "metadata",
                    Fault::None,
                    Bounds::Parquet(ParquetExportLimits {
                        metadata_bytes: 1,
                        ..p
                    }),
                ),
            ]);
        }
        for (name, fault, bound) in failures {
            let (outcome, observation, _) = export(
                &db,
                &prepared,
                &root.join(format!("failure-{name}.{extension}")),
                fragment,
                if fault == Fault::Flush && skip_flush_fault {
                    Fault::None
                } else {
                    fault
                },
                false,
                bound,
            )?;
            let error = outcome.expect_err("export fault must fail");
            match name {
                "write" | "flush" => assert!(
                    matches!(error, Error::Io { operation: "write result export", source } if source.kind()==io::ErrorKind::BrokenPipe)
                ),
                "cancel" => assert!(matches!(error, Error::Cancelled)),
                "rows" | "bytes" | "groups" | "text" | "metadata" => assert!(matches!(
                    error, Error::Resource { owner, required, limit }
                    if owner == limits.owner(name) && required > limit
                )),
                _ => unreachable!(),
            }
            assert!(observation.bytes > 0);
            assert_eq!(observation.flushes, u64::from(name == "flush"));
            println!(
                "failure={name} bytes={} writes={} flushes={} released=true",
                observation.bytes, observation.writes, observation.flushes
            );
        }
        if ordered {
            query::cancel(&db, &prepared, db.path())?;
        }
    }
    if rows == 8192 {
        let late = db.prepare(LATE)?;
        let (outcome, observation, _) = export(
            &db,
            &late,
            &root.join(format!("failure-query.{extension}")),
            fragment,
            Fault::None,
            false,
            limits,
        )?;
        let Error::ArithmeticOverflow {
            operation: "addition",
            span,
        } = outcome.expect_err("last row overflows")
        else {
            return Err("wrong late export error".into());
        };
        assert_eq!(&LATE[span.start()..span.end()], "id + 9223372036854767617");
        assert!(observation.bytes >= 1024 && observation.flushes == 0);
        println!(
            "failure=query bytes={} writes={} flushes=0 span={}:{} released=true",
            observation.bytes,
            observation.writes,
            span.start(),
            span.end()
        );
    }
    let (outcome, observation, elapsed) = export(
        &db,
        &prepared,
        &root.join(format!("retry.{extension}")),
        fragment,
        Fault::None,
        false,
        limits,
    )?;
    assert_eq!(outcome?, rows as u64);
    assert_eq!(observation.flushes, 1);
    report("retry", rows as u64, observation, elapsed);
    drop(prepared);
    db.close()?;
    println!("status=exported");
    Ok(())
}
