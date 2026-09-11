use super::order::{integers, query};
use super::*;

fn failure(db: &Database, sql: &str, operation: &'static str, expression: &str) {
    let baseline = db.reserved_memory_bytes();
    let prepared = db
        .prepare(sql)
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
    let cancel = CancellationToken::new();
    let mut result = db
        .execute(&prepared, &cancel)
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
    let start = sql.find(expression).unwrap();
    let mut failed = false;
    for _ in 0..100_000 {
        match result.step() {
            QueryStep::Progress | QueryStep::Rows(_) => (),
            QueryStep::Failed(pipesql::Error::ArithmeticOverflow {
                operation: actual,
                span,
            }) => {
                assert_eq!(*actual, operation, "{sql}");
                assert_eq!(span.start(), start, "{sql}");
                assert_eq!(span.end(), start + expression.len(), "{sql}");
                failed = true;
                break;
            }
            QueryStep::Finished => panic!("missing failure: {sql}"),
            QueryStep::Failed(error) => panic!("{sql}: {error}"),
        }
    }
    assert!(failed);
    drop(result);
    drop(prepared);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
}

#[test]
fn public_computed_demand_preserves_predicate_order_and_aggregate_finalization() {
    let (_directory, db) = join_fixture();
    for (sql, expected) in [
        (
            "FROM facts |> SELECT k,k * 9223372036854775807 AS bad |> WHERE k < 2 |> SELECT bad",
            integers(&[i64::MAX, i64::MAX]),
        ),
        (
            "FROM facts |> SELECT v,v * 9223372036854775807 AS bad |> SELECT v |> ORDER BY v",
            integers(&[10, 20, 30, 40]),
        ),
        (
            "FROM facts |> SELECT v,9223372036854775807 + 1 AS bad |> WHERE v < 0 |> SELECT bad",
            vec![],
        ),
        (
            "FROM facts |> AGGREGATE SUM(9223372036854775807) AS s,COUNT(*) AS n |> SELECT s + 0 AS x,n |> WHERE n < 1 |> SELECT x",
            vec![],
        ),
        (
            "FROM facts |> AGGREGATE SUM(9223372036854775807) AS s,COUNT(*) AS n |> SELECT s + 0 AS x,n |> SELECT n",
            integers(&[4]),
        ),
        (
            "FROM facts |> SELECT v * 2 AS w |> WHERE w > 40 |> AGGREGATE SUM(w) AS s |> SELECT s + 1 AS x",
            integers(&[141]),
        ),
        (
            "FROM facts |> SELECT k+1 AS g,v+1 AS w |> AGGREGATE SUM(w) AS s GROUP BY g |> SELECT g,s+1 AS z |> ORDER BY g NULLS FIRST",
            vec![
                vec![Cell::Null, Cell::Integer(42)],
                vec![Cell::Integer(2), Cell::Integer(33)],
                vec![Cell::Integer(3), Cell::Integer(32)],
            ],
        ),
        (
            "FROM facts |> SELECT k * 2 AS x |> ORDER BY x NULLS FIRST",
            vec![
                vec![Cell::Null],
                vec![Cell::Integer(2)],
                vec![Cell::Integer(2)],
                vec![Cell::Integer(4)],
            ],
        ),
    ] {
        query(&db, sql, expected);
    }
    for (sql, operation, expression) in [
        (
            "FROM facts |> SELECT k,k * 9223372036854775807 AS bad |> WHERE bad > 0 |> WHERE k < 2 |> SELECT bad",
            "multiplication",
            "k * 9223372036854775807",
        ),
        (
            "FROM facts |> AGGREGATE SUM(9223372036854775807) AS s,COUNT(*) AS n |> SELECT s+0 AS x,n |> WHERE x > 0 |> WHERE n < 1 |> SELECT x",
            "SUM",
            "SUM(9223372036854775807)",
        ),
        (
            "FROM facts |> SELECT v*9223372036854775807 AS x |> AGGREGATE SUM(x) AS s,COUNT(*) AS n |> WHERE n < 1 |> SELECT s",
            "multiplication",
            "v*9223372036854775807",
        ),
        (
            "FROM facts |> WHERE v > 35 |> SELECT k * (9223372036854775807 + 1) AS bad",
            "addition",
            "k * (9223372036854775807 + 1)",
        ),
    ] {
        failure(&db, sql, operation, expression);
    }
    failure(
        &db,
        "# 雪
FROM facts |> SELECT v*9223372036854775807 AS a,v *9223372036854775807 AS b |> SELECT b",
        "multiplication",
        "v *9223372036854775807",
    );
}

#[test]
fn public_computed_materialization_boundaries_and_join_replay() {
    let (_directory, db) = join_fixture();
    for sql in [
        "FROM facts |> SELECT v,v*9223372036854775807 AS bad |> LIMIT 0 |> SELECT bad",
        "FROM facts |> SELECT v,v*9223372036854775807 AS bad |> ORDER BY bad |> LIMIT 0 |> SELECT v",
        "FROM facts |> LIMIT 0 |> SELECT 9223372036854775807+1",
    ] {
        query(&db, sql, vec![]);
    }
    for (sql, expression) in [
        (
            "FROM facts |> SELECT v,v*9223372036854775807 AS bad |> ORDER BY bad |> WHERE v < 0 |> SELECT v",
            "v*9223372036854775807",
        ),
        (
            "FROM facts |> SELECT v,v*9223372036854775807 AS bad |> LIMIT 0 OFFSET 1 |> SELECT bad",
            "v*9223372036854775807",
        ),
        (
            "FROM facts |> SELECT k,v*9223372036854775807 AS bad |> AS f |> JOIN dimensions AS d ON f.k=d.k |> WHERE f.k < 0 |> SELECT f.bad",
            "v*9223372036854775807",
        ),
    ] {
        failure(&db, sql, "multiplication", expression);
    }
    for (sql, expected) in [
        (
            "FROM facts |> ORDER BY v |> SELECT v+1 AS x |> LIMIT 2 |> SELECT x*2 AS y",
            integers(&[22, 42]),
        ),
        (
            "FROM facts |> SELECT k+0 AS k,v+1 AS x |> AS f |> JOIN dimensions AS d ON f.k=d.k |> SELECT f.x*2 AS y |> ORDER BY y",
            integers(&[22, 22, 42, 42, 62]),
        ),
        (
            "FROM facts AS f |> JOIN dimensions AS d ON f.k=d.k |> SELECT f.v+1 AS x |> WHERE x > 20 |> ORDER BY x",
            integers(&[21, 21, 31]),
        ),
    ] {
        query(&db, sql, expected);
    }
}

#[test]
fn public_computed_names_types_scope_and_linear_dependencies() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    let sql = "FROM facts |> SELECT v+1,k,(v),+v,1.5 AS d";
    let prepared = db.prepare(sql).unwrap();
    for (index, name, kind, nullable) in [
        (0, None, DataType::Int64, false),
        (1, Some("k"), DataType::Int64, true),
        (2, Some("v"), DataType::Int64, false),
        (3, None, DataType::Int64, false),
        (4, Some("d"), DataType::Double, false),
    ] {
        let column = prepared.result_column(index).unwrap();
        assert_eq!(
            (column.name, column.data_type, column.nullable),
            (name, kind, nullable)
        );
    }
    drop(prepared);
    for sql in [
        "FROM facts |> SELECT v+1 AS x,x+1 AS y",
        "FROM facts |> SELECT v+1 |> WHERE v > 0",
        "FROM facts |> SELECT v+1 |> AS f |> SELECT f.v",
        "FROM facts |> SELECT v+1 AS x,k+1 AS x |> SELECT x",
        "FROM facts |> SELECT 9223372036854775808 AS x |> SELECT 1",
        "FROM facts |> SELECT missing+1 AS x |> SELECT 1",
        "FROM dimensions |> SELECT label+1",
        "FROM dimensions |> SELECT +label",
    ] {
        assert!(db.prepare(sql).is_err(), "{sql}");
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    query(
        &db,
        "FROM facts |> SELECT v+1 AS v,v AS old |> SELECT v-old AS delta",
        integers(&[1, 1, 1, 1]),
    );
    query(
        &db,
        "FROM facts |> SELECT v+1 |> ORDER BY 1 DESC",
        integers(&[41, 31, 21, 11]),
    );
    query(
        &db,
        "FROM dimensions |> SELECT ((label)) |> AS d |> SELECT d.label |> ORDER BY label",
        ["a", "b", "c", "null"]
            .iter()
            .map(|value| vec![Cell::Text((*value).to_owned())])
            .collect(),
    );
    query(
        &db,
        "FROM facts |> SELECT 1 |> AS f |> SELECT 2",
        integers(&[2, 2, 2, 2]),
    );
    let mut chain = "FROM facts".to_owned();
    // Fourteen transparent definitions plus WHERE and final projection fit the
    // stage bound. Each edge is repeated; tree expansion would require 2^14 work.
    for _ in 0..14 {
        chain.push_str(" |> SELECT v+v AS v");
    }
    chain.push_str(" |> WHERE v < 200000 |> SELECT v");
    query(&db, &chain, integers(&[10 * (1 << 14)]));
}

#[test]
fn public_computed_identity_capacity_exceeds_one_u128() {
    let directory = Directory::new();
    let db = Database::create_empty(&directory.database(), config()).unwrap();
    let cancel = CancellationToken::new();
    let names: Vec<_> = (0..64).map(|i| format!("c{i}")).collect();
    let columns: Vec<_> = names
        .iter()
        .map(|name| ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: false,
        })
        .collect();
    db.declare_table("wide", &columns, &cancel).unwrap();
    let mut append = db.begin_append("wide", limits(), &cancel).unwrap();
    let inputs = vec![
        ColumnInput {
            values: ColumnValues::Int64(&[1]),
            validity: &[1]
        };
        64
    ];
    append.write(&inputs, &cancel).unwrap();
    append.commit(&cancel).unwrap();
    let sql = format!(
        "FROM wide |> SELECT {} |> SELECT 2,3,4,5",
        vec!["1"; 64].join(",")
    );
    query(
        &db,
        &sql,
        vec![vec![
            Cell::Integer(2),
            Cell::Integer(3),
            Cell::Integer(4),
            Cell::Integer(5),
        ]],
    );
}

#[test]
fn public_computed_cancellation_releases_source_and_consuming_owners() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    for sql in [
        "FROM facts |> SELECT v+1 AS x |> WHERE x > 0",
        "FROM facts |> SELECT k+1 AS k,v+1 AS x |> ORDER BY x |> AGGREGATE SUM(x) AS s GROUP BY k |> SELECT s+1 AS x",
        "FROM facts |> SELECT k,v+1 AS v |> AS f |> JOIN dimensions AS d ON f.k=d.k |> SELECT f.v+1 AS x |> ORDER BY x |> LIMIT 3 |> SELECT x+1 AS y",
    ] {
        for after in [0, 3, 12, 30] {
            let prepared = db.prepare(sql).unwrap();
            let cancel = CancellationToken::new();
            let mut result = db.execute(&prepared, &cancel).unwrap();
            let mut terminal = false;
            for step in 0..1000 {
                if step == after {
                    cancel.cancel();
                }
                match result.step() {
                    QueryStep::Progress | QueryStep::Rows(_) => (),
                    QueryStep::Finished => {
                        terminal = true;
                        break;
                    }
                    QueryStep::Failed(Error::Cancelled) => {
                        assert!(step >= after);
                        terminal = true;
                        break;
                    }
                    QueryStep::Failed(error) => panic!("{sql}: {error}"),
                }
            }
            assert!(terminal);
            drop(result);
            drop(prepared);
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
    }
}
