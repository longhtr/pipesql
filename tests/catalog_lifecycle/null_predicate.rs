use super::order::{integers, query};
use super::*;

pub(super) fn fixture() -> Result<(Directory, Database), Error> {
    let directory = Directory::new();
    let db = Database::create_empty(&directory.database(), config())?;
    let cancel = CancellationToken::new();
    let columns = [
        ("id", DataType::Int64),
        ("n", DataType::Double),
        ("s", DataType::String),
        ("i", DataType::Int64),
        ("d", DataType::Date),
    ]
    .map(|(name, data_type)| ColumnDeclaration {
        name,
        data_type,
        nullable: name != "id",
    });
    db.declare_table("facts", &columns, &cancel)?;
    let day = DateValue::from_days_since_unix_epoch(0).unwrap();
    let mut append = db.begin_append(
        "facts",
        AppendLimits {
            batches: 1,
            encoded_bytes: 20_000,
        },
        &cancel,
    )?;
    append.write(
        &[
            ColumnInput {
                values: ColumnValues::Int64(&[0, 1, 2, 3]),
                validity: &[15],
            },
            ColumnInput {
                values: ColumnValues::Double(&[0., 0., f64::NAN, f64::INFINITY]),
                validity: &[14],
            },
            ColumnInput {
                values: ColumnValues::String(&["present", "ignored", "", "é"]),
                validity: &[13],
            },
            ColumnInput {
                values: ColumnValues::Int64(&[0, 7, 0, 9]),
                validity: &[11],
            },
            ColumnInput {
                values: ColumnValues::Date(&[day; 4]),
                validity: &[7],
            },
        ],
        &cancel,
    )?;
    append.commit(&cancel)?;
    Ok((directory, db))
}

#[test]
fn public_null_predicates_distinguish_all_types_and_compose() {
    let (_directory, db) = fixture().unwrap();
    for (column, null_id) in [("n", 0), ("s", 1), ("i", 2), ("d", 3)] {
        for negated in [false, true] {
            let test = if negated { "IS NOT NULL" } else { "IS NULL" };
            let expected: Vec<_> = (0..4).filter(|id| (*id == null_id) != negated).collect();
            for source in ["FROM facts", "FROM (FROM facts)"] {
                query(
                    &db,
                    &format!("{source} |> WHERE {column} {test} |> ORDER BY id |> SELECT id"),
                    integers(&expected),
                );
            }
        }
    }
    query(
        &db,
        "FROM facts |> ORDER BY id DESC |> WHERE s IS NOT NULL |> LIMIT 2 |> SELECT id",
        integers(&[3, 2]),
    );
    query(
        &db,
        "FROM facts |> ORDER BY id DESC |> WHERE s IS NULL |> SELECT id",
        integers(&[1]),
    );
    query(
        &db,
        "FROM facts AS a |> JOIN facts AS b ON a.id=b.id |> WHERE b.s IS NULL |> SELECT a.id",
        integers(&[1]),
    );
    query(
        &db,
        "FROM facts |> AGGREGATE COUNT(*) AS n GROUP AND ORDER BY s |> WHERE s IS NULL |> SELECT n",
        integers(&[1]),
    );
    query(
        &db,
        "FROM facts |> AGGREGATE COUNT(*) AS n GROUP AND ORDER BY s |> WHERE s IS NOT NULL |> SELECT s,n",
        ["", "present", "é"]
            .map(|s| vec![Cell::Text(s.to_owned()), Cell::Integer(1)])
            .to_vec(),
    );
    query(
        &db,
        "FROM facts |> WHERE id<0 |> AGGREGATE SUM(i) AS x |> WHERE x IS NULL",
        vec![vec![Cell::Null]],
    );
    query(
        &db,
        "FROM facts |> WHERE id<0 |> AGGREGATE SUM(i) AS x |> WHERE x IS NOT NULL",
        vec![],
    );
    query(
        &db,
        "FROM facts |> WHERE id<0 |> AGGREGATE COUNT(*) AS x |> WHERE x IS NOT NULL",
        integers(&[0]),
    );
    for sql in [
        "FROM facts |> WHERE s IS",
        "FROM facts |> WHERE s IS NOT",
        "FROM facts |> WHERE s IS TRUE",
        "FROM facts |> WHERE s IS UNKNOWN",
        "FROM facts |> WHERE s IS NOT NOT NULL",
        "FROM facts |> WHERE missing IS NULL",
        "FROM facts |> WHERE s = NULL",
        "FROM facts |> SELECT s AS x,i AS x |> WHERE x IS NULL",
        "FROM (FROM facts) |> WHERE facts.s IS NULL",
    ] {
        assert!(
            matches!(
                db.prepare(sql),
                Err(Error::Parse { .. } | Error::Bind { .. })
            ),
            "{sql}"
        );
    }
}

#[test]
fn null_tests_preserve_demanded_errors_and_prior_predicate_order() {
    let (_directory, db) = fixture().unwrap();
    let baseline = db.reserved_memory_bytes();
    for test in ["IS NULL", "IS NOT NULL"] {
        for boundary in ["", " |> ORDER BY x"] {
            let sql = format!(
                "FROM facts |> SELECT id*9223372036854775807 AS x{boundary} |> WHERE x {test}"
            );
            let prepared = db.prepare(&sql).unwrap();
            assert!(!prepared.result_column(0).unwrap().nullable);
            let cancel = CancellationToken::new();
            let mut result = db.execute(&prepared, &cancel).unwrap();
            let mut failed = false;
            for _ in 0..8192 {
                match result.step() {
                    QueryStep::Progress | QueryStep::Rows(_) => (),
                    QueryStep::Failed(error) => {
                        assert!(matches!(error, Error::ArithmeticOverflow { .. }));
                        failed = true;
                        break;
                    }
                    QueryStep::Finished => panic!("demanded error hidden: {sql}"),
                }
            }
            assert!(failed);
            drop(result);
            drop(prepared);
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
        }
    }
    query(
        &db,
        "FROM facts |> SELECT id,id*9223372036854775807 AS x |> WHERE id<2 AND x IS NOT NULL |> ORDER BY id |> SELECT id",
        integers(&[0, 1]),
    );
    query(
        &db,
        "FROM facts |> WHERE id<0 |> SELECT id*9223372036854775807 AS x |> WHERE x IS NULL",
        vec![],
    );
}
