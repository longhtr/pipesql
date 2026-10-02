//! Check that physical validation rejects plans with incorrect value mappings.
//!
//! Prepare a query, build its physical plan and require the unmodified plan to
//! pass. Then change a position, identity, input edge or operator setting and
//! require rejection. An in-bounds position can still be wrong: a STRING input
//! cannot replace the INT64 result of CHAR_LENGTH applied to that input.
//!
//! Expected positions are written directly in these tests rather than obtained
//! from the builder's mapping helpers. These checks exercise preparation and
//! validation without executing queries. Independent SQL result tests are needed
//! to check the shared semantic analysis and the runtime.

use super::*;
use crate::CancellationToken;
use crate::test_support::Directory;
use crate::value::DataType;

#[test]
fn string_length_output_rejects_a_raw_string_position() {
    let directory = Directory::new();
    let db = Database::create(
        &directory.0.join("database"),
        crate::Config::new(4_000_000, 1_000_000).unwrap(),
    )
    .unwrap();
    for function in ["BYTE_LENGTH", "CHAR_LENGTH"] {
        let sql = format!("FROM lineitem |> SELECT {function}(l_returnflag) AS length");
        let query = db.prepare(&sql).unwrap();
        let mut plan = lower(&db, &query, RootState::Empty, 0).unwrap();
        validate_physical(&plan, &query, &db, RootState::Empty, 0).unwrap();
        // Slot 4 holds the original STRING. Replacing the computed length with
        // its input must fail even though both refer to the same source column.
        plan.pipelines[0].columns[0] = 4;
        assert!(validate_physical(&plan, &query, &db, RootState::Empty, 0).is_err());
    }
}

#[test]
fn literal_predicates_and_decisions_match_the_semantic_plan() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("database"),
        crate::Config::new(4_000_000, 2_000_000).unwrap(),
    )
    .unwrap();
    db.declare_table(
        "facts",
        &[crate::ColumnDeclaration {
            name: "a",
            data_type: DataType::Int64,
            nullable: true,
        }],
        &CancellationToken::new(),
    )
    .unwrap();
    for sql in [
        "FROM facts |> WHERE NOT a IN (1, NULL, 3)",
        "FROM facts |> WHERE a NOT IN (1, NULL, 3)",
        "FROM facts |> WHERE a NOT BETWEEN 1 AND 3 AND a NOT IN (2)",
        "FROM facts |> WHERE NOT (a IS DISTINCT FROM 1 OR a IS NOT DISTINCT FROM NULL OR a IS DISTINCT FROM 3)",
    ] {
        let query = db.prepare(sql).unwrap();
        for mutation in 0..5 {
            let mut plan = lower(&db, &query, RootState::Empty, 0).unwrap();
            let pipeline = &mut plan.pipelines[0];
            assert_eq!(pipeline.filter_count, 3);
            match mutation {
                0 => (),
                1 => pipeline.filters[1].predicate = &query::Predicate::IsNull { negated: false },
                2 => pipeline.filters[1].control.negated = false,
                3 => pipeline.filters[0].control.matched = 0,
                4 => pipeline.filter_count = 2,
                _ => unreachable!(),
            }
            assert_eq!(
                validate_physical(&plan, &query, &db, RootState::Empty, 0).is_ok(),
                mutation == 0,
                "predicate mutation {mutation}: {sql}"
            );
        }
    }
}

#[test]
fn set_branch_positions_and_demands_are_validated_independently() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("database"),
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
    for (operator, right_columns) in [
        ("UNION ALL", 1),
        ("EXCEPT DISTINCT", 2),
        ("INTERSECT DISTINCT", 2),
        ("EXCEPT ALL", 2),
        ("INTERSECT ALL", 2),
    ] {
        let query = db.prepare(&format!("FROM facts |> SELECT a AS x, a AS y |> {operator} (FROM facts |> SELECT b, a) |> SELECT y |> WHERE y>0")).unwrap();
        for mutation in 0..9 {
            let mut plan = lower(&db, &query, RootState::Empty, 0).unwrap();
            assert_eq!(plan.pipelines.len(), 3);
            assert_eq!(plan.pipelines[0].column_count, 1);
            assert_eq!(plan.pipelines[1].column_count, right_columns);
            assert_eq!(plan.pipelines[2].columns[0], 1);
            match mutation {
                0 => (),
                1 => {
                    plan.pipelines[2].producer = Producer::SetOperation {
                        left: PipelineId(1),
                        right: PipelineId(0),
                        descriptor: 0,
                    }
                }
                2 => {
                    plan.pipelines[2].producer = Producer::SetOperation {
                        left: PipelineId(0),
                        right: PipelineId(0),
                        descriptor: 0,
                    }
                }
                3 => {
                    plan.pipelines[2].producer = Producer::SetOperation {
                        left: PipelineId(0),
                        right: PipelineId(1),
                        descriptor: 255,
                    }
                }
                4 => plan.pipelines[1].column_count = 0,
                5 => plan.pipelines[2].columns[0] = 0,
                6 => plan.pipelines[2].filters[0].column = 0,
                7 => plan.pipelines[2].identities[0] = plan.pipelines[0].identities[0],
                8 => {
                    let id = plan.pipelines[2].identities[0];
                    plan.pipelines[2].slots[id.value() as usize] = 0;
                }
                _ => unreachable!(),
            }
            assert_eq!(
                validate_physical(&plan, &query, &db, RootState::Empty, 0).is_ok(),
                mutation == 0,
                "{operator}: mutation {mutation}"
            );
        }
        drop(query);
    }
    // Rows with equal x but different y are distinct here. Removing y before
    // comparison would wrongly merge them; bypassing the union would lose one input.
    let query = db.prepare("FROM facts |> SELECT a AS x, b AS y |> UNION DISTINCT (FROM facts |> SELECT b, a) |> SELECT x").unwrap();
    for mutation in 0..4 {
        let mut plan = lower(&db, &query, RootState::Empty, 0).unwrap();
        assert_eq!(plan.pipelines.len(), 4);
        assert_eq!(plan.pipelines[0].column_count, 2);
        assert_eq!(plan.pipelines[1].column_count, 2);
        assert_eq!(plan.pipelines[2].column_count, 2);
        match mutation {
            0 => (),
            1 => plan.pipelines[2].column_count = 1,
            2 => {
                plan.pipelines[3].producer = Producer::Distinct {
                    input: PipelineId(0),
                    descriptor: 0,
                }
            }
            3 => {
                plan.pipelines[3].producer = Producer::SetOperation {
                    left: PipelineId(0),
                    right: PipelineId(1),
                    descriptor: 0,
                }
            }
            _ => unreachable!(),
        }
        assert_eq!(
            validate_physical(&plan, &query, &db, RootState::Empty, 0).is_ok(),
            mutation == 0,
            "union distinct mutation {mutation}",
        );
    }
}

#[test]
fn distinct_input_coverage_and_fresh_output_mapping_are_validated() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("database"),
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
        .prepare("FROM facts |> SELECT a, a AS duplicate, b |> DISTINCT |> SELECT a |> WHERE a > 0")
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
    let directory = Directory::new();
    let db = Database::create(
        &directory.0.join("database"),
        crate::Config::new(2_000_000, 1_000_000).unwrap(),
    )
    .unwrap();
    let query = db.prepare("FROM lineitem |> SELECT l_discount, l_quantity |> LIMIT 2 OFFSET 1 |> SELECT l_quantity, l_discount |> WHERE l_quantity > 0").unwrap();
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
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("database"),
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
    let query = database.prepare("FROM facts |> SELECT v AS x, k AS y |> ORDER BY 2 DESC NULLS FIRST, 1 |> SELECT x |> WHERE x > 0").unwrap();
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
    let directory = Directory::new();
    let database = Database::create_empty(
        &directory.0.join("database"),
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
    for (kind, modifier) in [
        (query::JoinKind::Inner, ""),
        (query::JoinKind::Left, "LEFT "),
    ] {
        let sql = format!(
            "FROM lineitem AS a |> WHERE a.l_quantity < 20 |> SELECT a.l_quantity AS q |> AS p \
         |> {modifier}JOIN lineitem AS b ON p.q = b.l_quantity \
         |> WHERE b.l_extendedprice > 10 |> SELECT b.l_extendedprice, p.q",
        );
        let query = database.prepare(&sql).unwrap();
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
                kind,
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
        for mutation in 0..13 {
            let mut plan = lower(
                &database,
                &query,
                query.snapshot.as_ref().unwrap().state(),
                0,
            )
            .unwrap();
            match mutation {
                0 => plan.pipelines[0].producer = Producer::Scan(1),
                1 => plan.pipelines[1].column_count = MAX_ROW_VALUES + 1,
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
                    // Point the first pipeline at a later pipeline whose column
                    // count exceeds its array. Reject the edge before inspecting
                    // that unchecked array; rejection must not become a panic.
                    plan.pipelines[0].producer = Producer::Join {
                        kind: query::JoinKind::Inner,
                        left: PipelineId(0),
                        right: PipelineId(1),
                        left_key: 0,
                        right_key: 0,
                    };
                    plan.pipelines[0].column_count = 0;
                    plan.pipelines[1].column_count = MAX_ROW_VALUES + 1;
                }
                11 => {
                    let Producer::Join { kind, .. } = &mut plan.pipelines[2].producer else {
                        unreachable!()
                    };
                    *kind = if *kind == query::JoinKind::Inner {
                        query::JoinKind::Left
                    } else {
                        query::JoinKind::Inner
                    };
                }
                12 => plan.pipelines[2].identities[0] = plan.pipelines[1].identities[0],
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
    }
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
            kind: query::JoinKind::Inner,
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

#[test]
fn retained_qualified_payloads_have_independent_identity_validation() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("database"),
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
    // DROP hides the unqualified name, but f.a still names the original column.
    // It must survive the sort even though it is absent from the visible row.
    let query = db
        .prepare("FROM facts AS f |> DROP a |> ORDER BY b |> SELECT f.a")
        .unwrap();
    for mutation in 0..5 {
        let mut plan = lower(&db, &query, RootState::Empty, 0).unwrap();
        let input = &mut plan.pipelines[0];
        assert_eq!(input.column_count, 2);
        assert_eq!(input.identities[0].value(), 2);
        assert_eq!(input.identities[1].value(), 1);
        match mutation {
            0 => (),
            1 => {
                input.column_count = 1;
                input.columns[1] = 0;
                input.identities[1] = ColumnId::EMPTY;
            }
            2 => input.identities[1] = input.identities[0],
            3 => input.columns[1] = input.columns[0],
            4 => input.identities[1] = ColumnId::EMPTY,
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
fn typed_copy_slots_preserve_fresh_identity_without_numeric_storage() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("database"),
        crate::Config::new(4_000_000, 2_000_000).unwrap(),
    )
    .unwrap();
    db.declare_table(
        "facts",
        &[
            crate::ColumnDeclaration {
                name: "a",
                data_type: DataType::Int64,
                nullable: false,
            },
            crate::ColumnDeclaration {
                name: "b",
                data_type: DataType::String,
                nullable: true,
            },
        ],
        &CancellationToken::new(),
    )
    .unwrap();
    let query = db
        .prepare("FROM facts AS f |> SET a=b |> ORDER BY a |> SELECT a, f.a")
        .unwrap();
    for mutation in 0..5 {
        let mut plan = lower(&db, &query, RootState::Empty, 0).unwrap();
        let copy = query.plan.computed[0].column.identity();
        assert_eq!(plan.pipelines[0].slots[copy.value() as usize], 1);
        match mutation {
            0 => (),
            1 => plan.pipelines[0].slots[copy.value() as usize] = 0,
            2 => plan.pipelines[0].slots[copy.value() as usize] = MAX_ROW_VALUES as u8,
            3 => plan.pipelines[0].columns[0] = 0,
            4 => plan.pipelines[1].columns[0] = 1,
            _ => unreachable!(),
        }
        assert_eq!(
            validate_physical(&plan, &query, &db, RootState::Empty, 0).is_ok(),
            mutation == 0,
            "mutation {mutation}",
        );
    }
}

#[test]
fn constant_slots_preserve_identity_across_materialization() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("database"),
        crate::Config::new(4_000_000, 2_000_000).unwrap(),
    )
    .unwrap();
    db.declare_table(
        "facts",
        &[crate::ColumnDeclaration {
            name: "a",
            data_type: DataType::Int64,
            nullable: false,
        }],
        &CancellationToken::new(),
    )
    .unwrap();
    let query = db
        .prepare("FROM facts |> SELECT '雪' AS label, DATE '1970-01-02' AS day |> ORDER BY label")
        .unwrap();
    for mutation in 0..5 {
        let mut plan = lower(&db, &query, RootState::Empty, 0).unwrap();
        match mutation {
            0 => (),
            1 => plan.pipelines[0].columns[0] = 0,
            2 => plan.pipelines[0].columns[0] = (MAX_ROW_VALUES + 1) as u8,
            3 => plan.pipelines[1].columns[0] = MAX_ROW_VALUES as u8,
            4 => {
                let id = query.plan.computed[0].column.identity();
                plan.pipelines[0].slots[id.value() as usize] = 0;
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
fn analytic_input_slots_and_evaluation_boundary_are_validated() {
    let directory = Directory::new();
    let db = Database::create_empty(
        &directory.0.join("database"),
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
        .prepare("FROM facts |> SELECT a+1 AS next, COUNT(*) OVER () AS n |> WHERE n>0 |> LIMIT 1")
        .unwrap();
    for mutation in 0..7 {
        let mut plan = lower(&db, &query, RootState::Empty, 0).unwrap();
        assert_eq!(plan.pipelines.len(), 3);
        assert_eq!(plan.pipelines[0].column_count, 1);
        assert_eq!(plan.pipelines[0].columns[0], 0);
        assert_eq!(
            plan.pipelines[1].columns[..2],
            [MAX_ROW_VALUES as u8, (MAX_ROW_VALUES + 1) as u8]
        );
        match mutation {
            0 => (),
            1 => {
                plan.pipelines[1].producer = Producer::Analytic {
                    input: PipelineId(1),
                    partition: [None; query::MAX_PARTITION_KEYS],
                    function: WindowFunction::Count,
                }
            }
            2 => plan.pipelines[1].slots[1] = 1,
            3 => plan.pipelines[1].slots[1] = u8::MAX,
            4 => plan.pipelines[1].columns[1] = 0,
            5 => plan.pipelines[1].filters[0].column = 0,
            6 => plan.pipelines[0].columns[0] = MAX_ROW_VALUES as u8,
            _ => unreachable!(),
        }
        assert_eq!(
            validate_physical(&plan, &query, &db, RootState::Empty, 0).is_ok(),
            mutation == 0,
            "analytic mutation {mutation}"
        );
    }
}

#[test]
fn partition_positions_require_the_demanded_semantic_keys() {
    let directory = Directory::new();
    let db = Database::create(
        &directory.0.join("database"),
        crate::Config::new(4_000_000, 1_000_000).unwrap(),
    )
    .unwrap();
    for retained in [true, false] {
        let suffix = if retained {
            ""
        } else {
            " |> SELECT l_shipdate"
        };
        let query = db.prepare(&format!("FROM lineitem |> SELECT l_shipdate, COUNT(*) OVER (PARTITION BY l_returnflag) AS n{suffix}")).unwrap();
        for mutation in 0..5 {
            let mut plan = lower(&db, &query, RootState::Empty, 0).unwrap();
            let Producer::Analytic { partition, .. } = &mut plan.pipelines[1].producer else {
                panic!("window producer");
            };
            assert_eq!(partition.iter().flatten().count(), usize::from(retained));
            match mutation {
                0 => (),
                1 => partition[0] = if retained { None } else { Some(0) },
                2 => partition[0] = Some(u8::MAX),
                3 => partition[7] = Some(0),
                4 => partition[0] = Some(1),
                _ => unreachable!(),
            }
            assert_eq!(
                validate_physical(&plan, &query, &db, RootState::Empty, 0).is_ok(),
                mutation == 0,
                "retained {retained}, mutation {mutation}"
            );
        }
    }
}

#[test]
fn running_sum_positions_and_order_policy_are_independently_validated() {
    let directory = Directory::new();
    let db = Database::create(
        &directory.0.join("database"),
        crate::Config::new(4_000_000, 1_000_000).unwrap(),
    )
    .unwrap();
    let query = db.prepare("FROM lineitem |> EXTEND EXTRACT(YEAR FROM l_shipdate) AS year |> SELECT SUM(year) OVER (PARTITION BY l_returnflag ORDER BY l_shipdate DESC NULLS FIRST) AS n").unwrap();
    for mutation in 0..9 {
        let mut plan = lower(&db, &query, RootState::Empty, 0).unwrap();
        let Producer::Analytic {
            function,
            partition,
            ..
        } = &mut plan.pipelines[1].producer
        else {
            panic!("analytic producer");
        };
        assert_eq!(partition[0], Some(0));
        let WindowFunction::RunningSum { argument, order } = function else {
            panic!("running sum");
        };
        assert_eq!(*argument, 2);
        assert_eq!(
            order[0],
            Some(OrderColumn {
                column: 1,
                direction: Direction::Descending,
                nulls: NullPlacement::First
            })
        );
        match mutation {
            0 => (),
            1 => *argument = 0,
            2 => *argument = u8::MAX,
            3 => order[0] = None,
            4 => order[0].as_mut().unwrap().column = 2,
            5 => order[0].as_mut().unwrap().direction = Direction::Ascending,
            6 => order[0].as_mut().unwrap().nulls = NullPlacement::Last,
            7 => order[7] = order[0],
            8 => *function = WindowFunction::Count,
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
fn date_year_output_rejects_a_raw_date_position() {
    let directory = Directory::new();
    let db = Database::create(
        &directory.0.join("database"),
        crate::Config::new(4_000_000, 1_000_000).unwrap(),
    )
    .unwrap();
    let query = db
        .prepare("FROM lineitem |> SELECT EXTRACT(YEAR FROM l_shipdate) AS y")
        .unwrap();
    let mut plan = lower(&db, &query, RootState::Empty, 0).unwrap();
    validate_physical(&plan, &query, &db, RootState::Empty, 0).unwrap();
    // Slot 6 holds the original DATE. The year output must refer to extraction,
    // even though extraction reads that same stored column.
    plan.pipelines[0].columns[0] = 6;
    assert!(validate_physical(&plan, &query, &db, RootState::Empty, 0).is_err());
}
