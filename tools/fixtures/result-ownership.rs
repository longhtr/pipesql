//! Allocation events around complete and failed public result prefixes.
use super::Live;
use crate::transient_ownership::{Observer, Samples};
use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, Error, QueryStep, Value,
};
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Control {
    Healthy,
    WrongPrefix,
    MissingTerminal,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Case {
    Overflow,
    Cancelled,
    Finished,
}

pub(crate) fn run(root: &Path, control: Control) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(root)?;
    let db = create(&root.join("database"))?;
    println!("entered partial result ownership: ordered 257-row input");
    for case in [Case::Overflow, Case::Cancelled, Case::Finished] {
        check(&db, case, control)?;
    }
    db.close()?;
    println!("partial result ownership passed: 3 cases; rows, terminal events and release");
    Ok(())
}

fn create(path: &Path) -> Result<Database, Error> {
    let db = Database::create_empty(path, Config::new(8_000_000, 4_000_000)?)?;
    let cancel = CancellationToken::new();
    db.declare_table(
        "facts",
        &[ColumnDeclaration {
            name: "amount",
            data_type: DataType::Int64,
            nullable: false,
        }],
        &cancel,
    )?;
    let values = std::array::from_fn::<_, 256, _>(|row| row as i64);
    let mut append = db.begin_append(
        "facts",
        AppendLimits {
            batches: 2,
            encoded_bytes: 20_000,
        },
        &cancel,
    )?;
    append.write(
        &[ColumnInput {
            values: ColumnValues::Int64(&values),
            validity: &[0xff; 32],
        }],
        &cancel,
    )?;
    append.write(
        &[ColumnInput {
            values: ColumnValues::Int64(&[i64::MAX]),
            validity: &[1],
        }],
        &cancel,
    )?;
    append.commit(&cancel)?;
    Ok(db)
}

fn check(db: &Database, case: Case, control: Control) -> Result<(), Error> {
    let (name, sql) = match case {
        Case::Overflow => (
            "overflow",
            "FROM facts\n|> ORDER BY amount\n|> SELECT amount + 1 AS next_amount",
        ),
        Case::Cancelled => ("cancelled", "FROM facts |> ORDER BY amount"),
        Case::Finished => ("finished", "FROM facts |> ORDER BY amount"),
    };
    let before = Live::now();
    let resident = db.reserved_memory_bytes();
    let mut minimum = (i128::MAX, i128::MAX);
    let preparation = Observer::new(db, resident, before.requested, before.usable);
    let query = preparation.during(|| db.prepare(sql))?;
    let prepared_samples = preparation.samples();
    check_headroom(prepared_samples, &mut minimum);
    assert!(prepared_samples.allocations > 0 && prepared_samples.frees > 0);

    // The borrowed logical report writes directly into caller-owned storage.
    // Observe both a complete report and an immediate sink failure. Preparation
    // above supplies a positive control for the same allocation-event observer.
    for length in [0, 1_024] {
        use std::fmt::Write;
        let mut text = crate::FixedText {
            bytes: [0; 1_024],
            length,
        };
        let formatting = Observer::new(db, resident, before.requested, before.usable);
        let result = formatting.during(|| write!(text, "{}", query.logical_plan()));
        assert_eq!(result.is_ok(), length == 0);
        assert_eq!(
            (formatting.samples().allocations, formatting.samples().frees),
            (0, 0),
            "logical plan formatting allocated or released heap storage"
        );
        assert_eq!(
            db.reserved_memory_bytes(),
            resident + query.accounted_memory_bytes()
        );
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    let prepared_live = Live::now();
    let cancel = CancellationToken::new();
    let construction = Observer::new(db, resident, before.requested, before.usable);
    let mut result = construction.during(|| db.execute(&query, &cancel))?;
    let constructed = construction.samples();
    check_headroom(constructed, &mut minimum);
    assert!(
        constructed.allocations > 0,
        "missing partial result construction events"
    );
    let mut rows = 0;
    let mut steps = 0;
    let terminal = loop {
        steps += 1;
        assert!(steps < 20_000, "partial result did not terminate");
        assert_eq!(
            db.reserved_memory_bytes(),
            resident + query.accounted_memory_bytes() + result.accounted_memory_bytes(),
        );
        let observer = Observer::new(db, resident, before.requested, before.usable);
        // Disable observation only after rows exist: successful construction and
        // early steps must not hide the missing terminal deallocation events.
        let step = if control == Control::MissingTerminal && rows > 0 {
            result.step()
        } else {
            observer.during(|| result.step())
        };
        let done = match step {
            QueryStep::Progress => false,
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 1);
                assert!(!batch.is_empty());
                assert!(!cancel.is_cancelled(), "rows after cancellation boundary");
                for row in 0..batch.len() {
                    let expected = match case {
                        Case::Overflow => {
                            assert!(rows < 256);
                            rows as i64 + 1
                        }
                        Case::Cancelled | Case::Finished if rows < 256 => rows as i64,
                        Case::Finished if rows == 256 => i64::MAX,
                        _ => panic!("unexpected partial result row count"),
                    };
                    assert_eq!(
                        batch.value(row, 0),
                        Some(Value::Int64(expected)),
                        "partial result row oracle"
                    );
                    rows += 1;
                }
                if case == Case::Cancelled {
                    cancel.cancel();
                }
                false
            }
            QueryStep::Finished => {
                assert!(case == Case::Finished);
                true
            }
            QueryStep::Failed(error) => {
                check_error(case, error);
                true
            }
        };
        let samples = observer.samples();
        check_headroom(samples, &mut minimum);
        if done {
            assert!(samples.frees > 0, "missing partial result terminal events");
            break samples;
        }
    };
    match case {
        Case::Overflow => assert_eq!(
            rows,
            256 + usize::from(control == Control::WrongPrefix),
            "partial result prefix oracle"
        ),
        Case::Cancelled => assert!(rows > 0 && rows < 257),
        Case::Finished => assert_eq!(rows, 257),
    }
    assert_eq!(Live::now(), prepared_live);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert_eq!(
        result.accounted_memory_bytes(),
        std::mem::size_of_val(&result) as u64
    );
    let repeated = Observer::new(db, resident, before.requested, before.usable);
    match repeated.during(|| result.step()) {
        QueryStep::Finished => assert!(case == Case::Finished),
        QueryStep::Failed(error) => check_error(case, error),
        _ => panic!("partial result lost terminal state"),
    }
    assert_eq!(
        (repeated.samples().allocations, repeated.samples().frees),
        (0, 0)
    );
    assert_eq!(Live::now(), prepared_live);
    assert_eq!(
        db.reserved_memory_bytes(),
        resident + query.accounted_memory_bytes() + result.accounted_memory_bytes(),
    );
    let release = Observer::new(db, resident, before.requested, before.usable);
    let error = release.during(|| result.into_error());
    assert_eq!(
        (release.samples().allocations, release.samples().frees),
        (0, 0)
    );
    assert_eq!(
        db.reserved_memory_bytes(),
        resident + query.accounted_memory_bytes()
    );
    let prepared_release = Observer::new(db, resident, before.requested, before.usable);
    prepared_release.during(|| drop(query));
    let released = prepared_release.samples();
    check_headroom(released, &mut minimum);
    assert_eq!(released.allocations, 0);
    assert!(
        released.frees > 0,
        "missing partial prepared release events"
    );
    // The owned error survives release of every query owner. Formatting it uses
    // the caller's existing fixed buffer, outside the allocation-event scope.
    if case == Case::Finished {
        assert!(error.is_none());
    } else {
        check_error(case, error.as_ref().expect("owned partial result error"));
        assert!(crate::workload::format_error(error.as_ref()));
    }
    assert_eq!(Live::now(), before);
    assert_eq!(db.reserved_memory_bytes(), resident);
    assert_eq!(db.reserved_temp_bytes(), 0);
    drop(error);
    assert_eq!(Live::now(), before);
    println!(
        "partial result case={name} rows={rows} steps={steps} prepare_allocations={} execute_allocations={} terminal_frees={} prepared_frees={} requested_headroom={} usable_headroom={} release=complete",
        prepared_samples.allocations,
        constructed.allocations,
        terminal.frees,
        released.frees,
        minimum.0,
        minimum.1,
    );
    Ok(())
}

fn check_error(case: Case, error: &Error) {
    match (case, error) {
        (
            Case::Overflow,
            Error::ArithmeticOverflow {
                operation: "addition",
                span,
            },
        ) => {
            assert_eq!((span.start(), span.end()), (40, 50));
        }
        (Case::Cancelled, Error::Cancelled) => (),
        _ => panic!("unexpected partial result terminal error: {error}"),
    }
}

fn check_headroom(samples: Samples, minimum: &mut (i128, i128)) {
    assert!(
        samples.requested_headroom >= 0 && samples.usable_headroom >= 0,
        "partial result allocation ownership: {samples:?}",
    );
    minimum.0 = minimum.0.min(samples.requested_headroom);
    minimum.1 = minimum.1.min(samples.usable_headroom);
}
