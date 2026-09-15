//! Check public outcomes under observed native I/O errors and short transfers.
//!
//! Setup finishes before arming the companion observer. Legacy calls check honest
//! publication/reopen outcomes; composed queries also check literal results and
//! complete memory/temp release before reuse. Positive short transfers must finish,
//! while refusals must reach the caller. Standard-library calls are interception
//! controls. `check-native-io.py` selects call positions and owns each process/root.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues,
    CommitResolution, Config, DataType, Database, Error, QueryResult, QueryStep, Value,
};
use std::io::{Read, Seek, Write};
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
unsafe extern "C" {
    fn io_probe_start(kind: u32, at: u32, count: u32, error: i32);
    fn io_probe_stop();
    fn io_probe_calls() -> u32;
    fn io_probe_refused() -> u32;
    fn io_probe_partial() -> u32;
}
fn consume(mut rows: QueryResult<'_, '_>) -> Result<(), Error> {
    for _ in 0..10_000 {
        match rows.step() {
            QueryStep::Rows(batch) => assert!(!batch.is_empty()),
            QueryStep::Progress => (),
            QueryStep::Finished => return Ok(()),
            QueryStep::Failed(_) => return Err(rows.into_error().expect("terminal query error")),
        }
    }
    panic!("query exceeded finite fixture step allowance");
}
fn composition_query(db: &Database, derived: bool) -> Result<(), Error> {
    let sql = if derived {
        "FROM (FROM facts |> WHERE category IS NULL OR category >= 'A' |> AGGREGATE SUM(n) AS total GROUP BY k) AS a |> LEFT JOIN (FROM facts |> WHERE k NOT BETWEEN 2 AND 3 |> AGGREGATE SUM(n) AS total GROUP BY k) AS b ON a.k = b.k |> EXTEND COUNT(*) OVER () AS partition_rows |> WHERE partition_rows=2 |> AGGREGATE AVG(a.total+b.total) AS mean"
    } else {
        "FROM facts |> AGGREGATE SUM(n) AS total GROUP BY k |> AGGREGATE SUM(total) AS subtotal GROUP BY total |> AGGREGATE AVG(subtotal) AS mean"
    };
    // The derived join retains two groups. Only key 1 contributes 30 + 30
    // to AVG; key 2 has a NULL right total and is skipped by AVG.
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
    // The repeated query fits its blocking minima but spills at 1.1 MB.
    // The census must observe reads and writes, so admission changes cannot
    // silently turn this failure campaign into an in-memory-only test.
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
        assert!(result.is_err(), "native refusal must propagate");
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

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 7);
    let root = PathBuf::from(&args[1]);
    let mode = args[2].as_str();
    let kind = args[3].parse::<u32>().unwrap();
    let at = args[4].parse::<u32>().unwrap();
    let burst = args[5].parse::<u32>().unwrap();
    let error = args[6].parse::<i32>().unwrap();
    if matches!(mode, "repeated" | "derived") {
        composition_io(&root, kind, at, burst, error, mode == "derived");
        return;
    }
    let db_path = root.join("database");
    let input = root.join("input.tbl");
    std::fs::write(
        &input,
        b"1|2|3|4|17.00|21168.23|0.04|8|R|F|1996-03-13|12|13|14|15|16|\n",
    )
    .unwrap();
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
        std::fs::remove_file(db_path.join("ROOT.B")).unwrap();
    }
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(root.join("file"))
        .unwrap();
    file.write_all(&[42; 8]).unwrap();
    file.rewind().unwrap();
    let mut buffer = [0; 8];
    // SAFETY: this single-threaded fixture exclusively owns bounded observer state.
    // No caller pointers are supplied to its control functions.
    unsafe { io_probe_start(kind, at, burst, error) };
    let result = match mode {
        "standard" => match kind {
            0 => file.read_exact(&mut buffer),
            1 => file.write_all(&[42; 8]),
            2 => file.read_exact_at(&mut buffer, 0),
            3 => file.write_all_at(&[42; 8], 0),
            _ => panic!("invalid I/O kind"),
        }
        .map_err(|source| Error::Io {
            operation: "standard I/O reference",
            source,
        }),
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
        "q6" | "q1" => {
            let db = database.as_ref().unwrap();
            let sql = if mode == "q6" {
                include_str!("../../tests/fixtures/q6.pipe.sql")
            } else {
                include_str!("../../tests/fixtures/upstream/q1-upstream.pipe.sql")
            };
            let plan = db.prepare(sql).unwrap();
            let cancellation = CancellationToken::new();
            db.execute(&plan, &cancellation).and_then(consume)
        }
        _ => panic!("unknown native I/O mode"),
    };
    // SAFETY: the observed operation has returned; no native I/O remains live.
    unsafe { io_probe_stop() };
    let (calls, refused, partial) =
        unsafe { (io_probe_calls(), io_probe_refused(), io_probe_partial()) };
    println!("calls={calls} refused={refused} partial={partial} result={result:?}");
    if at == 0 || error == 0 {
        assert!(result.is_ok());
    } else if mode != "standard" {
        assert!(
            result.is_err(),
            "native I/O refusal must propagate without retry"
        );
    }
    drop(database);
    if matches!(mode, "load" | "recover" | "q6" | "q1") {
        let mut healed = Database::open(&db_path, config).unwrap();
        if let Some(transaction) = token {
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
