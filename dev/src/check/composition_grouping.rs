//! Check grouping when projections, ordering and row limits surround aggregates.
//!
//! Expected groups come from explicit input-row calculations or literal boundary
//! answers. Cases reorder aliases, repeat input columns and move scalar operations
//! across stages so names and positions cannot substitute for column identity.
//! Global and grouped results retain their different empty-input behavior. The
//! shared checker requires complete rows and enforces order where the query promises
//! it; aggregate values alone are insufficient.

use super::*;
use Aggregate::{Average, Count, Sum};
const GLOBAL: &str =
    "FROM lineitem |> AGGREGATE SUM(l_quantity) AS s, AVG(l_quantity) AS a, COUNT(*) AS n";
const GROUPED: &str = "FROM lineitem |> AGGREGATE SUM(l_quantity) AS qs, AVG(l_quantity) AS qa, SUM(l_extendedprice) AS ps, AVG(l_extendedprice) AS pa, SUM(l_discount) AS ds, AVG(l_discount) AS da, SUM(l_tax) AS ts, COUNT(*) AS n GROUP AND ORDER BY l_returnflag, l_linestatus";

pub(super) fn run(queries: &mut Queries, rows: &[Row]) -> Result<()> {
    for (label, sql, expected, database) in [
        (
            "cast-precision-boundary",
            "FROM lineitem |> LIMIT 1 |> SELECT CAST(9007199254740993 AS FLOAT64) AS rounded, CAST(9007199254740993 - 9007199254740992 AS DOUBLE) AS exact_first, CAST(9007199254740993 AS DOUBLE) - CAST(9007199254740992 AS DOUBLE) AS cast_first",
            vec![
                "4340000000000000".into(),
                "3ff0000000000000".into(),
                "0000000000000000".into(),
            ],
            "data",
        ),
        (
            "cast-before-integer-limit",
            "FROM lineitem |> LIMIT 1 |> SELECT CAST(9223372036854775807 AS DOUBLE) + 1 AS n",
            vec!["43e0000000000000".into()],
            "data",
        ),
        (
            "cast-aggregate-identity",
            "FROM lineitem |> AGGREGATE COUNT(*) AS entries |> SELECT CAST(entries AS FLOAT64) AS n |> AGGREGATE SUM(n) AS total",
            vec!["40f0001000000000".into()],
            "data",
        ),
        (
            "cast-empty-aggregate",
            "FROM lineitem |> AGGREGATE SUM(CAST(1 AS DOUBLE)) AS total",
            vec!["null".into()],
            "empty",
        ),
        (
            "byte-length-ascii",
            "FROM lineitem |> SELECT BYTE_LENGTH(l_returnflag) AS width |> AGGREGATE SUM(width) AS total",
            vec![integer(rows.len() as i64)],
            "data",
        ),
        (
            "byte-length-unicode-literal",
            "FROM lineitem |> SELECT BYTE_LENGTH('é') AS width |> SELECT width + 1 AS width |> AGGREGATE SUM(width) AS total",
            vec![integer(3 * rows.len() as i64)],
            "data",
        ),
        (
            "byte-length-empty",
            "FROM lineitem |> SELECT BYTE_LENGTH(l_returnflag) AS width |> AGGREGATE SUM(width) AS total",
            vec!["null".into()],
            "empty",
        ),
        (
            "char-length-ascii",
            "FROM lineitem |> SELECT CHAR_LENGTH(l_returnflag) AS width |> AGGREGATE SUM(width) AS total",
            vec![integer(rows.len() as i64)],
            "data",
        ),
        (
            "char-length-unicode-literal",
            "FROM lineitem |> SELECT CHAR_LENGTH('é') AS width |> SELECT width + 1 AS width |> AGGREGATE SUM(width) AS total",
            vec![integer(2 * rows.len() as i64)],
            "data",
        ),
        (
            "char-length-empty",
            "FROM lineitem |> SELECT CHAR_LENGTH(l_returnflag) AS width |> AGGREGATE SUM(width) AS total",
            vec!["null".into()],
            "empty",
        ),
        (
            "date-year-stored",
            "FROM lineitem |> SELECT EXTRACT(YEAR FROM l_shipdate) AS y |> AGGREGATE SUM(y) AS total",
            vec![integer(
                (0..rows.len())
                    .map(|i| i64::from(DATES[i % DATES.len()].0))
                    .sum(),
            )],
            "data",
        ),
        (
            "date-year-boundaries",
            "FROM lineitem |> LIMIT 1 |> SELECT EXTRACT(YEAR FROM DATE '0001-01-01') AS first, EXTRACT(YEAR FROM DATE '2016-01-01') AS calendar, EXTRACT(YEAR FROM DATE '9999-12-31') AS last",
            vec![integer(1), integer(2016), integer(9999)],
            "data",
        ),
        (
            "date-year-shift",
            "FROM lineitem |> LIMIT 1 |> SELECT EXTRACT(YEAR FROM DATE_ADD(DATE '1999-12-31', INTERVAL 1 DAY)) AS y",
            vec![integer(2000)],
            "data",
        ),
        (
            "date-year-empty",
            "FROM lineitem |> SELECT EXTRACT(YEAR FROM l_shipdate) AS y |> AGGREGATE SUM(y) AS total",
            vec!["null".into()],
            "empty",
        ),
    ] {
        queries.check(label, sql, vec![expected], database, true)?;
    }
    let all: Vec<_> = rows.iter().collect();
    let global = [Sum(quantity), Average(quantity), Count];
    let entries = [
        Sum(quantity),
        Average(quantity),
        Sum(price),
        Average(price),
        Sum(discount),
        Average(discount),
        Sum(tax),
        Count,
    ];
    queries.check("global", GLOBAL, groups(&all, &[], &global), "data", false)?;
    queries.check(
        "empty-global",
        GLOBAL,
        groups(&[], &[], &global),
        "empty",
        false,
    )?;
    let ordered = groups(&all, &[0, 1], &entries);
    queries.check("all-groups", GROUPED, ordered.clone(), "data", true)?;
    // Slice ordered groups, never input rows. These limits also exercise values
    // far beyond the real result without narrowing them to a smaller integer.
    for count in [0_u64, 1, 3, 99999, i64::MAX as u64] {
        for offset in [0_u64, 1, 256, i64::MAX as u64] {
            let start = offset.min(ordered.len() as u64) as usize;
            let length = count.min((ordered.len() - start) as u64) as usize;
            queries.check(
                &format!("limit-groups-{count}-{offset}"),
                &format!("{GROUPED} |> LIMIT {count} OFFSET {offset}"),
                ordered[start..start + length].to_vec(),
                "data",
                true,
            )?;
        }
    }
    queries.check(
        "empty-groups",
        GROUPED,
        groups(&[], &[0, 1], &entries),
        "empty",
        true,
    )?;
    let filtered: Vec<_> = rows.iter().filter(|row| quantity(row) < 25.0).collect();
    queries.check(
        "filtered-groups",
        &GROUPED.replace(" |> AGGREGATE", " |> WHERE l_quantity < 25.0 |> AGGREGATE"),
        groups(&filtered, &[0, 1], &entries),
        "data",
        true,
    )?;
    queries.check(
        "filtered-empty",
        &GLOBAL.replace(" |> AGGREGATE", " |> WHERE l_quantity < 0.0 |> AGGREGATE"),
        groups(&[], &[], &global),
        "data",
        false,
    )?;
    queries.check("alias-swap", "FROM lineitem |> SELECT l_quantity AS price, l_extendedprice AS qty, l_returnflag AS flag |> AGGREGATE SUM(qty) AS total, AVG(price) AS mean GROUP AND ORDER BY flag", groups(&all, &[0], &[Sum(price), Average(quantity)]), "data", true)?;
    queries.check(
        "counts-only",
        "FROM lineitem |> AGGREGATE COUNT(*) AS n GROUP BY l_linestatus",
        groups(&all, &[1], &[Count]),
        "data",
        false,
    )?;
    queries.check(
        "no-input-columns",
        "FROM lineitem |> AGGREGATE COUNT(*) AS n",
        groups(&all, &[], &[Count]),
        "data",
        false,
    )?;
    queries.check("duplicate-input", "FROM lineitem |> SELECT l_quantity AS q, l_quantity AS other |> AGGREGATE SUM(other) AS s, SUM(q) AS again, AVG(q) AS a", groups(&all, &[], &[Sum(quantity), Sum(quantity), Average(quantity)]), "data", false)?;
    Ok(())
}
