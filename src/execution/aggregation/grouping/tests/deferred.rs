//! Check optional admission at the first input, using the production scheduler.
//!
//! Path-selection fixtures use explicit capacities elsewhere. These tests leave
//! capacity selection untouched and change input outcomes or another live owner.

use super::*;

#[test]
fn hash_waits_for_input_and_skips_empty_cancelled_or_failed_sources() {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Input {
        Sorted,
        Empty,
        Cancelled,
        Failed,
    }
    let directory = Directory::new();
    let database = database(
        &directory,
        &[
            (Some(1), Some(3), 3.0),
            (Some(2), Some(9), 9.0),
            (None, None, 0.0),
        ],
    );
    for input in [Input::Sorted, Input::Empty, Input::Cancelled, Input::Failed] {
        let sql = match input {
            Input::Sorted | Input::Cancelled => {
                "FROM facts |> ORDER BY n |> AGGREGATE SUM(n) AS total GROUP AND ORDER BY k"
            }
            Input::Empty => {
                "FROM facts |> WHERE n > 100 |> AGGREGATE SUM(n) AS total GROUP AND ORDER BY k"
            }
            Input::Failed => {
                "FROM facts |> EXTEND 1/(n-3) AS broken |> ORDER BY broken |> AGGREGATE SUM(broken) AS total GROUP AND ORDER BY k"
            }
        };
        let query = database.prepare(sql).unwrap();
        let baseline = database.reserved_memory_bytes();
        let cancel = CancellationToken::new();
        let mut result = database.execute(&query, &cancel).unwrap();
        let Some(Aggregation::General(owner)) = result.first_aggregate() else {
            panic!("grouped query");
        };
        assert!(owner[0].hash_pending && owner[0].memory.is_empty());
        let (mut sorting, mut hashed, mut terminal) = (false, false, false);
        let mut rows = Vec::new();
        for _ in 0..4096 {
            let Some(Aggregation::General(owner)) = result.first_aggregate() else {
                panic!("live controller");
            };
            let general = &owner[0];
            check_controller_account(general);
            if general.hash_pending {
                assert!(general.memory.is_empty());
                sorting |= database.reserved_temp_bytes() > 0;
                if input == Input::Cancelled && sorting {
                    cancel.cancel();
                }
            } else if !general.memory.is_empty() {
                assert_eq!(input, Input::Sorted);
                assert!(
                    sorting,
                    "upstream sort must run before optional hash admission"
                );
                hashed = true;
            }
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Rows(batch) => {
                    for row in 0..batch.len() {
                        let integer = |column| match batch.value(row, column).unwrap() {
                            Value::Null => None,
                            Value::Int64(value) => Some(value),
                            _ => panic!("integer grouped answer"),
                        };
                        rows.push((integer(0), integer(1)));
                    }
                }
                QueryStep::Finished => {
                    assert!(matches!(input, Input::Sorted | Input::Empty));
                    terminal = true;
                    break;
                }
                QueryStep::Failed(error) => {
                    assert!(
                        match input {
                            Input::Cancelled => matches!(error, Error::Cancelled),
                            Input::Failed => matches!(error, Error::DivisionByZero { .. }),
                            _ => false,
                        },
                        "{input:?}: {error}"
                    );
                    assert!(matches!(result.step(), QueryStep::Failed(_)));
                    terminal = true;
                    break;
                }
            }
        }
        assert!(terminal, "{input:?}");
        assert_eq!(hashed, input == Input::Sorted);
        assert_eq!(
            rows,
            if input == Input::Sorted {
                vec![(None, None), (Some(1), Some(3)), (Some(2), Some(9))]
            } else {
                vec![]
            }
        );
        drop(result);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        assert_eq!(database.reserved_temp_bytes(), 0);
    }
    database.close().unwrap();
}

#[test]
fn pressure_after_execute_refuses_hash_once_then_disk_finishes_after_release() {
    let directory = Directory::new();
    let database = database(
        &directory,
        &[
            (Some(1), Some(3), 3.0),
            (Some(2), Some(9), 9.0),
            (None, None, 0.0),
        ],
    );
    let query = database
        .prepare("FROM facts |> AGGREGATE SUM(n) AS total GROUP AND ORDER BY k")
        .unwrap();
    let baseline = database.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let mut result = database.execute(&query, &cancel).unwrap();
    // Another owner takes the remaining budget after required query admission,
    // before the first batch reaches grouping. Capacity must use this live state.
    let mut pressure = Some(
        database
            .reserve_memory(
                database.config().memory_limit_bytes() - database.reserved_memory_bytes(),
                "later admitted owner",
            )
            .unwrap(),
    );
    let (mut refused, mut spilled, mut finished) = (false, false, false);
    let mut rows = Vec::new();
    for _ in 0..4096 {
        let Some(Aggregation::General(owner)) = result.first_aggregate() else {
            panic!("live controller");
        };
        let general = &owner[0];
        check_controller_account(general);
        assert!(
            general.memory.is_empty(),
            "do not retry optional admission after refusal"
        );
        if !general.hash_pending && !refused {
            assert!(matches!(general.files, Files::Open(_)));
            refused = true;
            drop(pressure.take());
        }
        spilled |= database.reserved_temp_bytes() > 0;
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let integer = |column| match batch.value(row, column).unwrap() {
                        Value::Null => None,
                        Value::Int64(value) => Some(value),
                        _ => panic!("integer grouped answer"),
                    };
                    rows.push((integer(0), integer(1)));
                }
            }
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("admitted fallback must finish: {error}"),
        }
    }
    assert!(refused && spilled && finished);
    assert_eq!(rows, [(None, None), (Some(1), Some(3)), (Some(2), Some(9))]);
    assert!(pressure.is_none());
    drop(pressure);
    drop(result);
    assert_eq!(database.reserved_memory_bytes(), baseline);
    assert_eq!(database.reserved_temp_bytes(), 0);
    drop(query);
    database.close().unwrap();
}
