//! Refuse allocations while a public caller creates and uses a catalog database.
//!
//! The normal run declares a table, writes two batches, commits, reopens and runs
//! queries over four known rows. Each refusal run permits a chosen number of
//! allocations, then rejects later requests. Error formatting and dropped owners
//! remain inside that refusal window, so failure handling cannot rely on memory
//! becoming available again.
//!
//! After faults are disabled, check what remains usable. Query failures retain
//! the original prepared scan; failures that leave scratch cleanup unfinished
//! require reopening before another query can use scratch. Reopening also resolves
//! uncertain commits: the stored transaction result decides whether to expect
//! zero or four rows. A final begin/abort checks that writer access was released.
//! The declaration-refusal mode checks an earlier boundary: failed scratch and
//! pathname allocation must leave files and issuance unchanged and permit retry
//! through the same handle, without reopen.
//!
//! `allocation.rs` supplies the allocator. The allocation campaign
//! runs every prefix in a fresh process and checks that required phases were
//! reached. The separate mutex control below tests the native lock used by the
//! library without including thread startup in its allocation measurements.

#[path = "recovery.rs"]
pub(super) mod recovery;

use super::workload::{allocation_cause, arm, finish, format_error};
use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Commit,
    CommitResolution, Config, DataType, Database, Error, QueryResult, QueryStep, TransactionId,
    Value,
};
use std::path::Path;

const FIRST: &str = "first 雪";
// Stop the campaign if the healthy run exceeds this many allocations. This is
// a test-work limit, not the database's memory allowance.
pub(super) const ALLOCATION_LIMIT: usize = 1100;
const SECOND: &str = "next \t\n";
const QUERY: &str = "FROM facts |> SELECT note, amount";
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
const AGGREGATE: &str = "FROM facts |> AGGREGATE SUM(amount) AS ignored, AVG(amount) AS ai, SUM(measure) AS total, AVG(measure) AS mean, COUNT(*) AS n |> SELECT total, mean, ai, n";
const GROUPED: &str = "FROM facts |> EXTEND amount+0 AS adjusted |> SET note=note |> DROP amount |> RENAME adjusted AS amount |> AGGREGATE AVG(amount) AS ai, SUM(measure) AS total, AVG(measure) AS mean, COUNT(*) AS n, MIN(amount) AS amin, MAX(amount) AS amax, MIN(note) AS tmin, MAX(note) AS tmax GROUP AND ORDER BY note |> SELECT note, ai+0.0 AS ai, total+0.0 AS total, mean+0.0 AS mean, n+0 AS n, amin, amax, tmin, tmax";
const DISTINCT: &str = "FROM facts |> SELECT note, amount |> DISTINCT";
const UNION: &str = "FROM facts |> SELECT note, amount |> UNION ALL (FROM facts |> SELECT note, amount) |> ORDER BY note, amount |> AGGREGATE COUNT(*) AS n";
const UNION_DISTINCT: &str = "FROM facts |> SELECT note, amount |> UNION DISTINCT (FROM facts |> SELECT note, amount) |> AGGREGATE COUNT(*) AS n";
const EXCEPT: &str = "FROM facts |> SELECT amount |> EXCEPT DISTINCT (FROM facts |> WHERE note IS NULL |> SELECT amount) |> AGGREGATE COUNT(*) AS n";
const WINDOW: &str = "FROM facts |> EXTEND COUNT(*) OVER () AS n |> WHERE n=4 |> ORDER BY note, amount |> AGGREGATE SUM(n) AS total";
const REPEATED: &str = "FROM facts |> AGGREGATE COUNT(*) AS n GROUP BY note |> AGGREGATE SUM(n) AS subtotal GROUP BY n |> AGGREGATE SUM(subtotal) AS total, COUNT(*) AS distinct_sizes";
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
                // Group sizes are 2, 1 and 1; regrouping by size produces two groups.
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
                    assert_eq!(batch.column_count(), 9);
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
                    for column in 5..7 {
                        assert_eq!(
                            batch.value(row, column),
                            Some(Value::Int64(if seen == 0 {
                                i64::MAX
                            } else {
                                9_007_199_254_740_993
                            }))
                        );
                    }
                    for column in 7..9 {
                        match (seen, batch.value(row, column)) {
                            (0, Some(Value::Null)) => (),
                            (1, Some(Value::String(text))) => assert_eq!(text.as_str(), FIRST),
                            (2, Some(Value::String(text))) => assert_eq!(text.as_str(), SECOND),
                            _ => panic!("text extremum differs"),
                        }
                    }
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
                // COUNT alone would miss replacing one text row with another.
                // DISTINCT removes one duplicate NULL row; other queries retain it.
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
    "FROM facts |> ORDER BY note DESC NULLS FIRST |> SELECT note, amount |> LIMIT 4";
const DERIVED_JOIN: &str = "FROM (FROM facts |> WHERE note IS NULL OR NOT note NOT IN ('first 雪', 'absent', NULL) |> EXTEND 'branch 雪' AS tag, DATE '1970-01-02' AS day |> WHERE tag = 'branch 雪' AND day = DATE '1970-01-02' |> SELECT amount) AS a |> LEFT JOIN (FROM facts |> WHERE note IS NULL |> SELECT amount) AS b ON a.amount = b.amount |> AGGREGATE COUNT(*) AS n";
const JOINED_ORDER: &str = "FROM facts AS a |> JOIN facts AS b ON a.amount = b.amount |> ORDER BY a.amount DESC |> LIMIT 8 |> AGGREGATE COUNT(*) AS n";
fn consume_count(mut result: QueryResult<'_, '_>, expected: usize) -> Result<(), Error> {
    let mut rows = 0;
    for _ in 0..4096 {
        match result.step() {
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 1);
                assert_eq!(batch.len(), 1);
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

// After reopening, one sort checks every stored field, duplicate and scratch reuse.
// The query sequence in run tests each operator under allocation refusal.
fn check_healed_rows(db: &Database, expected: usize) -> Result<(), Error> {
    let baseline = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let query = db.prepare("FROM facts |> ORDER BY note DESC NULLS FIRST")?;
    let mut result = db.execute(&query, &cancel)?;
    let mut count = 0;
    let mut peak_temp = 0;
    let mut finished = false;
    for _ in 0..1024 {
        let step = result.step();
        peak_temp = peak_temp.max(db.reserved_temp_bytes());
        match step {
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 3);
                for row in 0..batch.len() {
                    assert!(count < expected, "healed catalog row count");
                    match count {
                        0 | 1 => {
                            assert_eq!(batch.value(row, 0), Some(Value::Null));
                            assert_eq!(batch.value(row, 1), Some(Value::Int64(i64::MAX)));
                            assert_eq!(batch.value(row, 2), Some(Value::Null));
                        }
                        2 | 3 => {
                            let Some(Value::String(text)) = batch.value(row, 0) else {
                                panic!("healed catalog text type");
                            };
                            assert_eq!(text.as_str(), if count == 2 { SECOND } else { FIRST });
                            assert_eq!(
                                batch.value(row, 1),
                                Some(Value::Int64(9_007_199_254_740_993))
                            );
                            assert_eq!(
                                batch.value(row, 2),
                                Some(Value::Double(3.5)),
                                "healed catalog measure"
                            );
                        }
                        _ => unreachable!(),
                    }
                    count += 1;
                }
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(_) => return Err(result.into_error().expect("healed query error")),
        }
    }
    assert!(finished, "healed catalog query exceeded bounded steps");
    assert_eq!(count, expected, "healed catalog complete multiset");
    if expected != 0 {
        assert!(peak_temp > 0, "healed catalog must reuse scratch");
    }
    drop(result);
    drop(query);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    Ok(())
}

pub(super) fn run(root: &Path, after: Option<usize>) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(root)?;
    let path = root.join("database");
    let config = Config::new(4_000_000, 2_000_000)?;
    let cancel = CancellationToken::new();
    // Reserve test input storage before counting engine allocations. Replacing
    // FIRST with SECOND during the writes must not grow this string.
    let mut text = String::with_capacity(128);
    text.push_str(FIRST);
    let mut phase = "create";
    let mut created = false;
    let mut declaration: Option<Commit> = None;
    let mut issued: Option<TransactionId> = None;
    let mut appended: Option<Commit> = None;
    println!("entered public catalog allocation probe");
    // Keep one allocation counter across the lifecycle: later prefixes reach
    // failures in query execution after creation and publication have succeeded.
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
            assert!(
                usable as u64 <= grouped_charge,
                "grouped usable allocations exceed admission"
            );
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
            consume_count(result, 8)?;
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
            // Each of the two MAX rows matches two right rows. FIRST has no match
            // but contributes one row because this is a left join: 2*2 + 1 = 5.
            consume_count(result, 5)?;
            drop(derived);
            phase = "union-prepare";
            let union = db.prepare(UNION)?;
            phase = "union-execute";
            let result = db.execute(&union, &cancel)?;
            phase = "union-step";
            consume_count(result, 8)?;
            drop(union);
            phase = "union-distinct-prepare";
            let union_distinct = db.prepare(UNION_DISTINCT)?;
            phase = "union-distinct-execute";
            let result = db.execute(&union_distinct, &cancel)?;
            phase = "union-distinct-step";
            consume_count(result, 3)?;
            drop(union_distinct);
            phase = "except-prepare";
            let except = db.prepare(EXCEPT)?;
            phase = "except-execute";
            let result = db.execute(&except, &cancel)?;
            phase = "except-step";
            // Removing MAX leaves one distinct amount shared by the two text rows.
            consume_count(result, 1)?;
            drop(except);
            phase = "window-prepare";
            let window = db.prepare(WINDOW)?;
            phase = "window-execute";
            let result = db.execute(&window, &cancel)?;
            phase = "window-step";
            consume_count(result, 16)?;
            drop(window);
            phase = "count-only-prepare";
            let count = db.prepare(
                "FROM facts |> SELECT COUNT(*) OVER () AS n |> AGGREGATE SUM(n) AS total",
            )?;
            phase = "count-only-execute";
            let result = db.execute(&count, &cancel)?;
            phase = "count-only-step";
            consume_count(result, 16)?;
            drop(count);
            phase = "division-prepare";
            let division = db.prepare("FROM facts |> SELECT ABS(-(measure/2)) AS ratio, SQRT(ROUND(CEIL(SIGN(MOD(amount, 2))))) AS odd, COALESCE(DIV(amount, 1), DIV(1, 0)) AS exact |> WHERE ratio=1.75 AND odd=1 AND exact=9007199254740993 |> EXTEND SAFE_DIVIDE(ratio, 0) AS missing |> EXTEND EXP(LN(FLOOR(COALESCE(missing, NULLIF(ratio, missing)))))+LOG10(POW(2, 3)/8) AS filled |> WHERE missing IS NOT DISTINCT FROM NULL AND filled=1 |> AGGREGATE COUNT(*) AS n")?;
            phase = "division-execute";
            let result = db.execute(&division, &cancel)?;
            phase = "division-step";
            consume_count(result, 2)?;
            drop(division);
            phase = "distinct-prepare";
            let distinct = db.prepare(DISTINCT)?;
            phase = "distinct-execute";
            let result = db.execute(&distinct, &cancel)?;
            phase = "distinct-step";
            consume_rows(result, 3, false)?;
            drop(distinct);
            Ok(())
        })();
        // The closure drops temporary plans and results while faults remain active.
        // Keep the original scan plan and database to test reuse after the failure.
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

    // finish disabled faults after checking release and formatting. Reopen now
    // and use transaction resolution to determine which writes became durable.
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
    match check_healed_rows(&db, rows) {
        Ok(()) => (),
        Err(Error::Bind { .. }) if declaration.is_none() && db.generation() == 0 => {
            assert_eq!(rows, 0);
            db.declare_table("facts", &COLUMNS, &cancel)?;
            check_healed_rows(&db, 0)?;
        }
        Err(error) => panic!("healed catalog verification failed: {error:?}"),
    }
    // Successful reads alone would not reveal a writer lock left held by failure.
    let retry = db.begin_append("facts", limits(), &cancel)?;
    let token = retry.transaction();
    retry.abort()?;
    assert_eq!(db.resolve_commit(token)?, CommitResolution::Aborted);
    db.close()?;
    println!("catalog healed rows={rows}");
    Ok(())
}

/// Refusal before issuance must release the writer without requiring reopen.
pub(super) fn declaration_refusal(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use std::fs;
    use std::sync::atomic::Ordering;

    fs::create_dir(root)?;
    let path = root.join("database");
    let db = Database::create_empty(&path, Config::new(4_000_000, 2_000_000)?)?;
    let cancel = CancellationToken::new();
    let memory = db.reserved_memory_bytes();
    // Include directory entries as well as file bytes: unchanged known roots
    // alone would miss an unexpected construction file. The namespace is shallow.
    let files = || {
        let mut entries = Vec::new();
        for directory in [path.clone(), path.join("units"), path.join("private")] {
            for entry in fs::read_dir(directory).unwrap() {
                let name = entry.unwrap().path();
                let bytes = name.is_file().then(|| fs::read(&name).unwrap());
                entries.push((name, bytes));
            }
        }
        entries.sort();
        entries
    };
    println!("entered declaration pre-issuance refusal probe");
    // Repeat against an empty catalog and a committed one. A successful retry
    // must consume the next literal attempt, with no gap from either refusal.
    for (name, attempt) in [("facts", 1_u64), ("other", 2)] {
        let before = files();
        let mut token = [0; 24];
        token[..16].copy_from_slice(db.database_identity().as_bytes());
        token[16..].copy_from_slice(&attempt.to_le_bytes());
        let token = TransactionId::from_bytes(token)?;
        for prefix in [0, 1] {
            // Scratch is the first allocation; the units pathname is second.
            // Keep refusal armed through error formatting and owner teardown.
            let baseline = arm(Some(prefix), 2);
            let result = db.declare_table(name, &COLUMNS, &cancel);
            let formatted = format_error(result.as_ref().err());
            finish("declaration", baseline);
            assert!(formatted);
            assert_eq!(super::CALLS.load(Ordering::Relaxed), prefix + 1);
            assert_eq!(super::REFUSED.load(Ordering::Relaxed), 1);
            match &result {
                Err(Error::Resource {
                    owner: "catalog construction scratch",
                    ..
                }) if prefix == 0 => (),
                Err(Error::Io {
                    operation: "construct filesystem path",
                    source,
                }) if prefix == 1 && source.kind() == std::io::ErrorKind::OutOfMemory => (),
                other => panic!("unexpected declaration refusal: {other:?}"),
            }
            assert_eq!(db.reserved_memory_bytes(), memory);
            assert_eq!(db.reserved_temp_bytes(), 0);
            assert_eq!(db.generation(), attempt - 1);
            assert_eq!(files(), before);
            assert!(matches!(db.resolve_commit(token), Err(Error::NotFound)));
        }
        let commit = db.declare_table(name, &COLUMNS, &cancel)?;
        assert_eq!(commit.transaction(), token);
        assert_eq!(commit.generation(), attempt);
        assert_eq!(db.resolve_commit(token)?, CommitResolution::Durable(commit));
        assert_eq!(db.reserved_memory_bytes(), memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    db.close()?;
    println!("declaration pre-issuance refusal and same-handle retry passed");
    Ok(())
}

/// Check that the library's native mutex needs no Rust allocation to acquire it.
///
/// A worker holds the lock while try_lock must report contention. Then lock
/// acquires it after release and updates the protected value. Thread startup and
/// teardown occur outside the refusal window; the Python caller enforces the time limit.
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
            // Leave time for the main thread to enter lock. try_lock proves
            // contention; scheduling may still let lock run after this release.
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
