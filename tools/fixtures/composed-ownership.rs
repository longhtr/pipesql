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

pub(super) fn reader_shapes(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use pipesql::DateValue;
    std::fs::create_dir(root)?;
    let dates = [2, 1, 2, 0].map(|day| DateValue::from_days_since_unix_epoch(day).unwrap());
    for (kind, values) in [
        (DataType::Int64, ColumnValues::Int64(&[2, 1, 2, 0])),
        (DataType::Double, ColumnValues::Double(&[2., 1., 2., 0.])),
        (DataType::Date, ColumnValues::Date(&dates)),
    ] {
        for width in [1, 64] {
            let path = root.join(format!("{kind:?}-{width}"));
            let db = Database::create_empty(&path, Config::new(64_000_000, TEMP)?)?;
            let cancel = CancellationToken::new();
            let names: Vec<_> = (0..width).map(|i| format!("c{i}")).collect();
            let declarations: Vec<_> = names
                .iter()
                .map(|name| ColumnDeclaration {
                    name,
                    data_type: kind,
                    nullable: true,
                })
                .collect();
            db.declare_table("typed", &declarations, &cancel)?;
            let columns = vec![
                ColumnInput {
                    values,
                    validity: &[7]
                };
                width
            ];
            let mut append = db.begin_append(
                "typed",
                AppendLimits {
                    batches: 1,
                    encoded_bytes: 100_000,
                },
                &cancel,
            )?;
            append.write(&columns, &cancel)?;
            append.commit(&cancel)?;
            let path_bytes = std::fs::canonicalize(&path)?.as_os_str().len() + "/units".len();
            for (sql, distinct) in [
                ("FROM typed |> ORDER BY c0 NULLS FIRST", false),
                ("FROM typed |> DISTINCT", true),
            ] {
                println!("reader shape type={kind:?} width={width} distinct={distinct}");
                let prepared = db.prepare(sql)?;
                let before = Live::now();
                let memory = db.reserved_memory_bytes();
                let mut rows = db.execute(&prepared, &cancel)?;
                for _ in 0..200_000 {
                    assert!(matches!(rows.step(), QueryStep::Progress));
                    if db.reserved_temp_bytes() != 0 {
                        break;
                    }
                }
                assert!(db.reserved_temp_bytes() != 0, "reader parks after spill");
                let owner = Owner {
                    charge: rows.accounted_memory_bytes(),
                    heap: Heap::now().increase_from(Heap {
                        requested: before.requested,
                        usable: before.usable,
                    }),
                };
                // Same independent equation as the composed readers. The native
                // payload padding is real requested capacity, not a new allowance.
                owner.report(
                    "typed-reader",
                    (std::mem::size_of::<QueryResult<'_, '_>>() + 4096 + 8192 - path_bytes) as u64,
                );
                assert!(
                    owner.heap.usable as u64 <= owner.charge,
                    "reader usable allocations exceed admission"
                );
                let mut counts = [0; 3];
                let mut previous = 0;
                let mut finished = false;
                for _ in 0..200_000 {
                    match rows.step() {
                        QueryStep::Progress => (),
                        QueryStep::Finished => {
                            finished = true;
                            break;
                        }
                        QueryStep::Failed(error) => panic!("typed reader: {error:?}"),
                        QueryStep::Rows(batch) => {
                            for row in 0..batch.len() {
                                let key = match batch.value(row, 0) {
                                    Some(Value::Null) => 0,
                                    Some(Value::Int64(n @ 1..=2)) if kind == DataType::Int64 => {
                                        n as usize
                                    }
                                    Some(Value::Double(n))
                                        if kind == DataType::Double && (n == 1. || n == 2.) =>
                                    {
                                        n as usize
                                    }
                                    Some(Value::Date(day)) if kind == DataType::Date => {
                                        let n = day.days_since_unix_epoch();
                                        assert!((1..=2).contains(&n));
                                        n as usize
                                    }
                                    unexpected => panic!("typed reader value: {unexpected:?}"),
                                };
                                for column in 1..width {
                                    assert_eq!(batch.value(row, column), batch.value(row, 0));
                                }
                                if !distinct {
                                    assert!(key >= previous);
                                }
                                previous = key;
                                counts[key] += 1;
                            }
                        }
                    }
                }
                assert!(finished);
                assert_eq!(counts, if distinct { [1, 1, 1] } else { [1, 1, 2] });
                drop(rows);
                assert_eq!(Live::now(), before);
                assert_eq!(db.reserved_memory_bytes(), memory);
                assert_eq!(db.reserved_temp_bytes(), 0);
            }
            db.close()?;
        }
    }
    println!(
        "reader shapes passed: 12 fixed-width ordering/distinct cases; rows, admission and release"
    );
    Ok(())
}

fn allocation_extent<T>(count: usize) -> (usize, usize) {
    let before = Heap::now();
    let mut allocation = Vec::<T>::new();
    allocation.try_reserve_exact(count).unwrap();
    assert_eq!(allocation.capacity(), count);
    let live = Heap::now().increase_from(before);
    assert_eq!(live.requested, count * std::mem::size_of::<T>());
    drop(allocation);
    assert_eq!(Heap::now(), before);
    (live.requested, live.usable - live.requested)
}

// Independent domains for the padded blocking buffers and the GROUPED caller's
// power-of-two hash layouts. Observe usable extents separately from capacities.
pub(super) fn grouped_allocation_shapes() {
    let mut buffers = (0, 0);
    // A 128-value frame plus 255 minimum-width rows is at most 8,430,080
    // encoded bytes. Its final allocation unit ends at 8,437,760 bytes.
    for units in 2..=515 {
        let observed = allocation_extent::<u8>(units * 16_384);
        assert!(
            observed.1 <= 16_384,
            "blocking allocation rounding: {observed:?}"
        );
        if cfg!(target_os = "macos") {
            assert_eq!(observed.1, 0, "aligned Darwin blocking allocation");
        }
        if observed.1 > buffers.1 {
            buffers = observed;
        }
    }
    let mut arrays = (0, 0);
    // One f64, one i128, two nullable counters, one count/flag, four extrema,
    // four 16-byte text spans, and one 32-byte key slot per group.
    for exponent in 0..=12 {
        let groups = 1 << exponent;
        for observed in [
            allocation_extent::<f64>(groups),
            allocation_extent::<i128>(groups),
            allocation_extent::<[u32; 2]>(groups),
            allocation_extent::<u32>(groups),
            allocation_extent::<[u64; 4]>(groups),
            allocation_extent::<[u64; 8]>(groups),
            allocation_extent::<[u64; 4]>(groups),
        ] {
            assert!(
                observed.1 <= 16_384,
                "hash allocation rounding: {observed:?}"
            );
            if observed.1 > arrays.1 {
                arrays = observed;
            }
        }
    }
    println!(
        "grouped allocation shapes passed: buffers=514 hash-layouts=91 largest-buffer={buffers:?} largest-hash={arrays:?}"
    );
}

// The append contract admits three retained allocations separately from its
// inline handle and paths. Challenge every permitted request size through the
// same global allocator as the public library; do not copy its admission code.
pub(super) fn allocation_shapes(wrong_bound: bool) {
    let ceiling = if wrong_bound { 0 } else { 16_384 };
    let mut largest = (0, 0);
    // 64-byte header + 64 32-byte descriptors + the 524,288-byte
    // maximum encoded column. Commit also needs a 65,536-byte workspace.
    for bytes in 65_536..=526_400 {
        let observed = allocation_extent::<u8>(bytes);
        assert!(
            observed.1 <= ceiling,
            "append allocation rounding: {observed:?}"
        );
        if observed.1 > largest.1 {
            largest = observed;
        }
    }
    let mut references = (0, 0);
    // A UnitRef occupies 32 bytes with eight-byte alignment; 4,096 units
    // includes both committed references and the admitted new batch count.
    for units in 1..=4_096 {
        let observed = allocation_extent::<[u64; 4]>(units);
        assert!(
            observed.1 <= ceiling,
            "append allocation rounding: {observed:?}"
        );
        if observed.1 > references.1 {
            references = observed;
        }
    }
    println!(
        "append allocation shapes passed: workspace=460865 references=4096 largest={largest:?} reference-largest={references:?}"
    );
}

pub(super) fn append_shapes(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let cancel = CancellationToken::new();
    let db = Database::create_empty(root, Config::new(MEMORY, TEMP)?)?;
    let names: Vec<_> = (0..64).map(|i| format!("c{i}")).collect();
    let declarations: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(i, name)| ColumnDeclaration {
            name,
            data_type: if i == 0 {
                DataType::String
            } else {
                DataType::Int64
            },
            nullable: false,
        })
        .collect();
    db.declare_table("wide", &declarations, &cancel)?;
    let text = "x".repeat(65_536);
    let tail = "x".repeat(65_499);
    // Eight STRING rows: one validity byte, nine four-byte offsets, and
    // 524,251 text bytes fill the 524,288-byte column boundary exactly.
    let wide = [
        text.as_str(),
        text.as_str(),
        text.as_str(),
        text.as_str(),
        text.as_str(),
        text.as_str(),
        text.as_str(),
        tail.as_str(),
    ];
    let small = [""; 8];
    let mut columns = [ColumnInput {
        values: ColumnValues::Int64(&[7; 8]),
        validity: &[255],
    }; 64];
    for (index, batches) in [1_025, 4_093].into_iter().enumerate() {
        // The second append retains the first three committed unit references,
        // reaching the 4,096-reference limit without creating thousands of files.
        let before = Heap::now();
        let charge = db.reserved_memory_bytes();
        let mut append = db.begin_append(
            "wide",
            AppendLimits {
                batches,
                encoded_bytes: 1_000_000,
            },
            &cancel,
        )?;
        let mut prior_workspace_charge = 0;
        for (phase, strings) in [&small, &wide, &small].into_iter().enumerate() {
            columns[0].values = ColumnValues::String(strings);
            append.write(&columns, &cancel)?;
            let owner = Owner {
                heap: Heap::now().increase_from(before),
                charge: db.reserved_memory_bytes() - charge,
            };
            owner.report(
                "wide-append",
                (std::mem::size_of::<Append<'_>>() + 8192 + 3 * 16_384) as u64,
            );
            assert!(
                owner.heap.usable as u64 <= owner.charge,
                "append usable allocations exceed admission"
            );
            if phase == 1 {
                assert_eq!(owner.charge - prior_workspace_charge, 526_400 - 65_536);
            } else if phase == 2 {
                assert_eq!(
                    owner.charge, prior_workspace_charge,
                    "reuse grown workspace"
                );
            }
            prior_workspace_charge = owner.charge;
        }
        append.commit(&cancel)?;
        assert_eq!(Heap::now(), before);
        assert_eq!(db.reserved_memory_bytes(), charge);
        assert_eq!(db.reserved_temp_bytes(), 0);
        let prepared = db.prepare("FROM wide |> AGGREGATE COUNT(*) AS n, SUM(c1) AS total")?;
        let mut query = db.execute(&prepared, &cancel)?;
        let mut seen = false;
        let mut finished = false;
        for _ in 0..200_000 {
            match query.step() {
                QueryStep::Rows(batch) => {
                    assert!(!seen);
                    assert_eq!(batch.len(), 1);
                    assert_eq!(
                        batch.value(0, 0),
                        Some(Value::Int64((index as i64 + 1) * 24))
                    );
                    assert_eq!(
                        batch.value(0, 1),
                        Some(Value::Int64((index as i64 + 1) * 168))
                    );
                    seen = true;
                }
                QueryStep::Progress => (),
                QueryStep::Finished => {
                    finished = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("wide append query: {error:?}"),
            }
        }
        assert!(seen && finished, "wide append aggregate row and completion");
    }
    db.close()?;
    println!("append shapes passed: full-width maximum-column growth reuse publication release");
    Ok(())
}

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
    let mut reader_excess = 0;
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
            // Paths are transient here. The three retained allocations each
            // keep their separately qualified 16-KiB rounding ceiling.
            writer_owner.report(
                "writer",
                if writer.is_some() {
                    (std::mem::size_of::<Append<'_>>() + 2 * 4096 + 3 * 16_384) as u64
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
                // The workers are parked at barriers. Defer the bound
                // failure until they have completed and joined.
                reader_excess =
                    reader_excess.max((reader.heap.usable as u64).saturating_sub(reader.charge));
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
    assert_eq!(
        reader_excess, 0,
        "reader usable allocations exceed admission"
    );
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
