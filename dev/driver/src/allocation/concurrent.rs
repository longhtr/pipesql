//! Attribute overlapping readers and a writer under one controlled schedule.
//!
//! Participants pause while the coordinator measures each owner. Panic wakes all
//! peers; their joins must finish before the shared database and inputs are dropped.
//! This schedule checks ownership transitions, not arbitrary thread interleavings.
//!
//! Old and newly prepared readers must see their respective snapshots across the
//! writer's publication. Checkpoints compare reservations, live heap bytes and
//! open descriptors only while the participating owners are held still.

use super::measurement::{Heap, Live, Owner, prepare_observed};
use pipesql::{
    Append, AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, Error, PreparedQuery, QueryResult, QueryStep, Value,
};
use std::{path::Path, sync::atomic::Ordering};

use super::{DENY, measurement::native_buffer_allowance};
use std::sync::{Condvar, Mutex};

const ROWS: usize = 4096;
const MEMORY: u64 = 4_000_000;
const TEMP: u64 = 8_000_000;

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
    // Sixteen readers cannot fit in four million bytes. Store their handles on
    // the stack so the test adds no heap allocation while filling the budget.
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

// Unlike Barrier, this three-party rendezvous releases its peers on panic.
// Mutex/Condvar state is reused across phases: channel queue allocations would
// contaminate the heap changes attributed to readers during this workload.
#[derive(Default)]
struct Rendezvous {
    state: Mutex<RendezvousState>,
    changed: Condvar,
}

#[derive(Default)]
struct RendezvousState {
    arrived: u8,
    generation: u64,
    aborted: bool,
}

impl Rendezvous {
    fn wait(&self) {
        let mut state = self.state.lock().unwrap();
        let generation = state.generation;
        state.arrived += 1;
        if state.arrived == 3 {
            state.arrived = 0;
            state.generation += 1;
            self.changed.notify_all();
        } else {
            while state.generation == generation && !state.aborted {
                state = self.changed.wait(state).unwrap();
            }
        }
        let aborted = state.aborted;
        drop(state);
        assert!(!aborted, "ownership participant stopped");
    }

    fn abort(&self) {
        // An unwinding participant must still wake its peers if a prior panic
        // poisoned the lock; no measured work continues after abort.
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .aborted = true;
        self.changed.notify_all();
    }
}

struct WakePeersOnPanic<'a>(&'a Rendezvous, &'a Rendezvous);

impl Drop for WakePeersOnPanic<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.abort();
            self.1.abort();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum OwnershipControl {
    Healthy,
    WrongRows,
    WrongAllowance,
    WorkerPanic,
    CoordinatorPanic,
}

/// Pause two readers around a writer's commit and reconcile their live resources.
///
/// Start readers one at a time, cancel one, publish a new key, then finish the
/// other. Existing plans must still read their original snapshot; a newly prepared
/// query must see the added key. Refused competing readers and writers must leave
/// the already admitted work intact.
pub(super) fn run(
    root: &Path,
    control: OwnershipControl,
) -> Result<(), Box<dyn std::error::Error>> {
    let negative = control == OwnershipControl::WrongRows;
    let wrong_allowance = control == OwnershipControl::WrongAllowance;
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Phase {
        Baseline,
        Writer,
        OrderedReader,
        DistinctReader,
        Cancelled,
        Published,
        Finished,
        Released,
    }
    // All three threads follow this schedule. At each phase, readers report their
    // owners and wait while the coordinator checks the listed open-file increase.
    const PHASES: [(Phase, &str, usize); 8] = [
        (Phase::Baseline, "baseline", 0),
        (Phase::Writer, "writer", 0),
        (Phase::OrderedReader, "reader-order", 4),
        (Phase::DistinctReader, "reader-distinct", 8),
        (Phase::Cancelled, "cancelled", 4),
        (Phase::Published, "published", 4),
        (Phase::Finished, "finished", 0),
        (Phase::Released, "released", 0),
    ];
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
    // The scan keeps one exact-length units path but reserves space for two
    // maximum paths. Subtract the retained request when computing unused allowance.
    let scan_path_bytes = std::fs::canonicalize(&path)?.as_os_str().len() + "/units".len();
    let ordered = prepare_observed(&db, "FROM facts |> ORDER BY k", "order", 1, wrong_allowance)?;
    let distinct = prepare_observed(&db, "FROM facts |> DISTINCT", "distinct", 2, false)?;
    let prepared = resident + ordered.accounted_memory_bytes() + distinct.accounted_memory_bytes();
    let observations = [Mutex::new(Owner::default()), Mutex::new(Owner::default())];
    let ready = Rendezvous::default();
    let resume = Rendezvous::default();
    let mut reader_excess = 0;
    std::thread::scope(|scope| {
        // Drop before scope joins workers, including when the coordinator fails.
        let _wake_peers = WakePeersOnPanic(&ready, &resume);
        let mut workers = [None, None];
        for (id, query, token) in [(0, &ordered, &cancelled), (1, &distinct, &cancel)] {
            let db = &db;
            let observations = &observations;
            let ready = &ready;
            let resume = &resume;
            workers[id] = Some(scope.spawn(move || {
                let _wake_peers = WakePeersOnPanic(ready, resume);
                assert!(
                    control != OwnershipControl::WorkerPanic || id != 0,
                    "injected ownership worker panic"
                );
                let mut result = None;
                let mut heap = Heap::default();
                ready.wait();
                resume.wait();
                for (phase, _, _) in PHASES {
                    match (phase, id) {
                        (Phase::OrderedReader, 0) | (Phase::DistinctReader, 1) => {
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
                        (Phase::Cancelled, 0) => {
                            let before = Heap::now();
                            let rows = result.as_mut().unwrap();
                            assert!(matches!(rows.step(), QueryStep::Failed(Error::Cancelled)));
                            assert_eq!(before.increase_from(Heap::now()), heap);
                            heap = Heap::default();
                        }
                        (Phase::Finished, 1) => {
                            let before = Heap::now();
                            consume(result.as_mut().unwrap(), true, false, false);
                            assert_eq!(before.increase_from(Heap::now()), heap);
                            heap = Heap::default();
                        }
                        (Phase::Released, _) => {
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
        assert!(
            control != OwnershipControl::CoordinatorPanic,
            "injected ownership coordinator panic"
        );
        let mut writer = None;
        let mut writer_owner = Owner::default();
        let mut baseline = None;
        // Exercise the barriers before taking the baseline so lazy synchronization
        // storage belongs to the test, not to the first measured reader.
        ready.wait();
        resume.wait();
        for (phase, label, descriptors) in PHASES {
            ready.wait();
            // Both readers are waiting at resume; their reported owners cannot
            // change while the coordinator measures or operates on the writer.
            let readers = observations.each_ref().map(|owner| *owner.lock().unwrap());
            let reader_charge: u64 = readers.iter().map(|owner| owner.charge).sum();
            if phase == Phase::Baseline {
                baseline = Some(Live::now());
            }
            let base = baseline.unwrap();
            if phase == Phase::Writer {
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
            if phase == Phase::DistinctReader {
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
            if phase == Phase::Published {
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
                // Another writer must be able to start after the rejected request.
                db.begin_append("facts", limits, &cancel)
                    .unwrap()
                    .abort()
                    .unwrap();
            }
            let observed = checkpoint(
                &db,
                label,
                prepared + writer_owner.charge + reader_charge,
                base,
            );
            assert_eq!(observed.descriptors, base.descriptors + descriptors);
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
            // No path allocation remains at this checkpoint. Each of the three
            // retained writer buffers has a separate 16-KiB rounding allowance.
            writer_owner.report(
                "writer",
                if writer.is_some() {
                    (std::mem::size_of::<Append<'_>>()
                        + 2 * 4096
                        + 3 * 16_384
                        + 2 * native_buffer_allowance(65_536)) as u64
                } else {
                    0
                },
            );
            assert!(
                writer_owner.heap.usable as u64 <= writer_owner.charge,
                "append usable allocations exceed admission"
            );
            for (reader, name) in readers.iter().zip(["order", "distinct"]) {
                let nonheap = if reader.charge == 0 {
                    0
                } else {
                    let inline = std::mem::size_of::<QueryResult<'_, '_>>();
                    // Finished/cancelled queries keep only their inline handle.
                    // Running queries also keep a plan allowance and path space;
                    // the retained path request already appears in measured heap.
                    let running = if reader.heap.requested == 0 {
                        0
                    } else {
                        4096 + 2 * 4096 - scan_path_bytes
                    };
                    (inline + running) as u64
                };
                reader.report(name, nonheap);
                // Finish the schedule before reporting the largest excess so
                // one mismatch does not hide the later ownership observations.
                reader_excess =
                    reader_excess.max((reader.heap.usable as u64).saturating_sub(reader.charge));
            }
            if phase == Phase::Released {
                assert_eq!(db.reserved_temp_bytes(), 0);
                assert_eq!(Live::now(), base);
            }
            resume.wait();
        }
        for worker in workers {
            worker.unwrap().join().unwrap();
        }
    });
    assert_eq!(
        reader_excess, 0,
        "reader usable allocations exceed admission"
    );
    let before_sync_drop = Live::now();
    // Darwin's synchronization objects can own heap storage; Linux's versions
    // need no destructor. Measure their release at the same point on both hosts.
    #[allow(clippy::drop_non_drop)]
    {
        drop(ready);
        drop(resume);
        drop(observations);
    }
    let after_sync_drop = Live::now();
    println!(
        "ownership caller-synchronization requested={} usable={}",
        before_sync_drop.requested - after_sync_drop.requested,
        before_sync_drop.usable - after_sync_drop.usable
    );
    consume(&mut db.execute(&ordered, &cancel)?, false, false, false);
    let latest = db.prepare("FROM facts |> DISTINCT")?;
    // The new snapshot contains the committed key. The negative control pretends
    // that key is absent and must fail the complete-row comparison.
    consume(&mut db.execute(&latest, &cancel)?, true, true, negative);
    drop(latest);
    let grouped = db.prepare("FROM facts |> AGGREGATE COUNT(*) AS n GROUP BY k |> SELECT k")?;
    let baseline = Live::now();
    let prior = db.reserved_memory_bytes();
    let mut rows = db.execute(&grouped, &cancel)?;
    let running = rows.accounted_memory_bytes();
    let held = checkpoint(&db, "hash-held", prior + running, baseline);
    assert!((held.requested - baseline.requested) as u64 <= running);
    // A small grouped query must leave room for another reader. Consume both:
    // the new grouped plan sees the extra key, and the old distinct plan does not.
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
