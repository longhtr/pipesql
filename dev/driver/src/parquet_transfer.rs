//! Import fresh external files and expose reopened results for independent checks.
//!
//! Arguments are a new absolute output directory, an input file, its row count
//! (777/8192), and caller read fragments (7/127 bytes, plus 4096/65536 for timing).
//! The supervisor owns input
//! values and judges JSON Lines; this driver checks API outcomes and lifetimes.
//! Failed attempts start with a sentinel row, retain their issued tokens and must
//! reopen without publishing a prefix. Final-source faults follow all page reads.
//! Optional --dense qualifies 513-row groups with clustered maximum strings; its
//! larger page/group bounds apply only to that 777-row input selection.
//! Optional --measure uses a warm-up and five imports into separate seeded databases.
//! Only import_parquet is timed, including publication. Setup, source open, reopen
//! and result export are outside; each complete result is checked by the supervisor.
//! Durable resolution names the API outcome, not a storage qualification.
//! `NEW_DIRECTORY --wide-export` checks maximum-width output admission and leaves
//! mixed-type files for the external reader, without an engine decode round trip.
//! `NEW_DIRECTORY --wide-import INPUT` qualifies a 64-column external input and
//! leaves complete reopened JSON Lines for the supervisor's independent checks.
//! `NEW_DIRECTORY --measure-wide-export ROWS GROUP_ROWS FRAGMENT` times a warm-up
//! and five complete exports. Wide cases report their native stack first.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues,
    CommitResolution, Config, DataType, Database, DateValue, Error, ExportLimits,
    ParquetImportLimits, ParquetReadLimits, TransactionId,
};
use std::cell::Cell;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Instant;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[path = "parquet_transfer/wide.rs"]
mod wide;
#[path = "parquet_transfer/wide_import.rs"]
mod wide_import;

fn small_stack(work: impl FnOnce() -> Result<()> + Send) -> Result<()> {
    let request = match std::env::var("PIPESQL_TRANSFER_STACK_CONTROL")
        .ok()
        .as_deref()
    {
        None => pipesql_filesystem::TEST_SMALL_STACK_REQUEST_BYTES,
        Some("oversized") => 2 * 1024 * 1024,
        Some(_) => return Err("unknown transfer stack control".into()),
    };
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("wide-parquet".into())
            .stack_size(request)
            .spawn_scoped(scope, move || {
                let reported = pipesql_filesystem::test_current_thread_stack_bytes();
                let limit = pipesql_filesystem::TEST_SMALL_STACK_LIMIT_BYTES;
                // Check the actual extent, including in the oversized control.
                // Reject before creating files or entering the transfer path.
                assert!(
                    (pipesql_filesystem::TEST_SMALL_STACK_REQUEST_BYTES..=limit)
                        .contains(&reported),
                    "wide transfer stack extent {reported} outside the small-stack bound"
                );
                println!("stack requested={request} reported={reported} limit={limit}");
                work().map_err(|error| error.to_string())
            })?;
        worker
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            .map_err(Into::into)
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    Receipt,
    Source,
    Cancel,
}

#[derive(Clone, Copy, Default)]
struct Observed {
    calls: u64,
    bytes: u64,
    end_seeks: u64,
    faulted: bool,
}

struct Input<'a> {
    file: File,
    fragment: usize,
    fault: Fault,
    cancel: &'a CancellationToken,
    observed: &'a Cell<Observed>,
}

impl Read for Input<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let limit = bytes.len().min(self.fragment);
        let count = self.file.read(&mut bytes[..limit])?;
        let mut observed = self.observed.get();
        observed.calls += 1;
        observed.bytes += count as u64;
        self.observed.set(observed);
        Ok(count)
    }
}

impl Seek for Input<'_> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        if matches!(to, SeekFrom::End(0)) {
            let mut observed = self.observed.get();
            observed.end_seeks += 1;
            if observed.end_seeks == 2 && matches!(self.fault, Fault::Source | Fault::Cancel) {
                observed.faulted = true;
                self.observed.set(observed);
                if self.fault == Fault::Source {
                    return Err(io::ErrorKind::BrokenPipe.into());
                }
                self.cancel.cancel();
            }
            self.observed.set(observed);
        }
        self.file.seek(to)
    }
}

fn config() -> Config {
    Config::new(8_000_000, 8_000_000).unwrap()
}

fn descriptors() -> io::Result<usize> {
    fs::read_dir(if cfg!(target_os = "linux") {
        "/proc/self/fd"
    } else {
        "/dev/fd"
    })?
    .try_fold(0, |count, entry| entry.map(|_| count + 1))
}

fn units(path: &Path) -> io::Result<Vec<std::ffi::OsString>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(path.join("units"))? {
        let entry = entry?;
        assert!(entry.file_type()?.is_file());
        names.push(entry.file_name());
    }
    names.sort();
    assert!(!names.is_empty(), "the sentinel has stored data");
    Ok(names)
}

fn seed(path: &Path) -> Result<Database> {
    let db = Database::create_empty(path, config())?;
    let cancel = CancellationToken::new();
    // Reverse the input's order and vary ID's case to require name mapping.
    let schema = [
        ("note", DataType::String, true),
        ("day", DataType::Date, true),
        ("number", DataType::Double, true),
        ("amount", DataType::Int64, true),
        ("ID", DataType::Int64, false),
    ]
    .map(|(name, data_type, nullable)| ColumnDeclaration {
        name,
        data_type,
        nullable,
    });
    db.declare_table("facts", &schema, &cancel)?;
    let mut append = db.begin_append(
        "facts",
        AppendLimits {
            batches: 1,
            encoded_bytes: 4096,
        },
        &cancel,
    )?;
    append.write(
        &[
            ColumnInput {
                values: ColumnValues::String(&["existing"]),
                validity: &[1],
            },
            ColumnInput {
                values: ColumnValues::Date(&[DateValue::from_days_since_unix_epoch(0).unwrap()]),
                validity: &[0],
            },
            ColumnInput {
                values: ColumnValues::Double(&[0.0]),
                validity: &[0],
            },
            ColumnInput {
                values: ColumnValues::Int64(&[0]),
                validity: &[0],
            },
            ColumnInput {
                values: ColumnValues::Int64(&[-1]),
                validity: &[1],
            },
        ],
        &cancel,
    )?;
    append.commit(&cancel)?;
    Ok(db)
}

fn export(db: &Database, path: &Path, rows: u64) -> Result<()> {
    let query =
        db.prepare("FROM facts |> ORDER BY ID |> SELECT ID AS id, amount, number, day, note")?;
    let mut output = OpenOptions::new().write(true).create_new(true).open(path)?;
    assert_eq!(
        db.export_jsonl(
            &query,
            &mut output,
            ExportLimits {
                rows,
                bytes: 8_000_000
            },
            &CancellationToken::new()
        )?,
        rows
    );
    Ok(())
}

fn measure(
    root: &Path,
    input: &Path,
    rows: u64,
    fragment: usize,
    length: u64,
    limits: ParquetImportLimits,
) -> Result<()> {
    for sample in 0..6 {
        let path = root.join(format!("database-{sample}"));
        seed(&path)?.close()?;
        let db = Database::open(&path, config())?;
        let generation = db.generation();
        assert_eq!(
            generation, 2,
            "each sample starts after declaration and seed"
        );
        let cancel = CancellationToken::new();
        let observed = Cell::new(Observed::default());
        let issued = Cell::new(None);
        let memory = db.reserved_memory_bytes();
        let handles = descriptors()?;
        let reader = Input {
            file: File::open(input)?,
            fragment,
            fault: Fault::None,
            cancel: &cancel,
            observed: &observed,
        };
        let started = Instant::now();
        let result = db.import_parquet("facts", reader, limits, &cancel, |token| {
            assert!(issued.replace(Some(token)).is_none());
            Ok(())
        });
        let elapsed = started.elapsed().as_nanos();
        let commit = result?;
        assert!(elapsed > 0);
        assert_eq!(issued.get(), Some(commit.transaction()));
        assert_eq!(db.generation(), generation + 1);
        assert_eq!(db.reserved_memory_bytes(), memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(descriptors()?, handles);
        let observation = observed.get();
        assert_eq!(observation.bytes, length);
        assert_eq!(observation.end_seeks, 2);
        assert!(!observation.faulted);
        assert!(observation.calls >= length.div_ceil(fragment as u64));
        db.close()?;
        let db = Database::open(&path, config())?;
        assert_eq!(
            db.resolve_commit(commit.transaction())?,
            CommitResolution::Durable(commit)
        );
        export(&db, &root.join(format!("sample-{sample}.jsonl")), rows + 1)?;
        db.close()?;
        // The supervisor retains databases and answers until it checks every value.
        println!(
            "sample={sample} elapsed_ns={elapsed} read_calls={} read_bytes={} end_seeks=2 rows={} baseline_generation=2 durable=true released=true reopened=true",
            observation.calls,
            observation.bytes,
            rows + 1
        );
    }
    println!("status=measured");
    Ok(())
}

fn main() -> Result<()> {
    let mut arguments: Vec<_> = std::env::args().skip(1).collect();
    if let [root, mode, input] = arguments.as_slice()
        && mode == "--wide-import"
    {
        return small_stack(|| wide_import::run(Path::new(root), Path::new(input)));
    }
    if let [root, mode] = arguments.as_slice()
        && mode == "--wide-export"
    {
        return small_stack(|| wide::run(Path::new(root)));
    }
    if let [root, mode, rows, group_rows, fragment] = arguments.as_slice()
        && mode == "--measure-wide-export"
    {
        let (rows, group_rows, fragment) = (rows.parse()?, group_rows.parse()?, fragment.parse()?);
        return small_stack(|| wide::measure(Path::new(root), rows, group_rows, fragment));
    }
    let measuring = arguments.last().is_some_and(|value| value == "--measure");
    let dense = arguments.last().is_some_and(|value| value == "--dense");
    if measuring || dense {
        arguments.pop();
    }
    let [root, input, rows, fragment] = arguments.as_slice() else {
        return Err("expected new directory, input file, rows and read fragment".into());
    };
    let root = PathBuf::from(root);
    let input = PathBuf::from(input);
    let rows: u64 = rows.parse()?;
    let fragment: usize = fragment.parse()?;
    if !root.is_absolute()
        || !input.is_absolute()
        || !matches!(rows, 777 | 8192)
        || dense && rows != 777
        || !(matches!(fragment, 7 | 127) || measuring && matches!(fragment, 4096 | 65536))
    {
        return Err("unsupported Parquet transfer selection".into());
    }
    let skip = match std::env::var("PIPESQL_IMPORT_CONTROL").ok().as_deref() {
        None => false,
        Some("skip-source-fault") => true,
        Some(_) => return Err("unknown import control".into()),
    };
    if measuring && skip {
        return Err("fault controls and measurement are separate selections".into());
    }
    let length = fs::metadata(&input)?.len();
    if !(12..=2_000_000).contains(&length) {
        return Err("input length outside selection".into());
    }
    let limits = ParquetImportLimits {
        parquet: ParquetReadLimits {
            input_bytes: 2_000_000,
            rows,
            metadata_bytes: 65_536,
            row_groups: 64,
            row_group_rows: if dense { 513 } else { 389 },
            row_group_bytes: if dense { 1_048_576 } else { 196_608 },
            page_bytes: if dense { 1_048_576 } else { 131_072 },
        },
        append: AppendLimits {
            batches: 128,
            encoded_bytes: 3_000_000,
        },
    };
    fs::create_dir(&root)?;
    if measuring {
        println!("input rows={rows} fragment={fragment} bytes={length}");
        return measure(&root, &input, rows, fragment, length, limits);
    }
    let path = root.join("database");
    let mut db = seed(&path)?;
    let generation = db.generation();
    let mut aborted: Vec<TransactionId> = Vec::new();
    println!("input rows={rows} fragment={fragment} bytes={length}");
    for (name, fault) in [
        ("receipt", Fault::Receipt),
        ("source", Fault::Source),
        ("cancel", Fault::Cancel),
    ] {
        let cancel = CancellationToken::new();
        let observed = Cell::new(Observed::default());
        let issued = Cell::new(None);
        let memory = db.reserved_memory_bytes();
        let handles = descriptors()?;
        let objects = units(&path)?;
        let result = db.import_parquet(
            "facts",
            Input {
                file: File::open(&input)?,
                fragment,
                fault: if skip && fault == Fault::Source {
                    Fault::None
                } else {
                    fault
                },
                cancel: &cancel,
                observed: &observed,
            },
            limits,
            &cancel,
            |token| {
                assert!(issued.replace(Some(token)).is_none());
                if fault == Fault::Receipt {
                    Err(io::ErrorKind::BrokenPipe.into())
                } else {
                    Ok(())
                }
            },
        );
        // The disabled-fault control must really import the complete input before
        // the expected-error assertion fails; its supervisor checks these values.
        if skip && fault == Fault::Source && result.is_ok() {
            export(&db, &root.join("unexpected-success.jsonl"), rows + 1)?;
        }
        let error = result.expect_err("import fault must fail");
        match fault {
            Fault::Receipt => assert!(
                matches!(error, Error::Io { operation: "report Parquet transaction", source } if source.kind() == io::ErrorKind::BrokenPipe)
            ),
            Fault::Source => assert!(
                matches!(error, Error::Io { operation: "seek Parquet input", source } if source.kind() == io::ErrorKind::BrokenPipe)
            ),
            Fault::Cancel => assert!(matches!(error, Error::Cancelled)),
            Fault::None => unreachable!(),
        }
        let token = issued.get().expect("import issued a receipt");
        assert_eq!(db.reserved_memory_bytes(), memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(descriptors()?, handles);
        assert_eq!(db.generation(), generation);
        // An aborted token may add receipt state. Its private data objects must
        // already be gone, without relying on close/reopen to remove them.
        assert_eq!(units(&path)?, objects);
        let observation = observed.get();
        assert!(observation.calls >= observation.bytes.div_ceil(fragment as u64));
        if fault == Fault::Receipt {
            assert!(observation.bytes > 12 && observation.bytes < length);
            assert_eq!(observation.end_seeks, 1);
        } else {
            assert_eq!(observation.bytes, length);
            assert_eq!(observation.end_seeks, 2);
            assert!(observation.faulted);
        }
        db.close()?;
        db = Database::open(&path, config())?;
        assert_eq!(db.resolve_commit(token)?, CommitResolution::Aborted);
        export(&db, &root.join(format!("{name}.jsonl")), 1)?;
        aborted.push(token);
        println!(
            "failure={name} read_calls={} read_bytes={} end_seeks={} aborted=true released=true reopened=true",
            observation.calls, observation.bytes, observation.end_seeks
        );
    }
    let cancel = CancellationToken::new();
    let observed = Cell::new(Observed::default());
    let issued = Cell::new(None);
    let memory = db.reserved_memory_bytes();
    let handles = descriptors()?;
    let commit = db.import_parquet(
        "facts",
        Input {
            file: File::open(&input)?,
            fragment,
            fault: Fault::None,
            cancel: &cancel,
            observed: &observed,
        },
        limits,
        &cancel,
        |token| {
            assert!(issued.replace(Some(token)).is_none());
            Ok(())
        },
    )?;
    assert_eq!(issued.get(), Some(commit.transaction()));
    assert_eq!(db.reserved_memory_bytes(), memory);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert_eq!(descriptors()?, handles);
    assert_eq!(db.generation(), generation + 1);
    let observation = observed.get();
    assert_eq!(observation.bytes, length);
    assert_eq!(observation.end_seeks, 2);
    assert!(observation.calls >= length.div_ceil(fragment as u64));
    db.close()?;
    let db = Database::open(&path, config())?;
    assert_eq!(
        db.resolve_commit(commit.transaction())?,
        CommitResolution::Durable(commit)
    );
    for token in aborted {
        assert_eq!(db.resolve_commit(token)?, CommitResolution::Aborted);
    }
    export(&db, &root.join("complete.jsonl"), rows + 1)?;
    db.close()?;
    println!(
        "complete rows={} read_calls={} read_bytes={} end_seeks=2 durable=true released=true reopened=true",
        rows + 1,
        observation.calls,
        observation.bytes
    );
    println!("status=imported");
    Ok(())
}
