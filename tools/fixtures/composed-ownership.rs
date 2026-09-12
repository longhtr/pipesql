//! Stock public owners sampled only while both reader threads are parked.
use super::{DENY, LIVE_REQUESTED, LIVE_USABLE};
use pipesql::{
    Append, AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, Error, PreparedQuery, QueryResult, QueryStep, Value,
};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Barrier, Mutex};

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

// These observations describe live allocations at parked boundaries, not peaks.
// Only one owner changes during each measured interval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Heap {
    requested: usize,
    usable: usize,
}

impl Heap {
    fn now() -> Self {
        Self {
            requested: LIVE_REQUESTED.load(Ordering::Relaxed),
            usable: LIVE_USABLE.load(Ordering::Relaxed),
        }
    }

    fn increase_from(self, before: Self) -> Self {
        Self {
            requested: self.requested.checked_sub(before.requested).unwrap(),
            usable: self.usable.checked_sub(before.usable).unwrap(),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Owner {
    charge: u64,
    heap: Heap,
}

impl Owner {
    fn report(self, label: &str, nonheap_reservation: u64) {
        assert!(self.heap.requested <= self.heap.usable);
        assert_eq!(
            self.charge,
            self.heap.requested as u64 + nonheap_reservation,
            "execution ownership attribution: {label}"
        );
        println!(
            "owner {label} charge={} requested={} usable={} nonheap-reservation={} rounding={}",
            self.charge,
            self.heap.requested,
            self.heap.usable,
            self.charge - self.heap.requested as u64,
            self.heap.usable - self.heap.requested,
        );
    }
}

fn prepare_observed<'db>(
    db: &'db Database,
    sql: &str,
    label: &str,
    retained_allocations: usize,
    wrong_allowance: bool,
) -> Result<PreparedQuery<'db>, Error> {
    let before = Live::now();
    let query = db.prepare(sql)?;
    let after = Live::now();
    let requested = after.requested.checked_sub(before.requested).unwrap();
    let usable = after.usable.checked_sub(before.usable).unwrap();
    let inline = std::mem::size_of::<PreparedQuery<'_>>();
    // Independent equation from the preparation contract: the retained plan
    // and each descriptor allocation receive 4,096 bytes of logical allowance.
    // Catalog/name-scope scratch has already dropped at this checkpoint.
    let allowance = retained_allocations * 4096 + usize::from(wrong_allowance);
    assert_eq!(
        query.accounted_memory_bytes(),
        (requested + inline + allowance) as u64,
        "prepared ownership attribution"
    );
    println!(
        "ownership prepared-{label} charge={} requested={requested} usable={usable} inline={inline} allowance={allowance} rounding={}",
        query.accounted_memory_bytes(),
        usable.checked_sub(requested).unwrap(),
    );
    Ok(query)
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

fn refuse_competing_readers(db: &Database, query: &PreparedQuery<'_>, cancel: &CancellationToken) {
    let before = Live::now();
    let memory = db.reserved_memory_bytes();
    let temporary = db.reserved_temp_bytes();
    // Sixteen native INT64 buffers alone exceed the four-million-byte budget.
    // Fixed caller slots avoid adding an allocation to the measured interval.
    let mut held = std::array::from_fn::<_, 16, _>(|_| None);
    let mut admitted = 0;
    let mut refused = false;
    for slot in &mut held {
        match db.execute(query, cancel) {
            Ok(rows) => {
                *slot = Some(rows);
                admitted += 1;
            }
            Err(Error::Resource {
                required, limit, ..
            }) => {
                assert_eq!(limit, MEMORY);
                assert!(required > limit, "logical admission must refuse the charge");
                refused = true;
                break;
            }
            Err(error) => panic!("unexpected competing-reader error: {error:?}"),
        }
    }
    assert!(refused, "competing readers must exhaust logical admission");
    drop(held);
    assert_eq!(Live::now(), before);
    assert_eq!(db.reserved_memory_bytes(), memory);
    assert_eq!(db.reserved_temp_bytes(), temporary);
    println!("ownership logical-refusal admitted={admitted}; existing owners preserved");
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

pub(super) fn run(
    root: &Path,
    negative: bool,
    wrong_allowance: bool,
) -> Result<(), Box<dyn std::error::Error>> {
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
    // The scan retains an exact-capacity canonical "database/units" path.
    // The source reserves two maximum paths, including transient name work.
    let scan_path_bytes = std::fs::canonicalize(&path)?.as_os_str().len() + "/units".len();
    let ordered = prepare_observed(&db, "FROM facts |> ORDER BY k", "order", 1, wrong_allowance)?;
    let distinct = prepare_observed(&db, "FROM facts |> DISTINCT", "distinct", 2, false)?;
    let prepared = resident + ordered.accounted_memory_bytes() + distinct.accounted_memory_bytes();
    let observations = [Mutex::new(Owner::default()), Mutex::new(Owner::default())];
    let ready = Barrier::new(3);
    let resume = Barrier::new(3);
    std::thread::scope(|scope| {
        let mut workers = [None, None];
        for (id, query, token) in [(0, &ordered, &cancelled), (1, &distinct, &cancel)] {
            let db = &db;
            let observations = &observations;
            let ready = &ready;
            let resume = &resume;
            workers[id] = Some(scope.spawn(move || {
                let mut result = None;
                let mut heap = Heap::default();
                ready.wait();
                resume.wait();
                for phase in 0..8 {
                    match phase {
                        2 | 3 if phase == id + 2 => {
                            let before = Heap::now();
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
                            heap = Heap::now().increase_from(before);
                        }
                        4 if id == 0 => {
                            let before = Heap::now();
                            let rows = result.as_mut().unwrap();
                            assert!(matches!(rows.step(), QueryStep::Failed(Error::Cancelled)));
                            assert_eq!(before.increase_from(Heap::now()), heap);
                            heap = Heap::default();
                        }
                        6 if id == 1 => {
                            let before = Heap::now();
                            consume(result.as_mut().unwrap(), true, false, false);
                            assert_eq!(before.increase_from(Heap::now()), heap);
                            heap = Heap::default();
                        }
                        7 => {
                            result = None;
                        }
                        _ => (),
                    }
                    *observations[id].lock().unwrap() = Owner {
                        charge: result
                            .as_ref()
                            .map_or(0, QueryResult::accounted_memory_bytes),
                        heap,
                    };
                    ready.wait();
                    resume.wait();
                }
            }));
        }
        let mut writer = None;
        let mut writer_owner = Owner::default();
        let mut baseline = None;
        // Complete one rendezvous before sampling: native barrier storage can
        // initialize lazily, and belongs to the caller throughout this interval.
        ready.wait();
        resume.wait();
        for phase in 0..8 {
            ready.wait();
            // Reader threads have published their owners and cannot allocate or
            // mutate them until the matching resume barrier.
            let readers = observations.each_ref().map(|owner| *owner.lock().unwrap());
            let reader_charge: u64 = readers.iter().map(|owner| owner.charge).sum();
            if phase == 0 {
                baseline = Some(Live::now());
            }
            let base = baseline.unwrap();
            if phase == 1 {
                let before_heap = Heap::now();
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
                writer_owner = Owner {
                    charge: db.reserved_memory_bytes() - before,
                    heap: Heap::now().increase_from(before_heap),
                };
                writer = Some(append);
            }
            if phase == 3 {
                refuse_competing_readers(&db, &distinct, &cancel);
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
                let before_heap = Heap::now();
                writer.take().unwrap().commit(&cancel).unwrap();
                assert_eq!(before_heap.increase_from(Heap::now()), writer_owner.heap);
                writer_owner = Owner::default();
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
                prepared + writer_owner.charge + reader_charge,
                base,
            );
            assert_eq!(
                observed.descriptors,
                base.descriptors + [0, 0, 4, 8, 4, 4, 0, 0][phase]
            );
            let requested = observed.requested.checked_sub(base.requested).unwrap();
            let expected_requested = writer_owner.heap.requested
                + readers
                    .iter()
                    .map(|owner| owner.heap.requested)
                    .sum::<usize>();
            let expected_usable = writer_owner.heap.usable
                + readers.iter().map(|owner| owner.heap.usable).sum::<usize>();
            assert_eq!(
                requested, expected_requested,
                "requested owner reconciliation"
            );
            assert_eq!(
                observed.usable - base.usable,
                expected_usable,
                "usable owner reconciliation"
            );
            // Append paths are transient here; only the inline handle and both
            // path allowances remain outside retained allocation requests.
            writer_owner.report(
                "writer",
                if writer.is_some() {
                    (std::mem::size_of::<Append<'_>>() + 2 * 4096) as u64
                } else {
                    0
                },
            );
            for (reader, name) in readers.iter().zip(["order", "distinct"]) {
                let nonheap = if reader.charge == 0 {
                    0
                } else {
                    let inline = std::mem::size_of::<QueryResult<'_, '_>>();
                    // A terminal query retains only its inline handle. Running
                    // queries also reserve physical-plan allocation allowance
                    // and two source paths, minus the one retained path request.
                    let running = if reader.heap.requested == 0 {
                        0
                    } else {
                        4096 + 2 * 4096 - scan_path_bytes
                    };
                    (inline + running) as u64
                };
                reader.report(name, nonheap);
            }
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
    let before_sync_drop = Live::now();
    drop(ready);
    drop(resume);
    drop(observations);
    let after_sync_drop = Live::now();
    println!(
        "ownership caller-synchronization requested={} usable={}",
        before_sync_drop.requested - after_sync_drop.requested,
        before_sync_drop.usable - after_sync_drop.usable
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
    // A small grouped query must leave room for the independently admitted reader.
    // Check complete rows from both snapshots rather than only successful opening.
    let mut competing = db
        .execute(&distinct, &cancel)
        .expect("small grouped query unnecessarily excluded a competing reader");
    consume(&mut competing, true, false, false);
    drop(competing);
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
