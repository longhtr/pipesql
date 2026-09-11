use super::*;
use crate::effects::Faults;
use crate::execution::blocking::MAX_ARGUMENT_RECORD_BYTES;
use crate::execution::blocking::test_support::{Directory, schema};
use crate::execution::planning::{lower, validate_physical};
use crate::execution::scan::{Source, declared};
use crate::execution::*;

type Row = (Option<i64>, Option<i64>, f64);

fn database(directory: &Directory, rows: &[Row]) -> Database {
    let database = Database::create_empty(
        &directory.0.join("db"),
        crate::Config::new(16_000_000, 8_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    database
        .declare_table(
            "facts",
            &[
                crate::ColumnDeclaration {
                    name: "k",
                    data_type: DataType::Int64,
                    nullable: true,
                },
                crate::ColumnDeclaration {
                    name: "n",
                    data_type: DataType::Int64,
                    nullable: true,
                },
                crate::ColumnDeclaration {
                    name: "d",
                    data_type: DataType::Double,
                    nullable: false,
                },
            ],
            &cancel,
        )
        .unwrap();
    // Multiple real units exercise source completion and replay across descriptors.
    for chunk in rows.chunks(3) {
        let keys: Vec<_> = chunk.iter().map(|row| row.0.unwrap_or(0)).collect();
        let values: Vec<_> = chunk.iter().map(|row| row.1.unwrap_or(0)).collect();
        let doubles: Vec<_> = chunk.iter().map(|row| row.2).collect();
        let key_valid = [chunk.iter().enumerate().fold(0, |mask, (index, row)| {
            mask | (u8::from(row.0.is_some()) << index)
        })];
        let value_valid = [chunk.iter().enumerate().fold(0, |mask, (index, row)| {
            mask | (u8::from(row.1.is_some()) << index)
        })];
        let valid = [(1 << chunk.len()) - 1];
        let mut append = database
            .begin_append(
                "facts",
                crate::AppendLimits {
                    batches: 1,
                    encoded_bytes: 20_000,
                },
                &cancel,
            )
            .unwrap();
        append
            .write(
                &[
                    crate::ColumnInput {
                        values: crate::ColumnValues::Int64(&keys),
                        validity: &key_valid,
                    },
                    crate::ColumnInput {
                        values: crate::ColumnValues::Int64(&values),
                        validity: &value_valid,
                    },
                    crate::ColumnInput {
                        values: crate::ColumnValues::Double(&doubles),
                        validity: &valid,
                    },
                ],
                &cancel,
            )
            .unwrap();
        append.commit(&cancel).unwrap();
    }
    database
}

// Holds the charged controller outside a query only while component tests inspect
// it. Each scheduler step transfers the same owner into and back out of runtime.
struct TestController<'db>(Vec<General<'db>>);

impl<'db> std::ops::Deref for TestController<'db> {
    type Target = General<'db>;

    fn deref(&self) -> &Self::Target {
        &self.0[0]
    }
}

impl std::ops::DerefMut for TestController<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0[0]
    }
}

// Component tests use trusted grouping metadata to force tiny internal buffers.
// Public integration tests below use ordinary grouping preparation and ownership.
fn connect<'db>(
    database: &'db Database,
    running: &mut QueryResult<'db, '_>,
    hash_groups: usize,
    arguments: usize,
    ordered: bool,
) -> TestController<'db> {
    let State::Running(runtime) = &mut running.state else {
        panic!("trusted running query");
    };
    let Aggregation::Dense(groups) = runtime.aggregates.pop().unwrap() else {
        panic!("trusted global aggregate");
    };
    let aggregate = groups.aggregate;
    let mut keys = schema(&[(DataType::Int64, true)]);
    keys.columns[0].input = 0;
    for column in &mut running.plan.output_mut().columns[..4] {
        *column += 1;
    }
    running.plan.output_mut().columns[0] = 0;
    let filter_count = running.plan.output().filter_count;
    for filter in &mut running.plan.output_mut().filters[..filter_count] {
        filter.column += 1;
    }
    TestController(vec![
        General::new(
            database,
            aggregate,
            keys,
            running.plan.output(),
            ordered,
            Limits {
                arguments,
                run_bytes: 140,
                run_rows: 2,
            },
            (hash_groups, 9 * hash_groups),
        )
        .unwrap(),
    ])
}

fn workspace<'a, 'db>(running: &'a mut QueryResult<'db, '_>) -> runtime::WorkspaceView<'a, 'db> {
    let State::Running(workspace) = &mut running.state else {
        panic!("live native workspace");
    };
    workspace.grouping_workspace()
}

fn step<'db>(
    general: &mut TestController<'db>,
    running: &mut QueryResult<'db, '_>,
    cancel: &CancellationToken,
    effects: &mut Effects,
) -> Result<Advance, Error> {
    let State::Running(workspace) = &mut running.state else {
        panic!("live native workspace");
    };
    assert!(workspace.aggregates.is_empty());
    assert!(workspace.aggregates.capacity() >= 1);
    workspace
        .aggregates
        .push(Aggregation::General(std::mem::take(&mut general.0)));
    let outcome = workspace.step(&running.plan, cancel, effects);
    let Aggregation::General(owner) = workspace.aggregates.pop().unwrap() else {
        panic!("installed component controller");
    };
    general.0 = owner;
    outcome
}

fn values(running: &mut QueryResult<'_, '_>) -> (Option<i64>, Option<i64>, u64, i64) {
    let output = &workspace(running).output;
    assert_eq!(output.len(), 1);
    let integer = |column| match output.value(0, column).unwrap() {
        Value::Int64(value) => Some(value),
        Value::Null => None,
        _ => panic!("integer result"),
    };
    let Value::Double(average) = output.value(0, 2).unwrap() else {
        panic!("average result");
    };
    (
        integer(0),
        integer(1),
        average.to_bits(),
        integer(3).unwrap(),
    )
}

const QUERY: &str =
    "FROM facts |> AGGREGATE SUM(k) AS key,SUM(n) AS total,AVG(d) AS average,COUNT(*) AS nrows";

fn check_controller_account(general: &General<'_>) {
    fn bytes<T>(values: &Vec<T>) -> usize {
        values.capacity() * size_of::<T>()
    }
    let cells = &general.aggregate.cells;
    // Count the actual retained owner and allocation capacities. AggregateState
    // charges arrays only; the nested sorter/argument owners charge their fields.
    // Optional hash storage and pending creation credit have separate accounts.
    let physical = size_of::<General<'_>>()
        + bytes(&general.record.bytes)
        + bytes(&cells.values)
        + bytes(&cells.integers)
        + bytes(&cells.nonnull_counts)
        + bytes(&cells.counts)
        + bytes(&cells.flags)
        + bytes(&general.aggregate.scratch)
        + bytes(&general.arguments.values)
        + general.sort.allocated_heap_bytes();
    let charged = general.reservation.bytes()
        + general.aggregate.reservation.bytes()
        + general.arguments.reservation.bytes()
        + general.sort.memory_bytes();
    assert_eq!(
        physical as u64, charged,
        "retained controller fields and heap capacities are charged once"
    );
}

mod admission;
mod failure;
mod output;
mod replay;
