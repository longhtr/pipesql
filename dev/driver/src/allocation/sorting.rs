//! Refuse each optional run allocation during a sort's first execution step.
//!
//! Scratch creation precedes the three replacement allocations and must succeed.
//! Optional replacement must fall back to the admitted buffers. Observe events,
//! then finish every query against literal ordered values. A disabled-observer control
//! must fail even when the query itself returns the correct answer.

use super::{CALLS, REFUSED, measurement::Live, transient_ownership::Observer, workload};
use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, QueryStep, Value,
};
use std::{path::Path, sync::atomic::Ordering};

pub(super) fn run(
    root: &Path,
    missing_observation: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(root)?;
    let cancel = CancellationToken::new();
    let database =
        Database::create_empty(&root.join("database"), Config::new(8_000_000, 8_000_000)?)?;
    database.declare_table(
        "facts",
        &[ColumnDeclaration {
            name: "v",
            data_type: DataType::Int64,
            nullable: false,
        }],
        &cancel,
    )?;
    let mut append = database.begin_append(
        "facts",
        AppendLimits {
            batches: 1,
            encoded_bytes: 65_536,
        },
        &cancel,
    )?;
    append.write(
        &[ColumnInput {
            values: ColumnValues::Int64(&[8, 7, 6, 5, 4, 3, 2, 1, 0]),
            validity: &[255, 1],
        }],
        &cancel,
    )?;
    append.commit(&cancel)?;
    let query = database.prepare("FROM facts |> ORDER BY v")?;
    // Initialize stdout's process-lifetime buffer before measuring query owners.
    println!("optional sort growth: first-step refusal and complete fallback");
    let prepared = Live::now();
    let prepared_memory = database.reserved_memory_bytes();
    let census = {
        let mut result = database.execute(&query, &cancel)?;
        workload::arm(None, 16);
        assert!(matches!(result.step(), QueryStep::Progress));
        workload::suspend_faults();
        let count = CALLS.load(Ordering::Relaxed);
        assert_eq!(REFUSED.load(Ordering::Relaxed), 0);
        assert!(
            (5..=16).contains(&count),
            "missing optional run allocations"
        );
        drop(result);
        assert_eq!(Live::now(), prepared);
        assert_eq!(database.reserved_memory_bytes(), prepared_memory);
        count
    };
    println!("sort growth census allocations={census}");
    let mut fallbacks = 0;
    for prefix in census - 3..=census {
        let mut result = database.execute(&query, &cancel)?;
        let observer = Observer::new(
            &database,
            prepared_memory,
            prepared.requested,
            prepared.usable,
        );
        workload::arm(Some(prefix), 16);
        let first = if missing_observation && prefix == census - 3 {
            result.step()
        } else {
            observer.during(|| result.step())
        };
        let survived = matches!(first, QueryStep::Progress);
        workload::suspend_faults();
        assert!(
            survived,
            "optional sort allocation refusal must retain the admitted buffers"
        );
        let calls = CALLS.load(Ordering::Relaxed);
        let refusals = REFUSED.load(Ordering::Relaxed);
        let samples = observer.samples();
        assert_eq!(
            samples.allocations, prefix,
            "missing optional sort allocation events"
        );
        assert!(
            samples.requested_headroom >= 0 && samples.usable_headroom >= 0,
            "optional sort allocation exceeds reservation: {samples:?}"
        );
        if prefix == census {
            assert_eq!((calls, refusals), (census, 0));
        } else {
            assert_eq!((calls, refusals), (prefix + 1, 1));
            fallbacks += 1;
        }
        let mut rows = 0;
        let mut finished = false;
        for _ in 0..4096 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Rows(batch) => {
                    assert_eq!(batch.column_count(), 1);
                    for row in 0..batch.len() {
                        assert!(rows < 9);
                        assert_eq!(batch.value(row, 0), Some(Value::Int64(rows)));
                        rows += 1;
                    }
                }
                QueryStep::Finished => {
                    finished = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("optional sort fallback failed: {error}"),
            }
        }
        assert!(finished);
        assert_eq!(rows, 9);
        drop(result);
        assert_eq!(Live::now(), prepared);
        assert_eq!(database.reserved_memory_bytes(), prepared_memory);
        assert_eq!(database.reserved_temp_bytes(), 0);
        println!(
            "sort growth prefix={prefix} calls={calls} refusals={refusals} rows={rows} samples={samples:?}"
        );
    }
    assert_eq!(
        fallbacks, 3,
        "each replacement allocation must permit fallback"
    );
    println!("sort growth passed: fallbacks=3; typed rows and release");
    drop(query);
    database.close()?;
    Ok(())
}
