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
        "FROM facts |> SELECT '雪' AS label,DATE '1970-01-01' AS day |> ORDER BY label |> DISTINCT",
        "FROM facts AS f |> EXTEND 'branch' AS tag |> JOIN dimensions AS d ON f.k=d.k |> SELECT tag,d.label |> ORDER BY tag",
        "FROM facts |> SELECT v+1 AS x |> WHERE x > 0",
        "FROM facts |> EXTEND v+1 AS x |> WHERE x > 0",
        "FROM facts AS f |> SET v=v+1 |> RENAME v AS adjusted |> DROP k |> WHERE f.k>0 |> ORDER BY adjusted |> SELECT f.v,adjusted",
        "FROM dimensions |> SET label=label |> DISTINCT |> ORDER BY label |> LIMIT 2",
        "FROM facts AS f |> EXTEND v+1 AS x |> JOIN dimensions AS d ON f.k=d.k |> ORDER BY x |> AGGREGATE SUM(x) AS s |> EXTEND s+1 AS total",
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

#[test]
fn extend_executes_through_projection_filters_groups_joins_and_derived_inputs() {
    let (_directory, db) = join_fixture();
    for (sql, expected) in [
        (
            "FROM facts |> EXTEND v*2 twice |> SELECT v,twice |> ORDER BY v",
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
            "FROM facts AS f |> EXTEND v+1 AS n |> JOIN (FROM facts |> SELECT k,v) AS d ON f.k=d.k |> AGGREGATE SUM(n) AS total",
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
    let (_directory, db) = super::null_predicate::fixture().unwrap();
    query(
        &db,
        "FROM facts AS f |> EXTEND s AS text,d AS day |> ORDER BY id |> SELECT f.s,text,f.d,day",
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
        "FROM facts |> EXTEND id+100 AS id,id+1 AS next |> SELECT next |> ORDER BY next",
        integers(&[1, 2, 3, 4]),
    );
    query(
        &db,
        "FROM facts |> WHERE id < 0 |> EXTEND s AS text,d AS day",
        vec![],
    );
}

#[test]
fn rename_preserves_values_order_and_qualified_inputs_through_composition() {
    let (_directory, db) = join_fixture();
    query(
        &db,
        "FROM facts AS f |> ORDER BY v DESC |> RENAME v AS amount |> SELECT f.v,amount",
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
        "FROM facts |> EXTEND v*9223372036854775807 AS bad |> DROP bad,k |> ORDER BY v",
        "FROM facts |> SELECT k AS discarded,k AS discarded,v |> DROP discarded |> ORDER BY v",
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
        "FROM facts AS f |> SET k=v,v=k |> ORDER BY k |> SELECT k,v,f.k,f.v",
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
        "FROM dimensions AS d |> SET label=d.label |> WHERE k=2 |> ORDER BY label |> SELECT label,d.label",
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
    let sql = "FROM dates AS d |> SET ordinal=day |> ORDER BY ordinal |> SELECT ordinal,d.ordinal";
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
        "FROM dates AS ordinal |> SET ordinal=day |> AGGREGATE MIN(ordinal) AS lo,MAX(ordinal) AS hi,COUNT(ordinal) AS n",
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
        "FROM facts |> SELECT v,1/0 AS bad |> SELECT v |> ORDER BY v",
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
    let sql = "# 雪\nFROM facts |> SELECT v/(k-1) AS ratio";
    let prepared = db.prepare(sql).unwrap();
    let cancel = CancellationToken::new();
    let mut result = db.execute(&prepared, &cancel).unwrap();
    let mut failed = false;
    for _ in 0..100_000 {
        match result.step() {
            QueryStep::Progress | QueryStep::Rows(_) => (),
            QueryStep::Failed(pipesql::Error::DivisionByZero { span }) => {
                assert_eq!(&sql[span.start()..span.end()], "v/(k-1)");
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
        "FROM facts |> SELECT k,v/(k-1) AS ratio |> WHERE k=1 OR ratio>0 |> SELECT k |> ORDER BY k",
        integers(&[1, 1, 2]),
    );
    query(
        &db,
        "FROM facts |> SELECT k,v/(k-1) AS ratio |> WHERE k!=1 AND ratio>0 |> SELECT ratio",
        vec![vec![Cell::Number(30.0_f64.to_bits())]],
    );
    query(
        &db,
        "FROM facts |> AGGREGATE SUM(v) AS total,COUNT(*) AS n GROUP AND ORDER BY k |> SELECT k,total/n AS ratio",
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
fn public_division_cancellation_and_early_drop_release_owners() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    let prepared = db
        .prepare("FROM facts |> SELECT v/2 AS ratio |> ORDER BY ratio")
        .unwrap();
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
    query(
        &db,
        "FROM facts |> AGGREGATE SUM(v/2) AS total",
        vec![vec![Cell::Number(50.0_f64.to_bits())]],
    );
}
