//! Check when computed values are required and which source expression owns failure.
//!
//! Stage cases come first: projection, extension, replacement and naming must
//! preserve column identity across filters, groups and joins. Numeric cases then
//! pair useful results with unused, conditionally skipped and demanded failures.
//! SAFE_DIVIDE handles its own division failure, not errors in its arguments.
//!
//! Shared cases own cancellation and stored DOUBLE bit checks across producers
//! and reopen. Function-specific cases keep their literal answers nearby; logarithm,
//! exponential and power comparisons state their independent reference and ULP
//! tolerance. Owned-error cases drop the SQL before execution and retain the error
//! after query teardown. These are public integration checks; scalar unit tests
//! own individual evaluation rules.

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
    assert!(matches!(
        result.step(),
        QueryStep::Failed(Error::ArithmeticOverflow { .. })
    ));
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
            "FROM facts |> SELECT k, k * 9223372036854775807 AS bad |> WHERE k < 2 |> SELECT bad",
            integers(&[i64::MAX, i64::MAX]),
        ),
        (
            "FROM facts |> SELECT v, v * 9223372036854775807 AS bad |> SELECT v |> ORDER BY v",
            integers(&[10, 20, 30, 40]),
        ),
        (
            "FROM facts |> SELECT v, 9223372036854775807 + 1 AS bad |> WHERE v < 0 |> SELECT bad",
            vec![],
        ),
        (
            "FROM facts |> AGGREGATE SUM(9223372036854775807) AS s, COUNT(*) AS n |> SELECT s + 0 AS x, n |> WHERE n < 1 |> SELECT x",
            vec![],
        ),
        (
            "FROM facts |> AGGREGATE SUM(9223372036854775807) AS s, COUNT(*) AS n |> SELECT s + 0 AS x, n |> SELECT n",
            integers(&[4]),
        ),
        (
            "FROM facts |> SELECT v * 2 AS w |> WHERE w > 40 |> AGGREGATE SUM(w) AS s |> SELECT s + 1 AS x",
            integers(&[141]),
        ),
        (
            "FROM facts |> SELECT k+1 AS g, v+1 AS w |> AGGREGATE SUM(w) AS s GROUP BY g |> SELECT g, s+1 AS z |> ORDER BY g NULLS FIRST",
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
            "FROM facts |> SELECT k, k * 9223372036854775807 AS bad |> WHERE bad > 0 |> WHERE k < 2 |> SELECT bad",
            "multiplication",
            "k * 9223372036854775807",
        ),
        (
            "FROM facts |> AGGREGATE SUM(9223372036854775807) AS s, COUNT(*) AS n |> SELECT s+0 AS x, n |> WHERE x > 0 |> WHERE n < 1 |> SELECT x",
            "SUM",
            "SUM(9223372036854775807)",
        ),
        (
            "FROM facts |> SELECT v*9223372036854775807 AS x |> AGGREGATE SUM(x) AS s, COUNT(*) AS n |> WHERE n < 1 |> SELECT s",
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
FROM facts |> SELECT v*9223372036854775807 AS a, v *9223372036854775807 AS b |> SELECT b",
        "multiplication",
        "v *9223372036854775807",
    );
}

#[test]
fn public_computed_materialization_boundaries_and_join_replay() {
    let (_directory, db) = join_fixture();
    for sql in [
        "FROM facts |> SELECT v, v*9223372036854775807 AS bad |> LIMIT 0 |> SELECT bad",
        "FROM facts |> SELECT v, v*9223372036854775807 AS bad |> ORDER BY bad |> LIMIT 0 |> SELECT v",
        "FROM facts |> LIMIT 0 |> SELECT 9223372036854775807+1",
    ] {
        query(&db, sql, vec![]);
    }
    for (sql, expression) in [
        (
            "FROM facts |> SELECT v, v*9223372036854775807 AS bad |> ORDER BY bad |> WHERE v < 0 |> SELECT v",
            "v*9223372036854775807",
        ),
        (
            "FROM facts |> SELECT v, v*9223372036854775807 AS bad |> LIMIT 0 OFFSET 1 |> SELECT bad",
            "v*9223372036854775807",
        ),
        (
            "FROM facts |> SELECT k, v*9223372036854775807 AS bad |> AS f |> JOIN dimensions AS d ON f.k=d.k |> WHERE f.k < 0 |> SELECT f.bad",
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
            "FROM facts |> SELECT k+0 AS k, v+1 AS x |> AS f |> JOIN dimensions AS d ON f.k=d.k |> SELECT f.x*2 AS y |> ORDER BY y",
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
    let sql = "FROM facts |> SELECT v+1, k, (v), +v, 1.5 AS d";
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
        "FROM facts |> SELECT v+1 AS x, x+1 AS y",
        "FROM facts |> SELECT v+1 |> WHERE v > 0",
        "FROM facts |> SELECT v+1 |> AS f |> SELECT f.v",
        "FROM facts |> SELECT v+1 AS x, k+1 AS x |> SELECT x",
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
        "FROM facts |> SELECT v+1 AS v, v AS old |> SELECT v-old AS delta",
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
        "FROM wide |> SELECT {} |> SELECT 2, 3, 4, 5",
        vec!["1"; 64].join(", ")
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
        "FROM facts |> SELECT '雪' AS label, DATE '1970-01-01' AS day |> ORDER BY label |> DISTINCT",
        "FROM facts AS f |> EXTEND 'branch' AS tag |> JOIN dimensions AS d ON f.k=d.k |> SELECT tag, d.label |> ORDER BY tag",
        "FROM facts |> SELECT v+1 AS x |> WHERE x > 0",
        "FROM facts |> EXTEND v+1 AS x |> WHERE x > 0",
        "FROM facts AS f |> SET v=v+1 |> RENAME v AS adjusted |> DROP k |> WHERE f.k>0 |> ORDER BY adjusted |> SELECT f.v, adjusted",
        "FROM dimensions |> SET label=label |> DISTINCT |> ORDER BY label |> LIMIT 2",
        "FROM facts AS f |> EXTEND v+1 AS x |> JOIN dimensions AS d ON f.k=d.k |> ORDER BY x |> AGGREGATE SUM(x) AS s |> EXTEND s+1 AS total",
        "FROM facts |> SELECT k+1 AS k, v+1 AS x |> ORDER BY x |> AGGREGATE SUM(x) AS s GROUP BY k |> SELECT s+1 AS x",
        "FROM facts |> SELECT k, v+1 AS v |> AS f |> JOIN dimensions AS d ON f.k=d.k |> SELECT f.v+1 AS x |> ORDER BY x |> LIMIT 3 |> SELECT x+1 AS y",
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

#[test]
fn extend_executes_through_projection_filters_groups_joins_and_derived_inputs() {
    let (_directory, db) = join_fixture();
    for (sql, expected) in [
        (
            "FROM facts |> EXTEND v*2 twice |> SELECT v, twice |> ORDER BY v",
            vec![
                vec![Cell::Integer(10), Cell::Integer(20)],
                vec![Cell::Integer(20), Cell::Integer(40)],
                vec![Cell::Integer(30), Cell::Integer(60)],
                vec![Cell::Integer(40), Cell::Integer(80)],
            ],
        ),
        (
            "FROM facts |> EXTEND 9223372036854775807+1 AS bad |> SELECT v |> ORDER BY v",
            integers(&[10, 20, 30, 40]),
        ),
        (
            "FROM facts |> EXTEND k*9223372036854775807 AS bad |> WHERE k < 2 |> SELECT bad",
            integers(&[i64::MAX, i64::MAX]),
        ),
        (
            "FROM facts |> EXTEND 9223372036854775807+1 AS bad |> WHERE v < 0 |> SELECT bad",
            vec![],
        ),
        (
            "FROM facts AS f |> EXTEND v+1 AS n |> JOIN (FROM facts |> SELECT k, v) AS d ON f.k=d.k |> AGGREGATE SUM(n) AS total",
            integers(&[95]),
        ),
        (
            "FROM (FROM facts |> EXTEND v*2 AS twice) AS d |> WHERE d.twice > 40 |> SELECT d.v |> ORDER BY v",
            integers(&[30, 40]),
        ),
        (
            "FROM facts |> EXTEND v*2 AS twice |> AGGREGATE SUM(twice) AS s GROUP AND ORDER BY k |> EXTEND s+1 AS next |> SELECT next",
            integers(&[81, 61, 61]),
        ),
        (
            "FROM facts |> ORDER BY v DESC |> EXTEND v+1 AS next |> SELECT next",
            integers(&[41, 31, 21, 11]),
        ),
        (
            "FROM facts |> EXTEND k+1 AS next |> SELECT next |> ORDER BY next NULLS FIRST",
            vec![
                vec![Cell::Null],
                vec![Cell::Integer(2)],
                vec![Cell::Integer(2)],
                vec![Cell::Integer(3)],
            ],
        ),
    ] {
        query(&db, sql, expected);
    }
    failure(
        &db,
        "FROM facts |> EXTEND v*9223372036854775807 AS bad |> SELECT bad",
        "multiplication",
        "v*9223372036854775807",
    );
}

#[test]
fn extend_retains_typed_values_nulls_and_original_range_members() {
    let (_directory, db) = nullable_facts().unwrap();
    query(
        &db,
        "FROM facts AS f |> EXTEND s AS text, d AS day |> ORDER BY id |> SELECT f.s, text, f.d, day",
        vec![
            vec![
                Cell::Text("present".into()),
                Cell::Text("present".into()),
                Cell::Day(0),
                Cell::Day(0),
            ],
            vec![Cell::Null, Cell::Null, Cell::Day(0), Cell::Day(0)],
            vec![
                Cell::Text("".into()),
                Cell::Text("".into()),
                Cell::Day(0),
                Cell::Day(0),
            ],
            vec![
                Cell::Text("é".into()),
                Cell::Text("é".into()),
                Cell::Null,
                Cell::Null,
            ],
        ],
    );
    // The second expression resolves the original id, despite the new alias.
    query(
        &db,
        "FROM facts |> EXTEND id+100 AS id, id+1 AS next |> SELECT next |> ORDER BY next",
        integers(&[1, 2, 3, 4]),
    );
    query(
        &db,
        "FROM facts |> WHERE id < 0 |> EXTEND s AS text, d AS day",
        vec![],
    );
}

#[test]
fn rename_preserves_values_order_and_qualified_inputs_through_composition() {
    let (_directory, db) = join_fixture();
    query(
        &db,
        "FROM facts AS f |> ORDER BY v DESC |> RENAME v AS amount |> SELECT f.v, amount",
        [40, 30, 20, 10]
            .map(|value| vec![Cell::Integer(value), Cell::Integer(value)])
            .to_vec(),
    );
    query(
        &db,
        "FROM (FROM facts |> RENAME v AS amount) AS f |> JOIN dimensions AS d ON f.k=d.k |> AGGREGATE SUM(f.amount) AS total |> RENAME total AS amount",
        integers(&[90]),
    );
    query(
        &db,
        "FROM facts |> EXTEND v+1 AS adjusted |> RENAME adjusted AS amount |> WHERE amount > 30 |> ORDER BY amount |> LIMIT 1 |> SELECT amount",
        integers(&[31]),
    );
}

#[test]
fn drop_removes_ordinary_outputs_and_undemanded_computations() {
    let (_directory, db) = join_fixture();
    for sql in [
        "FROM facts |> DROP k |> ORDER BY v",
        "FROM facts |> DROP k |> DISTINCT |> ORDER BY v",
        "FROM facts |> EXTEND v*9223372036854775807 AS bad |> DROP bad, k |> ORDER BY v",
        "FROM facts |> SELECT k AS discarded, k AS discarded, v |> DROP discarded |> ORDER BY v",
    ] {
        query(&db, sql, integers(&[10, 20, 30, 40]));
    }
    assert!(matches!(
        db.prepare("FROM facts AS k |> DROP k |> SELECT k.v"),
        Err(Error::Bind { .. })
    ));
}

#[test]
fn drop_retains_qualified_inputs_through_filters_and_blocking_operators() {
    let (_directory, db) = join_fixture();
    query(
        &db,
        "FROM facts AS f |> DROP k |> AGGREGATE SUM(v) AS total GROUP AND ORDER BY f.k",
        vec![
            vec![Cell::Null, Cell::Integer(40)],
            vec![Cell::Integer(1), Cell::Integer(30)],
            vec![Cell::Integer(2), Cell::Integer(30)],
        ],
    );
    query(
        &db,
        "FROM facts AS f |> DROP k |> WHERE f.k > 1 |> SELECT v",
        integers(&[30]),
    );
    query(
        &db,
        "FROM facts AS f |> DROP k |> ORDER BY v DESC |> SELECT f.k",
        vec![
            vec![Cell::Null],
            vec![Cell::Integer(2)],
            vec![Cell::Integer(1)],
            vec![Cell::Integer(1)],
        ],
    );
    query(
        &db,
        "FROM facts AS f |> DROP k |> JOIN dimensions AS d ON f.k=d.k |> SELECT f.v |> ORDER BY v",
        integers(&[10, 10, 20, 20, 30]),
    );
}

#[test]
fn set_replacements_preserve_original_inputs_and_typed_values() {
    let (_directory, db) = join_fixture();
    query(
        &db,
        "FROM facts AS f |> SET k=v, v=k |> ORDER BY k |> SELECT k, v, f.k, f.v",
        vec![
            vec![
                Cell::Integer(10),
                Cell::Integer(1),
                Cell::Integer(1),
                Cell::Integer(10),
            ],
            vec![
                Cell::Integer(20),
                Cell::Integer(1),
                Cell::Integer(1),
                Cell::Integer(20),
            ],
            vec![
                Cell::Integer(30),
                Cell::Integer(2),
                Cell::Integer(2),
                Cell::Integer(30),
            ],
            vec![Cell::Integer(40), Cell::Null, Cell::Null, Cell::Integer(40)],
        ],
    );
    query(
        &db,
        "FROM dimensions AS d |> SET label=d.label |> WHERE k=2 |> ORDER BY label |> SELECT label, d.label",
        vec![vec![Cell::Text("c".into()), Cell::Text("c".into())]],
    );
    query(
        &db,
        "FROM facts AS f |> SET v=v+1 |> ORDER BY v |> SELECT v-f.v AS difference",
        integers(&[1, 1, 1, 1]),
    );
}

#[test]
fn set_copies_date_nulls_and_changes_type_without_losing_original_values() {
    let directory = Directory::new();
    let db = Database::create_empty(&directory.database(), config()).unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "dates",
        &[
            ColumnDeclaration {
                name: "ordinal",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "day",
                data_type: DataType::Date,
                nullable: true,
            },
        ],
        &cancel,
    )
    .unwrap();
    let sql = "FROM dates AS d |> SET ordinal=day |> ORDER BY ordinal |> SELECT ordinal, d.ordinal";
    query(&db, sql, vec![]);
    let days = [0, -719162, 2932896].map(|day| DateValue::from_days_since_unix_epoch(day).unwrap());
    let mut append = db.begin_append("dates", limits(), &cancel).unwrap();
    append
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&[0, 1, 2]),
                    validity: &[7],
                },
                ColumnInput {
                    values: ColumnValues::Date(&days),
                    validity: &[6],
                },
            ],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    let prepared = db.prepare(sql).unwrap();
    assert_eq!(prepared.result_column(0).unwrap().data_type, DataType::Date);
    assert!(prepared.result_column(0).unwrap().nullable);
    assert_eq!(
        prepared.result_column(1).unwrap().data_type,
        DataType::Int64
    );
    assert!(!prepared.result_column(1).unwrap().nullable);
    drop(prepared);
    query(
        &db,
        sql,
        vec![
            vec![Cell::Null, Cell::Integer(0)],
            vec![Cell::Day(-719162), Cell::Integer(1)],
            vec![Cell::Day(2932896), Cell::Integer(2)],
        ],
    );
    query(
        &db,
        "FROM dates |> SET day=day |> SET day=day |> DISTINCT |> ORDER BY day |> LIMIT 2 |> SELECT day",
        vec![vec![Cell::Null], vec![Cell::Day(-719162)]],
    );
    query(
        &db,
        "FROM dates AS ordinal |> SET ordinal=day |> AGGREGATE MIN(ordinal) AS lo, MAX(ordinal) AS hi, COUNT(ordinal) AS n",
        vec![vec![
            Cell::Day(-719162),
            Cell::Day(2932896),
            Cell::Integer(2),
        ]],
    );
    assert!(matches!(
        db.prepare("FROM dates AS d |> SET day=day |> DISTINCT |> SELECT d.day"),
        Err(Error::Bind { .. })
    ));
}

#[test]
fn set_preserves_demanded_overflow_and_prunes_replaced_definitions() {
    let (_directory, db) = join_fixture();
    failure(
        &db,
        "FROM facts |> SET v=v*9223372036854775807 |> SELECT v",
        "multiplication",
        "v*9223372036854775807",
    );
    query(
        &db,
        "FROM facts |> SET v=v*9223372036854775807 |> SET v=1 |> SELECT v",
        integers(&[1, 1, 1, 1]),
    );
    query(
        &db,
        "FROM facts |> SET v=v*9223372036854775807 |> WHERE k<0 |> SELECT v",
        vec![],
    );
    query(
        &db,
        "FROM facts |> EXTEND v*9223372036854775807 AS bad |> SET bad=1 |> SELECT bad",
        integers(&[1, 1, 1, 1]),
    );
}

#[test]
fn public_division_preserves_precedence_types_and_demand() {
    let (_directory, db) = join_fixture();
    for (expression, expected) in [
        ("8/2*2", 8.0_f64),
        ("8/2/2", 2.0),
        ("1+3/2", 2.5),
        ("-(3/2)", -1.5),
        ("3.0/2", 1.5),
        ("3/2.0", 1.5),
    ] {
        query(
            &db,
            &format!("FROM facts |> LIMIT 1 |> SELECT {expression} AS ratio"),
            vec![vec![Cell::Number(expected.to_bits())]],
        );
    }
    for sql in [
        "FROM facts |> SELECT v, 1/0 AS bad |> SELECT v |> ORDER BY v",
        "FROM facts |> EXTEND 1/0 AS bad |> DROP bad |> SELECT v |> ORDER BY v",
    ] {
        query(&db, sql, integers(&[10, 20, 30, 40]));
    }
    query(
        &db,
        "FROM facts |> AGGREGATE SUM(v/2) AS total |> EXTEND COUNT(*) OVER () AS n |> SELECT total/n AS ratio",
        vec![vec![Cell::Number(50.0_f64.to_bits())]],
    );
    query(&db, "FROM facts |> SELECT 1/0 AS bad |> LIMIT 0", vec![]);
    query(
        &db,
        "FROM facts |> WHERE k IS NULL |> SELECT k/0 AS ratio",
        vec![vec![Cell::Null]],
    );
    failure(
        &db,
        "FROM facts |> SELECT 1e308/0.1 AS ratio",
        "division",
        "1e308/0.1",
    );
}

#[test]
fn public_division_zero_failure_releases_owners_and_keeps_source_span() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    for expression in [
        "v/(k-1)",
        "MOD(v, k-1)",
        "SAFE_DIVIDE(MOD(v, k-1), 0)",
        "DIV(v, k-1)",
        "SAFE_DIVIDE(DIV(v, k-1), 0)",
    ] {
        let sql = format!("# 雪\nFROM facts |> SELECT {expression} AS ratio");
        let prepared = db.prepare(&sql).unwrap();
        let cancel = CancellationToken::new();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        let mut failed = false;
        for _ in 0..100_000 {
            match result.step() {
                QueryStep::Progress | QueryStep::Rows(_) => (),
                QueryStep::Failed(pipesql::Error::DivisionByZero { span }) => {
                    assert_eq!(&sql[span.start()..span.end()], expression);
                    failed = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("{error}"),
                QueryStep::Finished => panic!("missing zero-denominator error"),
            }
        }
        assert!(failed);
        assert!(matches!(
            result.step(),
            QueryStep::Failed(pipesql::Error::DivisionByZero { .. })
        ));
        drop(result);
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    query(
        &db,
        "FROM facts |> SELECT v/2 AS ratio |> ORDER BY ratio",
        [5.0_f64, 10.0, 15.0, 20.0]
            .into_iter()
            .map(|n| vec![Cell::Number(n.to_bits())])
            .collect(),
    );
}

#[test]
fn public_division_composes_with_set_union_grouping_and_boolean_demand() {
    let (_directory, db) = join_fixture();
    for sql in [
        "FROM facts |> SET v=v/2 |> SELECT v |> ORDER BY v",
        "FROM (FROM facts |> SELECT v/2 AS ratio) AS x |> SELECT x.ratio |> ORDER BY ratio",
        "FROM facts |> SELECT v/2 AS ratio |> UNION DISTINCT (FROM facts |> SELECT v/2 AS ratio) |> ORDER BY ratio",
    ] {
        query(
            &db,
            sql,
            [5.0_f64, 10.0, 15.0, 20.0]
                .into_iter()
                .map(|n| vec![Cell::Number(n.to_bits())])
                .collect(),
        );
    }
    query(
        &db,
        "FROM facts |> SELECT k, v/(k-1) AS ratio |> WHERE k=1 OR ratio>0 |> SELECT k |> ORDER BY k",
        integers(&[1, 1, 2]),
    );
    query(
        &db,
        "FROM facts |> SELECT k, v/(k-1) AS ratio |> WHERE k!=1 AND ratio>0 |> SELECT ratio",
        vec![vec![Cell::Number(30.0_f64.to_bits())]],
    );
    query(
        &db,
        "FROM facts |> AGGREGATE SUM(v) AS total, COUNT(*) AS n GROUP AND ORDER BY k |> SELECT k, total/n AS ratio",
        vec![
            vec![Cell::Null, Cell::Number(40.0_f64.to_bits())],
            vec![Cell::Integer(1), Cell::Number(15.0_f64.to_bits())],
            vec![Cell::Integer(2), Cell::Number(30.0_f64.to_bits())],
        ],
    );
    failure(
        &db,
        "FROM facts |> SELECT (9223372036854775807+1)/2 AS ratio",
        "addition",
        "(9223372036854775807+1)/2",
    );
}

#[test]
fn public_numeric_cancellation_and_early_drop_release_owners() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    for sql in [
        "FROM facts |> SELECT v/2 AS ratio |> ORDER BY ratio",
        "FROM facts |> SELECT SAFE_DIVIDE(v, k-1) AS ratio |> ORDER BY ratio",
        "FROM facts |> SELECT ABS(v-25) AS deviation |> ORDER BY deviation",
        "FROM facts |> SELECT SIGN(v-25) AS direction |> ORDER BY direction",
        "FROM facts |> SELECT FLOOR(v/15) AS bucket |> ORDER BY bucket",
        "FROM facts |> SELECT CEILING(v/15) AS bucket |> ORDER BY bucket",
        "FROM facts |> SELECT ROUND(v/15) AS bucket |> ORDER BY bucket",
        "FROM facts |> SELECT SQRT(v) AS magnitude |> ORDER BY magnitude",
        "FROM facts |> SELECT LN(v) AS logarithm |> ORDER BY logarithm",
        "FROM facts |> SELECT LOG10(v) AS scale |> ORDER BY scale",
        "FROM facts |> SELECT EXP(v) AS growth |> ORDER BY growth",
        "FROM facts |> SELECT POWER(v, 3) AS cubed |> ORDER BY cubed",
        "FROM facts |> SELECT MOD(v, 3) AS remainder |> ORDER BY remainder",
        "FROM facts |> SELECT DIV(v, 15) AS quotient |> ORDER BY quotient",
        "FROM facts |> SELECT COALESCE(k, v) AS chosen |> ORDER BY chosen",
    ] {
        let prepared = db.prepare(sql).unwrap();
        let admitted = db.reserved_memory_bytes();
        for after in [0, 1, 3] {
            for cancel_query in [false, true] {
                let cancel = CancellationToken::new();
                let mut result = db.execute(&prepared, &cancel).unwrap();
                for _ in 0..after {
                    assert!(matches!(result.step(), QueryStep::Progress));
                }
                if cancel_query {
                    cancel.cancel();
                    assert!(matches!(result.step(), QueryStep::Failed(Error::Cancelled)));
                }
                drop(result);
                assert_eq!(db.reserved_memory_bytes(), admitted);
                assert_eq!(db.reserved_temp_bytes(), 0);
            }
        }
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    query(
        &db,
        "FROM facts |> AGGREGATE SUM(v/2) AS total",
        vec![vec![Cell::Number(50.0_f64.to_bits())]],
    );
}

#[test]
fn public_safe_divide_preserves_values_nulls_and_argument_errors() {
    let (_directory, db) = join_fixture();
    for (expression, expected) in [
        ("SAFE_DIVIDE(3, 2)", Some(1.5_f64)),
        ("SAFE_DIVIDE(3.0, 2)", Some(1.5)),
        ("SAFE_DIVIDE(3, 2.0)", Some(1.5)),
        ("SAFE_DIVIDE(1, 0)", None),
        ("SAFE_DIVIDE(1, -0.0)", None),
        ("SAFE_DIVIDE(1e308, 0.1)", None),
        ("1+SAFE_DIVIDE(3, 2)*2", Some(4.0)),
        ("SAFE_DIVIDE(SAFE_DIVIDE(9, 2), 3)", Some(1.5)),
        ("SAFE_DIVIDE(1, SAFE_DIVIDE(1, 0))", None),
        ("-SAFE_DIVIDE((3+1), 2)", Some(-2.0)),
        ("SAFE_DIVIDE(1, 0)+2", None),
    ] {
        query(
            &db,
            &format!("FROM facts |> LIMIT 1 |> SELECT {expression} AS ratio"),
            vec![vec![
                expected.map_or(Cell::Null, |n| Cell::Number(n.to_bits())),
            ]],
        );
    }
    query(
        &db,
        "FROM facts |> ORDER BY v |> SELECT k, SAFE_DIVIDE(v, k-1) AS ratio",
        vec![
            vec![Cell::Integer(1), Cell::Null],
            vec![Cell::Integer(1), Cell::Null],
            vec![Cell::Integer(2), Cell::Number(30.0_f64.to_bits())],
            vec![Cell::Null, Cell::Null],
        ],
    );
    for condition in [
        "v=SAFE_DIVIDE(1, 0)",
        "v>SAFE_DIVIDE(1, 0)",
        "NOT(v=SAFE_DIVIDE(1, 0))",
    ] {
        query(
            &db,
            &format!("FROM facts |> WHERE {condition} |> SELECT v"),
            vec![],
        );
    }
    failure(
        &db,
        "FROM facts |> SELECT SAFE_DIVIDE(9223372036854775807+1, 0) AS ratio",
        "addition",
        "SAFE_DIVIDE(9223372036854775807+1, 0)",
    );
    let baseline = db.reserved_memory_bytes();
    for expression in [
        "SAFE_DIVIDE()",
        "SAFE_DIVIDE(1)",
        "SAFE_DIVIDE(1, 2, 3)",
        "SAFE_DIVIDE(, 2)",
        "SAFE_DIVIDE(1, )",
        "SAFE_DIVIDE('x', 2)",
        "SAFE_DIVIDE(DATE '1970-01-01', 2)",
        "SAFE_DIVIDE(1, (2, 3))",
    ] {
        assert!(
            db.prepare(&format!("FROM facts |> SELECT {expression} AS ratio"))
                .is_err(),
            "{expression}"
        );
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    assert!(matches!(
        db.prepare("FROM dimensions |> WHERE label=SAFE_DIVIDE(1, 0)"),
        Err(Error::Bind { .. })
    ));
}

#[test]
fn public_safe_divide_composes_and_preserves_demand() {
    let (_directory, db) = join_fixture();
    query(
        &db,
        "FROM facts |> SELECT v AS SAFE_DIVIDE |> SELECT SAFE_DIVIDE |> ORDER BY SAFE_DIVIDE",
        integers(&[10, 20, 30, 40]),
    );
    for sql in [
        "FROM facts |> SELECT v, SAFE_DIVIDE(9223372036854775807+1, 0) AS unused |> SELECT v |> ORDER BY v",
        "FROM facts |> EXTEND SAFE_DIVIDE(9223372036854775807+1, 0) AS unused |> DROP unused |> SELECT v |> ORDER BY v",
    ] {
        query(&db, sql, integers(&[10, 20, 30, 40]));
    }
    query(
        &db,
        "FROM facts |> SELECT SAFE_DIVIDE(9223372036854775807+1, 0) AS unused |> LIMIT 0",
        vec![],
    );
    query(
        &db,
        "FROM facts |> SELECT k, SAFE_DIVIDE(v/(k-1), 1) AS ratio |> WHERE k=1 OR ratio>0 |> SELECT k |> ORDER BY k",
        integers(&[1, 1, 2]),
    );
    for sql in [
        "FROM facts |> SET v=SAFE_DIVIDE(v, k-1) |> SELECT v |> ORDER BY v NULLS FIRST",
        "FROM (FROM facts |> SELECT SAFE_DIVIDE(v, k-1) AS v) AS input |> SELECT v |> ORDER BY v NULLS FIRST",
    ] {
        query(
            &db,
            sql,
            vec![
                vec![Cell::Null],
                vec![Cell::Null],
                vec![Cell::Null],
                vec![Cell::Number(30.0_f64.to_bits())],
            ],
        );
    }
    query(
        &db,
        "FROM facts |> SELECT SAFE_DIVIDE(v, k-1) AS v |> UNION DISTINCT (FROM facts |> SELECT SAFE_DIVIDE(v, k-1) AS v) |> ORDER BY v NULLS FIRST",
        vec![vec![Cell::Null], vec![Cell::Number(30.0_f64.to_bits())]],
    );
    query(
        &db,
        "FROM facts |> SELECT SAFE_DIVIDE(v, k-1) AS ratio |> AGGREGATE SUM(ratio) AS s, COUNT(ratio) AS n",
        vec![vec![Cell::Number(30.0_f64.to_bits()), Cell::Integer(1)]],
    );
    query(
        &db,
        "FROM facts |> SELECT SAFE_DIVIDE(v, k-1) AS ratio |> EXTEND COUNT(*) OVER () AS n |> ORDER BY ratio NULLS FIRST |> SELECT ratio, n",
        vec![
            vec![Cell::Null, Cell::Integer(4)],
            vec![Cell::Null, Cell::Integer(4)],
            vec![Cell::Null, Cell::Integer(4)],
            vec![Cell::Number(30.0_f64.to_bits()), Cell::Integer(4)],
        ],
    );
}

#[test]
fn public_abs_preserves_numeric_types_values_and_argument_failures() {
    let (_directory, db) = join_fixture();
    for (expression, expected) in [
        ("ABS(-3)", Cell::Integer(3)),
        ("ABS(0)", Cell::Integer(0)),
        ("ABS(-9223372036854775807)", Cell::Integer(i64::MAX)),
        ("ABS(-3.5)", Cell::Number(3.5_f64.to_bits())),
        ("ABS(-0.0)", Cell::Number(0.0_f64.to_bits())),
        ("ABS(-ABS(-3))+2", Cell::Integer(5)),
        ("ABS(SAFE_DIVIDE(1, 0))", Cell::Null),
        ("SAFE_DIVIDE(ABS(-3), 2)", Cell::Number(1.5_f64.to_bits())),
    ] {
        query(
            &db,
            &format!("FROM facts |> LIMIT 1 |> SELECT {expression} AS magnitude"),
            vec![vec![expected]],
        );
    }
    query(
        &db,
        "FROM facts |> ORDER BY v |> SELECT ABS(k-2) AS distance",
        vec![
            vec![Cell::Integer(1)],
            vec![Cell::Integer(1)],
            vec![Cell::Integer(0)],
            vec![Cell::Null],
        ],
    );
    failure(
        &db,
        "FROM facts |> SELECT ABS(-9223372036854775808) AS magnitude",
        "absolute value",
        "ABS(-9223372036854775808)",
    );
    failure(
        &db,
        "FROM facts |> SELECT ABS(9223372036854775807+1) AS magnitude",
        "addition",
        "ABS(9223372036854775807+1)",
    );
    failure(
        &db,
        "FROM facts |> SELECT SAFE_DIVIDE(ABS(-9223372036854775808), 0) AS magnitude",
        "absolute value",
        "SAFE_DIVIDE(ABS(-9223372036854775808), 0)",
    );
    let baseline = db.reserved_memory_bytes();
    for expression in [
        "ABS()",
        "ABS(1, 2)",
        "ABS(, 1)",
        "ABS(1, )",
        "ABS('x')",
        "ABS(DATE '1970-01-01')",
        "ABS((1, 2))",
    ] {
        assert!(
            db.prepare(&format!("FROM facts |> SELECT {expression} AS magnitude"))
                .is_err(),
            "{expression}"
        );
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
}

#[test]
fn public_abs_composes_without_demanding_unused_failures() {
    let (_directory, db) = join_fixture();
    for sql in [
        "FROM facts |> SELECT v AS ABS |> SELECT ABS |> ORDER BY ABS",
        "FROM facts |> EXTEND ABS(-9223372036854775808) AS unused |> DROP unused |> SELECT v |> ORDER BY v",
        "FROM facts |> SELECT v, ABS(-9223372036854775808) AS unused |> SELECT v |> ORDER BY v",
    ] {
        query(&db, sql, integers(&[10, 20, 30, 40]));
    }
    query(
        &db,
        "FROM facts |> SELECT ABS(-9223372036854775808) AS unused |> LIMIT 0",
        vec![],
    );
    query(
        &db,
        "FROM facts |> EXTEND ABS(v/(k-1)) AS magnitude |> WHERE k=1 OR magnitude>0 |> SELECT k |> ORDER BY k",
        integers(&[1, 1, 2]),
    );
    query(
        &db,
        "FROM facts |> WHERE v>ABS(-15) |> SELECT v |> ORDER BY v |> LIMIT ABS(-2)",
        integers(&[20, 30]),
    );
    for sql in [
        "FROM facts |> SET v=ABS(v-25) |> SELECT v |> ORDER BY v",
        "FROM (FROM facts |> SELECT ABS(v-25) AS v) AS input |> SELECT v |> ORDER BY v",
    ] {
        query(&db, sql, integers(&[5, 5, 15, 15]));
    }
    query(
        &db,
        "FROM facts |> SELECT ABS(v-25) AS v |> UNION DISTINCT (FROM facts |> SELECT ABS(v-25) AS v) |> ORDER BY v",
        integers(&[5, 15]),
    );
    query(
        &db,
        "FROM facts |> AGGREGATE SUM(ABS(v-25)) AS total, COUNT(ABS(k-2)) AS present",
        vec![vec![Cell::Integer(40), Cell::Integer(3)]],
    );
    query(
        &db,
        "FROM facts |> SELECT ABS(v-25) AS deviation, COUNT(*) OVER () AS n |> ORDER BY deviation",
        vec![
            vec![Cell::Integer(5), Cell::Integer(4)],
            vec![Cell::Integer(5), Cell::Integer(4)],
            vec![Cell::Integer(15), Cell::Integer(4)],
            vec![Cell::Integer(15), Cell::Integer(4)],
        ],
    );
}

#[test]
fn public_mod_preserves_signed_results_nulls_and_binding_errors() {
    let (_directory, db) = join_fixture();
    for (expression, expected) in [
        ("MOD(5, 3)", 2),
        ("MOD(-5, 3)", -2),
        ("MOD(5, -3)", 2),
        ("MOD(-5, -3)", -2),
        ("MOD(-9223372036854775808, -1)", 0),
        ("MOD(-9223372036854775808, 3)", -2),
        ("MOD(9223372036854775807, 3)", 1),
        ("MOD(ABS(-17), MOD(9, 5))", 1),
        ("1+MOD(8, 3)*2", 5),
    ] {
        query(
            &db,
            &format!("FROM facts |> LIMIT 1 |> SELECT {expression} AS remainder"),
            integers(&[expected]),
        );
    }
    query(
        &db,
        "FROM facts |> WHERE k IS NULL |> SELECT MOD(k, 0) AS remainder",
        vec![vec![Cell::Null]],
    );
    query(
        &db,
        "FROM facts |> WHERE k IS NULL |> SELECT MOD(1, k) AS remainder",
        vec![vec![Cell::Null]],
    );
    let baseline = db.reserved_memory_bytes();
    for expression in [
        "MOD()",
        "MOD(1)",
        "MOD(1, 2, 3)",
        "MOD(, 2)",
        "MOD(1, )",
        "MOD((1, 2), 3)",
        "MOD('x', 2)",
        "MOD(DATE '1970-01-01', 2)",
    ] {
        assert!(
            db.prepare(&format!("FROM facts |> SELECT {expression} AS remainder"))
                .is_err(),
            "{expression}"
        );
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    for expression in [
        "MOD(1.0, 2)",
        "MOD(1, 2.0)",
        "MOD(1/2, 2)",
        "MOD(SAFE_DIVIDE(1, 0), 2)",
    ] {
        let sql = format!("FROM facts |> SELECT {expression} AS remainder");
        let Err(Error::Bind { span, .. }) = db.prepare(&sql) else {
            panic!("expected binding error: {sql}")
        };
        assert_eq!(&sql[span.start()..span.end()], expression);
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
}

#[test]
fn public_mod_composes_and_preserves_argument_demand() {
    let (_directory, db) = join_fixture();
    for sql in [
        "FROM facts |> SELECT v AS MOD |> SELECT MOD |> ORDER BY MOD",
        "FROM facts |> EXTEND MOD(v, 0) AS unused |> DROP unused |> SELECT v |> ORDER BY v",
        "FROM facts |> SELECT v, MOD(v, 0) AS unused |> SELECT v |> ORDER BY v",
    ] {
        query(&db, sql, integers(&[10, 20, 30, 40]));
    }
    query(
        &db,
        "FROM facts |> SELECT MOD(v, 0) AS unused |> LIMIT 0",
        vec![],
    );
    query(
        &db,
        "FROM facts |> EXTEND MOD(v, k-1) AS remainder |> WHERE k=1 OR remainder=0 |> SELECT k |> ORDER BY k",
        integers(&[1, 1, 2]),
    );
    query(
        &db,
        "FROM facts |> WHERE v>MOD(35, 20) |> SELECT v |> ORDER BY v |> LIMIT MOD(5, 3)",
        integers(&[20, 30]),
    );
    for sql in [
        "FROM facts |> SET v=MOD(v, 3) |> SELECT v |> ORDER BY v",
        "FROM (FROM facts |> SELECT MOD(v, 3) AS v) AS input |> SELECT v |> ORDER BY v",
    ] {
        query(&db, sql, integers(&[0, 1, 1, 2]));
    }
    query(
        &db,
        "FROM facts |> SELECT MOD(v, 3) AS v |> UNION DISTINCT (FROM facts |> SELECT MOD(v, 3) AS v) |> ORDER BY v",
        integers(&[0, 1, 2]),
    );
    query(
        &db,
        "FROM facts |> AGGREGATE SUM(MOD(v, 3)) AS total, COUNT(MOD(k, 2)) AS present",
        vec![vec![Cell::Integer(4), Cell::Integer(3)]],
    );
    query(
        &db,
        "FROM facts |> EXTEND MOD(v, 3) AS remainder |> AGGREGATE COUNT(*) AS n GROUP AND ORDER BY remainder",
        vec![
            vec![Cell::Integer(0), Cell::Integer(1)],
            vec![Cell::Integer(1), Cell::Integer(2)],
            vec![Cell::Integer(2), Cell::Integer(1)],
        ],
    );
    query(
        &db,
        "FROM facts |> SELECT MOD(v, 3) AS remainder, COUNT(*) OVER () AS n |> ORDER BY remainder",
        vec![
            vec![Cell::Integer(0), Cell::Integer(4)],
            vec![Cell::Integer(1), Cell::Integer(4)],
            vec![Cell::Integer(1), Cell::Integer(4)],
            vec![Cell::Integer(2), Cell::Integer(4)],
        ],
    );
    failure(
        &db,
        "FROM facts |> SELECT MOD(9223372036854775807+1, 0) AS remainder",
        "addition",
        "MOD(9223372036854775807+1, 0)",
    );
    let prepared = db
        .prepare("FROM facts |> SELECT MOD(k, 3) AS remainder")
        .unwrap();
    let output = prepared.result_column(0).unwrap();
    assert_eq!(output.data_type, DataType::Int64);
    assert!(output.nullable);
}

#[test]
fn public_div_preserves_exact_quotients_nulls_and_binding_errors() {
    let (_directory, db) = join_fixture();
    for (expression, expected) in [
        ("DIV(5, 3)", 1),
        ("DIV(-5, 3)", -1),
        ("DIV(5, -3)", -1),
        ("DIV(-5, -3)", 1),
        ("DIV(-2, 3)", 0),
        ("DIV(9007199254740995, 3)", 3002399751580331),
        ("DIV(-9223372036854775808, 1)", i64::MIN),
        ("DIV(9223372036854775807, 3)", 3074457345618258602),
        ("1+DIV(ABS(-17), MOD(9, 5))*2", 9),
    ] {
        query(
            &db,
            &format!("FROM facts |> LIMIT 1 |> SELECT {expression} AS quotient"),
            integers(&[expected]),
        );
    }
    for expression in ["DIV(k, 0)", "DIV(1, k)", "DIV(-9223372036854775808, k)"] {
        query(
            &db,
            &format!("FROM facts |> WHERE k IS NULL |> SELECT {expression} AS quotient"),
            vec![vec![Cell::Null]],
        );
    }
    let baseline = db.reserved_memory_bytes();
    for expression in [
        "DIV()",
        "DIV(1)",
        "DIV(1, 2, 3)",
        "DIV(, 2)",
        "DIV(1, )",
        "DIV((1, 2), 3)",
        "DIV('x', 2)",
        "DIV(DATE '1970-01-01', 2)",
    ] {
        assert!(
            db.prepare(&format!("FROM facts |> SELECT {expression} AS quotient"))
                .is_err(),
            "{expression}"
        );
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    for expression in [
        "DIV(1.0, 2)",
        "DIV(1, 2.0)",
        "DIV(1/2, 2)",
        "DIV(SAFE_DIVIDE(1, 0), 2)",
    ] {
        let sql = format!("FROM facts |> SELECT {expression} AS quotient");
        let Err(Error::Bind { span, .. }) = db.prepare(&sql) else {
            panic!("expected binding error: {sql}")
        };
        assert_eq!(&sql[span.start()..span.end()], expression);
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    for expression in [
        "DIV(-9223372036854775808, -1)",
        "SAFE_DIVIDE(DIV(-9223372036854775808, -1), 0)",
    ] {
        failure(
            &db,
            &format!("# 雪\nFROM facts |> SELECT {expression} AS quotient"),
            "division",
            expression,
        );
    }
    failure(
        &db,
        "FROM facts |> SELECT DIV(9223372036854775807+1, 0) AS quotient",
        "addition",
        "DIV(9223372036854775807+1, 0)",
    );
}

#[test]
fn public_div_composes_and_preserves_argument_demand() {
    let (_directory, db) = join_fixture();
    for sql in [
        "FROM facts |> SELECT v AS DIV |> SELECT DIV |> ORDER BY DIV",
        "FROM facts |> EXTEND DIV(v, 0) AS unused |> DROP unused |> SELECT v |> ORDER BY v",
        "FROM facts |> SELECT v, DIV(-9223372036854775808, -1) AS unused |> SELECT v |> ORDER BY v",
    ] {
        query(&db, sql, integers(&[10, 20, 30, 40]));
    }
    query(
        &db,
        "FROM facts |> SELECT DIV(v, 0) AS unused |> LIMIT 0",
        vec![],
    );
    query(
        &db,
        "FROM facts |> EXTEND DIV(v, k-1) AS quotient |> WHERE k=1 OR quotient=30 |> SELECT k |> ORDER BY k",
        integers(&[1, 1, 2]),
    );
    query(
        &db,
        "FROM facts |> WHERE v>DIV(35, 2) |> SELECT v |> ORDER BY v |> LIMIT DIV(5, 2)",
        integers(&[20, 30]),
    );
    for sql in [
        "FROM facts |> SET v=DIV(v, 15) |> SELECT v |> ORDER BY v",
        "FROM (FROM facts |> SELECT DIV(v, 15) AS v) AS input |> SELECT v |> ORDER BY v",
    ] {
        query(&db, sql, integers(&[0, 1, 2, 2]));
    }
    query(
        &db,
        "FROM facts |> SELECT DIV(v, 15) AS v |> UNION DISTINCT (FROM facts |> SELECT DIV(v, 15) AS v) |> ORDER BY v",
        integers(&[0, 1, 2]),
    );
    query(
        &db,
        "FROM facts |> AGGREGATE SUM(DIV(v, 15)) AS total, COUNT(DIV(k, 2)) AS present",
        vec![vec![Cell::Integer(5), Cell::Integer(3)]],
    );
    query(
        &db,
        "FROM facts |> EXTEND DIV(v, 15) AS quotient |> AGGREGATE COUNT(*) AS n GROUP AND ORDER BY quotient",
        vec![
            vec![Cell::Integer(0), Cell::Integer(1)],
            vec![Cell::Integer(1), Cell::Integer(1)],
            vec![Cell::Integer(2), Cell::Integer(2)],
        ],
    );
    query(
        &db,
        "FROM facts |> SELECT DIV(v, 15) AS quotient, COUNT(*) OVER () AS n |> ORDER BY quotient",
        vec![
            vec![Cell::Integer(0), Cell::Integer(4)],
            vec![Cell::Integer(1), Cell::Integer(4)],
            vec![Cell::Integer(2), Cell::Integer(4)],
            vec![Cell::Integer(2), Cell::Integer(4)],
        ],
    );
    let prepared = db
        .prepare("FROM facts |> SELECT DIV(k, 3) AS quotient")
        .unwrap();
    let output = prepared.result_column(0).unwrap();
    assert_eq!(output.data_type, DataType::Int64);
    assert!(output.nullable);
}

#[test]
fn public_coalesce_selects_defaults_and_skips_unused_dependencies() {
    let (_directory, db) = join_fixture();
    for (expression, expected) in [
        ("COALESCE(-9223372036854775808, 0)", i64::MIN),
        ("COALESCE(9223372036854775807, 0)", i64::MAX),
        ("COALESCE(9007199254740993, 0)", 9_007_199_254_740_993),
    ] {
        query(
            &db,
            &format!("FROM facts |> LIMIT 1 |> SELECT {expression}"),
            integers(&[expected]),
        );
    }
    let mut nested = "v".to_owned();
    for _ in 0..15 {
        nested = format!("COALESCE(k, {nested})");
    }
    query(
        &db,
        &format!("FROM facts |> SELECT -{nested} AS n |> ORDER BY n"),
        integers(&[-40, -2, -1, -1]),
    );
    assert!(
        db.prepare(&format!("FROM facts |> SELECT COALESCE(k, {nested})"))
            .is_err()
    );

    for (sql, expected) in [
        (
            "FROM facts |> SELECT COALESCE(k, 9) AS n |> ORDER BY n",
            integers(&[1, 1, 2, 9]),
        ),
        (
            "FROM facts |> SELECT COALESCE(v, 9223372036854775807+1) AS n |> ORDER BY n",
            integers(&[10, 20, 30, 40]),
        ),
        (
            "FROM facts |> EXTEND 9223372036854775807+1 AS bad |> SELECT COALESCE(v, bad) AS n |> ORDER BY n",
            integers(&[10, 20, 30, 40]),
        ),
        (
            "FROM facts |> EXTEND 9223372036854775807+1 AS bad |> EXTEND bad+1 AS worse |> SELECT COALESCE(v, worse) AS n |> ORDER BY n",
            integers(&[10, 20, 30, 40]),
        ),
        (
            "FROM facts |> AGGREGATE SUM(9223372036854775807) AS bad, COUNT(*) AS n |> SELECT COALESCE(n, bad)",
            integers(&[4]),
        ),
        (
            "FROM facts AS f |> LEFT JOIN dimensions AS d ON f.k=d.k |> SELECT COALESCE(d.k, 9) AS n |> ORDER BY n",
            integers(&[1, 1, 1, 1, 2, 9]),
        ),
        (
            "FROM facts |> AGGREGATE SUM(COALESCE(k, 9)) AS s",
            integers(&[13]),
        ),
        (
            "FROM facts |> WHERE v > 100 |> AGGREGATE SUM(v) AS s |> SELECT COALESCE(s, 9)",
            integers(&[9]),
        ),
        (
            "FROM facts |> EXTEND COALESCE(k, 9) AS n |> WHERE n > 2 |> SELECT v",
            integers(&[40]),
        ),
        (
            "FROM facts |> SELECT COALESCE(SAFE_DIVIDE(1, 0), 2) AS n |> ORDER BY n",
            vec![vec![Cell::Number(2.0_f64.to_bits())]; 4],
        ),
    ] {
        query(&db, sql, expected);
    }
    let prepared = db
        .prepare("FROM facts |> SELECT COALESCE(k, v), COALESCE(k, k), COALESCE(k, 0.5)")
        .unwrap();
    assert!(!prepared.result_column(0).unwrap().nullable);
    assert!(prepared.result_column(1).unwrap().nullable);
    assert!(!prepared.result_column(2).unwrap().nullable);
    assert_eq!(
        prepared.result_column(0).unwrap().data_type,
        DataType::Int64
    );
    assert_eq!(
        prepared.result_column(2).unwrap().data_type,
        DataType::Double
    );
    drop(prepared);
    for (sql, operation, expression) in [
        (
            "FROM facts |> SELECT COALESCE(k, 9223372036854775807+1)",
            "addition",
            "COALESCE(k, 9223372036854775807+1)",
        ),
        (
            "FROM facts |> EXTEND 9223372036854775807+1 AS bad |> SELECT COALESCE(k, bad)",
            "addition",
            "9223372036854775807+1",
        ),
        (
            "FROM facts |> SELECT COALESCE(9223372036854775807+1, v)",
            "addition",
            "COALESCE(9223372036854775807+1, v)",
        ),
        (
            "FROM facts |> AGGREGATE SUM(9223372036854775807) AS bad |> SELECT COALESCE(bad, 0)",
            "SUM",
            "SUM(9223372036854775807)",
        ),
    ] {
        failure(&db, sql, operation, expression);
    }
    query(&db, "FROM facts |> AGGREGATE COUNT(*) AS n", integers(&[4]));
}

#[test]
fn public_sign_preserves_types_classification_and_demand() {
    let (_directory, db) = join_fixture();
    for (expression, expected) in [
        ("SIGN(-9223372036854775808)", Cell::Integer(-1)),
        ("SIGN(9223372036854775807)", Cell::Integer(1)),
        ("SIGN(-9007199254740993)", Cell::Integer(-1)),
        ("SIGN(0)", Cell::Integer(0)),
        ("SIGN(-0.0)", Cell::Number(0.0_f64.to_bits())),
        ("SIGN(-0.25)", Cell::Number((-1.0_f64).to_bits())),
        ("SIGN(0.25)", Cell::Number(1.0_f64.to_bits())),
        ("SIGN(SAFE_DIVIDE(1, 0))", Cell::Null),
        (
            "SIGN(COALESCE(1, ABS(-9223372036854775808)))",
            Cell::Integer(1),
        ),
        (
            "COALESCE(1, SIGN(ABS(-9223372036854775808)))",
            Cell::Integer(1),
        ),
        ("SIGN(-SIGN(-3))+2", Cell::Integer(3)),
    ] {
        query(
            &db,
            &format!("FROM facts |> LIMIT SIGN(1) |> SELECT {expression} AS direction"),
            vec![vec![expected]],
        );
    }
    query(
        &db,
        "FROM facts |> EXTEND SIGN(v-25) AS direction |> AGGREGATE SUM(v) AS total, COUNT(*) AS n GROUP AND ORDER BY direction",
        vec![
            vec![Cell::Integer(-1), Cell::Integer(30), Cell::Integer(2)],
            vec![Cell::Integer(1), Cell::Integer(70), Cell::Integer(2)],
        ],
    );
    query(
        &db,
        "FROM facts |> SELECT SIGN(k-1) AS direction |> ORDER BY direction",
        vec![
            vec![Cell::Null],
            vec![Cell::Integer(0)],
            vec![Cell::Integer(0)],
            vec![Cell::Integer(1)],
        ],
    );
    failure(
        &db,
        "FROM facts |> SELECT SIGN(ABS(-9223372036854775808)) AS direction",
        "absolute value",
        "SIGN(ABS(-9223372036854775808))",
    );
    query(
        &db,
        "FROM facts |> EXTEND SIGN(v/(k-1)) AS direction |> WHERE k=1 OR direction>0 |> SELECT k |> ORDER BY k",
        integers(&[1, 1, 2]),
    );
    query(
        &db,
        "FROM facts |> EXTEND SIGN(v*9223372036854775807) AS unused |> DROP unused |> SELECT v |> ORDER BY v",
        integers(&[10, 20, 30, 40]),
    );
    failure(
        &db,
        "FROM facts |> SELECT SIGN(v*9223372036854775807) AS direction",
        "multiplication",
        "SIGN(v*9223372036854775807)",
    );
    let baseline = db.reserved_memory_bytes();
    for expression in [
        "SIGN()",
        "SIGN(1, 2)",
        "SIGN(, 1)",
        "SIGN(1, )",
        "SIGN('x')",
        "SIGN(DATE '1970-01-01')",
        "SIGN(NULL)",
        "SIGN(missing)",
    ] {
        assert!(
            db.prepare(&format!("FROM facts |> SELECT {expression} AS direction"))
                .is_err(),
            "{expression}"
        );
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
}

#[test]
fn public_numeric_preserves_stored_double_bits_across_producers_and_reopen() {
    let directory = Directory::new();
    let path = directory.database();
    let mut db = Database::create_empty(&path, config()).unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "samples",
        &[
            ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "value",
                data_type: DataType::Double,
                nullable: true,
            },
        ],
        &cancel,
    )
    .unwrap();
    let mut append = db.begin_append("samples", limits(), &cancel).unwrap();
    append
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&[0, 1, 2, 3, 4, 5, 6, 7, 8]),
                    validity: &[255, 1],
                },
                ColumnInput {
                    values: ColumnValues::Double(&[
                        -0.0,
                        0.0,
                        -f64::from_bits(1),
                        f64::from_bits(1),
                        f64::NEG_INFINITY,
                        f64::INFINITY,
                        f64::from_bits(0xfff8_0000_0000_0042),
                        -7.0,
                        999.0,
                    ]),
                    validity: &[255, 0],
                },
            ],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    for reopened in [false, true] {
        if reopened {
            db.close().unwrap();
            db = Database::open(&path, config()).unwrap();
        }
        for source in [
            "FROM samples",
            "FROM (FROM samples)",
            "FROM samples |> ORDER BY id DESC",
            "FROM samples |> UNION ALL (FROM samples |> LIMIT 0)",
        ] {
            query(
                &db,
                &format!(
                    "{source} |> WHERE id IN (0, 1, 3, 5, 6, 8) |> ORDER BY id |> SELECT SQRT(value) AS magnitude"
                ),
                vec![
                    vec![Cell::Number(0x8000_0000_0000_0000)],
                    vec![Cell::Number(0)],
                    vec![Cell::Number(0x1e60_0000_0000_0000)],
                    vec![Cell::Number(0x7ff0_0000_0000_0000)],
                    vec![Cell::Number(0xfff8_0000_0000_0042)],
                    vec![Cell::Null],
                ],
            );
            for function in ["LN", "LOG10"] {
                query(
                    &db,
                    &format!(
                        "{source} |> WHERE id IN (4, 5, 6, 8) |> ORDER BY id |> SELECT {function}(value) AS logarithm"
                    ),
                    vec![
                        vec![Cell::Number(0x7ff8_0000_0000_0000)],
                        vec![Cell::Number(0x7ff0_0000_0000_0000)],
                        vec![Cell::Number(0xfff8_0000_0000_0042)],
                        vec![Cell::Null],
                    ],
                );
            }
            query(
                &db,
                &format!(
                    "{source} |> WHERE id IN (0, 1, 4, 5, 6, 8) |> ORDER BY id |> SELECT EXP(value) AS growth"
                ),
                vec![
                    vec![Cell::Number(0x3ff0_0000_0000_0000)],
                    vec![Cell::Number(0x3ff0_0000_0000_0000)],
                    vec![Cell::Number(0)],
                    vec![Cell::Number(0x7ff0_0000_0000_0000)],
                    vec![Cell::Number(0xfff8_0000_0000_0042)],
                    vec![Cell::Null],
                ],
            );
            for (expression, expected) in [
                (
                    "POW(value, 1)",
                    vec![
                        0x8000_0000_0000_0000,
                        0,
                        0x8000_0000_0000_0001,
                        1,
                        0xfff0_0000_0000_0000,
                        0x7ff0_0000_0000_0000,
                        0xfff8_0000_0000_0042,
                        (-7.0_f64).to_bits(),
                    ],
                ),
                ("POWER(value, 0)", vec![1.0_f64.to_bits(); 8]),
                ("POW(1, value)", vec![1.0_f64.to_bits(); 8]),
                (
                    "COALESCE(POWER(value, 1), 9)",
                    vec![
                        0x8000_0000_0000_0000,
                        0,
                        0x8000_0000_0000_0001,
                        1,
                        0xfff0_0000_0000_0000,
                        0x7ff0_0000_0000_0000,
                        0xfff8_0000_0000_0042,
                        (-7.0_f64).to_bits(),
                    ],
                ),
            ] {
                let mut rows: Vec<_> = expected
                    .into_iter()
                    .map(|bits| vec![Cell::Number(bits)])
                    .collect();
                rows.push(vec![if expression.starts_with("COALESCE") {
                    Cell::Number(9.0_f64.to_bits())
                } else {
                    Cell::Null
                }]);
                query(
                    &db,
                    &format!("{source} |> ORDER BY id |> SELECT {expression} AS powered"),
                    rows,
                );
            }
            // Explicit answers distinguish SIGN's positive zero from rounding's
            // preserved zero sign and exercise subnormal values without an oracle
            // that calls the implementation's rounding primitive.
            for (function, expected) in [
                (
                    "SIGN",
                    [
                        0.0_f64,
                        0.0,
                        -1.0,
                        1.0,
                        -1.0,
                        1.0,
                        f64::from_bits(0xfff8_0000_0000_0042),
                        -1.0,
                    ],
                ),
                (
                    "FLOOR",
                    [
                        -0.0,
                        0.0,
                        -1.0,
                        0.0,
                        f64::NEG_INFINITY,
                        f64::INFINITY,
                        f64::from_bits(0xfff8_0000_0000_0042),
                        -7.0,
                    ],
                ),
                (
                    "CEIL",
                    [
                        -0.0,
                        0.0,
                        -0.0,
                        1.0,
                        f64::NEG_INFINITY,
                        f64::INFINITY,
                        f64::from_bits(0xfff8_0000_0000_0042),
                        -7.0,
                    ],
                ),
                (
                    "ROUND",
                    [
                        -0.0,
                        0.0,
                        -0.0,
                        0.0,
                        f64::NEG_INFINITY,
                        f64::INFINITY,
                        f64::from_bits(0xfff8_0000_0000_0042),
                        -7.0,
                    ],
                ),
                (
                    "CEILING",
                    [
                        -0.0,
                        0.0,
                        -0.0,
                        1.0,
                        f64::NEG_INFINITY,
                        f64::INFINITY,
                        f64::from_bits(0xfff8_0000_0000_0042),
                        -7.0,
                    ],
                ),
            ] {
                let mut rows: Vec<_> = expected
                    .map(|value| vec![Cell::Number(value.to_bits())])
                    .into();
                rows.push(vec![Cell::Null]);
                query(
                    &db,
                    &format!("{source} |> ORDER BY id |> SELECT {function}(value) AS result"),
                    rows,
                );
            }
        }
    }
    db.close().unwrap();
}

#[test]
fn public_integral_rounding_preserves_promotion_and_demand() {
    let (_directory, db) = join_fixture();
    for function in ["FLOOR", "CEIL", "CEILING", "ROUND"] {
        for (argument, expected) in [
            ("-9223372036854775808", -9_223_372_036_854_775_808.0_f64),
            ("9223372036854775807", 9_223_372_036_854_775_808.0),
            ("9007199254740993", 9_007_199_254_740_992.0),
            ("9007199254740995", 9_007_199_254_740_996.0),
            ("-0.0", -0.0),
        ] {
            query(
                &db,
                &format!("FROM facts |> LIMIT 1 |> SELECT {function}({argument}) AS rounded"),
                vec![vec![Cell::Number(expected.to_bits())]],
            );
        }
        query(
            &db,
            &format!("FROM facts |> LIMIT 1 |> SELECT {function}(SAFE_DIVIDE(1, 0)) AS missing"),
            vec![vec![Cell::Null]],
        );
        query(
            &db,
            &format!(
                "FROM facts |> SELECT COALESCE(1, {function}(v*9223372036854775807)) AS chosen |> LIMIT 1"
            ),
            vec![vec![Cell::Number(1.0_f64.to_bits())]],
        );
        query(
            &db,
            &format!(
                "FROM facts |> EXTEND {function}(v/(k-1)) AS rounded |> WHERE k=1 OR rounded>0 |> SELECT k |> ORDER BY k"
            ),
            integers(&[1, 1, 2]),
        );
        query(
            &db,
            &format!(
                "FROM facts |> EXTEND {function}(v*9223372036854775807) AS unused |> DROP unused |> SELECT v |> ORDER BY v"
            ),
            integers(&[10, 20, 30, 40]),
        );
        failure(
            &db,
            &format!("FROM facts |> SELECT {function}(v*9223372036854775807) AS rounded"),
            "multiplication",
            &format!("{function}(v*9223372036854775807)"),
        );
        for argument in [
            "",
            "1, 2",
            ", 1",
            "1, ",
            "'text'",
            "DATE '1970-01-01'",
            "NULL",
            "missing",
        ] {
            let baseline = db.reserved_memory_bytes();
            assert!(
                db.prepare(&format!(
                    "FROM facts |> SELECT {function}({argument}) AS rounded"
                ))
                .is_err()
            );
            assert_eq!(db.reserved_memory_bytes(), baseline);
        }
        assert!(
            db.prepare(&format!("FROM facts |> LIMIT {function}(1)"))
                .is_err()
        );
    }
    for (expression, expected) in [
        ("FLOOR(-2.75)", -3.0_f64),
        ("CEIL(-2.75)", -2.0),
        ("FLOOR(2.75)", 2.0),
        ("CEILING(2.75)", 3.0),
        ("CEIL(-0.25)", -0.0),
        ("FLOOR(0.25)", 0.0),
        ("FLOOR(CEIL(2.25)/2)", 1.0),
        ("ROUND(2.5)", 3.0),
        ("ROUND(-2.5)", -3.0),
        ("ROUND(0.49999999999999994)", 0.0),
        ("ROUND(-0.49999999999999994)", -0.0),
        ("ROUND(0.5)", 1.0),
        ("ROUND(-0.5)", -1.0),
        ("ROUND(CEIL(2.25)/2)", 2.0),
    ] {
        query(
            &db,
            &format!("FROM facts |> LIMIT 1 |> SELECT {expression} AS rounded"),
            vec![vec![Cell::Number(expected.to_bits())]],
        );
    }
    query(
        &db,
        "FROM facts |> EXTEND FLOOR(v/15) AS bucket |> AGGREGATE SUM(v) AS total, COUNT(*) AS n GROUP AND ORDER BY bucket",
        vec![
            vec![
                Cell::Number(0.0_f64.to_bits()),
                Cell::Integer(10),
                Cell::Integer(1),
            ],
            vec![
                Cell::Number(1.0_f64.to_bits()),
                Cell::Integer(20),
                Cell::Integer(1),
            ],
            vec![
                Cell::Number(2.0_f64.to_bits()),
                Cell::Integer(70),
                Cell::Integer(2),
            ],
        ],
    );
    query(
        &db,
        "FROM facts |> EXTEND ROUND(v/15) AS bucket |> AGGREGATE SUM(v) AS total, COUNT(*) AS n GROUP AND ORDER BY bucket",
        vec![
            vec![
                Cell::Number(1.0_f64.to_bits()),
                Cell::Integer(30),
                Cell::Integer(2),
            ],
            vec![
                Cell::Number(2.0_f64.to_bits()),
                Cell::Integer(30),
                Cell::Integer(1),
            ],
            vec![
                Cell::Number(3.0_f64.to_bits()),
                Cell::Integer(40),
                Cell::Integer(1),
            ],
        ],
    );
    for expression in [
        "ROUND(1, 0)",
        "ROUND(1, 0, 'ROUND_HALF_EVEN')",
        "MOD(ROUND(1), 1)",
    ] {
        assert!(
            db.prepare(&format!("FROM facts |> SELECT {expression}"))
                .is_err()
        );
    }
    query(
        &db,
        "FROM facts |> ORDER BY v |> SELECT NULLIF(FLOOR(v/15), 1) AS bucket",
        vec![
            vec![Cell::Number(0.0_f64.to_bits())],
            vec![Cell::Null],
            vec![Cell::Number(2.0_f64.to_bits())],
            vec![Cell::Number(2.0_f64.to_bits())],
        ],
    );
}

#[test]
fn public_sqrt_preserves_values_demand_and_domain_spans() {
    let (_directory, db) = join_fixture();
    for (expression, expected) in [
        ("SQRT(4)", Some(2.0_f64)),
        // Decimal square roots of the independently rounded INT64 inputs.
        (
            "SQRT(9007199254740993)",
            Some(f64::from_bits(0x4196_a09e_667f_3bcd)),
        ),
        (
            "SQRT(9007199254740995)",
            Some(f64::from_bits(0x4196_a09e_667f_3bce)),
        ),
        (
            "SQRT(9223372036854775807)",
            Some(f64::from_bits(0x41e6_a09e_667f_3bcd)),
        ),
        ("SQRT(2.0)", Some(f64::from_bits(0x3ff6_a09e_667f_3bcd))),
        ("SQRT(-0.0)", Some(-0.0)),
        ("SQRT(SAFE_DIVIDE(1, 0))", None),
        ("COALESCE(9, SQRT(-1))", Some(9.0)),
    ] {
        query(
            &db,
            &format!("FROM facts |> LIMIT 1 |> SELECT {expression} AS magnitude"),
            vec![vec![
                expected.map_or(Cell::Null, |n| Cell::Number(n.to_bits())),
            ]],
        );
    }
    query(
        &db,
        "FROM facts |> EXTEND SQRT(-v) AS unused |> DROP unused |> SELECT v |> ORDER BY v",
        integers(&[10, 20, 30, 40]),
    );
    query(
        &db,
        "FROM facts |> SELECT SQRT(v*v) AS magnitude |> ORDER BY magnitude",
        [10.0_f64, 20.0, 30.0, 40.0]
            .into_iter()
            .map(|n| vec![Cell::Number(n.to_bits())])
            .collect(),
    );
    query(
        &db,
        "FROM facts |> AGGREGATE AVG(v*v) AS mean_square |> SELECT SQRT(mean_square) AS rms",
        vec![vec![Cell::Number(0x403b_62d9_46c4_4ef3)]],
    );
    for argument in [
        "",
        "1, 2",
        ", 1",
        "1, ",
        "'text'",
        "DATE '1970-01-01'",
        "NULL",
        "missing",
    ] {
        let baseline = db.reserved_memory_bytes();
        assert!(
            db.prepare(&format!(
                "FROM facts |> SELECT SQRT({argument}) AS magnitude"
            ))
            .is_err()
        );
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    for sql in [
        "FROM facts |> LIMIT SQRT(4)",
        "FROM facts |> SELECT DIV(SQRT(4), 1) AS bad",
        "FROM facts |> SELECT MOD(1, SQRT(4)) AS bad",
    ] {
        let baseline = db.reserved_memory_bytes();
        assert!(db.prepare(sql).is_err());
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    query(
        &db,
        "FROM facts |> EXTEND SQRT(-v) AS bad |> WHERE v>0 OR bad>0 |> SELECT v |> ORDER BY v",
        integers(&[10, 20, 30, 40]),
    );
    let baseline = db.reserved_memory_bytes();
    let sql = "FROM facts |> WHERE v>SQRT(-2)";
    let error = db
        .prepare(sql)
        .err()
        .expect("constant predicate domain error");
    let Error::ArithmeticDomain { operation, span } = error else {
        panic!("expected domain error: {error}");
    };
    assert_eq!(operation, "square root");
    assert_eq!(&sql[span.start()..span.end()], "SQRT(-2)");
    assert_eq!(db.reserved_memory_bytes(), baseline);
    for expression in [
        "SQRT(-v)",
        "SQRT(-9223372036854775808)",
        "SAFE_DIVIDE(SQRT(-v), 0)",
        "COALESCE(SAFE_DIVIDE(1, 0), SQRT(-v))",
    ] {
        let baseline = db.reserved_memory_bytes();
        let sql = format!("FROM facts |> SELECT {expression} AS magnitude");
        let start = sql.find(expression).unwrap();
        let end = start + expression.len();
        let prepared = db.prepare(&sql).unwrap();
        drop(sql);
        let cancel = CancellationToken::new();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        let mut failed = false;
        for _ in 0..100_000 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Failed(Error::ArithmeticDomain { operation, span }) => {
                    assert_eq!(*operation, "square root");
                    assert_eq!((span.start(), span.end()), (start, end));
                    failed = true;
                    break;
                }
                QueryStep::Failed(error) => panic!("expected square-root domain failure: {error}"),
                QueryStep::Rows(_) | QueryStep::Finished => {
                    panic!("missing square-root domain failure")
                }
            }
        }
        assert!(failed);
        assert!(matches!(
            result.step(),
            QueryStep::Failed(Error::ArithmeticDomain { .. })
        ));
        drop(result);
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}

#[test]
fn public_ln_preserves_promotion_composition_and_demand() {
    check_public_logarithm(
        "LN",
        &[
            ("2", 0x3fe6_2e42_fefa_39ef_u64),
            ("2.0", 0x3fe6_2e42_fefa_39ef),
            ("9007199254740993", 0x4042_5e4f_7b27_37fa),
            ("9223372036854775807", 0x4045_d589_f2fe_5107),
        ],
    );
}

#[test]
fn public_log10_preserves_promotion_composition_and_demand() {
    check_public_logarithm(
        "LOG10",
        &[
            ("2", 0x3fd3_4413_509f_79ff),
            ("2.0", 0x3fd3_4413_509f_79ff),
            ("9007199254740993", 0x402f_e8bf_fd88_220e),
            ("9223372036854775807", 0x4032_f703_035c_fc17),
        ],
    );
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    // 10*Decimal(v/10).log10() at precision 100, then rounded to binary64.
    let prepared = db
        .prepare("FROM facts |> ORDER BY v |> SELECT 10*LOG10(v/10) AS relative_db")
        .unwrap();
    let cancel = CancellationToken::new();
    let mut result = db.execute(&prepared, &cancel).unwrap();
    let rows = collect_unordered(&mut result);
    assert_eq!(rows.len(), 4);
    for (row, expected) in rows.iter().zip([
        0_u64,
        0x4008_1518_24c7_587f,
        0x4013_15b8_bdf1_e5be,
        0x4018_1518_24c7_587f,
    ]) {
        let [Cell::Number(actual)] = row.as_slice() else {
            panic!("DOUBLE power ratio");
        };
        if expected == 0 {
            assert_eq!(*actual, 0);
        } else {
            assert!(actual.abs_diff(expected) <= 3, "{actual:016x}");
        }
    }
    drop(result);
    drop(prepared);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
}

fn check_public_logarithm(function: &str, references: &[(&str, u64)]) {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    // Independent 100-digit Decimal logarithms of the converted input values.
    for &(argument, expected) in references {
        let prepared = db
            .prepare(&format!(
                "FROM facts |> LIMIT 1 |> SELECT {function}({argument}) AS logarithm"
            ))
            .unwrap();
        let column = prepared.result_column(0).unwrap();
        assert_eq!(
            (column.data_type, column.nullable),
            (DataType::Double, false)
        );
        let cancel = CancellationToken::new();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        let rows = collect_unordered(&mut result);
        let [row] = rows.as_slice() else {
            panic!("one logarithm row")
        };
        let [Cell::Number(actual)] = row.as_slice() else {
            panic!("DOUBLE logarithm")
        };
        assert!(
            actual.abs_diff(expected) <= 2,
            "{function}({argument}): {actual:016x}"
        );
        drop(result);
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    for (expression, expected) in [
        (format!("{function}(1)"), Cell::Number(0)),
        (format!("{function}(SAFE_DIVIDE(1, 0))"), Cell::Null),
        (
            format!("COALESCE(9, {function}(0))"),
            Cell::Number(9.0_f64.to_bits()),
        ),
        (
            format!("COALESCE({function}(NULLIF(1, 1)), 9)"),
            Cell::Number(9.0_f64.to_bits()),
        ),
    ] {
        query(
            &db,
            &format!("FROM facts |> LIMIT 1 |> SELECT {expression} AS logarithm"),
            vec![vec![expected]],
        );
    }
    for sql in [
        format!(
            "FROM facts |> EXTEND {function}(-v) AS unused |> DROP unused |> SELECT v |> ORDER BY v"
        ),
        format!(
            "FROM facts |> EXTEND {function}(-v) AS bad |> WHERE v>0 OR bad>0 |> SELECT v |> ORDER BY v"
        ),
        format!(
            "FROM facts |> EXTEND {function}(v) AS logarithm |> WHERE logarithm>{function}(1) |> SELECT v |> ORDER BY v"
        ),
    ] {
        query(&db, &sql, integers(&[10, 20, 30, 40]));
    }
    query(
        &db,
        &format!("FROM facts |> EXTEND {function}(-v) AS bad |> WHERE v<0 AND bad>0 |> SELECT v"),
        vec![],
    );
    query(
        &db,
        &format!(
            "FROM facts |> EXTEND SIGN({function}(v/20)) AS scale |> AGGREGATE SUM(v) AS total, COUNT(*) AS n GROUP AND ORDER BY scale"
        ),
        vec![
            vec![
                Cell::Number((-1.0_f64).to_bits()),
                Cell::Integer(10),
                Cell::Integer(1),
            ],
            vec![Cell::Number(0), Cell::Integer(20), Cell::Integer(1)],
            vec![
                Cell::Number(1.0_f64.to_bits()),
                Cell::Integer(70),
                Cell::Integer(2),
            ],
        ],
    );
    query(
        &db,
        &format!(
            "FROM facts AS l |> LEFT JOIN facts AS r ON l.k=r.k |> SELECT SIGN({function}(r.v)) AS scale |> DISTINCT |> ORDER BY scale"
        ),
        vec![vec![Cell::Null], vec![Cell::Number(1.0_f64.to_bits())]],
    );
    query(
        &db,
        &format!(
            "FROM facts |> SELECT {function}(v/v) AS zero |> UNION DISTINCT (FROM facts |> SELECT {function}(1) AS zero)"
        ),
        vec![vec![Cell::Number(0)]],
    );
    query(
        &db,
        &format!("FROM facts |> LIMIT 0 |> AGGREGATE AVG({function}(v)) AS mean"),
        vec![vec![Cell::Null]],
    );
    failure(
        &db,
        &format!("FROM facts |> SELECT {function}(v*9223372036854775807) AS bad"),
        "multiplication",
        &format!("{function}(v*9223372036854775807)"),
    );
    for expression in [
        format!("{function}()"),
        format!("{function}(1, 2)"),
        format!("{function}(, 1)"),
        format!("{function}(1, )"),
        format!("{function}('x')"),
        format!("{function}(DATE '1970-01-01')"),
        format!("{function}(NULL)"),
        format!("{function}(missing)"),
        format!("DIV({function}(1), 1)"),
        format!("MOD(1, {function}(1))"),
        "LOG(1)".to_owned(),
        "LOG2(1)".to_owned(),
        "LOG1P(1)".to_owned(),
    ] {
        assert!(
            db.prepare(&format!("FROM facts |> SELECT {expression}"))
                .is_err(),
            "{expression}"
        );
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    assert!(
        db.prepare(&format!("FROM facts |> LIMIT {function}(1)"))
            .is_err()
    );
    assert_eq!(db.reserved_memory_bytes(), baseline);
}

#[test]
fn public_logarithms_own_demanded_domain_error_spans() {
    for (function, expected_operation) in
        [("LN", "natural logarithm"), ("LOG10", "base-ten logarithm")]
    {
        let (_directory, db) = join_fixture();
        let baseline = db.reserved_memory_bytes();
        let sql = format!("FROM facts |> WHERE v>{function}(0)");
        let error = db
            .prepare(&sql)
            .err()
            .expect("constant predicate domain error");
        let Error::ArithmeticDomain { operation, span } = error else {
            panic!("domain error")
        };
        assert_eq!(operation, expected_operation);
        assert_eq!(&sql[span.start()..span.end()], format!("{function}(0)"));
        assert_eq!(db.reserved_memory_bytes(), baseline);
        for expression in [
            format!("{function}(0)"),
            format!("{function}(-0.0)"),
            format!("{function}(-v)"),
            format!("SUM({function}(-v))"),
            format!("{function}(-9223372036854775808)"),
            format!("SAFE_DIVIDE({function}(-v), 0)"),
            format!("SAFE_DIVIDE(1, {function}(-v))"),
            format!("COALESCE(SAFE_DIVIDE(1, 0), {function}(-v))"),
        ] {
            let stage = if expression.starts_with("SUM(") {
                "AGGREGATE"
            } else {
                "SELECT"
            };
            let sql = format!("# 雪\nFROM facts |> {stage} {expression} AS logarithm");
            let start = sql.find(&expression).unwrap();
            let end = start + expression.len();
            let prepared = db.prepare(&sql).unwrap();
            drop(sql);
            let cancel = CancellationToken::new();
            let mut result = db.execute(&prepared, &cancel).unwrap();
            let mut failed = false;
            for _ in 0..100_000 {
                match result.step() {
                    QueryStep::Progress => (),
                    QueryStep::Failed(Error::ArithmeticDomain { operation, span }) => {
                        assert_eq!(*operation, expected_operation);
                        assert_eq!((span.start(), span.end()), (start, end));
                        failed = true;
                        break;
                    }
                    _ => panic!("missing {function} domain failure"),
                }
            }
            assert!(failed);
            assert!(matches!(
                result.step(),
                QueryStep::Failed(Error::ArithmeticDomain { .. })
            ));
            let error = result.into_error().unwrap();
            drop(prepared);
            let Error::ArithmeticDomain { operation, span } = error else {
                panic!("owned domain error")
            };
            assert_eq!(operation, expected_operation);
            assert_eq!((span.start(), span.end()), (start, end));
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
    }
}

#[test]
fn public_exp_preserves_promotion_composition_and_demand() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    // Decimal exp at precision 100 supplies the scalar answers. The geometric
    // mean uses two independent Decimal square roots of 10*20*30*40, avoiding
    // the implementation's logarithm/exponential composition as an oracle.
    for (sql, nullable, expected, tolerance) in [
        (
            "FROM facts |> LIMIT 1 |> SELECT EXP(1) AS growth",
            false,
            0x4005_bf0a_8b14_5769,
            2,
        ),
        (
            "FROM facts |> LIMIT 1 |> SELECT EXP(1.0) AS growth",
            false,
            0x4005_bf0a_8b14_5769,
            2,
        ),
        (
            "FROM facts |> LIMIT 1 |> SELECT EXP(-1) AS growth",
            false,
            0x3fd7_8b56_362c_ef38,
            2,
        ),
        (
            "FROM facts |> LIMIT 1 |> SELECT EXP(-1.0) AS growth",
            false,
            0x3fd7_8b56_362c_ef38,
            2,
        ),
        (
            "FROM facts |> AGGREGATE AVG(LN(v)) AS mean_log |> SELECT EXP(mean_log) AS geometric_mean",
            true,
            0x4036_2236_2033_bf62,
            8,
        ),
    ] {
        let prepared = db.prepare(sql).unwrap();
        let column = prepared.result_column(0).unwrap();
        assert_eq!(
            (column.data_type, column.nullable),
            (DataType::Double, nullable)
        );
        let cancel = CancellationToken::new();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        let rows = collect_unordered(&mut result);
        let [row] = rows.as_slice() else {
            panic!("one exponential row")
        };
        let [Cell::Number(actual)] = row.as_slice() else {
            panic!("DOUBLE exponential")
        };
        assert!(
            actual.abs_diff(expected) <= tolerance,
            "{sql}: {actual:016x}"
        );
        drop(result);
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    for (expression, expected) in [
        ("EXP(0)", Cell::Number(1.0_f64.to_bits())),
        ("EXP(-0.0)", Cell::Number(1.0_f64.to_bits())),
        ("EXP(-1000)", Cell::Number(0)),
        ("EXP(-9223372036854775808)", Cell::Number(0)),
        ("EXP(SAFE_DIVIDE(1, 0))", Cell::Null),
        ("COALESCE(9, EXP(1000))", Cell::Number(9.0_f64.to_bits())),
        (
            "COALESCE(EXP(NULLIF(1, 1)), 9)",
            Cell::Number(9.0_f64.to_bits()),
        ),
    ] {
        query(
            &db,
            &format!("FROM facts |> LIMIT 1 |> SELECT {expression} AS growth"),
            vec![vec![expected]],
        );
    }
    for sql in [
        "FROM facts |> EXTEND EXP(v*1000) AS unused |> DROP unused |> SELECT v |> ORDER BY v",
        "FROM facts |> EXTEND EXP(v*1000) AS bad |> WHERE v>0 OR bad>0 |> SELECT v |> ORDER BY v",
        "FROM facts |> EXTEND EXP(v) AS growth |> WHERE growth>EXP(0) |> SELECT v |> ORDER BY v",
    ] {
        query(&db, sql, integers(&[10, 20, 30, 40]));
    }
    query(
        &db,
        "FROM facts |> EXTEND EXP(v*1000) AS bad |> WHERE v<0 AND bad>0 |> SELECT v",
        vec![],
    );
    query(
        &db,
        "FROM facts |> EXTEND SIGN(EXP(v/20)-3) AS scale |> AGGREGATE SUM(v) AS total, COUNT(*) AS n GROUP AND ORDER BY scale",
        vec![
            vec![
                Cell::Number((-1.0_f64).to_bits()),
                Cell::Integer(30),
                Cell::Integer(2),
            ],
            vec![
                Cell::Number(1.0_f64.to_bits()),
                Cell::Integer(70),
                Cell::Integer(2),
            ],
        ],
    );
    query(
        &db,
        "FROM facts AS l |> LEFT JOIN facts AS r ON l.k=r.k |> SELECT SIGN(EXP(r.v)) AS scale |> DISTINCT |> ORDER BY scale",
        vec![vec![Cell::Null], vec![Cell::Number(1.0_f64.to_bits())]],
    );
    query(
        &db,
        "FROM facts |> SELECT EXP(v-v) AS one |> UNION DISTINCT (FROM facts |> SELECT EXP(0) AS one)",
        vec![vec![Cell::Number(1.0_f64.to_bits())]],
    );
    query(
        &db,
        "FROM facts |> LIMIT 0 |> AGGREGATE AVG(EXP(v)) AS mean",
        vec![vec![Cell::Null]],
    );
    query(
        &db,
        "FROM facts |> LIMIT 0 |> AGGREGATE AVG(LN(v)) AS mean_log |> SELECT EXP(mean_log) AS geometric_mean",
        vec![vec![Cell::Null]],
    );
    failure(
        &db,
        "FROM facts |> SELECT EXP(v*9223372036854775807) AS bad",
        "multiplication",
        "EXP(v*9223372036854775807)",
    );
    for expression in [
        "EXP()",
        "EXP(1, 2)",
        "EXP(, 1)",
        "EXP(1, )",
        "EXP('x')",
        "EXP(DATE '1970-01-01')",
        "EXP(NULL)",
        "EXP(missing)",
        "DIV(EXP(0), 1)",
        "MOD(1, EXP(0))",
        "EXP2(1)",
        "EXPM1(1)",
    ] {
        assert!(
            db.prepare(&format!("FROM facts |> SELECT {expression}"))
                .is_err(),
            "{expression}"
        );
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    assert!(db.prepare("FROM facts |> LIMIT EXP(0)").is_err());
    assert_eq!(db.reserved_memory_bytes(), baseline);
}

#[test]
fn public_exp_owns_demanded_overflow_spans() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    let sql = "FROM facts |> WHERE v>EXP(1000)";
    let error = db.prepare(sql).err().expect("constant predicate overflow");
    let Error::ArithmeticOverflow { operation, span } = error else {
        panic!("overflow")
    };
    assert_eq!(operation, "exponentiation");
    assert_eq!(&sql[span.start()..span.end()], "EXP(1000)");
    assert_eq!(db.reserved_memory_bytes(), baseline);
    for expression in [
        "EXP(1000)",
        "EXP(v*1000)",
        "SUM(EXP(v*1000))",
        "EXP(9223372036854775807)",
        "SAFE_DIVIDE(EXP(v*1000), 0)",
        "SAFE_DIVIDE(1, EXP(v*1000))",
        "COALESCE(SAFE_DIVIDE(1, 0), EXP(v*1000))",
    ] {
        let stage = if expression.starts_with("SUM(") {
            "AGGREGATE"
        } else {
            "SELECT"
        };
        let sql = format!("# 雪\nFROM facts |> {stage} {expression} AS growth");
        let start = sql.find(expression).unwrap();
        let end = start + expression.len();
        let prepared = db.prepare(&sql).unwrap();
        drop(sql);
        let cancel = CancellationToken::new();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        let mut failed = false;
        for _ in 0..100_000 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Failed(Error::ArithmeticOverflow { operation, span }) => {
                    assert_eq!(*operation, "exponentiation");
                    assert_eq!((span.start(), span.end()), (start, end));
                    failed = true;
                    break;
                }
                _ => panic!("missing EXP overflow"),
            }
        }
        assert!(failed);
        assert!(matches!(
            result.step(),
            QueryStep::Failed(Error::ArithmeticOverflow { .. })
        ));
        let error = result.into_error().unwrap();
        drop(prepared);
        let Error::ArithmeticOverflow { operation, span } = error else {
            panic!("owned overflow")
        };
        assert_eq!(operation, "exponentiation");
        assert_eq!((span.start(), span.end()), (start, end));
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}

#[test]
fn public_power_preserves_promotion_composition_and_demand() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    for function in ["POW", "POWER"] {
        for (arguments, expected) in [
            ("2, 3", Some(8.0_f64)),
            ("2.0, 3", Some(8.0)),
            ("2, 3.0", Some(8.0)),
            ("2.0, 3.0", Some(8.0)),
            ("-2, 3", Some(-8.0)),
            ("-2, -3", Some(-0.125)),
            ("-1, 9007199254740991", Some(-1.0)),
            ("-1, 9007199254740993", Some(1.0)),
            ("-1, 9223372036854775807", Some(1.0)),
            ("-1, -9223372036854775808", Some(1.0)),
            ("9007199254740993, 1", Some(9_007_199_254_740_992.0)),
            ("9223372036854775807, 1", Some(9_223_372_036_854_775_808.0)),
            ("0, 0", Some(1.0)),
            ("-0.0, 3", Some(-0.0)),
            ("2, -1074", Some(f64::from_bits(1))),
            ("2, -1075", Some(0.0)),
            ("NULLIF(1, 1), 0", None),
            ("1, SAFE_DIVIDE(1, 0)", None),
        ] {
            let sql = format!("FROM facts |> LIMIT 1 |> SELECT {function}({arguments}) AS growth");
            let prepared = db.prepare(&sql).unwrap();
            let column = prepared.result_column(0).unwrap();
            assert_eq!(
                (column.data_type, column.nullable),
                (DataType::Double, expected.is_none())
            );
            drop(prepared);
            query(
                &db,
                &sql,
                vec![vec![
                    expected.map_or(Cell::Null, |value| Cell::Number(value.to_bits())),
                ]],
            );
        }
        for expression in [
            format!("COALESCE(9, {function}(-1, 0.5))"),
            format!("COALESCE({function}(NULLIF(1, 1), 0), 9)"),
        ] {
            query(
                &db,
                &format!("FROM facts |> LIMIT 1 |> SELECT {expression} AS chosen"),
                vec![vec![Cell::Number(9.0_f64.to_bits())]],
            );
        }
        for sql in [
            format!(
                "FROM facts |> EXTEND {function}(-v, 0.5) AS bad |> DROP bad |> SELECT v |> ORDER BY v"
            ),
            format!(
                "FROM facts |> EXTEND {function}(-v, 0.5) AS bad |> WHERE v>0 OR bad>0 |> SELECT v |> ORDER BY v"
            ),
            format!(
                "FROM facts |> EXTEND {function}(v, 2) AS square |> WHERE square>{function}(0, 2) |> SELECT v |> ORDER BY v"
            ),
        ] {
            query(&db, &sql, integers(&[10, 20, 30, 40]));
        }
        query(
            &db,
            &format!(
                "FROM facts |> EXTEND {function}(-v, 0.5) AS bad |> WHERE v<0 AND bad>0 |> SELECT v"
            ),
            vec![],
        );
        query(
            &db,
            &format!(
                "FROM facts |> AGGREGATE SUM({function}(v/10, 2)) AS total GROUP AND ORDER BY k"
            ),
            vec![
                vec![Cell::Null, Cell::Number(16.0_f64.to_bits())],
                vec![Cell::Integer(1), Cell::Number(5.0_f64.to_bits())],
                vec![Cell::Integer(2), Cell::Number(9.0_f64.to_bits())],
            ],
        );
        query(
            &db,
            &format!(
                "FROM facts AS l |> LEFT JOIN facts AS r ON l.k=r.k |> SELECT {function}(r.v, 0) AS one |> DISTINCT |> ORDER BY one"
            ),
            vec![vec![Cell::Null], vec![Cell::Number(1.0_f64.to_bits())]],
        );
        query(
            &db,
            &format!(
                "FROM facts |> SELECT {function}(v, 0) AS one |> UNION DISTINCT (FROM facts |> SELECT {function}(1, v) AS one)"
            ),
            vec![vec![Cell::Number(1.0_f64.to_bits())]],
        );
        query(
            &db,
            &format!("FROM facts |> LIMIT 0 |> AGGREGATE AVG({function}(v, 3)) AS mean"),
            vec![vec![Cell::Null]],
        );
        for arguments in [
            "",
            "1",
            "1, 2, 3",
            ", 1",
            "1, ",
            "'x', 1",
            "1, 'x'",
            "DATE '1970-01-01', 1",
            "1, DATE '1970-01-01'",
            "NULL, 1",
            "1, NULL",
            "missing, 1",
        ] {
            assert!(
                db.prepare(&format!(
                    "FROM facts |> SELECT {function}({arguments}) AS bad"
                ))
                .is_err(),
                "{arguments}"
            );
        }
        for sql in [
            format!("FROM facts |> LIMIT {function}(1, 0)"),
            format!("FROM facts |> LIMIT 1 OFFSET {function}(1, 0)"),
            format!("FROM facts |> SELECT DIV({function}(1, 0), 1) AS bad"),
            format!("FROM facts |> SELECT MOD(1, {function}(1, 0)) AS bad"),
        ] {
            assert!(db.prepare(&sql).is_err(), "{sql}");
        }
        // A leaf and fifteen binary calls occupy 31 operations. Negation fills
        // slot 32; the next binary call exceeds the same bound before token 160.
        let nested = format!(
            "{}v{}",
            format!("{function}(").repeat(15),
            ", 1)".repeat(15)
        );
        query(
            &db,
            &format!("FROM facts |> SELECT -({nested}) AS value |> ORDER BY value"),
            vec![40.0_f64, 30.0, 20.0, 10.0]
                .into_iter()
                .map(|value| vec![Cell::Number((-value).to_bits())])
                .collect(),
        );
        assert!(
            db.prepare(&format!(
                "FROM facts |> SELECT {function}({nested}, 1) AS bad"
            ))
            .is_err()
        );
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    // Exact rational compounding gives 1331, 1728, 2197 and 2744. Six ULPs
    // cover this complete DOUBLE query, independently of its native powf path.
    let prepared = db
        .prepare("FROM facts |> ORDER BY v |> SELECT 1000*POWER(1+v/100, 3) AS compounded")
        .unwrap();
    let cancel = CancellationToken::new();
    let mut result = db.execute(&prepared, &cancel).unwrap();
    let rows = collect_unordered(&mut result);
    assert_eq!(rows.len(), 4);
    for (row, expected) in rows.iter().zip([1331.0_f64, 1728.0, 2197.0, 2744.0]) {
        let [Cell::Number(actual)] = row.as_slice() else {
            panic!("DOUBLE compounded value");
        };
        assert!(actual.abs_diff(expected.to_bits()) <= 6, "{actual:016x}");
    }
    drop(result);
    drop(prepared);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
}

#[test]
fn public_power_owns_demanded_domain_and_overflow_spans() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    for (expression, domain) in [("POW(-1, 0.5)", true), ("POWER(2, 1024)", false)] {
        let sql = format!("FROM facts |> WHERE v>{expression}");
        let error = db.prepare(&sql).err().expect("constant power failure");
        let (operation, span) = match (domain, error) {
            (true, Error::ArithmeticDomain { operation, span })
            | (false, Error::ArithmeticOverflow { operation, span }) => (operation, span),
            _ => panic!("power failure category"),
        };
        assert_eq!(operation, "power");
        assert_eq!(&sql[span.start()..span.end()], expression);
        assert_eq!(db.reserved_memory_bytes(), baseline);
    }
    for (expression, domain) in [
        ("POW(-v, 0.5)", true),
        ("POWER(0, -1)", true),
        ("POW(-0.0, -0.5)", true),
        ("SUM(POWER(-v, 0.5))", true),
        ("SAFE_DIVIDE(POW(-v, 0.5), 0)", true),
        ("SAFE_DIVIDE(1, POWER(-v, 0.5))", true),
        ("COALESCE(NULLIF(1, 1), POW(-v, 0.5))", true),
        ("POWER(2, v*1000)", false),
        ("SUM(POW(2, v*1000))", false),
        ("SAFE_DIVIDE(POWER(2, v*1000), 0)", false),
        ("SAFE_DIVIDE(1, POW(2, v*1000))", false),
        ("COALESCE(NULLIF(1, 1), POWER(2, v*1000))", false),
    ] {
        let stage = if expression.starts_with("SUM(") {
            "AGGREGATE"
        } else {
            "SELECT"
        };
        let sql = format!("# 雪\nFROM facts |> {stage} {expression} AS bad");
        let start = sql.find(expression).unwrap();
        let end = start + expression.len();
        let prepared = db.prepare(&sql).unwrap();
        drop(sql);
        let cancel = CancellationToken::new();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        let mut failed = false;
        for _ in 0..100_000 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Failed(error) => {
                    let (operation, span) = match (domain, error) {
                        (true, Error::ArithmeticDomain { operation, span })
                        | (false, Error::ArithmeticOverflow { operation, span }) => {
                            (operation, span)
                        }
                        _ => panic!("power failure category"),
                    };
                    assert_eq!(*operation, "power");
                    assert_eq!((span.start(), span.end()), (start, end));
                    failed = true;
                    break;
                }
                _ => panic!("missing demanded power failure"),
            }
        }
        assert!(failed);
        assert!(matches!(result.step(), QueryStep::Failed(_)));
        let error = result.into_error().unwrap();
        drop(prepared);
        let (operation, span) = match (domain, error) {
            (true, Error::ArithmeticDomain { operation, span })
            | (false, Error::ArithmeticOverflow { operation, span }) => (operation, span),
            _ => panic!("owned power failure category"),
        };
        assert_eq!(operation, "power");
        assert_eq!((span.start(), span.end()), (start, end));
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    for expression in [
        "POW(v*9223372036854775807, 0)",
        "POWER(1, v*9223372036854775807)",
        "POW(NULLIF(1, 1), v*9223372036854775807)",
    ] {
        failure(
            &db,
            &format!("FROM facts |> SELECT {expression} AS bad"),
            "multiplication",
            expression,
        );
    }
}
