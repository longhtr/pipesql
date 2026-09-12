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

// Row-evaluation payload on the qualified 64-bit targets: 80 optional nullable
// values (16 bytes), 208 demand flags, 32 optional column identities (8 bytes),
// 32 optional numeric inputs (48 bytes), three 32-word arrays, 32 four-word
// vectors and 32 type tags. This equation does not call engine admission code.
const ANALYTIC_ROW_SCRATCH: usize = 80 * 16 + 208 + 32 * 8 + 32 * 48 + 3 * 32 * 8 + 32 * 32 + 32;

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
        vec!["COUNT(*) OVER ()"; 19].join(",")
    );
    // Four stage tokens plus seven per call and separators admit nineteen.
    let rejected = format!(
        "FROM facts |> SELECT {}",
        vec!["COUNT(*) OVER ()"; 20].join(",")
    );
    let rejected_heap = Heap::now();
    assert!(matches!(db.prepare(&rejected), Err(Error::Parse { .. })));
    assert_eq!(Heap::now(), rejected_heap);
    let wide = format!(
        "FROM facts |> SELECT {},COUNT(*) OVER () AS n",
        vec!["t"; 63].join(",")
    );
    let queries = [
        "FROM empty |> SELECT COUNT(*) OVER () AS n",
        "FROM facts |> SELECT COUNT(*) OVER () AS n",
        &repeated,
        "FROM facts |> SELECT v,d,t,COUNT(*) OVER () AS n",
        &wide,
        "FROM facts |> SELECT COUNT(*) OVER () AS n |> EXTEND COUNT(*) OVER () AS second",
        "FROM facts |> EXTEND COUNT(*) OVER () AS n |> AGGREGATE SUM(n) AS total GROUP BY v |> AGGREGATE SUM(total) AS total",
    ];
    println!("entered analytic ownership shapes");
    let path_bytes = std::fs::canonicalize(&path)?.as_os_str().len() + "/units".len();
    let resident = db.reserved_memory_bytes();
    for (case, sql) in queries.into_iter().enumerate() {
        let before = Live::now();
        let query = prepare_observed(&db, sql, "analytic", if case == 6 { 5 } else { 2 }, false)?;
        let prepared_heap = Heap::now();
        let mut result = db.execute(&query, &cancel)?;
        let inline = std::mem::size_of::<QueryResult<'_, '_>>();
        let running_nonheap = inline + 4096 + 8192 - path_bytes + ANALYTIC_ROW_SCRATCH;
        if case < 6 {
            Owner {
                charge: result.accounted_memory_bytes(),
                heap: Heap::now().increase_from(prepared_heap),
            }
            .report(
                "analytic-admitted",
                (running_nonheap
                    + if case == 5 { 16384 } else { 8192 }
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
                .report("analytic-spilled", running_nonheap as u64);
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
                        5 => 2,
                        _ => 1,
                    };
                    assert_eq!(batch.column_count(), width);
                    for row in 0..batch.len() {
                        assert!(seen < if case == 6 { 1 } else { 512 });
                        for column in 0..width {
                            let expected = match (case, column) {
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
                                (6, _) => Value::Int64(262_144),
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
                        .report("analytic-emitting", running_nonheap as u64);
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
                _ => 512,
            }
        );
        assert!(case == 0 || peak_temp > 0, "analytic input must spill");
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
    println!("analytic shapes passed: 7 cases; rows, attribution and release");
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
    // Every accepted width allocates one entry vector. Larger widths exercise
    // rejection under the ten-output aggregate budget and syntax bounds.
    for partition in
        (1..=64)
            .map(|width| vec![width])
            .chain([vec![1, 9], vec![5, 5], vec![9, 1], vec![1; 10]])
    {
        let mut sql = String::from("FROM lineitem");
        for &width in &partition {
            let entries: Vec<_> = (0..width).map(|i| format!("COUNT(*) AS n{i}")).collect();
            sql.push_str(" |> AGGREGATE ");
            sql.push_str(&entries.join(","));
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

// Legacy scan attribution derives from its explicit fixed reservation. The
// arena and typed batch allocations are charged at their requested extents;
// 1,344 sixteen-byte descriptors and 4,096 u32 selection entries consume the
// heap portion of the 106,496-byte fixed scan reservation. Runtime nodes have
// exact charges; the physical plan adds its independent 4,096-byte allowance.
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
                vec![expression; width].join(",")
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
                "FROM lineitem |> SELECT {expression} AS label |> AGGREGATE MIN(label) AS lo,MAX(label) AS hi,COUNT(*) AS n{suffix}"
            );
            let prepared = prepare_observed(&db, &sql, "legacy-extrema", 4, false)?;
            let before = Heap::now();
            let mut result = db.execute(&prepared, &cancel)?;
            // The aggregate vector adds 4,096 bytes of allowance. General
            // grouping charges its indirect controller and buffers exactly,
            // but retains two 4,096-byte scratch paths before creating files.
            let nonheap = (std::mem::size_of::<QueryResult<'_, '_>>()
                + LEGACY_SCAN_NONHEAP
                + 4096
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

pub(super) fn reader_shapes(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use pipesql::DateValue;
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    if let Ok(threshold) = std::env::var("MALLOC_MMAP_THRESHOLD_") {
        if threshold == "131072" || threshold == "67108864" {
            // The fresh caller must observe the selected allocation path, not
            // merely inherit an environment variable that the allocator ignores.
            let extent = allocation_extent::<u8>(524_288);
            if threshold == "131072" {
                assert!(extent.1 > 32, "mapped allocation control: {extent:?}");
            } else {
                assert!(extent.1 <= 32, "arena allocation control: {extent:?}");
            }
            println!("reader allocator threshold={threshold} observed {extent:?}");
        }
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
            // Maximum text rows and padded payloads need larger admitted owners.
            // Fixed-width cases retain their original memory and scratch budgets.
            let temporary = if kind == DataType::String {
                64_000_000
            } else {
                TEMP
            };
            let memory = if kind == DataType::String {
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

// Independent domains for the padded blocking buffers and the GROUPED caller's
// power-of-two hash layouts. Observe usable extents separately from capacities.
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

// The workload matches the learning example; expected values below follow the
// literal two source rows per key, independently of the sorter and hash layout.
pub(super) fn joined_shapes(
    root: &Path,
    wrong_attribution: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(root)?;
    let path = root.join("database");
    let cancel = CancellationToken::new();
    create_joined_sales(&path, &cancel)?;
    // Initialize stdout before the observed interval. No caller heap storage
    // changes between this baseline and destruction of prepared/result owners.
    println!("joined shapes: nullable self-join, grouping, descending order");
    for budget in [2_200_000, 12_000_000] {
        let db = Database::open(&path, Config::new(budget, TEMP)?)?;
        let before = Live::now();
        let memory = db.reserved_memory_bytes();
        let query = db.prepare(
            "FROM sales AS s |> JOIN sales AS copies ON s.region = copies.region \
             |> AGGREGATE COUNT(*) AS n,COUNT(s.amount) AS present,SUM(s.amount) AS total \
             GROUP BY s.region |> ORDER BY region DESC",
        )?;
        let mut result = db.execute(&query, &cancel)?;
        let mut seen = 0;
        let mut progress = 0;
        let mut row_steps = 0;
        let mut finished = false;
        let mut minimum_headroom = i128::MAX;
        let mut peak_temp = 0;
        // Sample execute, every returned step, and Finished before dropping the
        // owner. Borrowed rows are checked before sampling and allocate nothing.
        // The campaign bounds this complete workload with its subprocess deadline.
        loop {
            let charge = query.accounted_memory_bytes() + result.accounted_memory_bytes();
            assert_eq!(db.reserved_memory_bytes(), memory + charge);
            let requested = LIVE_REQUESTED
                .load(Ordering::Relaxed)
                .checked_sub(before.requested)
                .unwrap();
            let usable = LIVE_USABLE
                .load(Ordering::Relaxed)
                .checked_sub(before.usable)
                .unwrap();
            assert!(
                requested as u64 <= charge,
                "joined requested allocation admission"
            );
            // The negative control adds a nonexistent owner to the measured heap.
            // It must fail this same guard after full rows and release are checked.
            let attributed = usable as u128 + u128::from(wrong_attribution) * u128::from(charge);
            minimum_headroom = minimum_headroom.min(i128::from(charge) - attributed as i128);
            peak_temp = peak_temp.max(db.reserved_temp_bytes());
            if finished {
                break;
            }
            match result.step() {
                QueryStep::Progress => progress += 1,
                QueryStep::Finished => finished = true,
                QueryStep::Failed(error) => panic!("joined query: {error:?}"),
                QueryStep::Rows(batch) => {
                    row_steps += 1;
                    assert_eq!(batch.column_count(), 4);
                    for row in 0..batch.len() {
                        assert!(seen < ROWS);
                        let key = (ROWS - 1 - seen) as i64;
                        let (present, total) = match key % 4 {
                            0 => (0, Value::Null),
                            1 => (2, Value::Int64(6)),
                            _ => (4, Value::Int64(8)),
                        };
                        assert_eq!(batch.value(row, 0), Some(Value::Int64(key)));
                        assert_eq!(batch.value(row, 1), Some(Value::Int64(4)));
                        assert_eq!(batch.value(row, 2), Some(Value::Int64(present)));
                        assert_eq!(batch.value(row, 3), Some(total));
                        seen += 1;
                    }
                }
            }
        }
        assert!(
            finished && seen == ROWS && progress > 0 && row_steps > 0,
            "joined incomplete: finished={finished} rows={seen} progress={progress} row_steps={row_steps}"
        );
        assert!(peak_temp > 0, "joined workload must exercise sorted inputs");
        drop(result);
        drop(query);
        assert_eq!(Live::now(), before);
        assert_eq!(db.reserved_memory_bytes(), memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
        db.close()?;
        println!(
            "joined budget={budget} rows={seen} progress={progress} row-steps={row_steps} minimum-usable-headroom={minimum_headroom} temporary={peak_temp} release=complete"
        );
        assert!(minimum_headroom >= 0, "joined usable ownership attribution");
    }
    println!("joined shapes passed: 2 budgets; complete rows, step ownership and release");
    Ok(())
}

fn create_joined_sales(path: &Path, cancel: &CancellationToken) -> Result<(), pipesql::Error> {
    // Build with a separate budget so the experiment measures query admission.
    let db = Database::create_empty(path, Config::new(32_000_000, TEMP)?)?;
    db.declare_table(
        "sales",
        &["region", "amount"].map(|name| ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: name == "amount",
        }),
        cancel,
    )?;
    let mut append = db.begin_append(
        "sales",
        AppendLimits {
            batches: (2 * ROWS / 256) as u32,
            encoded_bytes: 1_000_000,
        },
        cancel,
    )?;
    for amount in [1, 3] {
        for start in (0..ROWS).step_by(256) {
            let regions: [i64; 256] = std::array::from_fn(|row| (ROWS - 1 - start - row) as i64);
            let mut validity = [0_u8; 256 / 8];
            for (row, region) in regions.iter().enumerate() {
                if region % 4 >= 2 || (region % 4 == 1 && amount == 3) {
                    validity[row / 8] |= 1 << (row % 8);
                }
            }
            append.write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&regions),
                        validity: &[255; 256 / 8],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&[amount; 256]),
                        validity: &validity,
                    },
                ],
                cancel,
            )?;
        }
    }
    append.commit(cancel)?;
    db.close()
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
    assert!(
        usable as u64 <= query.accounted_memory_bytes(),
        "prepared usable allocations exceed admission"
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
