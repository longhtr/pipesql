//! Physical graph mutations must be rejected without using builder inverses.

use super::*;
use crate::CancellationToken;
use crate::frontend::DataType;

struct Directory(std::path::PathBuf);

impl Drop for Directory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn distinct_input_coverage_and_fresh_output_mapping_are_validated() {
    let directory = Directory(
        std::env::temp_dir().join(format!("pipesql-physical-distinct-{}", std::process::id())),
    );
    let db = Database::create_empty(
        &directory.0,
        crate::Config::new(4_000_000, 2_000_000).unwrap(),
    )
    .unwrap();
    db.declare_table(
        "facts",
        &["a", "b"].map(|name| crate::ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: false,
        }),
        &CancellationToken::new(),
    )
    .unwrap();
    let query = db
        .prepare("FROM facts |> SELECT a,a AS duplicate,b |> DISTINCT |> SELECT a |> WHERE a > 0")
        .unwrap();
    for mutation in 0..8 {
        let mut plan = lower(&db, &query, RootState::Empty, 0).unwrap();
        assert_eq!(plan.pipelines.len(), 2);
        assert_eq!(plan.pipelines[0].column_count, 2);
        assert_eq!(plan.pipelines[0].filter_count, 0);
        assert_eq!(plan.pipelines[1].filter_count, 1);
        match mutation {
            0 => (),
            1 => plan.pipelines[0].column_count = 1,
            2 => {
                plan.pipelines[1].producer = Producer::Distinct {
                    input: PipelineId(1),
                    descriptor: 0,
                }
            }
            3 => {
                plan.pipelines[1].producer = Producer::Distinct {
                    input: PipelineId(0),
                    descriptor: 255,
                }
            }
            4 => plan.pipelines[1].identities[0] = plan.pipelines[0].identities[0],
            5 => plan.pipelines[1].columns[0] = 1,
            6 => plan.pipelines[1].filters[0].column = 1,
            7 => {
                let id = plan.pipelines[1].identities[0];
                plan.pipelines[1].slots[id.value() as usize] = 1;
            }
            _ => unreachable!(),
        }
        assert_eq!(
            validate_physical(&plan, &query, &db, RootState::Empty, 0).is_ok(),
            mutation == 0,
            "mutation {mutation}"
        );
    }
}

#[test]
fn limit_inputs_bounds_positions_and_filter_placement_are_validated() {
    let directory = Directory(
        std::env::temp_dir().join(format!("pipesql-physical-limit-{}", std::process::id())),
    );
    let db = Database::create(
        &directory.0,
        crate::Config::new(2_000_000, 1_000_000).unwrap(),
    )
    .unwrap();
    let query = db.prepare("FROM lineitem |> SELECT l_discount,l_quantity |> LIMIT 2 OFFSET 1 |> SELECT l_quantity,l_discount |> WHERE l_quantity > 0").unwrap();
    let different = db.prepare("FROM lineitem |> LIMIT 3").unwrap();
    let Stage::Limit(other_bounds) = different.plan.nodes()[0].stage else {
        unreachable!();
    };
    for mutation in 0..6 {
        let mut plan = lower(&db, &query, RootState::Empty, 0).unwrap();
        assert_eq!(plan.pipelines.len(), 2);
        assert_eq!(&plan.pipelines[0].columns[..2], &[2, 0]);
        assert_eq!(&plan.pipelines[1].columns[..2], &[1, 0]);
        assert_eq!(plan.pipelines[0].filter_count, 0);
        assert_eq!(plan.pipelines[1].filters[0].column, 1);
        match mutation {
            0 => (),
            1 => {
                let Producer::Limit { input, .. } = &mut plan.pipelines[1].producer else {
                    unreachable!();
                };
                *input = PipelineId(1);
            }
            2 => {
                let Producer::Limit { bounds, .. } = &mut plan.pipelines[1].producer else {
                    unreachable!();
                };
                *bounds = other_bounds;
            }
            3 => plan.pipelines[1].columns[0] = 0,
            4 => plan.pipelines[1].filter_count = 0,
            5 => plan.pipelines[1].filters[0].column = 0,
            _ => unreachable!(),
        }
        assert_eq!(
            validate_physical(&plan, &query, &db, RootState::Empty, 0).is_ok(),
            mutation == 0,
            "{mutation}"
        );
    }
}

#[test]
fn ordering_positions_flags_and_hidden_demand_are_independently_validated() {
    let directory = Directory(
        std::env::temp_dir().join(format!("pipesql-physical-order-{}", std::process::id())),
    );
    let database = Database::create_empty(
        &directory.0,
        crate::Config::new(2_000_000, 1_000_000).unwrap(),
    )
    .unwrap();
    database
        .declare_table(
            "facts",
            &["k", "v"].map(|name| crate::ColumnDeclaration {
                name,
                data_type: DataType::Int64,
                nullable: true,
            }),
            &crate::CancellationToken::new(),
        )
        .unwrap();
    let query = database.prepare("FROM facts |> SELECT v AS x,k AS y |> ORDER BY 2 DESC NULLS FIRST,1 |> SELECT x |> WHERE x > 0").unwrap();
    for mutation in 0..10 {
        let mut plan = lower(
            &database,
            &query,
            query.snapshot.as_ref().unwrap().state(),
            0,
        )
        .unwrap();
        assert_eq!(plan.pipelines.len(), 2);
        assert_eq!(&plan.pipelines[0].columns[..2], &[1, 0]);
        assert_eq!(plan.order_columns[0].column, 1);
        assert_eq!(plan.order_columns[1].column, 0);
        assert_eq!(plan.pipelines[1].column_count, 1);
        match mutation {
            0 => (),
            1 => plan.order_columns[0].column = 0,
            2 => plan.order_columns[0].direction = Direction::Ascending,
            3 => plan.order_columns[0].nulls = NullPlacement::Last,
            4 => plan.order_columns[2] = plan.order_columns[0],
            5 => plan.pipelines[0].column_count = 1,
            6 => plan.pipelines[1].columns[0] = 1,
            7 => {
                plan.pipelines[1].producer = Producer::Order {
                    input: PipelineId(1),
                    start: 0,
                    len: 2,
                }
            }
            8 => {
                plan.pipelines[1].producer = Producer::Order {
                    input: PipelineId(0),
                    start: 1,
                    len: 1,
                }
            }
            9 => {
                plan.pipelines[1].producer = Producer::Order {
                    input: PipelineId(0),
                    start: 0,
                    len: 0,
                }
            }
            _ => unreachable!(),
        }
        assert_eq!(
            validate_physical(
                &plan,
                &query,
                &database,
                query.snapshot.as_ref().unwrap().state(),
                0
            )
            .is_ok(),
            mutation == 0,
            "mutation {mutation}"
        );
    }
    drop(query);
    database.close().unwrap();
}

#[test]
fn join_pipelines_bind_both_inputs_and_validate_positions_independently() {
    let directory = Directory(
        std::env::temp_dir().join(format!("pipesql-physical-join-{}", std::process::id())),
    );
    let database = Database::create_empty(
        &directory.0,
        crate::Config::new(2_000_000, 1_000_000).unwrap(),
    )
    .unwrap();
    let names = [
        "l_quantity",
        "l_extendedprice",
        "l_discount",
        "l_tax",
        "l_returnflag",
        "l_linestatus",
        "l_shipdate",
    ];
    let columns = names.map(|name| crate::ColumnDeclaration {
        name,
        data_type: if name == "l_shipdate" {
            DataType::Date
        } else if name == "l_returnflag" || name == "l_linestatus" {
            DataType::String
        } else {
            DataType::Double
        },
        nullable: false,
    });
    database
        .declare_table("lineitem", &columns, &crate::CancellationToken::new())
        .unwrap();
    let query = database
        .prepare(
            "FROM lineitem AS a |> WHERE a.l_quantity < 20 |> SELECT a.l_quantity AS q |> AS p \
         |> JOIN lineitem AS b ON p.q = b.l_quantity \
         |> WHERE b.l_extendedprice > 10 |> SELECT b.l_extendedprice,p.q",
        )
        .unwrap();
    let before = database.reserved_memory_bytes();
    let plan = lower(
        &database,
        &query,
        query.snapshot.as_ref().unwrap().state(),
        0,
    )
    .unwrap();
    validate_physical(
        &plan,
        &query,
        &database,
        query.snapshot.as_ref().unwrap().state(),
        0,
    )
    .unwrap();
    assert_eq!(plan.pipelines.len(), 3);
    assert_eq!(plan.pipelines[0].producer, Producer::Scan(0));
    assert_eq!(plan.pipelines[1].producer, Producer::Scan(1));
    assert_eq!(
        &plan.pipelines[0].columns[..plan.pipelines[0].column_count],
        &[0]
    );
    assert_eq!(
        &plan.pipelines[1].columns[..plan.pipelines[1].column_count],
        &[0, 1]
    );
    assert_eq!(
        plan.pipelines[2].producer,
        Producer::Join {
            left: PipelineId(0),
            right: PipelineId(1),
            left_key: 0,
            right_key: 0,
        }
    );
    assert_eq!(
        &plan.output().columns[..plan.output().column_count],
        &[2, 0]
    );
    assert_eq!(plan.output().filters[0].column, 2);
    assert_eq!(plan.scan().filters[0].column, 0);
    assert_eq!(plan.scan().filter_count, 1);
    assert_eq!(plan.output().filter_count, 1);
    assert_eq!(
        database.reserved_memory_bytes() - before,
        plan.memory_bytes()
    );
    drop(plan);
    assert_eq!(database.reserved_memory_bytes(), before);
    for mutation in 0..11 {
        let mut plan = lower(
            &database,
            &query,
            query.snapshot.as_ref().unwrap().state(),
            0,
        )
        .unwrap();
        match mutation {
            0 => plan.pipelines[0].producer = Producer::Scan(1),
            1 => plan.pipelines[1].column_count = MAX_COLUMNS + 1,
            2 => plan.pipelines[2].columns[0] = 1,
            3 => plan.pipelines[2].filters[0].column = 1,
            4 => plan.pipelines[2].relation = RelationId(u8::MAX),
            5 => plan.pipelines[1].identities[0] = plan.pipelines[0].identities[0],
            6 => plan.pipelines[0].filter_count = 0,
            7..=9 => {
                let Producer::Join {
                    left,
                    right,
                    right_key,
                    ..
                } = &mut plan.pipelines[2].producer
                else {
                    unreachable!()
                };
                match mutation {
                    7 => *left = PipelineId(1),
                    8 => *right = PipelineId(2),
                    9 => *right_key = 1,
                    _ => unreachable!(),
                }
            }
            10 => {
                // A malformed edge must not let computation mapping inspect an
                // unvalidated future row before rejecting the producer graph.
                plan.pipelines[0].producer = Producer::Join {
                    left: PipelineId(0),
                    right: PipelineId(1),
                    left_key: 0,
                    right_key: 0,
                };
                plan.pipelines[0].column_count = 0;
                plan.pipelines[1].column_count = MAX_COLUMNS + 1;
            }
            _ => unreachable!(),
        }
        assert!(
            validate_physical(
                &plan,
                &query,
                &database,
                query.snapshot.as_ref().unwrap().state(),
                0
            )
            .is_err(),
            "mutation {mutation}"
        );
    }
    drop(query);
    let query = database
        .prepare(
            "FROM lineitem |> AGGREGATE SUM(l_quantity) AS s GROUP BY l_returnflag |> AS g \
         |> JOIN lineitem AS b ON g.s = b.l_quantity |> SELECT b.l_extendedprice",
        )
        .unwrap();
    let plan = lower(
        &database,
        &query,
        query.snapshot.as_ref().unwrap().state(),
        0,
    )
    .unwrap();
    validate_physical(
        &plan,
        &query,
        &database,
        query.snapshot.as_ref().unwrap().state(),
        0,
    )
    .unwrap();
    assert_eq!(plan.pipelines.len(), 4);
    assert_eq!(
        &plan.pipelines[0].columns[..plan.pipelines[0].column_count],
        &[0, 4]
    );
    assert_eq!(
        plan.pipelines[1].producer,
        Producer::Aggregate {
            aggregate: 0,
            input: PipelineId(0),
            demand: 1
        }
    );
    assert_eq!(
        &plan.pipelines[1].columns[..plan.pipelines[1].column_count],
        &[1]
    );
    assert_eq!(
        plan.pipelines[3].producer,
        Producer::Join {
            left: PipelineId(1),
            right: PipelineId(2),
            left_key: 0,
            right_key: 0,
        }
    );
    drop(plan);
    drop(query);
    database.close().unwrap();
}
