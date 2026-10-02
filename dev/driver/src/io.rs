//! Exercise database operations while the native observer changes file-I/O results.
//!
//! The development runner starts this driver with a workload, call kind and fault
//! position. Setup runs before observation starts. Short transfers still move
//! real bytes and must allow the operation to finish; injected errors must reach
//! the public caller with their OS code, including commit and cleanup causes.
//! The standard-library mode independently checks payload bytes, guarded extents
//! and stream positions. Its exact-I/O methods retry interrupted calls, unlike
//! the database boundary.
//!
//! Creation failures must remove the partial database directory. Load and recovery
//! cases reopen without faults and check the committed generation and transaction
//! when one was returned. Base Q1/Q6 queries compare every typed cell, both during
//! observation and after reopen or a healthy load retry. Composed queries compare
//! literal answers and check memory and temporary reservations after failure,
//! then run again on the same handle and a reopened database. The development
//! runner owns each directory.

use pipesql::{
    AppendLimits, CancellationToken, CauseKind, ColumnDeclaration, ColumnInput, ColumnValues,
    CommitResolution, Config, DataType, Database, Error, QueryResult, QueryStep, Value,
};
use std::io::{Read, Seek, Write};
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
mod io_capture;
mod io_census_control;
mod io_join;
mod io_observer;
mod io_window;
#[allow(dead_code)]
#[path = "../../../examples/operator_cost.rs"]
mod operators;
use io_observer::*;
const INPUT: &[u8] = b"1|2|3|4|17.00|21168.23|0.04|8|R|F|1996-03-13|12|13|14|15|16|\n";
const Q1: &str = include_str!("../../../test/data/upstream/q1-upstream.pipe.sql");
const Q6: &str = include_str!("../../../test/data/q6.pipe.sql");

fn base_query(db: &Database, q1: bool) -> Result<(), Error> {
    let plan = db.prepare(if q1 { Q1 } else { Q6 })?;
    let cancellation = CancellationToken::new();
    consume(db.execute(&plan, &cancellation)?, q1)
}

fn consume(mut rows: QueryResult<'_, '_>, q1: bool) -> Result<(), Error> {
    let mut seen = 0;
    for _ in 0..10_000 {
        match rows.step() {
            QueryStep::Rows(batch) => {
                assert!(!batch.is_empty());
                assert_eq!(batch.column_count(), if q1 { 10 } else { 1 });
                for row in 0..batch.len() {
                    assert_eq!(seen, 0, "base query returned an extra row");
                    if q1 {
                        for (column, expected) in [(0, "R"), (1, "F")] {
                            let Some(Value::String(actual)) = batch.value(row, column) else {
                                panic!("base query returned a non-STRING key");
                            };
                            assert_eq!(actual.as_str(), expected);
                        }
                        // The single included row has quantity 17, price 21168.23,
                        // discount .04 and tax 8. These literal binary64 answers
                        // include rounding after each arithmetic operation; the
                        // discounted price is one ULP below decimal 20321.5008.
                        for (column, bits) in [
                            (2, 0x4031_0000_0000_0000),
                            (3, 0x40d4_ac0e_b851_eb85),
                            (4, 0x40d3_d860_0d1b_7175),
                            (5, 0x4106_536c_0ebe_dfa4),
                            (6, 0x4031_0000_0000_0000),
                            (7, 0x40d4_ac0e_b851_eb85),
                            (8, 0x3fa4_7ae1_47ae_147b),
                        ] {
                            let Some(Value::Double(actual)) = batch.value(row, column) else {
                                panic!("base query returned a non-DOUBLE aggregate");
                            };
                            assert_eq!(actual.to_bits(), bits, "base query column {column}");
                        }
                        assert_eq!(batch.value(row, 9), Some(Value::Int64(1)));
                    } else {
                        // The 1996 input is outside Q6's 1994 interval. Empty SUM
                        // produces one NULL cell, not zero or an absent row.
                        assert_eq!(batch.value(row, 0), Some(Value::Null));
                    }
                    seen += 1;
                }
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                assert_eq!(seen, 1, "base query omitted its expected row");
                return Ok(());
            }
            QueryStep::Failed(_) => return Err(rows.into_error().expect("terminal query error")),
        }
    }
    panic!("query exceeded finite fixture step allowance");
}

// A refusal may become a commit or recovery outcome, and a burst can also
// prevent cleanup. Each retained cause must still be the injected OS error;
// an unrelated query, resource or corruption failure does not qualify the cut.
fn injected_cause(cause: &CauseKind, code: i32) -> bool {
    matches!(cause, CauseKind::Io { source, .. } if source.raw_os_error() == Some(code))
}

fn injected_error(error: &Error, code: i32) -> bool {
    match error {
        Error::Io { source, .. } => source.raw_os_error() == Some(code),
        Error::CommitAmbiguous { source, .. } | Error::RecoveryRequired { source, .. } => {
            injected_cause(source.kind(), code)
        }
        Error::CleanupRequired { primary, cleanup } => {
            injected_cause(primary.kind(), code) && injected_cause(cleanup.kind(), code)
        }
        _ => false,
    }
}

fn require_injected_error(result: &Result<(), Error>, code: i32) {
    assert!(
        result
            .as_ref()
            .is_err_and(|error| injected_error(error, code.abs())),
        "native refusal lost its injected OS error {code}: {result:?}"
    );
}

fn composition_query(db: &Database, derived: bool) -> Result<(), Error> {
    let sql = if derived {
        "FROM (FROM facts |> WHERE category IS NULL OR category >= 'A' |> AGGREGATE SUM(n) AS total GROUP BY k) AS a |> LEFT JOIN (FROM facts |> WHERE k NOT BETWEEN 2 AND 3 |> AGGREGATE SUM(n) AS total GROUP BY k) AS b ON a.k = b.k |> EXTEND COUNT(*) OVER () AS partition_rows |> WHERE partition_rows=2 |> AGGREGATE AVG(a.total+b.total) AS mean"
    } else {
        "FROM facts |> AGGREGATE SUM(n) AS total GROUP BY k |> AGGREGATE SUM(total) AS subtotal GROUP BY total |> AGGREGATE AVG(subtotal) AS mean"
    };
    // Repeated grouping leaves totals 30 and 90, whose average is 60. The derived
    // join also yields 60: only key 1 contributes 30 + 30; key 2 has a NULL right
    // total and AVG skips it. Both answers follow directly from the input below.
    let queries = std::iter::once((sql, Value::Double(60.0)))
        .chain(derived.then_some((
            "FROM facts |> SELECT COUNT(*) OVER () AS n |> AGGREGATE SUM(SIGN(DIV(MOD(n, 4), 2))) AS total",
            Value::Int64(3),
        )))
        .chain(derived.then_some((
            "FROM facts |> SELECT COUNT(*) OVER () AS n |> AGGREGATE SUM(ABS(-(n/2))) AS ratio",
            Value::Double(4.5),
        )))
        .chain(derived.then_some((
            "FROM facts |> SELECT COUNT(*) OVER () AS n |> AGGREGATE SUM(FLOOR(n/2)+CEIL(ROUND(SQRT(n*n)/2))+POWER(EXP(LN(n/n))+1, 3)-8+LOG10(n/n)) AS rounded",
            Value::Double(9.0),
        )))
        .chain(derived.then_some((
            "FROM facts |> SELECT SAFE_DIVIDE(n, 0) AS ratio |> EXTEND COALESCE(ratio, 0) AS filled |> WHERE filled IS NOT DISTINCT FROM 0 |> AGGREGATE COUNT(NULLIF(filled, 0)) AS present |> SELECT COALESCE(present, DIV(1, 0)) AS present",
            Value::Int64(0),
        )))
        .chain(derived.then_some((
            "FROM facts |> SELECT k, n |> EXCEPT DISTINCT (FROM facts |> WHERE k=1 |> SELECT k, n) |> AGGREGATE SUM(n) AS total",
            Value::Int64(90),
        )));
    for (sql, expected) in queries {
        let plan = db.prepare(sql)?;
        let cancel = CancellationToken::new();
        let mut result = db.execute(&plan, &cancel)?;
        let mut seen = false;
        let mut finished = false;
        for _ in 0..10_000 {
            match result.step() {
                QueryStep::Rows(batch) => {
                    assert!(!seen && batch.len() == 1 && batch.column_count() == 1);
                    assert_eq!(batch.value(0, 0), Some(expected));
                    seen = true;
                }
                QueryStep::Progress => (),
                QueryStep::Finished => {
                    assert!(seen);
                    finished = true;
                    break;
                }
                QueryStep::Failed(_) => {
                    assert!(!seen, "failed aggregate must not publish a partial result");
                    return Err(result.into_error().expect("terminal repeated query error"));
                }
            }
        }
        assert!(finished, "query exceeded finite fixture step allowance");
    }
    Ok(())
}

fn composition_io(
    root: &std::path::Path,
    kind: u32,
    at: u32,
    burst: u32,
    error: i32,
    derived: bool,
) {
    let path = root.join("database");
    // At 1.1 MB, repeated grouping has enough memory to start but must spill.
    // The Python runner requires positioned reads and writes in the healthy run
    // so a memory-policy change cannot silently remove the disk-failure cases.
    let memory = if derived { 4_000_000 } else { 1_100_000 };
    let config = Config::new(memory, 2_000_000).unwrap();
    let db = Database::create_empty(&path, config).unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "facts",
        &[
            ColumnDeclaration {
                name: "k",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "n",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "category",
                data_type: DataType::String,
                nullable: false,
            },
        ],
        &cancel,
    )
    .unwrap();
    let mut append = db
        .begin_append(
            "facts",
            AppendLimits {
                batches: 1,
                encoded_bytes: 20_000,
            },
            &cancel,
        )
        .unwrap();
    append
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&[1, 1, 2]),
                    validity: &[7],
                },
                ColumnInput {
                    values: ColumnValues::Int64(&[10, 20, 90]),
                    validity: &[7],
                },
                ColumnInput {
                    values: ColumnValues::String(&["A", "A", "B"]),
                    validity: &[7],
                },
            ],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    let baseline = db.reserved_memory_bytes();
    let generation = db.generation();
    // SAFETY: this synchronous caller exclusively owns the bounded observer;
    // setup has finished and the query retains no asynchronous I/O.
    unsafe { io_probe_start(kind, at, burst, error) };
    let result = composition_query(&db, derived);
    // SAFETY: the query and all of its owners have returned or dropped.
    unsafe { io_probe_stop() };
    let (calls, refused, partial) =
        unsafe { (io_probe_calls(), io_probe_refused(), io_probe_partial()) };
    println!("calls={calls} refused={refused} partial={partial} result={result:?}");
    if at == 0 || error == 0 {
        assert!(result.is_ok());
    } else {
        require_injected_error(&result, error);
    }
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert_eq!(db.generation(), generation);
    composition_query(&db, derived).unwrap();
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close().unwrap();
    let reopened = Database::open(&path, config).unwrap();
    assert_eq!(reopened.generation(), generation);
    composition_query(&reopened, derived).unwrap();
    reopened.close().unwrap();
}

fn standard_io(root: &std::path::Path, kind: u32, at: u32, burst: u32, error: i32) {
    // These controls use either no fault or a burst starting at the first call.
    // Distinct payload bytes expose repeated/skipped offsets; surrounding bytes
    // expose writes outside the requested extent. Positioned I/O must preserve
    // the independently chosen stream cursor.
    assert!(at <= 1 && (at == 0 || burst == 3));
    const PAYLOAD: [u8; 8] = [3, 17, 29, 61, 97, 131, 193, 251];
    let reading = kind == 0 || kind == 2;
    let mut expected = [0xa5; 16];
    if reading {
        expected[3..11].copy_from_slice(&PAYLOAD);
    }
    let path = root.join("file");
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    file.write_all(&expected).unwrap();
    file.seek(std::io::SeekFrom::Start(if kind < 2 { 3 } else { 5 }))
        .unwrap();
    let mut buffer = [0xcc; 12];
    // SAFETY: this synchronous fixture owns the observer until the operation
    // returns; setup and verification run outside the observed interval.
    unsafe { io_probe_start(kind, at, burst, error) };
    let result = match kind {
        0 => file.read_exact(&mut buffer[2..10]),
        1 => file.write_all(&PAYLOAD),
        2 => file.read_exact_at(&mut buffer[2..10], 3),
        3 => file.write_all_at(&PAYLOAD, 3),
        _ => panic!("invalid I/O kind"),
    }
    .map_err(|source| Error::Io {
        operation: "standard I/O reference",
        source,
    });
    // SAFETY: no observed I/O remains live after the synchronous call returns.
    unsafe { io_probe_stop() };
    let (calls, refused, partial) =
        unsafe { (io_probe_calls(), io_probe_refused(), io_probe_partial()) };
    println!("calls={calls} refused={refused} partial={partial} result={result:?}");
    let transferred = if at == 0 || error == 0 || error.abs() == libc::EINTR {
        assert!(result.is_ok(), "standard exact I/O must finish: {result:?}");
        if reading {
            assert_eq!(&buffer[2..10], &PAYLOAD, "standard read bytes differ");
        }
        PAYLOAD.len()
    } else {
        require_injected_error(&result, error);
        // Negative codes schedule one real byte before the refusal. Failed
        // read_exact leaves buffer contents unspecified, so check its cursor
        // and guards rather than demanding an undocumented buffer prefix.
        usize::from(error < 0)
    };
    let requested = match (at, error) {
        (0, _) => 8,
        (_, 0) => 8 + 7 + 6 + 5,
        (_, libc::EINTR) => 8 * 4,
        (_, code) if code == -libc::EINTR => 8 + 7 * 4,
        (_, libc::EIO) => 8,
        (_, code) if code == -libc::EIO => 8 + 7,
        _ => panic!("unsupported standard byte control"),
    };
    assert_eq!(unsafe { io_probe_requested_bytes() }, requested);
    assert_eq!(unsafe { io_probe_transferred_bytes() }, transferred as u64);
    assert_eq!(&buffer[..2], &[0xcc; 2], "read changed its leading guard");
    assert_eq!(&buffer[10..], &[0xcc; 2], "read changed its trailing guard");
    assert_eq!(
        file.stream_position().unwrap(),
        if kind < 2 { 3 + transferred as u64 } else { 5 },
        "standard I/O changed the wrong stream position"
    );
    if !reading {
        expected[3..3 + transferred].copy_from_slice(&PAYLOAD[..transferred]);
    }
    assert_eq!(
        std::fs::read(path).unwrap(),
        expected,
        "standard file bytes differ"
    );
}

fn main() {
    io_observer::load();
    let args: Vec<_> = std::env::args().collect();
    if args.get(2).is_some_and(|name| name == "census-control") {
        assert_eq!(args.len(), 5);
        io_census_control::run(&PathBuf::from(&args[1]), &args[3], &args[4]);
        return;
    }
    if args.get(2).is_some_and(|name| name == "census") {
        assert_eq!(args.len(), 9);
        let kind = args[3].parse().unwrap();
        assert!(matches!(kind, 2 | 3));
        let rows = operators::census(
            &PathBuf::from(&args[1]).join("database"),
            &args[4..],
            || unsafe { io_census_start(kind) },
            || unsafe { io_probe_stop() },
        )
        .unwrap();
        let (calls, refused, partial, requested, transferred) = unsafe {
            (
                io_probe_calls(),
                io_probe_refused(),
                io_probe_partial(),
                io_probe_requested_bytes(),
                io_probe_transferred_bytes(),
            )
        };
        assert_eq!((refused, partial), (0, 0));
        assert!(transferred <= requested);
        println!(
            "census rows={rows} calls={calls} requested={requested} transferred={transferred}"
        );
        println!("checked census complete");
        return;
    }
    assert_eq!(args.len(), 7);
    let root = PathBuf::from(&args[1]);
    let mode = args[2].as_str();
    let kind = args[3].parse::<u32>().unwrap();
    let at = args[4].parse::<u32>().unwrap();
    let burst = args[5].parse::<u32>().unwrap();
    let error = args[6].parse::<i32>().unwrap();
    if mode == "window" {
        io_window::run(&root, kind, at, burst, error);
        return;
    }
    if matches!(
        mode,
        "join-inner"
            | "join-left"
            | "join-numeric"
            | "join-double"
            | "join-date"
            | "join-double-numeric"
            | "join-date-numeric"
            | "join-string"
            | "join-string-numeric"
    ) {
        io_join::run(&root, kind, at, burst, error, mode);
        return;
    }
    if matches!(mode, "repeated" | "derived") {
        composition_io(&root, kind, at, burst, error, mode == "derived");
        return;
    }
    if mode == "standard" {
        standard_io(&root, kind, at, burst, error);
        return;
    }
    let db_path = root.join("database");
    let input = root.join("input.tbl");
    std::fs::write(&input, INPUT).unwrap();
    let config = Config::new(2_000_000, 1_000_000).unwrap();
    let mut database = if matches!(mode, "load" | "recover" | "q6" | "q1") {
        Some(Database::create(&db_path, config).unwrap())
    } else {
        None
    };
    let mut token = None;
    if matches!(mode, "recover" | "q6" | "q1") {
        token = Some(
            database
                .as_mut()
                .unwrap()
                .load_lineitem(&input, &CancellationToken::new())
                .unwrap()
                .transaction(),
        );
    }
    if mode == "recover" {
        drop(database.take());
        // Remove one root copy so open must perform recovery writes as well as
        // read and validate the committed state.
        std::fs::remove_file(db_path.join("ROOT.B")).unwrap();
    }
    // SAFETY: this single-threaded fixture exclusively owns bounded observer state.
    // No caller pointers are supplied to its control functions.
    unsafe { io_probe_start(kind, at, burst, error) };
    let result = match mode {
        "create" => Database::create(&db_path, config).map(drop),
        "recover" => Database::open(&db_path, config).map(drop),
        "load" => {
            let result = database
                .as_mut()
                .unwrap()
                .load_lineitem(&input, &CancellationToken::new());
            match &result {
                Ok(commit) => token = Some(commit.transaction()),
                Err(Error::CommitAmbiguous { transaction, .. }) => token = Some(*transaction),
                Err(_) => {}
            }
            result.map(|_| ())
        }
        "q6" | "q1" => base_query(database.as_ref().unwrap(), mode == "q1"),
        _ => panic!("unknown native I/O mode"),
    };
    // SAFETY: the observed operation has returned; no native I/O remains live.
    unsafe { io_probe_stop() };
    let (calls, refused, partial) =
        unsafe { (io_probe_calls(), io_probe_refused(), io_probe_partial()) };
    println!("calls={calls} refused={refused} partial={partial} result={result:?}");
    if at == 0 || error == 0 {
        assert!(result.is_ok());
    } else {
        require_injected_error(&result, error);
    }
    drop(database);
    if matches!(mode, "load" | "recover" | "q6" | "q1") {
        let mut healed = Database::open(&db_path, config).unwrap();
        if let Some(transaction) = token {
            // At these byte-I/O cuts, a returned load token must resolve durable.
            // This is a fixture expectation, not the general CommitAmbiguous rule.
            assert!(
                matches!(
                    healed.resolve_commit(transaction).unwrap(),
                    CommitResolution::Durable(_)
                ),
                "published graph or prior acknowledgement was lost"
            );
            assert_eq!(healed.generation(), 1);
        } else {
            assert_eq!(healed.generation(), 0);
            healed
                .load_lineitem(&input, &CancellationToken::new())
                .unwrap();
        }
        base_query(&healed, true).unwrap();
        base_query(&healed, false).unwrap();
        healed.close().unwrap();
    } else if mode == "create" {
        if result.is_err() {
            assert!(
                !db_path.exists(),
                "I/O-only create refusal left a partial namespace"
            );
            Database::create(&db_path, config).unwrap().close().unwrap();
        } else {
            Database::open(&db_path, config).unwrap().close().unwrap();
        }
    }
}

#[cfg(test)]
use operators::support as test_support;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_refusals_require_the_original_os_error() {
        for code in [libc::EINTR, libc::EIO] {
            let result = Err(Error::Io {
                operation: "native refusal control",
                source: std::io::Error::from_raw_os_error(code),
            });
            require_injected_error(&result, code);
            require_injected_error(&result, -code);
            assert!(injected_cause(
                &CauseKind::Io {
                    operation: "wrapped refusal control",
                    source: std::io::Error::from_raw_os_error(code),
                },
                code,
            ));
            let other = if code == libc::EIO {
                libc::EINTR
            } else {
                libc::EIO
            };
            assert!(!injected_error(result.as_ref().unwrap_err(), other));
            assert!(!injected_cause(
                &CauseKind::Io {
                    operation: "substituted errno",
                    source: std::io::Error::from_raw_os_error(other),
                },
                code,
            ));
            for error in [
                Error::Cancelled,
                Error::Corrupt("unrelated corruption"),
                Error::Io {
                    operation: "message without OS error",
                    source: std::io::Error::other("injected I/O error"),
                },
            ] {
                assert!(!injected_error(&error, code));
            }
            assert!(!injected_cause(&CauseKind::Cancelled, code));
        }
    }

    #[test]
    fn base_answers_reject_wrong_values_and_incomplete_results() {
        let directory = test_support::Directory::new();
        let input = directory.0.join("input.tbl");
        std::fs::write(&input, INPUT).unwrap();
        let mut db = Database::create(
            &directory.0.join("database"),
            Config::new(2_000_000, 1_000_000).unwrap(),
        )
        .unwrap();
        db.load_lineitem(&input, &CancellationToken::new()).unwrap();
        base_query(&db, true).unwrap();
        base_query(&db, false).unwrap();
        for (sql, q1) in [
            (Q1.replace("1 + l_tax", "2 + l_tax"), true),
            (
                format!(
                    "{} |> SELECT 1.0 AS revenue",
                    Q6.trim_end().trim_end_matches(';')
                ),
                false,
            ),
            (
                format!("{} |> LIMIT 0", Q1.trim_end().trim_end_matches(';')),
                true,
            ),
            (
                format!("{} |> LIMIT 0", Q6.trim_end().trim_end_matches(';')),
                false,
            ),
        ] {
            // Prepare and execute outside catch_unwind: setup failures cannot
            // masquerade as a consumer rejecting a wrong or incomplete answer.
            let plan = db.prepare(&sql).unwrap();
            let cancellation = CancellationToken::new();
            let rows = db.execute(&plan, &cancellation).unwrap();
            let rejected =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| consume(rows, q1)));
            assert!(rejected.is_err(), "accepted incorrect answer: {sql}");
        }
        db.close().unwrap();
    }
}
