//! Refuse native allocations through sorting and replay of running-SUM peers.
//!
//! Capture typed rows in caller storage allocated before the census. The supervisor
//! owns expected totals and prefix checks. Denial remains active through terminal
//! repetition, owned-error extraction, formatting and release. Linux spill lengths
//! are inspected between calls, outside both denial and event observation.

use super::{
    measurement::Live,
    transient_ownership::{Observer, Samples},
    workload,
};
use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, Error, PreparedQuery, QueryStep, Value,
};
use std::{io::Write, path::Path};

const ROWS: usize = 8200;
const PEERS: usize = 1025;
const ALLOCATIONS: usize = 1024;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Control {
    Healthy,
    Cancel,
    MissingTerminal,
}

const SQL: &str = "FROM facts |> EXTEND SUM(amount) OVER (PARTITION BY part ORDER BY peer) AS total |> SELECT id, part, peer, total";

#[derive(Clone, Copy, Default)]
struct Row {
    id: i64,
    part: Option<i64>,
    peer: Option<i64>,
    total: Option<i64>,
}

struct Attempt {
    rows: usize,
    error: Option<Error>,
    spill: u64,
    temporary: u64,
    samples: Samples,
    terminal_frees: usize,
}

impl Attempt {
    fn observe(&mut self, samples: Samples) {
        self.samples.allocations += samples.allocations;
        self.samples.frees += samples.frees;
        self.samples.requested_headroom = self
            .samples
            .requested_headroom
            .min(samples.requested_headroom);
        self.samples.usable_headroom = self.samples.usable_headroom.min(samples.usable_headroom);
    }
}

fn create(path: &Path) -> Database {
    let db = Database::create_empty(path, Config::new(8_000_000, 8_000_000).unwrap()).unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "facts",
        &["id", "part", "peer", "amount"].map(|name| ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: name != "id",
        }),
        &cancel,
    )
    .unwrap();
    let mut append = db
        .begin_append(
            "facts",
            AppendLimits {
                batches: 33,
                encoded_bytes: 500_000,
            },
            &cancel,
        )
        .unwrap();
    for start in (0..ROWS).step_by(256) {
        let length = (ROWS - start).min(256);
        let ids: Vec<_> = (start..start + length)
            .map(|row| (ROWS - 1 - row) as i64)
            .collect();
        let mut parts = Vec::with_capacity(length);
        let mut peers = Vec::with_capacity(length);
        let mut amounts = Vec::with_capacity(length);
        let mut present = [[0_u8; 32]; 4];
        for (row, &id) in ids.iter().enumerate() {
            let group = id as usize / PEERS;
            let within = id as usize % PEERS;
            parts.push(group as i64 / 2 - 2);
            peers.push(group as i64 % 2);
            let amount = match (group, within) {
                (0, 0..256) => Some(i64::MAX),
                (0, 256..512) => Some(-i64::MAX),
                (0, 1024) => Some(7),
                (1, 0) => Some(9),
                (3, 0) => Some(i64::MAX),
                (4, 0) => Some(i64::MIN),
                (4, 1) => Some(-1),
                (4, 2) => Some(1),
                (5, 0) => Some(i64::MAX),
                (5, 1) => Some(1),
                (6, 0..512) => Some(1),
                (7, 0..512) => Some(-1),
                _ => None,
            };
            amounts.push(amount.unwrap_or(0));
            for (column, valid) in [true, group >= 2, !group.is_multiple_of(2), amount.is_some()]
                .into_iter()
                .enumerate()
            {
                if valid {
                    present[column][row / 8] |= 1 << (row % 8);
                }
            }
        }
        append
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&ids),
                        validity: &present[0][..length.div_ceil(8)],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&parts),
                        validity: &present[1][..length.div_ceil(8)],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&peers),
                        validity: &present[2][..length.div_ceil(8)],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&amounts),
                        validity: &present[3][..length.div_ceil(8)],
                    },
                ],
                &cancel,
            )
            .unwrap();
    }
    append.commit(&cancel).unwrap();
    db
}

#[cfg(target_os = "linux")]
fn written_spill(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|entry| {
            let descriptor = entry.unwrap().path();
            let target = match std::fs::read_link(&descriptor) {
                Ok(target) => target,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
                Err(error) => panic!("read window spill descriptor: {error}"),
            };
            if !target.starts_with(path) {
                return None;
            }
            let metadata = std::fs::metadata(descriptor).unwrap();
            (metadata.is_file() && metadata.nlink() == 0).then_some(metadata.len())
        })
        .sum()
}

#[cfg(not(target_os = "linux"))]
fn written_spill(_: &Path) -> u64 {
    0
}

fn cell(value: Option<Value<'_>>) -> Option<i64> {
    match value {
        Some(Value::Int64(value)) => Some(value),
        Some(Value::Null) => None,
        _ => panic!("window capture lost INT64 type"),
    }
}

fn attempt(
    db: &Database,
    query: &PreparedQuery<'_>,
    path: &Path,
    records: &mut [Row],
    control: Control,
) -> Attempt {
    let baseline = workload::live();
    let memory = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let observer = Observer::new(db, memory, baseline.requested, baseline.usable);
    let mut attempt = Attempt {
        rows: 0,
        error: None,
        spill: 0,
        temporary: 0,
        terminal_frees: 0,
        samples: Samples {
            allocations: 0,
            frees: 0,
            requested_headroom: i128::MAX,
            usable_headroom: i128::MAX,
        },
    };
    let execution = observer.during(|| db.execute(query, &cancel));
    attempt.observe(observer.samples());
    let mut result = match execution {
        Ok(result) => result,
        Err(error) => {
            attempt.error = Some(error);
            return attempt;
        }
    };
    let mut terminal = false;
    let mut failed = None;
    for _ in 0..500_000 {
        let observer = Observer::new(db, memory, baseline.requested, baseline.usable);
        let step = if control == Control::MissingTerminal && attempt.rows > 0 {
            result.step()
        } else {
            observer.during(|| result.step())
        };
        match step {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 4);
                assert!(!batch.is_empty());
                for row in 0..batch.len() {
                    assert!(attempt.rows < records.len(), "window returned extra rows");
                    records[attempt.rows] = Row {
                        id: cell(batch.value(row, 0)).expect("required window id"),
                        part: cell(batch.value(row, 1)),
                        peer: cell(batch.value(row, 2)),
                        total: cell(batch.value(row, 3)),
                    };
                    attempt.rows += 1;
                }
                if control != Control::Healthy {
                    super::ALLOW.store(
                        super::CALLS.load(std::sync::atomic::Ordering::Relaxed),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    super::DENY.store(true, std::sync::atomic::Ordering::Relaxed);
                    cancel.cancel();
                }
            }
            QueryStep::Finished => {
                terminal = true;
            }
            QueryStep::Failed(error) => {
                assert!(workload::format_error(Some(error)));
                failed = Some(std::mem::discriminant(error));
                terminal = true;
            }
        }
        attempt.observe(observer.samples());
        attempt.temporary = attempt.temporary.max(db.reserved_temp_bytes());
        if terminal {
            attempt.terminal_frees = observer.samples().frees;
            break;
        }
        if cfg!(target_os = "linux") && attempt.spill == 0 && attempt.temporary > 0 {
            // File discovery allocates as observer work. Exclude it from the
            // refusal prefix and restore denial before the next engine call.
            let faults = workload::suspend_faults();
            attempt.spill = written_spill(path);
            workload::resume_faults(faults);
        }
    }
    assert!(terminal, "window did not reach a terminal step");
    let observer = Observer::new(db, memory, baseline.requested, baseline.usable);
    match observer.during(|| result.step()) {
        QueryStep::Finished => assert!(failed.is_none()),
        QueryStep::Failed(error) => {
            assert_eq!(failed, Some(std::mem::discriminant(error)));
            assert!(workload::format_error(Some(error)));
        }
        _ => panic!("window lost its terminal outcome"),
    }
    assert_eq!(
        (observer.samples().allocations, observer.samples().frees),
        (0, 0)
    );
    let observer = Observer::new(db, memory, baseline.requested, baseline.usable);
    attempt.error = observer.during(|| result.into_error());
    attempt.observe(observer.samples());
    assert_eq!(attempt.error.is_some(), failed.is_some());
    assert!(workload::format_error(attempt.error.as_ref()));
    assert_eq!(
        workload::live(),
        baseline,
        "window retained heap with owned outcome"
    );
    assert_eq!(db.reserved_memory_bytes(), memory);
    assert_eq!(db.reserved_temp_bytes(), 0);
    attempt
}

fn save(path: &Path, rows: &[Row]) {
    let mut file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    for row in rows {
        write!(file, "{}", row.id).unwrap();
        for value in [row.part, row.peer, row.total] {
            match value {
                Some(value) => write!(file, "|{value}"),
                None => write!(file, "|null"),
            }
            .unwrap();
        }
        writeln!(file).unwrap();
    }
    file.flush().unwrap();
}

pub(super) fn run(root: &Path, after: Option<usize>, control: Control) {
    std::fs::create_dir(root).unwrap();
    let path = root.join("database");
    println!("entered window refusal: 8200 rows, four partitions, 1025-row peers");
    let before = Live::now();
    create(&path).close().unwrap();
    let config = Config::new(1_800_000, 8_000_000).unwrap();
    let db = Database::open(&path, config).unwrap();
    for name in ["CONTROL", "ROOT.A", "ROOT.B", "WAL"] {
        std::fs::copy(path.join(name), root.join(name)).unwrap();
    }
    let query = db.prepare(SQL).unwrap();
    assert_eq!(query.result_column_count(), 4);
    for (column, name) in ["id", "part", "peer", "total"].into_iter().enumerate() {
        let descriptor = query.result_column(column).unwrap();
        assert_eq!(descriptor.name, Some(name));
        assert_eq!(descriptor.data_type, DataType::Int64);
        assert_eq!(descriptor.nullable, column != 0);
    }
    println!(
        "window schema=id:int64:required|part:int64:nullable|peer:int64:nullable|total:int64:nullable"
    );
    let generation = db.generation();
    let mut initial = vec![Row::default(); ROWS];
    let mut retry = vec![Row::default(); ROWS];
    let baseline = workload::arm(after, ALLOCATIONS);
    let outcome = attempt(&db, &query, &path, &mut initial, control);
    assert!(outcome.samples.requested_headroom >= 0 && outcome.samples.usable_headroom >= 0);
    let label = match &outcome.error {
        None => {
            assert_eq!(outcome.rows, ROWS);
            "finished"
        }
        Some(Error::Cancelled) if control != Control::Healthy => {
            assert!(outcome.rows > 0 && outcome.rows < ROWS);
            let mut canary = Vec::<u8>::new();
            assert!(canary.try_reserve_exact(1).is_err());
            "cancelled"
        }
        Some(Error::Resource { .. }) => "refused",
        Some(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::OutOfMemory => {
            "refused"
        }
        Some(Error::RecoveryRequired {
            generation: observed,
            source,
        }) if *observed == generation && workload::allocation_cause(source.kind()) => "recovery",
        Some(error) => panic!("unexpected window allocation error: {error}"),
    };
    workload::finish("window", baseline);
    println!(
        "window attempt outcome={label} rows={} spill={} temp={}",
        outcome.rows, outcome.spill, outcome.temporary
    );
    if outcome.error.is_none() || control != Control::Healthy {
        assert!(
            outcome.terminal_frees > 0,
            "window terminal release lacked free events"
        );
    }
    println!("window terminal frees={}", outcome.terminal_frees);
    println!("window events={:?}", outcome.samples);
    save(&root.join("initial.rows"), &initial[..outcome.rows]);
    let healthy = if label == "recovery" {
        let blocked = attempt(&db, &query, &path, &mut retry, Control::Healthy);
        assert_eq!(blocked.rows, 0);
        assert!(
            matches!(blocked.error, Some(Error::RecoveryRequired { generation: observed, .. }) if observed == generation)
        );
        drop(query);
        db.close().unwrap();
        let db = Database::open(&path, config).unwrap();
        assert_eq!(db.generation(), generation);
        let query = db.prepare(SQL).unwrap();
        let healthy = attempt(&db, &query, &path, &mut retry, Control::Healthy);
        drop(query);
        db.close().unwrap();
        healthy
    } else {
        let healthy = attempt(&db, &query, &path, &mut retry, Control::Healthy);
        assert_eq!(db.generation(), generation);
        drop(query);
        db.close().unwrap();
        healthy
    };
    assert!(healthy.error.is_none());
    assert_eq!(healthy.rows, ROWS);
    assert!(healthy.samples.allocations > 0 && healthy.samples.frees > 0);
    assert!(
        healthy.terminal_frees > 0,
        "window retry terminal release lacked free events"
    );
    assert!(healthy.samples.requested_headroom >= 0 && healthy.samples.usable_headroom >= 0);
    assert!(healthy.temporary > 0, "window retry did not reserve spill");
    if cfg!(target_os = "linux") {
        assert!(healthy.spill > 0, "window retry did not write spill");
    }
    println!(
        "window retry rows={} spill={} temp={}",
        healthy.rows, healthy.spill, healthy.temporary
    );
    println!("window retry-events={:?}", healthy.samples);
    save(&root.join("retry.rows"), &retry[..healthy.rows]);
    assert!(workload::format_error(outcome.error.as_ref()));
    drop(initial);
    drop(retry);
    drop(outcome);
    drop(healthy);
    assert_eq!(
        Live::now(),
        before,
        "window retained a database, plan or descriptor"
    );
    println!("window release and complete retry passed");
}
