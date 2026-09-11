//! Public catalog lifecycle under the shared caller-owned allocator observer.
#[path = "catalog-recovery-allocation.rs"]
pub(super) mod recovery;

use super::workload::{allocation_cause, arm, finish, format_error};
use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Commit,
    CommitResolution, Config, DataType, Database, Error, QueryResult, QueryStep, TransactionId,
    Value,
};
use std::path::Path;

const FIRST: &str = "first 雪";
pub(super) const ALLOCATION_LIMIT: usize = 710;
const SECOND: &str = "next \t\n";
const QUERY: &str = "FROM facts |> SELECT note,amount";
const COLUMNS: [ColumnDeclaration<'static>; 3] = [
    ColumnDeclaration {
        name: "note",
        data_type: DataType::String,
        nullable: true,
    },
    ColumnDeclaration {
        name: "amount",
        data_type: DataType::Int64,
        nullable: false,
    },
    ColumnDeclaration {
        name: "measure",
        data_type: DataType::Double,
        nullable: true,
    },
];
const AGGREGATE: &str = "FROM facts |> AGGREGATE SUM(amount) AS ignored, AVG(amount) AS ai, SUM(measure) AS total, AVG(measure) AS mean, COUNT(*) AS n |> SELECT total,mean,ai,n";
const GROUPED: &str = "FROM facts |> SELECT note,amount+0 AS amount,measure |> AGGREGATE AVG(amount) AS ai,SUM(measure) AS total,AVG(measure) AS mean,COUNT(*) AS n GROUP AND ORDER BY note |> SELECT note,ai+0.0 AS ai,total+0.0 AS total,mean+0.0 AS mean,n+0 AS n";
const DISTINCT: &str = "FROM facts |> SELECT note,amount |> DISTINCT";
const REPEATED: &str = "FROM facts |> AGGREGATE COUNT(*) AS n GROUP BY note |> AGGREGATE SUM(n) AS subtotal GROUP BY n |> AGGREGATE SUM(subtotal) AS total,COUNT(*) AS distinct_sizes";
fn consume_repeated(mut result: QueryResult<'_, '_>, expected: usize) -> Result<(), Error> {
    let mut seen = false;
    for _ in 0..4096 {
        match result.step() {
            QueryStep::Rows(batch) => {
                assert!(!seen && batch.len() == 1 && batch.column_count() == 2);
                assert_eq!(
                    batch.value(0, 0),
                    Some(if expected == 0 {
                        Value::Null
                    } else {
                        Value::Int64(4)
                    })
                );
                // The NULL key has two rows; the two text keys have one each.
                assert_eq!(
                    batch.value(0, 1),
                    Some(Value::Int64(if expected == 0 { 0 } else { 2 }))
                );
                seen = true;
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                assert!(seen);
                return Ok(());
            }
            QueryStep::Failed(_) => {
                return Err(result
                    .into_error()
                    .expect("terminal repeated aggregate error"));
            }
        }
    }
    panic!("repeated aggregation exceeded bounded step allowance");
}
fn consume_grouped(mut result: QueryResult<'_, '_>, expected: usize) -> Result<(), Error> {
    let mut seen = 0;
    for _ in 0..1024 {
        match result.step() {
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    assert!(expected != 0 && seen < 3);
                    assert_eq!(batch.column_count(), 5);
                    match (seen, batch.value(row, 0)) {
                        (0, Some(Value::Null)) => (),
                        (1, Some(Value::String(text))) => assert_eq!(text.as_str(), FIRST),
                        (2, Some(Value::String(text))) => assert_eq!(text.as_str(), SECOND),
                        _ => panic!("group key/order differs"),
                    }
                    assert_eq!(
                        batch.value(row, 1),
                        Some(Value::Double(if seen == 0 {
                            i64::MAX as f64
                        } else {
                            9_007_199_254_740_993_i64 as f64
                        }))
                    );
                    for column in 2..4 {
                        assert_eq!(
                            batch.value(row, column),
                            Some(if seen == 0 {
                                Value::Null
                            } else {
                                Value::Double(3.5)
                            })
                        );
                    }
                    assert_eq!(
                        batch.value(row, 4),
                        Some(Value::Int64(if seen == 0 { 2 } else { 1 }))
                    );
                    seen += 1;
                }
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                assert_eq!(seen, if expected == 0 { 0 } else { 3 });
                return Ok(());
            }
            QueryStep::Failed(_) => {
                return Err(result.into_error().expect("terminal grouped error"));
            }
        }
    }
    panic!("catalog grouping exceeded bounded step allowance");
}
fn consume_aggregate(mut result: QueryResult<'_, '_>, expected: usize) -> Result<(), Error> {
    let mut seen = false;
    for _ in 0..1024 {
        match result.step() {
            QueryStep::Rows(batch) => {
                assert!(!seen && batch.len() == 1 && batch.column_count() == 4);
                for (column, value) in [7.0, 3.5, ((1_u64 << 62) + (1_u64 << 52)) as f64]
                    .into_iter()
                    .enumerate()
                {
                    assert_eq!(
                        batch.value(0, column),
                        Some(if expected == 0 {
                            Value::Null
                        } else {
                            Value::Double(value)
                        })
                    );
                }
                assert_eq!(
                    batch.value(0, 3),
                    Some(Value::Int64(i64::try_from(expected).unwrap()))
                );
                seen = true;
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                assert!(seen);
                return Ok(());
            }
            QueryStep::Failed(_) => return Err(result.into_error().expect("terminal error")),
        }
    }
    panic!("catalog aggregate exceeded bounded step allowance");
}
fn limits() -> AppendLimits {
    AppendLimits {
        batches: 2,
        encoded_bytes: 100_000,
    }
}

fn consume(result: QueryResult<'_, '_>, expected: usize) -> Result<(), Error> {
    consume_rows(result, expected, false)
}
fn consume_rows(
    mut result: QueryResult<'_, '_>,
    expected: usize,
    ordered: bool,
) -> Result<(), Error> {
    let mut count = 0;
    let mut categories = [0; 3];
    for _ in 0..1024 {
        match result.step() {
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 2);
                for row in 0..batch.len() {
                    assert!(count < expected);
                    let category =
                        match (batch.value(row, 0).unwrap(), batch.value(row, 1).unwrap()) {
                            (Value::String(text), Value::Int64(9_007_199_254_740_993))
                                if text.as_str() == FIRST =>
                            {
                                0
                            }
                            (Value::String(text), Value::Int64(9_007_199_254_740_993))
                                if text.as_str() == SECOND =>
                            {
                                1
                            }
                            (Value::Null, Value::Int64(i64::MAX)) => 2,
                            _ => panic!("catalog row differs"),
                        };
                    if ordered {
                        assert_eq!(
                            category,
                            [2, 2, 1, 0][count],
                            "NULL-first descending text order"
                        );
                    }
                    categories[category] += 1;
                    count += 1;
                }
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                assert_eq!(count, expected);
                // Both paths preserve the complete multiset, including duplicates.
                assert_eq!(
                    categories,
                    match expected {
                        0 => [0, 0, 0],
                        3 => [1, 1, 1],
                        _ => [1, 1, 2],
                    }
                );
                return Ok(());
            }
            QueryStep::Failed(_) => return Err(result.into_error().expect("terminal error")),
        }
    }
    panic!("catalog fixture exceeded bounded step allowance");
}

const ORDERED: &str =
    "FROM facts |> ORDER BY note DESC NULLS FIRST |> SELECT note,amount |> LIMIT 4";
const DERIVED_JOIN: &str = "FROM (FROM facts |> WHERE note = 'first 雪' OR note = 'absent' |> SELECT amount) AS a |> JOIN (FROM facts |> SELECT amount) AS b ON a.amount = b.amount |> AGGREGATE COUNT(*) AS n";
const JOINED_ORDER: &str = "FROM facts AS a |> JOIN facts AS b ON a.amount = b.amount |> ORDER BY a.amount DESC |> LIMIT 8 |> AGGREGATE COUNT(*) AS n";
fn consume_joined(mut result: QueryResult<'_, '_>, expected: usize) -> Result<(), Error> {
    let mut rows = 0;
    for _ in 0..4096 {
        match result.step() {
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 1);
                assert_eq!(batch.len(), 1);
                // Two occurrences of each of two keys produce 2*2 + 2*2 pairs.
                assert_eq!(
                    batch.value(0, 0),
                    Some(Value::Int64(i64::try_from(expected).unwrap()))
                );
                rows += 1;
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                assert_eq!(rows, 1);
                return Ok(());
            }
            QueryStep::Failed(_) => return Err(result.into_error().expect("terminal error")),
        }
    }
    panic!("joined order fixture exceeded bounded step allowance");
}

pub(super) fn run(root: &Path, after: Option<usize>) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(root)?;
    let path = root.join("database");
    let config = Config::new(4_000_000, 2_000_000)?;
    let cancel = CancellationToken::new();
    // Caller input capacity exists before fault injection; changing its contents
    // introduces no allocation into the engine's census.
    let mut text = String::with_capacity(128);
    text.push_str(FIRST);
    let mut phase = "create";
    let mut created = false;
    let mut declaration: Option<Commit> = None;
    let mut issued: Option<TransactionId> = None;
    let mut appended: Option<Commit> = None;
    println!("entered public catalog allocation probe");
    // Measured lifecycle census on short and 384-byte database paths.
    let baseline = arm(after, ALLOCATION_LIMIT);
    let result = (|| -> Result<(), Error> {
        let db = Database::create_empty(&path, config)?;
        created = true;
        phase = "declare";
        declaration = Some(db.declare_table("facts", &COLUMNS, &cancel)?);
        phase = "begin";
        let mut append = db.begin_append("FACTS", limits(), &cancel)?;
        issued = Some(append.transaction());
        for stage in ["first-write", "second-write"] {
            phase = stage;
            append.write(
                &[
                    ColumnInput {
                        values: ColumnValues::String(&[text.as_str(), "ignored"]),
                        validity: &[1],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&[9_007_199_254_740_993, i64::MAX]),
                        validity: &[3],
                    },
                    ColumnInput {
                        values: ColumnValues::Double(&[3.5, f64::NAN]),
                        validity: &[1],
                    },
                ],
                &cancel,
            )?;
            text.clear();
            text.push_str(SECOND);
        }
        phase = "commit";
        appended = Some(append.commit(&cancel)?);
        phase = "close";
        db.close()?;
        phase = "open";
        let db = Database::open(&path, config)?;
        phase = "prepare";
        let query = db.prepare(QUERY)?;
        let query_memory = db.reserved_memory_bytes();
        let query_live = super::workload::live();
        let query_generation = db.generation();
        let query_result = (|| -> Result<(), Error> {
            phase = "execute";
            let result = db.execute(&query, &cancel)?;
            phase = "step";
            consume(result, 4)?;
            phase = "aggregate-prepare";
            let aggregate = db.prepare(AGGREGATE)?;
            phase = "aggregate-execute";
            consume_aggregate(db.execute(&aggregate, &cancel)?, 4)?;
            drop(aggregate);
            phase = "grouped-prepare";
            let grouped = db.prepare(GROUPED)?;
            phase = "grouped-execute";
            let grouped_result = db.execute(&grouped, &cancel)?;
            let grouped_charge =
                grouped.accounted_memory_bytes() + grouped_result.accounted_memory_bytes();
            assert_eq!(db.reserved_memory_bytes(), query_memory + grouped_charge);
            let grouped_live = super::workload::live();
            let requested = grouped_live
                .requested
                .checked_sub(query_live.requested)
                .unwrap();
            let usable = grouped_live.usable.checked_sub(query_live.usable).unwrap();
            assert!(requested as u64 <= grouped_charge);
            println!(
                "catalog grouped held charge={grouped_charge} requested={requested} usable={usable}"
            );
            consume_grouped(grouped_result, 4)?;
            drop(grouped);
            phase = "order-prepare";
            let ordered = db.prepare(ORDERED)?;
            phase = "order-execute";
            let result = db.execute(&ordered, &cancel)?;
            phase = "order-step";
            consume_rows(result, 4, true)?;
            drop(ordered);
            phase = "joined-order-prepare";
            let joined = db.prepare(JOINED_ORDER)?;
            phase = "joined-order-execute";
            let result = db.execute(&joined, &cancel)?;
            phase = "joined-order-step";
            consume_joined(result, 8)?;
            drop(joined);
            phase = "repeated-prepare";
            let repeated = db.prepare(REPEATED)?;
            phase = "repeated-execute";
            let result = db.execute(&repeated, &cancel)?;
            phase = "repeated-step";
            consume_repeated(result, 4)?;
            drop(repeated);
            phase = "derived-prepare";
            let derived = db.prepare(DERIVED_JOIN)?;
            phase = "derived-execute";
            let result = db.execute(&derived, &cancel)?;
            phase = "derived-step";
            consume_joined(result, 2)?;
            drop(derived);
            phase = "distinct-prepare";
            let distinct = db.prepare(DISTINCT)?;
            phase = "distinct-execute";
            let result = db.execute(&distinct, &cancel)?;
            phase = "distinct-step";
            consume_rows(result, 3, false)?;
            drop(distinct);
            Ok(())
        })();
        // All query-local owners drop under continuing refusal. The original
        // prepared scan and database remain alive for the same-handle check.
        assert!(format_error(query_result.as_ref().err()));
        assert_eq!(super::workload::live(), query_live);
        assert_eq!(db.reserved_memory_bytes(), query_memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(db.generation(), query_generation);
        if query_result.is_err() {
            let fault_state = super::workload::suspend_faults();
            consume(db.execute(&query, &cancel).unwrap(), 4).unwrap();
            let distinct = db.prepare(DISTINCT).unwrap();
            let healed = db
                .execute(&distinct, &cancel)
                .and_then(|result| consume_rows(result, 3, false));
            if matches!(
                query_result,
                Err(Error::RecoveryRequired { .. } | Error::CleanupRequired { .. })
            ) {
                assert!(matches!(healed, Err(Error::RecoveryRequired { .. })));
                println!("catalog query retained scan; scratch requires reopen");
            } else {
                healed.unwrap();
                let retry = db.begin_append("facts", limits(), &cancel).unwrap();
                let token = retry.transaction();
                retry.abort().unwrap();
                assert_eq!(db.resolve_commit(token).unwrap(), CommitResolution::Aborted);
                println!("catalog query same-handle healed rows=4 distinct=3 writer=usable");
            }
            drop(distinct);
            assert_eq!(db.reserved_memory_bytes(), query_memory);
            assert_eq!(db.reserved_temp_bytes(), 0);
            assert_eq!(db.generation(), query_generation);
            assert_eq!(super::workload::live(), query_live);
            super::workload::resume_faults(fault_state);
        }
        query_result?;
        phase = "reclaim";
        db.reclaim(&cancel)?;
        drop(query);
        phase = "resolve";
        assert_eq!(
            db.resolve_commit(issued.unwrap())?,
            CommitResolution::Durable(appended.unwrap())
        );
        phase = "close";
        db.close()
    })();
    let formatted = format_error(result.as_ref().err());
    finish("catalog", baseline);
    assert!(formatted, "catalog diagnostic exceeded fixed caller output");
    println!("catalog phase={phase}");
    let mut ambiguous = None;
    let mut create_cleanup = false;
    match &result {
        Ok(()) => println!("returned healthy catalog"),
        Err(Error::Resource { .. }) if after.is_some() => {
            println!("returned catalog allocation refusal")
        }
        Err(Error::Io { source, .. })
            if after.is_some() && source.kind() == std::io::ErrorKind::OutOfMemory =>
        {
            println!("returned catalog allocation refusal");
        }
        Err(Error::RecoveryRequired { source, .. })
            if after.is_some() && allocation_cause(source.kind()) =>
        {
            println!("returned catalog allocation refusal with recovery debt");
        }
        Err(Error::CleanupRequired { primary, cleanup })
            if after.is_some()
                && allocation_cause(primary.kind())
                && allocation_cause(cleanup.kind()) =>
        {
            create_cleanup = phase == "create";
            println!("returned catalog allocation refusal with cleanup debt");
        }
        Err(Error::CommitAmbiguous {
            transaction,
            source,
        }) if after.is_some() && allocation_cause(source.kind()) => {
            assert!(matches!(phase, "declare" | "commit"));
            if phase == "commit" {
                assert_eq!(Some(*transaction), issued);
            }
            ambiguous = Some(*transaction);
            println!("returned catalog allocation refusal with ambiguous commit");
        }
        Err(error) => panic!("unexpected catalog {phase} outcome: {error:?}"),
    }

    // Allocation is healed only after all owners and diagnostics were checked
    // under continuing refusal. Reopen, not filename guesses, resolves outcomes.
    let db = if path.exists() {
        match Database::open(&path, config) {
            Ok(db) => db,
            Err(Error::Corrupt(_) | Error::NotFound) if create_cleanup => {
                assert!(!created);
                println!("catalog explicit partial-create cleanup debt");
                return Ok(());
            }
            Err(error) => panic!("healed catalog reopen failed: {error:?}"),
        }
    } else {
        assert!(!created && phase == "create");
        Database::create_empty(&path, config)?
    };
    assert_eq!(db.reserved_temp_bytes(), 0);
    if let Some(commit) = declaration {
        assert_eq!(
            db.resolve_commit(commit.transaction())?,
            CommitResolution::Durable(commit)
        );
    }
    let mut rows = 0;
    if let Some(token) = issued {
        match db.resolve_commit(token)? {
            CommitResolution::Durable(commit) => {
                assert!(Some(commit) == appended || ambiguous == Some(token));
                rows = 4;
            }
            CommitResolution::Aborted => assert!(appended.is_none()),
        }
    }
    if let Some(token) = ambiguous {
        match db.resolve_commit(token)? {
            CommitResolution::Durable(commit) => {
                assert_eq!(commit.generation(), if phase == "declare" { 1 } else { 2 })
            }
            CommitResolution::Aborted => assert!(appended.is_none()),
        }
    }
    match db.prepare(QUERY) {
        Ok(query) => consume(db.execute(&query, &cancel)?, rows)?,
        Err(Error::Bind { .. }) if declaration.is_none() && db.generation() == 0 => {
            assert_eq!(rows, 0);
            db.declare_table("facts", &COLUMNS, &cancel)?;
        }
        Err(error) => panic!("healed catalog preparation failed: {error:?}"),
    }
    let aggregate = db.prepare(AGGREGATE)?;
    consume_aggregate(db.execute(&aggregate, &cancel)?, rows)?;
    drop(aggregate);
    let grouped = db.prepare(GROUPED)?;
    consume_grouped(db.execute(&grouped, &cancel)?, rows)?;
    drop(grouped);
    let ordered = db.prepare(ORDERED)?;
    consume_rows(db.execute(&ordered, &cancel)?, rows, true)?;
    drop(ordered);
    let joined = db.prepare(JOINED_ORDER)?;
    consume_joined(db.execute(&joined, &cancel)?, rows * 2)?;
    drop(joined);
    let repeated = db.prepare(REPEATED)?;
    consume_repeated(db.execute(&repeated, &cancel)?, rows)?;
    drop(repeated);
    let distinct = db.prepare(DISTINCT)?;
    consume_rows(
        db.execute(&distinct, &cancel)?,
        if rows == 0 { 0 } else { 3 },
        false,
    )?;
    drop(distinct);
    let derived = db.prepare(DERIVED_JOIN)?;
    consume_joined(db.execute(&derived, &cancel)?, rows / 2)?;
    drop(derived);
    // A healed writer must actually be usable after the failed attempt.
    let retry = db.begin_append("facts", limits(), &cancel)?;
    let token = retry.transaction();
    retry.abort()?;
    assert_eq!(db.resolve_commit(token)?, CommitResolution::Aborted);
    db.close()?;
    println!("catalog healed rows={rows}");
    Ok(())
}

/// Challenge contention on the exact private OS owner linked into the public rlib.
/// Worker startup/teardown stay outside the denied region and live-byte census.
pub(super) fn mutex_contention() {
    use std::sync::atomic::{AtomicU8, Ordering};
    let mutex = pipesql_filesystem::Mutex::new(0).unwrap();
    let phase = AtomicU8::new(0);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let mut guard = mutex.lock().unwrap();
            *guard = 1;
            phase.store(1, Ordering::Release);
            while phase.load(Ordering::Acquire) < 2 {
                std::thread::yield_now();
            }
            // Give the main thread a contention window; the caller's 20-second
            // process deadline bounds scheduler failure. No engine wait uses this.
            std::thread::sleep(std::time::Duration::from_millis(10));
            drop(guard);
            phase.store(3, Ordering::Release);
            while phase.load(Ordering::Acquire) < 4 {
                std::thread::yield_now();
            }
        });
        while phase.load(Ordering::Acquire) < 1 {
            std::thread::yield_now();
        }
        let baseline = arm(Some(0), 0);
        assert!(mutex.try_lock().unwrap().is_none());
        phase.store(2, Ordering::Release);
        {
            let mut guard = mutex.lock().unwrap();
            assert_eq!(*guard, 1);
            *guard = 2;
        }
        while phase.load(Ordering::Acquire) < 3 {
            std::thread::yield_now();
        }
        finish("mutex", baseline);
        assert_eq!(super::CALLS.load(Ordering::Relaxed), 0);
        phase.store(4, Ordering::Release);
    });
    assert_eq!(*mutex.lock().unwrap(), 2);
    println!("native mutex contention passed without Rust allocation");
}
