//! Stock public owners sampled only while both reader threads are parked.
use super::{DENY, LIVE_REQUESTED, LIVE_USABLE};
use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, Error, QueryResult, QueryStep, Value,
};
use std::path::Path;
use std::sync::Barrier;
use std::sync::atomic::{AtomicU64, Ordering};

const ROWS: usize = 4096;
const MEMORY: u64 = 4_000_000;
const TEMP: u64 = 8_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Live {
    requested: usize,
    usable: usize,
    descriptors: usize,
}

impl Live {
    fn now() -> Self {
        // Directory enumeration ends before sampling heap ownership. Its own
        // descriptor is included consistently, and no listing storage survives.
        let descriptors = std::fs::read_dir("/dev/fd").unwrap().count();
        Self {
            requested: LIVE_REQUESTED.load(Ordering::Relaxed),
            usable: LIVE_USABLE.load(Ordering::Relaxed),
            descriptors,
        }
    }
}
fn checkpoint(db: &Database, label: &str, expected: u64, baseline: Live) -> Live {
    let live = Live::now();
    assert_eq!(db.reserved_memory_bytes(), expected, "{label}: owner sum");
    assert!(expected <= MEMORY);
    assert!(db.reserved_temp_bytes() <= TEMP);
    assert!(live.requested <= live.usable);
    println!(
        "ownership {label} charge={expected} temp={} requested={} usable={} descriptors={} requested-change={} rounding={}",
        db.reserved_temp_bytes(),
        live.requested,
        live.usable,
        live.descriptors,
        live.requested as i64 - baseline.requested as i64,
        live.usable - live.requested,
    );
    live
}
fn consume(result: &mut QueryResult<'_, '_>, distinct: bool, extra: bool, negative: bool) {
    let mut seen = [0_u8; ROWS / 2 + 1];
    for _ in 0..200_000 {
        match result.step() {
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let Some(Value::Int64(key)) = batch.value(row, 0) else {
                        panic!("integer fixture output");
                    };
                    let key = usize::try_from(key).unwrap();
                    assert!(key < ROWS / 2 + usize::from(extra));
                    seen[key] = seen[key].checked_add(1).unwrap();
                }
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                assert!(
                    seen[..ROWS / 2]
                        .iter()
                        .all(|&n| n == if distinct { 1 } else { 2 })
                );
                assert_eq!(
                    seen[ROWS / 2],
                    u8::from(extra && !negative),
                    "complete-row oracle"
                );
                return;
            }
            QueryStep::Failed(error) => panic!("unexpected query failure: {error:?}"),
        }
    }
    panic!("finite fixture step bound");
}

pub(super) fn run(root: &Path, negative: bool) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(root)?;
    let path = root.join("database");
    let cancel = CancellationToken::new();
    let cancelled = CancellationToken::new();
    println!("entered composed public ownership probe");
    let outside = Live::now();
    let db = Database::create_empty(&path, Config::new(MEMORY, TEMP)?)?;
    db.declare_table(
        "facts",
        &[ColumnDeclaration {
            name: "k",
            data_type: DataType::Int64,
            nullable: false,
        }],
        &cancel,
    )?;
    let limits = AppendLimits {
        batches: 1,
        encoded_bytes: 100_000,
    };
    let mut writer = db.begin_append("facts", limits, &cancel)?;
    let input: [i64; ROWS] = std::array::from_fn(|i| (i / 2) as i64);
    writer.write(
        &[ColumnInput {
            values: ColumnValues::Int64(&input),
            validity: &[255; ROWS / 8],
        }],
        &cancel,
    )?;
    writer.commit(&cancel)?;
    let resident = db.reserved_memory_bytes();
    let ordered = db.prepare("FROM facts |> ORDER BY k")?;
    let distinct = db.prepare("FROM facts |> DISTINCT")?;
    let prepared = resident + ordered.accounted_memory_bytes() + distinct.accounted_memory_bytes();
    let charges = [AtomicU64::new(0), AtomicU64::new(0)];
    let ready = Barrier::new(3);
    let resume = Barrier::new(3);
    std::thread::scope(|scope| {
        let mut workers = [None, None];
        for (id, query, token) in [(0, &ordered, &cancelled), (1, &distinct, &cancel)] {
            let db = &db;
            let charges = &charges;
            let ready = &ready;
            let resume = &resume;
            workers[id] = Some(scope.spawn(move || {
                let mut result = None;
                ready.wait();
                resume.wait();
                for phase in 0..8 {
                    match phase {
                        2 | 3 if phase == id + 2 => {
                            let prior = db.reserved_temp_bytes();
                            let mut rows = db.execute(query, token).unwrap();
                            let mut spilled = false;
                            for _ in 0..200_000 {
                                assert!(matches!(rows.step(), QueryStep::Progress));
                                if db.reserved_temp_bytes() > prior {
                                    spilled = true;
                                    break;
                                }
                            }
                            assert!(spilled, "reader owns scratch before parking");
                            result = Some(rows);
                        }
                        4 if id == 0 => {
                            let rows = result.as_mut().unwrap();
                            assert!(matches!(rows.step(), QueryStep::Failed(Error::Cancelled)));
                        }
                        6 if id == 1 => consume(result.as_mut().unwrap(), true, false, false),
                        7 => {
                            result = None;
                        }
                        _ => (),
                    }
                    charges[id].store(
                        result
                            .as_ref()
                            .map_or(0, QueryResult::accounted_memory_bytes),
                        Ordering::Release,
                    );
                    ready.wait();
                    resume.wait();
                }
            }));
        }
        let mut writer = None;
        let mut writer_charge = 0;
        let mut baseline = None;
        // Complete one rendezvous before sampling: native barrier storage can
        // initialize lazily, and belongs to the caller throughout this interval.
        ready.wait();
        resume.wait();
        for phase in 0..8 {
            ready.wait();
            // Reader threads have published their owners and cannot allocate or
            // mutate them until the matching resume barrier.
            let readers: u64 = charges.iter().map(|n| n.load(Ordering::Acquire)).sum();
            if phase == 0 {
                baseline = Some(Live::now());
            }
            let base = baseline.unwrap();
            if phase == 1 {
                let before = db.reserved_memory_bytes();
                let mut append = db.begin_append("facts", limits, &cancel).unwrap();
                append
                    .write(
                        &[ColumnInput {
                            values: ColumnValues::Int64(&[(ROWS / 2) as i64]),
                            validity: &[1],
                        }],
                        &cancel,
                    )
                    .unwrap();
                writer_charge = db.reserved_memory_bytes() - before;
                writer = Some(append);
            }
            if phase == 3 {
                let before = Live::now();
                let memory = db.reserved_memory_bytes();
                let temporary = db.reserved_temp_bytes();
                DENY.store(true, Ordering::Relaxed);
                let refusal = db.execute(&distinct, &cancel);
                DENY.store(false, Ordering::Relaxed);
                assert!(matches!(refusal, Err(Error::Resource { .. })));
                drop(refusal);
                assert_eq!(Live::now(), before);
                assert_eq!(db.reserved_memory_bytes(), memory);
                assert_eq!(db.reserved_temp_bytes(), temporary);
                cancelled.cancel();
            }
            if phase == 5 {
                writer.take().unwrap().commit(&cancel).unwrap();
                writer_charge = 0;
                let before = Live::now();
                let temporary = db.reserved_temp_bytes();
                let memory = db.reserved_memory_bytes();
                let refused = db.begin_append(
                    "facts",
                    AppendLimits {
                        batches: 1,
                        encoded_bytes: TEMP + 1,
                    },
                    &cancel,
                );
                assert!(matches!(refused, Err(Error::Resource { .. })));
                drop(refused);
                assert_eq!(db.reserved_memory_bytes(), memory);
                assert_eq!(db.reserved_temp_bytes(), temporary);
                assert_eq!(Live::now(), before);
                // Resource refusal must release writer authority too.
                db.begin_append("facts", limits, &cancel)
                    .unwrap()
                    .abort()
                    .unwrap();
            }
            let observed = checkpoint(
                &db,
                [
                    "baseline",
                    "writer",
                    "reader-order",
                    "reader-distinct",
                    "cancelled",
                    "published",
                    "finished",
                    "released",
                ][phase],
                prepared + writer_charge + readers,
                base,
            );
            assert_eq!(
                observed.descriptors,
                base.descriptors + [0, 0, 4, 8, 4, 4, 0, 0][phase]
            );
            let requested = observed.requested.checked_sub(base.requested).unwrap();
            assert!(requested as u64 <= writer_charge + readers);
            if phase == 7 {
                assert_eq!(db.reserved_temp_bytes(), 0);
                assert_eq!(Live::now(), base);
            }
            resume.wait();
        }
        for worker in workers {
            worker.unwrap().join().unwrap();
        }
    });
    let before_barrier_drop = Live::now();
    drop(ready);
    drop(resume);
    let after_barrier_drop = Live::now();
    println!(
        "ownership caller-barriers requested={} usable={}",
        before_barrier_drop.requested - after_barrier_drop.requested,
        before_barrier_drop.usable - after_barrier_drop.usable
    );
    consume(&mut db.execute(&ordered, &cancel)?, false, false, false);
    let latest = db.prepare("FROM facts |> DISTINCT")?;
    // A deliberate false expectation must fail after public query completion.
    // The negative control omits the committed key from its expected domain.
    consume(&mut db.execute(&latest, &cancel)?, true, true, negative);
    drop(latest);
    let grouped = db.prepare("FROM facts |> AGGREGATE COUNT(*) AS n GROUP BY k |> SELECT k")?;
    let baseline = Live::now();
    let prior = db.reserved_memory_bytes();
    let mut rows = db.execute(&grouped, &cancel)?;
    let running = rows.accounted_memory_bytes();
    let held = checkpoint(&db, "hash-held", prior + running, baseline);
    assert!((held.requested - baseline.requested) as u64 <= running);
    let denied = db.execute(&distinct, &cancel);
    assert!(matches!(denied, Err(Error::Resource { .. })));
    drop(denied);
    assert_eq!(Live::now(), held);
    assert_eq!(db.reserved_memory_bytes(), prior + running);
    consume(&mut rows, true, true, false);
    drop(rows);
    assert_eq!(Live::now(), baseline);
    assert_eq!(db.reserved_memory_bytes(), prior);
    drop(grouped);
    drop(ordered);
    drop(distinct);
    assert_eq!(db.reserved_memory_bytes(), resident);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close()?;
    assert_eq!(Live::now(), outside);
    println!(
        "ownership overlap passed: old/new rows, allocation/memory/temp refusal, cancellation, publication and owner release"
    );
    Ok(())
}
