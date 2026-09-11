use super::super::lexer::{RESERVED_IDENTIFIERS, ZERO_SPAN, lex};
use super::super::parser::{
    ParsedAggregateEntry, ParsedExpression, ParsedLiteral, ParsedOp, ParsedStage,
};
use super::super::*;
use super::*;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);
const Q6: &str = include_str!("../../../tests/fixtures/q6.pipe.sql");
const Q1: &str = include_str!("../../../tests/fixtures/upstream/q1-upstream.pipe.sql");

struct Temp(PathBuf);

impl Temp {
    fn database(&self) -> PathBuf {
        self.0.join("database")
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn database(limit: u64) -> (Temp, Database) {
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let temp =
        Temp(std::env::temp_dir().join(format!("pipesql-frontend-{}-{id}", std::process::id())));
    fs::create_dir(&temp.0).unwrap();
    let database =
        Database::create(&temp.database(), crate::Config::new(limit, 1).unwrap()).unwrap();
    (temp, database)
}

#[test]
fn derived_scope_preparation_admits_exact_peak_and_releases_it() {
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let temp =
        Temp(std::env::temp_dir().join(format!("pipesql-scope-{}-{id}", std::process::id())));
    fs::create_dir(&temp.0).unwrap();
    let db = Database::create_empty(
        &temp.database(),
        crate::Config::new(4_000_000, 1_000_000).unwrap(),
    )
    .unwrap();
    db.declare_table(
        "facts",
        &[
            crate::ColumnDeclaration {
                name: "k",
                data_type: DataType::Int64,
                nullable: false,
            },
            crate::ColumnDeclaration {
                name: "n",
                data_type: DataType::Int64,
                nullable: true,
            },
        ],
        &crate::CancellationToken::new(),
    )
    .unwrap();
    let sql = "FROM facts AS a |> JOIN (FROM facts AS b |> JOIN (FROM facts |> SELECT k) AS c ON b.k = c.k |> SELECT b.k AS k) AS d ON a.k = d.k |> AGGREGATE COUNT(*) AS n";
    let baseline = db.reserved_memory_bytes();
    let query = db.prepare(sql).unwrap();
    let retained = query.accounted_memory_bytes();
    assert_eq!(db.reserved_memory_bytes(), baseline + retained);
    let syntax = parse_query(sql).unwrap();
    let source_columns = usize::from(query.plan.source_count);
    let scopes = SavedScopes::new(&db, &syntax, source_columns).unwrap();
    let physical = (scopes.values.capacity() * size_of::<Output>()
        + scopes.ranges.capacity() * size_of::<Range>()
        + 2 * PREPARED_ALLOCATION_ALLOWANCE) as u64;
    assert_eq!(scopes._reservation.bytes(), physical);
    let binding_peak = retained + physical;
    let catalog_peak = crate::catalog::SNAPSHOT_SCRATCH_BYTES as u64;
    let peak = binding_peak.max(catalog_peak);
    let refusal_owner = if catalog_peak > binding_peak {
        "query catalog binding"
    } else {
        "binder scope storage"
    };
    drop(scopes);
    drop(query);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    for shortfall in [0, 1] {
        let pressure = db
            .reserve_memory(
                db.config().memory_limit_bytes() - baseline - peak + shortfall,
                "scope admission pressure",
            )
            .unwrap();
        let pressured = db.reserved_memory_bytes();
        match db.prepare(sql) {
            Ok(query) => {
                assert_eq!(shortfall, 0);
                assert_eq!(query.accounted_memory_bytes(), retained);
                assert_eq!(db.reserved_memory_bytes(), pressured + retained);
                drop(query);
            }
            Err(error) => assert!(
                shortfall == 1
                    && matches!(
                        error,
                        Error::Resource {
                            owner,
                            ..
                        } if owner == refusal_owner
                    ),
                "{error}"
            ),
        }
        assert_eq!(db.reserved_memory_bytes(), pressured);
        assert_eq!(db.reserved_temp_bytes(), 0);
        drop(pressure);
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    // Catalog scratch is released before scope binding. Exercise the smaller
    // scope boundary separately so a larger catalog minimum cannot hide it.
    for shortfall in [0, 1] {
        let pressure = db
            .reserve_memory(
                db.config().memory_limit_bytes() - baseline - physical + shortfall,
                "isolated scope pressure",
            )
            .unwrap();
        let pressured = db.reserved_memory_bytes();
        match SavedScopes::new(&db, &syntax, source_columns) {
            Ok(scopes) => {
                assert_eq!(shortfall, 0);
                assert_eq!(db.reserved_memory_bytes(), pressured + physical);
                drop(scopes);
            }
            Err(error) => assert!(
                shortfall == 1
                    && matches!(
                        error,
                        Error::Resource {
                            owner: "binder scope storage",
                            ..
                        }
                    )
            ),
        }
        assert_eq!(db.reserved_memory_bytes(), pressured);
        drop(pressure);
    }
    assert_eq!(db.reserved_memory_bytes(), baseline);
    println!(
        "scope payload+allowances={physical} retained={retained} binding peak={binding_peak} catalog peak={catalog_peak} preparation peak={peak}"
    );
}

#[test]
fn repeated_aggregate_binding_preserves_identity_and_transitive_demand() {
    let (_temp, database) = database(4_000_000);
    let baseline = database.reserved_memory_bytes();
    let sql = "FROM lineitem |> AGGREGATE SUM(l_quantity) AS l_quantity, SUM(l_extendedprice) AS unused |> SELECT l_quantity * 2 AS l_quantity |> AGGREGATE AVG(l_quantity) AS average";
    let mut query = database.prepare(sql).unwrap();
    assert_eq!(query.plan.aggregates.len(), 2);
    assert_eq!(query.plan.aggregate_demand(0), 0b01);
    assert_eq!(query.plan.aggregate_demand(1), 0b1);
    assert_eq!(query.result_column(0).unwrap().data_type, DataType::Double);
    assert_ne!(
        query.plan.aggregates[0].first_output,
        query.plan.aggregates[1].first_output
    );
    assert_eq!(
        database.reserved_memory_bytes(),
        baseline + query.accounted_memory_bytes()
    );
    assert!(query.accounted_memory_bytes() <= PreparedQuery::memory_requirement_bytes());
    let last = usize::from(query.plan.count) - 1;
    for index in [0, 2, u8::MAX] {
        query.plan.stages[last].stage = Stage::Aggregate(index);
        assert!(
            validate(&query.plan).is_err(),
            "wrong aggregate owner {index}"
        );
    }
    query.plan.stages[last].stage = Stage::Aggregate(1);
    validate(&query.plan).unwrap();
    drop(query);
    assert_eq!(database.reserved_memory_bytes(), baseline);
}

#[test]
fn repeated_aggregate_legacy_empty_chain_respects_identity_limit() {
    let (_temp, database) = database(4_000_000);
    let baseline = database.reserved_memory_bytes();
    let cancel = crate::CancellationToken::new();
    let mut sql = String::from("FROM lineitem |> AGGREGATE COUNT(*) AS n");
    for count in 1..=MAX_AGGREGATE_COLUMNS {
        let query = database.prepare(&sql).unwrap();
        assert_eq!(query.plan.aggregates.len(), count);
        let mut result = database.execute(&query, &cancel).unwrap();
        let mut observed = Vec::new();
        let mut finished = false;
        for _ in 0..1024 {
            match result.step() {
                crate::QueryStep::Rows(batch) => {
                    for row in 0..batch.len() {
                        let crate::Value::Int64(value) = batch.value(row, 0).unwrap() else {
                            panic!("count chain value");
                        };
                        observed.push(value);
                    }
                }
                crate::QueryStep::Progress => (),
                crate::QueryStep::Finished => {
                    finished = true;
                    break;
                }
                crate::QueryStep::Failed(error) => panic!("{count}: {error}"),
            }
        }
        assert!(finished);
        assert_eq!(observed, [0]);
        drop(result);
        drop(query);
        assert_eq!(database.reserved_memory_bytes(), baseline);
        sql.push_str(" |> AGGREGATE SUM(n) AS n");
    }
    assert!(database.prepare(&sql).is_err());
    assert_eq!(database.reserved_memory_bytes(), baseline);
}

#[test]
fn computed_definitions_reject_invalid_scope_identity_and_provenance() {
    let (_temp, database) = database(4_000_000);
    let sql = "FROM lineitem |> SELECT l_quantity+1 AS x |> SELECT x*2 AS y";
    for mutation in 0..8 {
        let mut query = database.prepare(sql).unwrap();
        let second = query.plan.computed[1].column;
        match mutation {
            0 => query.plan.computed[0].column = second,
            1 => query.plan.computed[1].input = RelationId::SOURCE,
            2 => query.plan.computed[0].span = ZERO_SPAN,
            3 => query.plan.computed[0].span.end = u16::MAX,
            4 => query.plan.computed[0].expression.ops[0] = Op::Column(second),
            5 => query.plan.computed[0].expression.data_type = DataType::Int64,
            6 => {
                query.plan.computed.pop();
            }
            7 => query.plan.projections[1] = query.plan.computed[0].column.identity(),
            _ => unreachable!(),
        }
        assert!(
            validate(&query.plan).is_err(),
            "computed mutation {mutation}"
        );
    }
    println!(
        "computed storage Parsed={} Plan={} Computed={}",
        size_of::<Parsed>(),
        size_of::<Plan>(),
        size_of::<Computed>()
    );
}

#[test]
fn limit_bounds_and_shared_parser_storage_are_checked() {
    let (_temp, db) = database(2_000_000);
    for bad_count in [true, false] {
        let mut query = db
            .prepare("FROM lineitem |> LIMIT 0 OFFSET 9223372036854775807")
            .unwrap();
        let Stage::Limit(bounds) = &mut query.plan.stages[0].stage else {
            unreachable!();
        };
        if bad_count {
            bounds.count = u64::MAX;
        } else {
            bounds.offset = u64::MAX;
        }
        assert!(validate(&query.plan).is_err());
    }
    // Numeric expressions must not multiply full stacks by stages or calls.
    assert!(std::mem::size_of::<ParsedStage>() <= std::mem::size_of::<ParsedLiteral>() + 16);
    assert!(std::mem::size_of::<ParsedAggregateEntry>() <= 24);
    assert!(std::mem::size_of::<Parsed>() <= 4500);
    eprintln!(
        "parser bytes: parsed={} stage={} expression={} shared_numeric_ops={}",
        std::mem::size_of::<Parsed>(),
        std::mem::size_of::<ParsedStage>(),
        std::mem::size_of::<ParsedExpression>(),
        std::mem::size_of::<[ParsedOp; MAX_TOKENS]>()
    );
}

#[test]
fn boolean_controls_preserve_scope_bounds_and_finite_paths() {
    let (_temp, db) = database(2_000_000);
    for mutation in 0..4 {
        let mut query = db
            .prepare("FROM lineitem |> WHERE l_quantity<0 OR l_quantity>1 |> SELECT l_quantity")
            .unwrap();
        let Stage::Where(filter) = &mut query.plan.stages[0].stage else {
            unreachable!()
        };
        match mutation {
            0 => filter.control.end = u8::MAX,
            1 => filter.control.matched = 3,
            2 => filter.control.end = 3,
            3 => filter.control.other = 3,
            _ => unreachable!(),
        }
        assert!(validate(&query.plan).is_err());
    }
    for syntax in ["NOT ".repeat(100), "(".repeat(70)] {
        let ending = if syntax.starts_with('(') {
            ")".repeat(70)
        } else {
            String::new()
        };
        let query = format!("FROM lineitem |> WHERE {syntax}l_quantity<0{ending}");
        db.prepare(&query).unwrap();
    }
    assert!(
        db.prepare(&format!(
            "FROM lineitem |> WHERE {}l_quantity<0",
            "NOT ".repeat(160)
        ))
        .is_err()
    );
}

#[test]
fn null_predicates_keep_scope_validation_and_stage_bounds() {
    let (_temp, db) = database(2_000_000);
    for mutation in 0..2 {
        let mut query = db
            .prepare("FROM lineitem |> SELECT l_quantity |> WHERE l_quantity IS NULL")
            .unwrap();
        let Stage::Where(filter) = &mut query.plan.stages[1].stage else {
            unreachable!()
        };
        assert_eq!(filter.predicate, Predicate::IsNull { negated: false });
        if mutation == 0 {
            // PRICE exists in source facts but is outside this stage's input.
            filter.column = SourceColumn::PRICE.identity;
        } else {
            filter.span = ZERO_SPAN;
        }
        assert!(validate(&query.plan).is_err());
    }
    let sql = format!(
        "FROM lineitem{}",
        " |> WHERE l_quantity IS NOT NULL".repeat(MAX_STAGES)
    );
    assert!(db.prepare(&sql).is_ok());
    assert!(matches!(
        db.prepare(&(sql + " |> WHERE l_quantity IS NULL")),
        Err(Error::Parse { .. })
    ));
}

#[test]
fn pooled_numeric_programs_preserve_individual_bounds_and_spans() {
    let (_temp, db) = database(2_000_000);
    // Four maximal programs fit one query. They cannot share a per-program
    // cursor or consume each other's operation allowance.
    let expression = format!("-({}0)", "0+".repeat(15));
    let source = format!(
        "FROM lineitem |> LIMIT {expression} OFFSET {expression} |> LIMIT {expression} OFFSET {expression}"
    );
    let parsed = parse_query(&source).unwrap();
    assert_eq!(parsed.numeric_op_count, 128);
    assert!(db.prepare(&source).is_ok());
    let too_many = source.replacen(&expression, &format!("-{expression}"), 1);
    assert!(matches!(db.prepare(&too_many), Err(Error::Parse { .. })));

    let prefix = "# 雪\nFROM lineitem |> WHERE l_quantity BETWEEN (0+1) AND (3*4) |> AGGREGATE SUM(l_quantity+5) AS s,AVG(l_quantity*6) AS a |> WHERE s > (7-8) |> LIMIT (9+10) OFFSET ";
    let source = format!("{prefix}(9223372036854775807+1)");
    let resident = db.reserved_memory_bytes();
    match db.prepare(&source) {
        Err(Error::ArithmeticOverflow { span, .. }) => {
            assert_eq!(span.start(), prefix.len());
            assert_eq!(span.end(), source.len());
        }
        outcome => panic!("expected final constant overflow: {:?}", outcome.err()),
    }
    assert_eq!(db.reserved_memory_bytes(), resident);
}

#[test]
fn legacy_joins_refuse_during_preparation_and_release_ownership() {
    let (_temp, database) = database(2_000_000);
    let baseline = database.reserved_memory_bytes();
    let sql = "FROM lineitem AS a |> JOIN lineitem AS b ON a.l_quantity = b.l_quantity";
    let error = database
        .prepare(sql)
        .err()
        .expect("unsupported storage profile");
    assert!(matches!(error, Error::Bind { .. }));
    assert!(
        error
            .to_string()
            .contains("joins require declared-table storage")
    );
    assert_eq!(database.reserved_memory_bytes(), baseline);
    assert_eq!(database.reserved_temp_bytes(), 0);
    database.close().unwrap();
}

#[test]
fn ordering_facts_preserve_hidden_identity_and_reject_invalid_item_slices() {
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let temp = Temp(std::env::temp_dir().join(format!(
        "pipesql-frontend-order-{}-{id}",
        std::process::id()
    )));
    fs::create_dir(&temp.0).unwrap();
    let database = Database::create_empty(
        &temp.database(),
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
    let sql = "FROM facts |> ORDER BY k DESC NULLS FIRST,v |> SELECT v AS x |> WHERE x > 0 |> AS p";
    let query = database.prepare(sql).unwrap();
    let keys = query.plan.order_items(0, 2).unwrap();
    assert_eq!(
        keys[0],
        OrderKey {
            column: ColumnId::new(1),
            direction: Direction::Descending,
            nulls: NullPlacement::First
        }
    );
    assert_eq!(keys[1].column, ColumnId::new(2));
    for relation in 1..=4 {
        assert_eq!(
            query.plan.order_key(RelationId(relation), 0).unwrap(),
            Some(keys[0])
        );
        assert_eq!(
            query.plan.order_key(RelationId(relation), 1).unwrap(),
            Some(keys[1])
        );
        assert_eq!(query.plan.order_key(RelationId(relation), 2).unwrap(), None);
    }
    assert_eq!(query.plan.outputs().collect::<Vec<_>>(), [ColumnId::new(2)]);
    drop(query);
    for mutation in 0..8 {
        let mut query = database.prepare(sql).unwrap();
        match mutation {
            0 => query.plan.order_count = 81,
            1 => query.plan.order_count = 1,
            2 => query.plan.order_items[0].column = ColumnId::EMPTY,
            3 => query.plan.order_items[0].column = ColumnId::new(3),
            4 => query.plan.order_items[2] = query.plan.order_items[0],
            5 => query.plan.stages[0].stage = Stage::Order { start: 1, len: 1 },
            6 => query.plan.stages[0].stage = Stage::Order { start: 0, len: 0 },
            7 => query.plan.stages[0].stage = Stage::Order { start: 0, len: 81 },
            _ => unreachable!(),
        }
        assert!(validate(&query.plan).is_err(), "mutation {mutation}");
    }
    for (sql, ordered) in [
        ("FROM facts |> ORDER BY k |> ORDER BY v", true),
        (
            "FROM facts |> ORDER BY k |> JOIN facts AS b ON facts.k = b.k",
            false,
        ),
        (
            "FROM facts |> ORDER BY k |> AGGREGATE COUNT(*) AS n GROUP BY v",
            false,
        ),
        (
            "FROM facts |> ORDER BY k |> AGGREGATE COUNT(*) AS n GROUP AND ORDER BY v",
            true,
        ),
    ] {
        let query = database.prepare(sql).unwrap();
        let key = query
            .plan
            .order_key(query.plan.final_relation(), 0)
            .unwrap();
        assert_eq!(key.is_some(), ordered, "{sql}");
        if let Some(key) = key {
            assert_ne!(key.column, ColumnId::new(1));
        }
    }
    database.close().unwrap();
    let (_temp, legacy) = self::database(2_000_000);
    let baseline = legacy.reserved_memory_bytes();
    assert!(matches!(
        legacy.prepare("FROM lineitem |> ORDER BY l_quantity"),
        Err(Error::Bind { .. })
    ));
    assert_eq!(legacy.reserved_memory_bytes(), baseline);
    legacy.close().unwrap();
}

#[test]
fn join_binding_preserves_occurrences_ranges_and_both_input_edges() {
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let temp = Temp(std::env::temp_dir().join(format!(
        "pipesql-frontend-joins-{}-{id}",
        std::process::id()
    )));
    fs::create_dir(&temp.0).unwrap();
    let database = Database::create_empty(
        &temp.database(),
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

    let sql = "FROM lineitem AS a |> JOIN lineitem AS b ON a.l_quantity = b.l_quantity \
               |> SELECT b.l_extendedprice, a.l_extendedprice";
    let query = database.prepare(sql).unwrap();
    assert_eq!(query.plan.occurrence_count, 2);
    assert_eq!(
        query.plan.occurrences[0].table,
        query.plan.occurrences[1].table
    );
    assert_eq!(
        query.plan.outputs().collect::<Vec<_>>(),
        [ColumnId::new(9), ColumnId::new(2)]
    );
    assert_eq!(
        query.plan.source_columns[0].storage,
        query.plan.source_columns[7].storage
    );
    assert_ne!(
        query.plan.source_columns[0].identity,
        query.plan.source_columns[7].identity
    );
    assert_eq!(query.plan.stages[1].input, RelationId::SOURCE);
    assert_eq!(
        query.plan.stages[1].stage,
        Stage::Join {
            right: RelationId(1),
            left_key: ColumnId::new(1),
            right_key: ColumnId::new(8),
        }
    );
    drop(query);
    for mutation in 0..9 {
        let mut query = database.prepare(sql).unwrap();
        match mutation {
            0 => query.plan.stages[1].input = RelationId(1),
            1 => query.plan.stages[1].columns = 13,
            2 => query.plan.occurrences[1].start = 6,
            3 => query.plan.source_columns[7].identity = ColumnId::new(1),
            4 => query.plan.stages[2].input = RelationId(1),
            5..=8 => {
                let Stage::Join {
                    right,
                    left_key,
                    right_key,
                } = &mut query.plan.stages[1].stage
                else {
                    unreachable!()
                };
                match mutation {
                    5 => *right = RelationId(2),
                    6 => *right = RelationId(0),
                    7 => *left_key = ColumnId::new(8),
                    8 => *right_key = ColumnId::new(1),
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        }
        assert!(validate(&query.plan).is_err(), "mutation {mutation}");
    }
    for sql in [
        "FROM lineitem AS a |> JOIN lineitem AS a ON a.l_quantity = a.l_quantity",
        "FROM lineitem AS a |> JOIN lineitem AS b ON a.l_quantity = a.l_extendedprice",
        "FROM lineitem AS a |> SELECT l_quantity |> SELECT a.l_quantity",
        "FROM lineitem AS a |> AS b |> SELECT a.l_quantity",
        "FROM lineitem |> SELECT l_quantity AS q,l_quantity AS q |> AS p |> SELECT p.q",
        "FROM lineitem AS a |> JOIN lineitem AS b ON a.l_quantity = b.l_quantity |> SELECT l_quantity",
        "FROM lineitem AS a |> JOIN lineitem AS b ON a.l_quantity < b.l_quantity",
    ] {
        assert!(database.prepare(sql).is_err(), "{sql}");
    }
    for sql in [
        "FROM lineitem AS a |> SELECT a . l_quantity AS q |> AS p |> SELECT p.q",
        "FROM lineitem AS a |> AGGREGATE SUM(a.l_quantity) AS s |> AS p |> SELECT p.s",
        "FROM lineitem AS a |> SELECT l_quantity AS q |> AS p |> JOIN lineitem AS b ON b.l_quantity = p.q |> SELECT p.q",
        "FROM lineitem |> AGGREGATE SUM(l_quantity) AS s |> AS p |> JOIN lineitem AS b ON p.s = b.l_quantity |> SELECT p.s",
    ] {
        let query = database.prepare(sql).unwrap();
        validate(&query.plan).unwrap();
    }
    let query = database
        .prepare(
            "FROM lineitem |> AGGREGATE SUM(l_quantity) AS s,COUNT(*) AS n |> AS p \
         |> JOIN lineitem AS b ON p.s = b.l_quantity |> SELECT b.l_quantity",
        )
        .unwrap();
    assert_eq!(
        query.plan.aggregate_demand(0),
        1,
        "a join key demands its aggregate even when projection drops it"
    );
    drop(query);
    database.close().unwrap();
}

#[test]
fn relation_edges_preserve_every_operator_and_reject_invalid_producers() {
    let (_temp, database) = database(2_000_000);
    for sql in [
        "FROM lineitem |> WHERE l_quantity > 0 |> SELECT l_quantity AS q |> WHERE q < 20",
        "FROM lineitem |> SELECT l_quantity AS q |> AGGREGATE SUM(q) AS s,COUNT(*) AS n |> WHERE n > 0 |> SELECT s",
    ] {
        let mut query = database.prepare(sql).unwrap();
        let count = usize::from(query.plan.count);
        for index in 0..count {
            let original = query.plan.stages[index].input;
            for candidate in (0..=MAX_STAGES + 1)
                .map(|value| value as u8)
                .chain([u8::MAX])
            {
                query.plan.stages[index].input = RelationId(candidate);
                assert_eq!(
                    validate(&query.plan).is_ok(),
                    candidate == original.0,
                    "node {index}, input {candidate}, query {sql}",
                );
            }
            query.plan.stages[index].input = original;
        }
        query.plan.stages[count].input = RelationId(1);
        assert!(
            validate(&query.plan).is_err(),
            "unused node input must be empty"
        );
        query.plan.stages[count] = Node::EMPTY;
        validate(&query.plan).unwrap();
        assert_eq!(
            query
                .plan
                .relation_columns(RelationId(query.plan.count))
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            query.plan.outputs().collect::<Vec<_>>(),
        );
    }
}

#[test]
fn query_identity_is_independent_of_catalog_origin() {
    let stored = crate::catalog_schema::ColumnSpec::new(
        crate::catalog_schema::ColumnId::new(91).unwrap(),
        "value",
        DataType::Int64,
        false,
    )
    .unwrap();
    let left = SourceColumn::bind_declaration(stored, 7, 1);
    let right = SourceColumn::bind_declaration(stored, 7, 2);
    assert_ne!(left.semantic(), right.semantic());
    assert_eq!(left.storage_slot(), right.storage_slot());

    let mut expression = Expression::EMPTY;
    expression.ops[..3].copy_from_slice(&[
        Op::Column(left.semantic()),
        Op::Column(right.semantic()),
        Op::Subtract,
    ]);
    expression.len = 3;
    expression.data_type = expression
        .infer(&[left.semantic(), right.semantic()])
        .unwrap();
    assert!(expression.validate(&[left.semantic()]).is_err());
    let left_values = [7_i64];
    let right_values = [11_i64];
    let inputs = [
        Some(
            crate::scalar::NumericInput::new(
                right.semantic(),
                crate::scalar::NumericValues::Int64(&right_values),
                None,
            )
            .unwrap(),
        ),
        Some(
            crate::scalar::NumericInput::new(
                left.semantic(),
                crate::scalar::NumericValues::Int64(&left_values),
                None,
            )
            .unwrap(),
        ),
    ];
    let mut scratch = [0_u64; 2];
    let output = expression
        .evaluate_batch(&inputs, 0..1, &mut scratch)
        .unwrap();
    assert_eq!(output.value(0), Some((-4_i64) as u64));
}

#[test]
fn catalog_binding_preserves_declared_identity_types_and_generation() {
    use crate::catalog_schema::{ColumnId as StoredColumn, ColumnSpec, TableId};
    use crate::native_unit::{InputColumn, InputValues};
    let (temp, stock) = database(2_000_000);
    stock.close().unwrap();
    let db = Database::create_catalog_with_effects(
        &temp.0.join("catalog"),
        crate::Config::new(2_000_000, 1_000_000).unwrap(),
        &mut crate::effects::Effects::default(),
    )
    .unwrap();
    let cancel = crate::CancellationToken::new();
    let table = TableId::new(901).unwrap();
    let columns = [
        ColumnSpec::new(
            StoredColumn::new(29).unwrap(),
            "word",
            DataType::String,
            true,
        )
        .unwrap(),
        ColumnSpec::new(
            StoredColumn::new(3).unwrap(),
            "amount",
            DataType::Double,
            false,
        )
        .unwrap(),
    ];
    db.catalog_writer()
        .unwrap()
        .create_table(
            "facts",
            table,
            &columns,
            &cancel,
            &mut crate::effects::Effects::default(),
        )
        .unwrap();
    let query = prepare_catalog(
        &db,
        "FROM FaCtS |> SELECT amount AS n,word AS s |> WHERE n > 0",
    )
    .unwrap();
    assert_eq!(query.plan.table(), Some(table));
    assert_eq!(query.plan.generation, 1);
    assert_eq!(&query.plan.catalog_columns[..2], &[29, 3]);
    for (index, column) in query.plan.source_columns().enumerate() {
        assert!(
            query
                .plan
                .matches_declaration(column, columns[index], index)
        );
        assert!(
            !query
                .plan
                .matches_declaration(column, columns[1 - index], index)
        );
    }

    assert_eq!(
        query
            .plan
            .columns()
            .map(|column| (
                column.identity.0,
                column.storage,
                column.kind,
                column.nullable,
            ))
            .collect::<Vec<_>>(),
        [
            (2, 1, DataType::Double, false),
            (1, 0, DataType::String, true),
        ]
    );
    assert_eq!(
        query.result_column(0),
        Some(ResultColumn {
            name: Some("n"),
            data_type: DataType::Double,
            nullable: false
        })
    );
    assert_eq!(
        query.result_column(1),
        Some(ResultColumn {
            name: Some("s"),
            data_type: DataType::String,
            nullable: true
        })
    );
    let inputs = [
        InputColumn {
            id: StoredColumn::new(3).unwrap(),
            values: InputValues::Double(&[1.0]),
            validity: &[1],
        },
        InputColumn {
            id: StoredColumn::new(29).unwrap(),
            values: InputValues::String(&["雪"]),
            validity: &[1],
        },
    ];
    db.catalog_writer()
        .unwrap()
        .append(
            table,
            &inputs,
            &cancel,
            &mut crate::effects::Effects::default(),
        )
        .unwrap();
    assert_eq!(db.generation(), 2);
    assert_eq!(query.snapshot.as_ref().unwrap().generation(), 1);
    let mut bytes = [0; crate::catalog::MAX_BYTES];
    let old = query
        .snapshot
        .as_ref()
        .unwrap()
        .read_catalog(&mut bytes, &cancel, &mut crate::effects::Effects::default())
        .unwrap()
        .unwrap();
    assert_eq!(old.table(0).unwrap().units(), 0);
    let mut current = prepare_catalog(&db, "FROM facts").unwrap();
    assert_eq!(current.plan.generation, 2);
    assert!(db.execute(&current, &cancel).is_ok());
    let public = db.prepare("FROM facts").unwrap();
    assert_eq!(public.plan.generation, current.plan.generation);
    assert_eq!(public.result_column_count(), current.result_column_count());
    for index in 0..public.result_column_count() {
        assert_eq!(public.result_column(index), current.result_column(index));
    }
    drop(public);
    for _ in 0..16 {
        assert!(prepare_catalog(&db, "FROM facts |> SELECT missing").is_err());
        assert!(prepare_catalog(&db, "FROM absent").is_err());
    }
    assert!(prepare_catalog(&db, "FROM facts |> AGGREGATE COUNT(*) AS n").is_ok());
    let pin = db.catalog_snapshot().unwrap();
    drop(pin);
    let origins = current.plan.catalog_columns;
    for (slot, value) in [(0, 0), (1, 29), (2, 31)] {
        current.plan.catalog_columns[slot] = value;
        assert!(validate(&current.plan).is_err());
        current.plan.catalog_columns = origins;
    }
    current.plan.catalog_columns[0] = 30;
    assert!(validate(&current.plan).is_ok());
    let owned = db.reserved_memory_bytes();
    assert!(matches!(
        db.execute(&current, &cancel),
        Err(Error::Corrupt(_))
    ));
    assert_eq!(db.reserved_memory_bytes(), owned);
    current.plan.catalog_columns = origins;
    drop(current);
    drop(query);
    assert_eq!(
        db.reserved_memory_bytes(),
        crate::catalog_snapshot::REGISTRY_BYTES + db.path_memory_bytes()
    );
}

#[test]
fn reserved_keywords_cannot_be_bare_output_aliases() {
    let (_temp, database) = database(1_048_576);
    assert_eq!(RESERVED_IDENTIFIERS.len(), 99);
    assert!(
        RESERVED_IDENTIFIERS
            .windows(2)
            .all(|pair| pair[0] < pair[1])
    );
    for alias in RESERVED_IDENTIFIERS
        .iter()
        .flat_map(|word| [word.to_string(), word.to_ascii_lowercase()])
    {
        for source in [
            Q6.replace("AS revenue", &format!("AS {alias}")),
            Q1.replace("AS sum_qty", &format!("AS {alias}")),
        ] {
            let Err(Error::Parse { span, .. }) = database.prepare(&source) else {
                panic!("accepted bare reserved alias {alias}");
            };
            assert_eq!(&source[span.start()..span.end()], alias);
        }
    }
}

#[test]
fn nonreserved_syntax_words_remain_valid_output_aliases() {
    let (_temp, database) = database(1_048_576);
    for alias in ["date", "aggregate", "function", "language", "safe_cast"] {
        for (source, column) in [
            (Q6.replace("AS revenue", &format!("AS {alias}")), 0),
            (Q1.replace("AS sum_qty", &format!("AS {alias}")), 2),
        ] {
            let prepared = database.prepare(&source).unwrap();
            assert_eq!(prepared.result_column(column).unwrap().name, Some(alias));
        }
    }
}

#[test]
fn diagnostics_identify_exact_source_bytes() {
    let (_temp, database) = database(1_048_576);
    let unknown = Q6.replacen("lineitem", "unknownxx", 1);
    match database.prepare(&unknown) {
        Err(Error::Bind { span, .. }) => {
            assert_eq!(&unknown[span.start()..span.end()], "unknownxx");
        }
        outcome => panic!("unexpected bind outcome: {}", outcome.err().unwrap()),
    }
    let offset = Q6.find("l_quantity <").unwrap();
    let invalid = Q6.replacen("l_quantity", "@_quantity", 1);
    match database.prepare(&invalid) {
        Err(Error::Parse { span, .. }) => {
            assert_eq!((span.start(), span.end()), (offset, offset + 1))
        }
        outcome => panic!("unexpected parse outcome: {}", outcome.err().unwrap()),
    }
}

#[test]
fn lexical_stage_and_name_boundaries() {
    assert_eq!(lex(Q1).unwrap().len, 98);
    assert_eq!(lex(&"a ".repeat(MAX_TOKENS)).unwrap().len, MAX_TOKENS);
    assert!(matches!(
        lex(&"a ".repeat(MAX_TOKENS + 1)),
        Err(Error::Parse { .. })
    ));
    assert!(lex(&format!("#{}", "x".repeat(MAX_SOURCE_BYTES - 1))).is_ok());
    assert!(matches!(
        lex(&format!("#{}", "x".repeat(MAX_SOURCE_BYTES))),
        Err(Error::Parse { .. })
    ));
    for value in ['a', '1'] {
        assert!(lex(&value.to_string().repeat(MAX_NAME_BYTES)).is_ok());
        assert!(matches!(
            lex(&value.to_string().repeat(MAX_NAME_BYTES + 1)),
            Err(Error::Parse { .. })
        ));
    }
    assert!(lex(&format!("'{}'", "x".repeat(MAX_NAME_BYTES))).is_ok());
    assert!(matches!(
        lex(&format!("'{}'", "x".repeat(MAX_NAME_BYTES + 1))),
        Err(Error::Parse { .. })
    ));
    let (_temp, database) = database(1_048_576);
    let source = format!(
        "FROM lineitem{}",
        " |> SELECT l_quantity".repeat(MAX_STAGES)
    );
    assert!(database.prepare(&source).is_ok());
    assert!(matches!(
        database.prepare(&(source + " |> SELECT l_quantity")),
        Err(Error::Parse { .. })
    ));
    let alias = "a".repeat(MAX_NAME_BYTES);
    assert_eq!(
        database
            .prepare(&Q6.replacen("revenue", &alias, 1))
            .unwrap()
            .result_column(0)
            .unwrap()
            .name,
        Some(alias.as_str())
    );
}

#[test]
fn malformed_byte_campaigns_release_all_prepared_owners() {
    let (_temp, database) = database(1_048_576);
    for original in [Q6, Q1] {
        let mut outcomes = [0_u32; 3];
        for index in 0..original.len() {
            for replacement in [b' ', b'#', b'\'', b'|', 0xff] {
                let mut bytes = original.as_bytes().to_vec();
                bytes[index] = replacement;
                match std::str::from_utf8(&bytes) {
                    Ok(source) => match database.prepare(source) {
                        Ok(plan) => {
                            outcomes[0] += 1;
                            drop(plan);
                        }
                        Err(Error::Parse { .. }) => outcomes[1] += 1,
                        Err(Error::Bind { .. }) => outcomes[2] += 1,
                        Err(error) => panic!("unexpected mutated query error: {error}"),
                    },
                    Err(_) => outcomes[1] += 1,
                }
                assert_eq!(
                    database.reserved_memory_bytes(),
                    database.path_memory_bytes()
                );
            }
        }
        assert_eq!(outcomes.iter().sum::<u32>() as usize, original.len() * 5);
        assert!(outcomes.iter().all(|count| *count > 0));
        eprintln!(
            "query_mutations={} success={} parse={} bind={}",
            original.len() * 5,
            outcomes[0],
            outcomes[1],
            outcomes[2]
        );
    }
}

#[test]
fn exact_prepared_admission_and_concurrent_owners() {
    let (temp, db) = database(1_048_576);
    let resident = db.reserved_memory_bytes();
    let required = db.prepare(Q6).unwrap().accounted_memory_bytes();
    db.close().unwrap();
    let total = resident + required;
    let refused =
        Database::open(&temp.database(), crate::Config::new(total - 1, 1).unwrap()).unwrap();
    assert_eq!(refused.reserved_memory_bytes(), resident);
    assert!(
        matches!(refused.prepare(Q6),Err(Error::Resource { required: value, limit, .. }) if value==total && limit==total-1)
    );
    assert_eq!(refused.reserved_memory_bytes(), resident);
    refused.close().unwrap();
    const QUERIES: usize = 8;
    for (limit, expected) in [
        (required * QUERIES as u64, QUERIES),
        (required * QUERIES as u64 - 1, QUERIES - 1),
    ] {
        let db = Database::open(
            &temp.database(),
            crate::Config::new(resident + limit, 1).unwrap(),
        )
        .unwrap();
        let barrier = std::sync::Barrier::new(QUERIES + 1);
        std::thread::scope(|scope| {
            let handles: [_; QUERIES] = std::array::from_fn(|_| {
                let db = &db;
                let barrier = &barrier;
                scope.spawn(move || {
                    let prepared = std::panic::catch_unwind(|| db.prepare(Q6));
                    barrier.wait();
                    barrier.wait();
                    match prepared.expect("prepare panic") {
                        Ok(prepared) => {
                            drop(prepared);
                            true
                        }
                        Err(Error::Resource { .. }) => false,
                        Err(error) => panic!("unexpected prepare error: {error}"),
                    }
                })
            });
            barrier.wait();
            let observed = db.reserved_memory_bytes();
            barrier.wait();
            assert_eq!(observed, resident + required * expected as u64);
            assert_eq!(
                handles
                    .into_iter()
                    .map(|handle| usize::from(handle.join().unwrap()))
                    .sum::<usize>(),
                expected
            );
        });
        assert_eq!(db.reserved_memory_bytes(), db.path_memory_bytes());
    }
}
