//! Check that query and writer memory stays covered by public reservations.
//!
//! Workloads vary column types, row widths, grouping and temporary-storage use.
//! Each checks answers as well as memory: an operation that silently skips work
//! must not pass merely because it allocates less. Equations beside the checks
//! account for inline storage and unused allowances separately from heap requests.
//! Allocator observations also include usable bytes beyond the requested size.
//!
//! Some cases sample between calls; others observe allocation and free events
//! inside preparation, execution and cleanup.
//!
//! `allocation.rs` provides the allocator; the allocation campaign
//! selects workloads, enforces process deadlines and checks their reports. Negative
//! controls alter expected rows, memory attribution or observation itself and must
//! be rejected. These checks do not bound foreign allocations or total process memory.

use pipesql::{
    Append, AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, Error, QueryResult, QueryStep, Value,
};
use std::path::Path;

use super::measurement::{Heap, Live, Owner, native_buffer_allowance, prepare_observed};

const MEMORY: u64 = 4_000_000;
const TEMP: u64 = 8_000_000;

// Count row-evaluation storage independently of the engine's admission formula.
// On the qualified 64-bit targets it contains 80 optional nullable
// values (16 bytes), 208 demand flags, 32 optional column identities (8 bytes),
// 32 optional numeric inputs (48 bytes), three 32-word arrays, 32 four-word
// vectors and 32 type tags. Conditional evaluation adds a cursor with one
// borrowed expression, four 32-byte control/type arrays, 32 tagged numeric values
// (16 bytes each) and two word-sized positions; construction uses two further
// 32-byte arrays, and dependency traversal uses 80 byte-sized indices.
const CONDITIONAL_ROW_SCRATCH: usize = 8 + 4 * 32 + 32 * 16 + 2 * 8 + 2 * 32 + 80;
const ANALYTIC_ROW_SCRATCH: usize =
    80 * 16 + 208 + 32 * 8 + 32 * 48 + 3 * 32 * 8 + 32 * 32 + 32 + CONDITIONAL_ROW_SCRATCH;

// Independently account for the documented macOS sort-buffer allowance. The
// requested capacity includes padding; only the remaining charge belongs here.
// These equations use workload field widths, not the engine's private helpers.
fn sort_buffer_allowance(required: usize) -> usize {
    let capacity = if required <= 16_384 {
        required
    } else {
        (required + 32).div_ceil(16_384) * 16_384 - 32
    };
    native_buffer_allowance(capacity)
}

pub(super) fn analytic_shapes(
    root: &Path,
    wrong_attribution: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use pipesql::DateValue;
    std::fs::create_dir(root)?;
    let path = root.join("database");
    let cancel = CancellationToken::new();
    let db = Database::create_empty(&path, Config::new(32_000_000, 32_000_000)?)?;
    for table in ["empty", "facts"] {
        db.declare_table(
            table,
            &[
                ColumnDeclaration {
                    name: "v",
                    data_type: DataType::Int64,
                    nullable: false,
                },
                ColumnDeclaration {
                    name: "d",
                    data_type: DataType::Date,
                    nullable: false,
                },
                ColumnDeclaration {
                    name: "t",
                    data_type: DataType::String,
                    nullable: true,
                },
            ],
            &cancel,
        )?;
    }
    let text = "雪\0".repeat(32);
    let day = DateValue::from_days_since_unix_epoch(-1).unwrap();
    let mut append = db.begin_append(
        "facts",
        AppendLimits {
            batches: 2,
            encoded_bytes: 200_000,
        },
        &cancel,
    )?;
    for start in [0, 256] {
        let values: [i64; 256] = std::array::from_fn(|i| (start + i) as i64);
        append.write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&values),
                    validity: &[255; 32],
                },
                ColumnInput {
                    values: ColumnValues::Date(&[day; 256]),
                    validity: &[255; 32],
                },
                ColumnInput {
                    values: ColumnValues::String(&[text.as_str(); 256]),
                    validity: &[0x55; 32],
                },
            ],
            &cancel,
        )?;
    }
    append.commit(&cancel)?;
    let repeated = format!(
        "FROM facts |> SELECT {}",
        vec!["COUNT(*) OVER ()"; 19].join(", ")
    );
    // Nineteen calls fit the parser's token budget; the twentieth must fail
    // preparation without retaining memory from the failed plan.
    let rejected = format!(
        "FROM facts |> SELECT {}",
        vec!["COUNT(*) OVER ()"; 20].join(", ")
    );
    let rejected_heap = Heap::now();
    let rejected_memory = db.reserved_memory_bytes();
    assert!(matches!(db.prepare(&rejected), Err(Error::Parse { .. })));
    assert_eq!(Heap::now(), rejected_heap);
    assert_eq!(db.reserved_memory_bytes(), rejected_memory);
    let wide = format!(
        "FROM facts |> SELECT {}, COUNT(*) OVER () AS n",
        vec!["t"; 63].join(", ")
    );
    let queries = [
        "FROM empty |> SELECT COUNT(*) OVER () AS n",
        "FROM facts |> SELECT COUNT(*) OVER () AS n",
        &repeated,
        "FROM facts |> SELECT v, d, t, COUNT(*) OVER () AS n",
        &wide,
        "FROM facts |> SELECT COUNT(*) OVER () AS n |> EXTEND COUNT(*) OVER () AS second",
        "FROM facts |> EXTEND COUNT(*) OVER () AS n |> AGGREGATE SUM(n) AS total GROUP BY v |> AGGREGATE SUM(total) AS total",
        "FROM facts |> EXCEPT DISTINCT (FROM facts |> WHERE v<256) |> SELECT COUNT(*) OVER () AS n",
        "FROM facts |> INTERSECT DISTINCT (FROM facts |> WHERE v<256) |> SELECT COUNT(*) OVER () AS n",
        "FROM facts |> UNION ALL (FROM facts) |> EXCEPT ALL (FROM facts |> WHERE v<256) |> SELECT COUNT(*) OVER () AS n",
        "FROM facts |> UNION ALL (FROM facts) |> INTERSECT ALL (FROM facts |> UNION ALL (FROM facts |> WHERE v<256)) |> SELECT COUNT(*) OVER () AS n",
        "FROM facts |> SELECT BYTE_LENGTH(t) AS bytes |> SELECT bytes, COUNT(*) OVER () AS n",
        "FROM facts |> SELECT t, COUNT(*) OVER () AS n |> SELECT BYTE_LENGTH(t) AS bytes, n",
        "FROM facts |> SELECT CHAR_LENGTH(t) AS characters |> SELECT characters, COUNT(*) OVER () AS n",
        "FROM facts |> SELECT t, COUNT(*) OVER () AS n |> SELECT CHAR_LENGTH(t) AS characters, n",
        "FROM facts |> SELECT CAST(v AS DOUBLE) AS v |> SELECT v, COUNT(*) OVER () AS n",
        "FROM facts |> SELECT v, COUNT(*) OVER () AS n |> SELECT CAST(v AS FLOAT64) AS v, n",
        "FROM facts |> SELECT EXTRACT(YEAR FROM d) AS y |> SELECT y, COUNT(*) OVER () AS n",
        "FROM facts |> SELECT d, COUNT(*) OVER () AS n |> SELECT EXTRACT(YEAR FROM d) AS y, n",
        "FROM facts |> SELECT COUNT(*) OVER (PARTITION BY d) AS n",
        "FROM facts |> SELECT COUNT(*) OVER (PARTITION BY t) AS n",
        "FROM facts |> SELECT COUNT(*) OVER (PARTITION BY v) AS n",
        "FROM facts |> SELECT SUM(v) OVER (ORDER BY d) AS n",
        "FROM facts |> SELECT SUM(v) OVER (ORDER BY v) AS n",
        "FROM facts |> SELECT v, SUM(v) OVER (PARTITION BY t ORDER BY v) AS n |> ORDER BY v",
    ];
    println!("entered analytic ownership shapes");
    let path_bytes = std::fs::canonicalize(&path)?.as_os_str().len() + "/units".len();
    let resident = db.reserved_memory_bytes();
    for (case, sql) in queries.into_iter().enumerate() {
        let before = Live::now();
        let descriptors = match case {
            6 => 5,
            7..=10 => 3,
            _ => 2,
        };
        let query = prepare_observed(&db, sql, "analytic", descriptors, false)?;
        let prepared_heap = Heap::now();
        let mut result = db.execute(&query, &cancel)?;
        let inline = std::mem::size_of::<QueryResult<'_, '_>>();
        let running_nonheap = inline + 4096 + 8192 - path_bytes + ANALYTIC_ROW_SCRATCH;
        // Cases 3 and 4 retain text: v/date/text in case 3 and one shared text
        // input for the repeated projections in case 4. COUNT has no sort key.
        let (frame, fields) = match case {
            3 => (32 + 9 + 5 + 65_541, 3),
            4 => (32 + 65_541, 1),
            _ => (0, 0),
        };
        let retained_sort = 3 * sort_buffer_allowance(frame);
        let run_sort = if fields == 0 {
            0
        } else {
            sort_buffer_allowance(frame + 255 * (32 + fields))
        };
        if case < 6 {
            Owner {
                charge: result.accounted_memory_bytes(),
                heap: Heap::now().increase_from(prepared_heap),
            }
            .report(
                "analytic-admitted",
                (running_nonheap
                    + retained_sort
                    + run_sort
                    + if case <= 2 { 0 } else { 8192 }
                    + usize::from(wrong_attribution)) as u64,
            );
        }
        let mut seen = 0;
        let mut finished = false;
        let mut sampled_rows = false;
        let mut sampled_spill = false;
        let mut peak_temp = 0;
        let mut minimum_headroom = i128::MAX;
        let mut steps = 0;
        loop {
            let charge = query.accounted_memory_bytes() + result.accounted_memory_bytes();
            assert_eq!(db.reserved_memory_bytes(), resident + charge);
            let heap = Heap::now().increase_from(Heap {
                requested: before.requested,
                usable: before.usable,
            });
            assert!(
                heap.requested as u64 <= charge,
                "analytic requested admission"
            );
            minimum_headroom = minimum_headroom.min(i128::from(charge) - heap.usable as i128);
            peak_temp = peak_temp.max(db.reserved_temp_bytes());
            if !finished && !sampled_spill && peak_temp != 0 && case < 6 {
                Owner {
                    charge: result.accounted_memory_bytes(),
                    heap: Heap::now().increase_from(prepared_heap),
                }
                .report(
                    "analytic-spilled",
                    (running_nonheap + retained_sort + run_sort) as u64,
                );
                sampled_spill = true;
            }
            if finished {
                break;
            }
            steps += 1;
            assert!(steps < 200_000, "analytic query did not finish");
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Finished => finished = true,
                QueryStep::Failed(error) => panic!("analytic case {case}: {error}"),
                QueryStep::Rows(batch) => {
                    let width = match case {
                        2 => 19,
                        4 => 64,
                        3 => 4,
                        5 | 11..=18 | 24 => 2,
                        _ => 1,
                    };
                    assert_eq!(batch.column_count(), width);
                    for row in 0..batch.len() {
                        assert!(
                            seen < if case == 6 {
                                1
                            } else if matches!(case, 9 | 10) {
                                768
                            } else {
                                512
                            }
                        );
                        for column in 0..width {
                            let expected = match (case, column) {
                                (19, _) => Value::Int64(512),
                                (20, _) => Value::Int64(256),
                                (21, _) => Value::Int64(1),
                                (22, _) => Value::Int64(130_816),
                                (23, _) => Value::Int64((seen * (seen + 1) / 2) as i64),
                                (24, 0) => Value::Int64(seen as i64),
                                (24, 1) => {
                                    let pairs = seen / 2;
                                    Value::Int64(if seen % 2 == 0 {
                                        (pairs * (pairs + 1)) as i64
                                    } else {
                                        ((pairs + 1) * (pairs + 1)) as i64
                                    })
                                }
                                (3, 0) => Value::Int64(seen as i64),
                                (3, 1) => Value::Date(day),
                                (3, 2) | (4, 0..=62) => {
                                    if seen % 2 == 0 {
                                        let Some(Value::String(actual)) = batch.value(row, column)
                                        else {
                                            panic!("analytic text value");
                                        };
                                        assert_eq!(actual.as_str(), text);
                                        continue;
                                    } else {
                                        Value::Null
                                    }
                                }
                                (15 | 16, 0) => Value::Double(seen as f64),
                                (17 | 18, 0) => Value::Int64(1969),
                                (6, _) => Value::Int64(262_144),
                                (7 | 8, _) => Value::Int64(256),
                                (9 | 10, _) => Value::Int64(768),
                                (11..=14, 0) => {
                                    if seen % 2 == 0 {
                                        Value::Int64(if case <= 12 { 128 } else { 64 })
                                    } else {
                                        Value::Null
                                    }
                                }
                                _ => Value::Int64(512),
                            };
                            assert_eq!(batch.value(row, column), Some(expected));
                        }
                        seen += 1;
                    }
                    if !sampled_rows && case < 6 {
                        Owner {
                            charge: result.accounted_memory_bytes(),
                            heap: Heap::now().increase_from(prepared_heap),
                        }
                        .report(
                            "analytic-emitting",
                            (running_nonheap + retained_sort) as u64,
                        );
                        sampled_rows = true;
                    }
                }
            }
        }
        assert_eq!(
            seen,
            match case {
                0 => 0,
                6 => 1,
                7 | 8 => 256,
                9 | 10 => 768,
                _ => 512,
            }
        );
        if case <= 2 {
            assert_eq!(peak_temp, 0, "zero-field count must not spool");
        } else {
            assert!(peak_temp > 0, "demanded input must spill");
        }
        Owner {
            charge: result.accounted_memory_bytes(),
            heap: Heap::now().increase_from(prepared_heap),
        }
        .report("analytic-finished", inline as u64);
        assert!(
            minimum_headroom >= 0,
            "analytic usable ownership attribution"
        );
        drop(result);
        assert_eq!(Heap::now(), prepared_heap);
        drop(query);
        assert_eq!(Live::now(), before);
        assert_eq!(db.reserved_memory_bytes(), resident);
        assert_eq!(db.reserved_temp_bytes(), 0);
        println!(
            "analytic case={case} rows={seen} steps={steps} minimum-usable-headroom={minimum_headroom} temporary={peak_temp} release=complete"
        );
    }
    db.close()?;
    println!("analytic shapes passed: 25 cases; rows, attribution and release");
    Ok(())
}

pub(super) fn prepared_aggregate_shapes(
    root: &Path,
    wrong_attribution: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(root)?;
    let input = root.join("lineitem.tbl");
    std::fs::write(
        &input,
        b"1|2|3|4|1|100|0.08|8|R|F|1994-01-01|12|13|14|15|16|\n".repeat(512),
    )?;
    let cancel = CancellationToken::new();
    let mut db = Database::create(&root.join("database"), Config::new(16_000_000, TEMP)?)?;
    db.load_lineitem(&input, &cancel)?;
    println!("entered prepared aggregate ownership shapes");
    let resident = db.reserved_memory_bytes();
    let mut accepted = 0;
    let mut rejected = 0;
    // Test one through 64 outputs and several ways to divide ten outputs among
    // stages. Each accepted stage owns an entry vector; widths above ten fail.
    for partition in
        (1..=64)
            .map(|width| vec![width])
            .chain([vec![1, 9], vec![5, 5], vec![9, 1], vec![1; 10]])
    {
        let mut sql = String::from("FROM lineitem");
        for &width in &partition {
            let entries: Vec<_> = (0..width).map(|i| format!("COUNT(*) AS n{i}")).collect();
            sql.push_str(" |> AGGREGATE ");
            sql.push_str(&entries.join(", "));
        }
        let before = Heap::now();
        if partition[0] > 10 {
            assert!(matches!(
                db.prepare(&sql),
                Err(Error::Parse { .. } | Error::Bind { .. })
            ));
            rejected += 1;
        } else {
            // One plan allocation, one aggregate-controller vector, and one
            // entry vector per stage. COUNT adds no computation descriptor.
            let prepared = prepare_observed(
                &db,
                &sql,
                "aggregate-descriptors",
                2 + partition.len(),
                wrong_attribution,
            )?;
            let prepared_heap = Heap::now();
            assert_eq!(
                db.reserved_memory_bytes(),
                resident + prepared.accounted_memory_bytes()
            );
            let mut result = db.execute(&prepared, &cancel)?;
            let mut rows = 0;
            let mut finished = false;
            let expected = if partition.len() == 1 { 512 } else { 1 };
            for _ in 0..100_000 {
                match result.step() {
                    QueryStep::Progress => (),
                    QueryStep::Rows(batch) => {
                        assert_eq!(batch.len(), 1);
                        assert_eq!(batch.column_count(), *partition.last().unwrap());
                        for column in 0..batch.column_count() {
                            assert_eq!(batch.value(0, column), Some(Value::Int64(expected)));
                        }
                        rows += batch.len();
                    }
                    QueryStep::Finished => {
                        finished = true;
                        break;
                    }
                    QueryStep::Failed(error) => panic!("prepared aggregate rows: {error}"),
                }
            }
            assert!(finished);
            assert_eq!(rows, 1);
            drop(result);
            assert_eq!(Heap::now(), prepared_heap);
            drop(prepared);
            accepted += 1;
        }
        assert_eq!(Heap::now(), before);
        assert_eq!(db.reserved_memory_bytes(), resident);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    assert_eq!((accepted, rejected), (14, 54));
    println!(
        "prepared aggregate shapes passed: 14 accepted and 54 rejected; attribution, rows and release"
    );
    Ok(())
}

// Subtract the scan's known heap buffers from its fixed reservation to find the
// part that need not appear in allocator observations: 1,344 descriptors of 16
// bytes and 4,096 selection indices of four bytes. Add the plan's 4,096-byte
// allowance. Arena and typed-batch requests are charged separately at exact size.
const LEGACY_SCAN_NONHEAP: usize = 106_496 - 1_344 * 16 - 4_096 * 4 + 4_096;

pub(super) fn legacy_constant_shapes(
    root: &Path,
    wrong_attribution: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(root)?;
    let path = root.join("database");
    let input = root.join("lineitem.tbl");
    const ROW: &[u8] = b"1|2|3|4|1|100|0.08|8|R|F|1994-01-01|12|13|14|15|16|\n";
    std::fs::write(&input, ROW.repeat(512))?;
    let cancel = CancellationToken::new();
    let mut db = Database::create(&path, Config::new(16_000_000, 8_000_000)?)?;
    db.load_lineitem(&input, &cancel)?;
    println!("entered legacy constant ownership shapes");
    let resident = db.reserved_memory_bytes();
    for (expression, expected, computed) in [
        ("l_returnflag", "R", false),
        ("''", "", true),
        (
            "'雪12345678901234567890123456789'",
            "雪12345678901234567890123456789",
            true,
        ),
    ] {
        for width in [1, 64] {
            let sql = format!(
                "FROM lineitem |> SELECT {}",
                vec![expression; width].join(", ")
            );
            let prepared =
                prepare_observed(&db, &sql, "legacy-text", 1 + usize::from(computed), false)?;
            let before = Heap::now();
            let mut result = db.execute(&prepared, &cancel)?;
            let nonheap = (std::mem::size_of::<QueryResult<'_, '_>>()
                + LEGACY_SCAN_NONHEAP
                + usize::from(wrong_attribution)) as u64;
            let observe = |result: &QueryResult<'_, '_>| {
                let owner = Owner {
                    charge: result.accounted_memory_bytes(),
                    heap: Heap::now().increase_from(before),
                };
                owner.report("legacy-text", nonheap);
                assert!(
                    owner.heap.usable as u64 <= owner.charge,
                    "legacy usable allocations exceed admission"
                );
            };
            observe(&result);
            let mut rows = 0;
            let mut batches = 0;
            let mut finished = false;
            for _ in 0..100_000 {
                match result.step() {
                    QueryStep::Progress => (),
                    QueryStep::Rows(batch) => {
                        assert_eq!(batch.column_count(), width);
                        assert_eq!(batch.len(), 256);
                        for row in 0..batch.len() {
                            for column in 0..width {
                                let Some(Value::String(text)) = batch.value(row, column) else {
                                    panic!("legacy text value");
                                };
                                assert_eq!(text.as_str(), expected);
                            }
                        }
                        rows += batch.len();
                        batches += 1;
                        observe(&result);
                    }
                    QueryStep::Finished => {
                        finished = true;
                        break;
                    }
                    QueryStep::Failed(error) => panic!("legacy constant shape: {error}"),
                }
            }
            assert!(finished);
            assert_eq!((rows, batches), (512, 2));
            Owner {
                charge: result.accounted_memory_bytes(),
                heap: Heap::now().increase_from(before),
            }
            .report(
                "legacy-text-finished",
                std::mem::size_of::<QueryResult<'_, '_>>() as u64,
            );
            drop(result);
            assert_eq!(Heap::now(), before);
            drop(prepared);
            assert_eq!(db.reserved_memory_bytes(), resident);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
    }
    for (expression, expected) in [
        ("''", ""),
        (
            "'雪12345678901234567890123456789'",
            "雪12345678901234567890123456789",
        ),
    ] {
        for grouped in [false, true] {
            let suffix = if grouped {
                " GROUP AND ORDER BY label"
            } else {
                ""
            };
            let sql = format!(
                "FROM lineitem |> SELECT {expression} AS label |> AGGREGATE MIN(label) AS lo, MAX(label) AS hi, COUNT(*) AS n{suffix}"
            );
            let prepared = prepare_observed(&db, &sql, "legacy-extrema", 4, false)?;
            let before = Heap::now();
            let mut result = db.execute(&prepared, &cancel)?;
            // Reservations also include 4,096 bytes for the aggregate vector
            // and, for grouping, two 4,096-byte paths before scratch files exist.
            // The grouped case retains its unused disk fallback while emitting
            // from the hash table: three output strings, one shared MIN/MAX
            // argument, a 4,096-row run and two arrays of 16-byte spans.
            let sort_allowance = if grouped {
                sort_buffer_allowance(32 + 3 * 65_541 + 9)
                    + 2 * sort_buffer_allowance(32 + 65_541 + 8 + 65_536)
                    + sort_buffer_allowance(32 + 65_541 + 8 + 65_536 + 4095 * (32 + 1 + 8))
                    + 2 * sort_buffer_allowance(4096 * 16)
                    + sort_buffer_allowance(65_541)
            } else {
                0
            };
            let nonheap = (std::mem::size_of::<QueryResult<'_, '_>>()
                + LEGACY_SCAN_NONHEAP
                + 4096
                + sort_allowance
                + if grouped { 8192 } else { 0 }) as u64;
            let observe = |result: &QueryResult<'_, '_>| {
                let owner = Owner {
                    charge: result.accounted_memory_bytes(),
                    heap: Heap::now().increase_from(before),
                };
                owner.report("legacy-extrema", nonheap);
                assert!(owner.heap.usable as u64 <= owner.charge);
            };
            observe(&result);
            let mut rows = 0;
            let mut finished = false;
            for _ in 0..100_000 {
                match result.step() {
                    QueryStep::Progress => (),
                    QueryStep::Rows(batch) => {
                        let text_columns = if grouped { 3 } else { 2 };
                        assert_eq!(batch.column_count(), text_columns + 1);
                        assert_eq!(batch.len(), 1);
                        for column in 0..text_columns {
                            let Some(Value::String(text)) = batch.value(0, column) else {
                                panic!("legacy extrema value");
                            };
                            assert_eq!(text.as_str(), expected);
                        }
                        assert_eq!(batch.value(0, text_columns), Some(Value::Int64(512)));
                        rows += batch.len();
                        observe(&result);
                    }
                    QueryStep::Finished => {
                        finished = true;
                        break;
                    }
                    QueryStep::Failed(error) => panic!("legacy extrema: {error}"),
                }
                assert_eq!(
                    db.reserved_temp_bytes(),
                    0,
                    "small groups retain the hash path"
                );
            }
            assert!(finished);
            assert_eq!(rows, 1);
            Owner {
                charge: result.accounted_memory_bytes(),
                heap: Heap::now().increase_from(before),
            }
            .report(
                "legacy-extrema-finished",
                std::mem::size_of::<QueryResult<'_, '_>>() as u64,
            );
            drop(result);
            assert_eq!(Heap::now(), before);
            drop(prepared);
            assert_eq!(db.reserved_memory_bytes(), resident);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
    }
    println!(
        "legacy constant shapes passed: 6 direct and 4 extrema cases; rows, attribution and release"
    );
    Ok(())
}

/// Compare reader reservations with heap use after ORDER BY and DISTINCT spill.
///
/// One- and 64-column tables cover every stored type, NULLs, duplicate rows and
/// maximum-length text. Check all returned values, then require execution to
/// release its memory and temporary space while the prepared plan remains alive.
pub(super) fn reader_shapes(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use pipesql::DateValue;
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    if let Ok(threshold) = std::env::var("MALLOC_MMAP_THRESHOLD_")
        && (threshold == "131072" || threshold == "67108864")
    {
        // Check the actual extra bytes returned for a 512-KiB request. This
        // distinguishes the two selected glibc paths even if an environment
        // variable was ignored; setting the variable alone proves nothing.
        let extent = allocation_extent::<u8>(524_288);
        if threshold == "131072" {
            assert!(extent.1 > 32, "mapped allocation control: {extent:?}");
        } else {
            assert!(extent.1 <= 32, "arena allocation control: {extent:?}");
        }
        println!("reader allocator threshold={threshold} observed {extent:?}");
    }
    std::fs::create_dir(root)?;
    let dates = [2, 1, 2, 0].map(|day| DateValue::from_days_since_unix_epoch(day).unwrap());
    let maximum = format!("{}x", "雪".repeat(21_845));
    assert_eq!(maximum.len(), 65_536);
    let short = ["雪\t\n", "", "雪\t\n", "ignored"];
    let long = [maximum.as_str(), "a", maximum.as_str(), "ignored"];
    for (profile, kind, values) in [
        ("int64", DataType::Int64, ColumnValues::Int64(&[2, 1, 2, 0])),
        (
            "float",
            DataType::Double,
            ColumnValues::Double(&[2., 1., 2., 0.]),
        ),
        ("dates", DataType::Date, ColumnValues::Date(&dates)),
        ("short", DataType::String, ColumnValues::String(&short)),
        ("large", DataType::String, ColumnValues::String(&long)),
    ] {
        for width in [1, 64] {
            let path = root.join(format!("{profile}-{width:02}"));
            // Up to 64 columns each contain two 64-KiB strings. Their sort data
            // needs more memory and temporary space than the fixed-width cases.
            let temporary = if kind == DataType::String {
                64_000_000
            } else {
                TEMP
            };
            let memory = if kind == DataType::String && cfg!(target_os = "macos") {
                // Full-width text retains large capture and merge buffers plus
                // their cached-allocation allowances throughout the sort.
                128 * 1024 * 1024
            } else if kind == DataType::String {
                64 * 1024 * 1024
            } else {
                64_000_000
            };
            let config = Config::new(memory, temporary)?;
            let db = Database::create_empty(&path, config)?;
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
                    encoded_bytes: if kind == DataType::String {
                        // Two maximum cells plus one short cell, offsets and
                        // validity in each of at most 64 columns.
                        9_000_000
                    } else {
                        100_000
                    },
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
                println!(
                    "reader shape profile={profile} type={kind:?} width={width} distinct={distinct}"
                );
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
                // Subtract inline storage, the plan allowance and unused path
                // allowance from the charge. Payload padding is part of the heap
                // request, so it must remain in the observed bytes. Short rows
                // fit in one buffered write, which becomes visible as the run
                // is freed. Maximum text forces earlier writes with the run
                // still live. Capture, two merge records and the key remain.
                let field_bytes = match kind {
                    DataType::Int64 | DataType::Double => 9,
                    DataType::Date => 5,
                    DataType::String => 65_541,
                };
                let sort_allowance = 3 * sort_buffer_allowance(32 + width * field_bytes)
                    + sort_buffer_allowance(if distinct { width } else { 1 } * field_bytes)
                    + if profile == "large" {
                        sort_buffer_allowance(32 + width * field_bytes + 255 * (32 + width))
                    } else {
                        0
                    };
                owner.report(
                    "typed-reader",
                    (std::mem::size_of::<QueryResult<'_, '_>>() + 4096 + 8192 - path_bytes
                        + sort_allowance) as u64,
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
                                    Some(Value::String(text)) if kind == DataType::String => {
                                        let expected = if profile == "short" {
                                            ["", "雪\t\n"]
                                        } else {
                                            ["a", maximum.as_str()]
                                        };
                                        if text.as_str() == expected[0] {
                                            1
                                        } else {
                                            assert_eq!(text.as_str(), expected[1]);
                                            2
                                        }
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
        "reader shapes passed: 12 fixed-width and 8 STRING ordering/distinct cases; rows, admission and release"
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

// Ask the allocator for the buffer sizes used by blocking operators and grouped
// hashes. These calls test allocator rounding; they do not execute queries.
pub(super) fn grouped_allocation_shapes() {
    let mut buffers = (0, 0);
    // A 128-value frame plus 255 minimum-width rows is at most 8,430,080
    // encoded bytes. Its final allocation unit ends at 8,437,760 bytes.
    for units in 2..=515 {
        let observed = allocation_extent::<u8>(units * 16_384 - 32);
        assert!(
            observed.1 <= 32,
            "blocking allocation rounding: {observed:?}"
        );
        if cfg!(target_os = "macos") {
            assert_eq!(observed.1, 32, "Darwin blocking allocation boundary");
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
    println!("grouped allocation extents: largest-buffer={buffers:?} largest-hash={arrays:?}");
    println!("grouped allocation shapes passed: buffers=514 hash-layouts=91");
}

// Append reserves extra space for allocator rounding on three retained buffers.
// Request every workspace size and reference-array length in their allowed ranges
// and check the extra usable bytes against the 16-KiB allowance.
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
    println!("append allocation extents: largest={largest:?} reference-largest={references:?}");
    println!("append allocation shapes passed: workspace=460865 references=4096");
}

pub(super) fn append_shapes(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("entered append cache-history checks");
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
        // Three committed units plus 4,093 admitted batches require capacity for
        // 4,096 references. Only three batches are written in each append.
        let before = Heap::now();
        let charge = db.reserved_memory_bytes();
        super::cache_allocation(245_760);
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
            super::cache_allocation(1_032_192);
            append.write(&columns, &cancel)?;
            let owner = Owner {
                heap: Heap::now().increase_from(before),
                charge: db.reserved_memory_bytes() - charge,
            };
            assert!(
                owner.heap.usable as u64 <= owner.charge,
                "append usable allocations exceed admission"
            );
            let reference_bytes = (3 * index + batches as usize) * 32;
            let workspace_bytes = if phase == 0 { 65_536 } else { 526_400 };
            owner.report(
                "wide-append",
                (std::mem::size_of::<Append<'_>>()
                    + 8192
                    + 3 * 16_384
                    + native_buffer_allowance(65_536)
                    + native_buffer_allowance(reference_bytes)
                    + native_buffer_allowance(workspace_bytes)) as u64,
            );
            if phase == 1 {
                let growth = if cfg!(target_os = "macos") {
                    1_081_344 - 131_072
                } else {
                    526_400 - 65_536
                };
                assert_eq!(owner.charge - prior_workspace_charge, growth);
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

// Repeat one text column into 61 output fields to make wide rows from a small
// input. Set operations must preserve the listed duplicate counts and all text
// fields; UNION ALL streams, while the other five cases use temporary storage.
pub(super) fn wide_set_shapes(
    root: &Path,
    wrong_attribution: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(root)?;
    let path = root.join("database");
    let cancel = CancellationToken::new();
    let db = Database::create_empty(&path, Config::new(192_000_000, 256_000_000)?)?;
    let long = "雪".repeat(21_845) + "x";
    let short = "a\0雪";
    let texts = [None, Some(short), Some(long.as_str()), Some("")];
    for (table, frequencies) in [
        ("left_rows", [3, 2, 1, 0, 3, 2, 1, 0]),
        ("right_rows", [1, 2, 3, 4, 0, 1, 0, 1]),
    ] {
        // Store the right side wide to avoid spelling two long projections in
        // one query. Its 62 columns plus the left's two reach the source-column
        // limit, while the left still tests repeated use of one stored payload.
        let width = if table == "right_rows" { 62 } else { 2 };
        let names: Vec<_> = (0..width)
            .map(|column| match column {
                0 => "id".to_owned(),
                1 => "text".to_owned(),
                _ => format!("text{column}"),
            })
            .collect();
        let declarations: Vec<_> = names
            .iter()
            .enumerate()
            .map(|(column, name)| ColumnDeclaration {
                name,
                data_type: if column == 0 {
                    DataType::Int64
                } else {
                    DataType::String
                },
                nullable: column != 0,
            })
            .collect();
        db.declare_table(table, &declarations, &cancel)?;
        let mut append = db.begin_append(
            table,
            AppendLimits {
                batches: 12,
                encoded_bytes: 32_000_000,
            },
            &cancel,
        )?;
        for (id, count) in frequencies.into_iter().enumerate() {
            for _ in 0..count {
                let text = texts[id % 4];
                let ids = [id as i64];
                let values = [text.unwrap_or("hidden")];
                let validity = [u8::from(text.is_some())];
                let inputs: Vec<_> = (0..width)
                    .map(|column| {
                        if column == 0 {
                            ColumnInput {
                                values: ColumnValues::Int64(&ids),
                                validity: &[1],
                            }
                        } else {
                            ColumnInput {
                                values: ColumnValues::String(&values),
                                validity: &validity,
                            }
                        }
                    })
                    .collect();
                append.write(&inputs, &cancel)?;
            }
        }
        append.commit(&cancel)?;
    }
    let columns = format!("id, {}", vec!["text"; 61].join(", "));
    println!("wide sets: 62 positions, nullable maximum STRING, duplicate occurrences");
    for (operation, expected) in [
        (
            "UNION ALL",
            &[
                0, 0, 0, 1, 1, 2, 4, 4, 4, 5, 5, 6, 0, 1, 1, 2, 2, 2, 3, 3, 3, 3, 5, 7,
            ][..],
        ),
        ("UNION DISTINCT", &[0, 1, 2, 3, 4, 5, 6, 7][..]),
        ("EXCEPT DISTINCT", &[4, 6][..]),
        ("INTERSECT DISTINCT", &[0, 1, 2, 5][..]),
        ("EXCEPT ALL", &[0, 0, 4, 4, 4, 5, 6][..]),
        ("INTERSECT ALL", &[0, 1, 1, 2, 5][..]),
    ] {
        let sql = format!("FROM left_rows |> SELECT {columns} |> {operation} (FROM right_rows)");
        let before = Live::now();
        let memory = db.reserved_memory_bytes();
        let query = db.prepare(&sql)?;
        let mut result = db.execute(&query, &cancel)?;
        let mut seen = 0;
        let mut finished = false;
        let mut steps = 0;
        let mut peak_temp = 0;
        let mut minimum_headroom = i128::MAX;
        loop {
            let charge = query.accounted_memory_bytes() + result.accounted_memory_bytes();
            assert_eq!(db.reserved_memory_bytes(), memory + charge);
            let heap = Heap::now().increase_from(Heap {
                requested: before.requested,
                usable: before.usable,
            });
            assert!(
                heap.requested as u64 <= charge,
                "wide set requested admission"
            );
            let attributed =
                heap.usable as u128 + u128::from(wrong_attribution) * u128::from(charge);
            minimum_headroom = minimum_headroom.min(i128::from(charge) - attributed as i128);
            peak_temp = peak_temp.max(db.reserved_temp_bytes());
            if finished {
                break;
            }
            steps += 1;
            assert!(steps < 200_000, "wide set did not finish");
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Finished => finished = true,
                QueryStep::Failed(error) => panic!("wide set {operation}: {error}"),
                QueryStep::Rows(batch) => {
                    assert_eq!(batch.column_count(), 62);
                    for row in 0..batch.len() {
                        let id = *expected.get(seen).expect("unexpected set row");
                        assert_eq!(batch.value(row, 0), Some(Value::Int64(id)));
                        for column in 1..62 {
                            match texts[id as usize % 4] {
                                None => assert_eq!(batch.value(row, column), Some(Value::Null)),
                                Some(expected) => {
                                    let Some(Value::String(actual)) = batch.value(row, column)
                                    else {
                                        panic!("wide set STRING output");
                                    };
                                    assert_eq!(actual.as_str(), expected);
                                }
                            }
                        }
                        seen += 1;
                    }
                }
            }
        }
        assert_eq!(seen, expected.len());
        if operation == "UNION ALL" {
            assert_eq!(peak_temp, 0);
        } else {
            assert!(peak_temp > 0, "wide set must use external storage");
        }
        drop(result);
        drop(query);
        assert_eq!(Live::now(), before);
        assert_eq!(db.reserved_memory_bytes(), memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
        println!(
            "wide set operation={operation} rows={seen} steps={steps} minimum-usable-headroom={minimum_headroom} temporary={peak_temp} release=complete"
        );
        assert!(
            minimum_headroom >= 0,
            "wide set usable ownership attribution"
        );
    }
    db.close()?;
    println!("wide set shapes passed: 6 cases; complete rows, step ownership and release");
    Ok(())
}
