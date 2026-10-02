//! Check that derived queries preserve scope, cardinality and aggregate demand.
//!
//! Literal answers distinguish aggregation over source rows from aggregation over
//! an inner query's groups. Nested computations, ordered prefixes and empty inputs
//! exercise those boundaries through the CLI. Invalid references must fail during
//! preparation; a demanded overflowing aggregate must fail before publishing rows.
//! An outer query that does not demand that value must retain its own valid result.

use super::*;
const GROUP_SUM: &str = "FROM lineitem |> AGGREGATE SUM(l_quantity) AS total GROUP BY l_returnflag";
const GLOBAL: &str =
    "FROM lineitem |> AGGREGATE SUM(l_quantity) AS s, AVG(l_quantity) AS a, COUNT(*) AS n";

pub(super) fn run(queries: &mut Queries) -> Result<()> {
    // Averaging the two group sums gives 60; averaging the source rows gives 40.
    queries.seed("mean-only", &[(f64::MAX, b'A'), (f64::MAX, b'A')])?;
    for (label, sql, expected, database) in [
        ("derived-average-of-group-sums", format!("FROM ({GROUP_SUM}) AS totals |> AGGREGATE AVG(totals.total) AS mean"), vec![double(60.0)], "repeated"),
        ("nested-derived-computation", "FROM (FROM (FROM lineitem |> SELECT l_quantity+1 AS x) |> SELECT x*2 AS y) |> AGGREGATE SUM(y) AS total".into(), vec![double(246.0)], "repeated"),
        ("derived-ordered-group-prefix", "FROM (FROM lineitem |> AGGREGATE SUM(l_quantity) AS total GROUP AND ORDER BY l_returnflag |> LIMIT 1) |> SELECT total".into(), vec![double(30.0)], "repeated"),
        ("derived-duplicate-outputs", "FROM (FROM lineitem |> AGGREGATE COUNT(*) AS n |> SELECT n AS x, n AS x)".into(), vec![integer(3),integer(3)], "repeated"),
        ("derived-empty-global", "FROM (FROM lineitem |> AGGREGATE AVG(l_quantity) AS x) |> SELECT x".into(), vec!["null".into()], "empty"),
        ("derived-undemanded-overflow", format!("FROM ({GLOBAL}) |> SELECT a"), vec![double(f64::MAX)], "mean-only"),
    ] { queries.check(label, &sql, vec![expected], database, true)?; }
    queries.overflow(
        "mean-only",
        &format!("FROM ({GLOBAL}) |> SELECT s"),
        "SUM",
        "SUM(l_quantity)",
    )?;
    for (sql, class, message, context, token) in [
        (
            "FROM (FROM lineitem AS hidden) |> SELECT hidden.l_quantity",
            "bind",
            "table alias is not visible",
            "hidden.l_quantity",
            "hidden.l_quantity",
        ),
        (
            "FROM (FROM lineitem) |> SELECT lineitem.l_quantity",
            "bind",
            "table alias is not visible",
            "lineitem.l_quantity",
            "lineitem.l_quantity",
        ),
        (
            "FROM (FROM lineitem |> SELECT l_quantity AS x, l_quantity AS x) |> SELECT x",
            "bind",
            "ambiguous column name",
            "SELECT x",
            "x",
        ),
        (
            "FROM (SELECT l_quantity FROM lineitem)",
            "parse",
            "unexpected token",
            "SELECT",
            "SELECT",
        ),
    ] {
        let region = source_range(sql, context)?;
        let token = source_range(context, token)?;
        queries.preparation(
            "repeated",
            sql,
            class,
            message,
            region.start + token.start..region.start + token.end,
        )?;
    }
    let sql = "FROM (FROM lineitem;)";
    queries.preparation(
        "repeated",
        sql,
        "parse",
        "unfinished derived input",
        sql.len()..sql.len(),
    )?;
    Ok(())
}
