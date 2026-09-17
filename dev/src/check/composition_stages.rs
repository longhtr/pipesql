//! Check column identity and aggregate demand across successive pipeline stages.
//!
//! Separate selections cover column transformations, repeated aggregation and demand
//! for outputs that later stages keep or drop. Aliases and duplicate projections
//! must preserve the underlying values even when their positions change. Dropping
//! an overflowing aggregate argument can remove its evaluation, but dropping a
//! visible grouping key cannot erase grouping or its promised order. Each case
//! supplies complete expected rows to the shared CLI checker.

use super::*;
const GLOBAL: &str =
    "FROM lineitem |> AGGREGATE SUM(l_quantity) AS s, AVG(l_quantity) AS a, COUNT(*) AS n";
const GROUP_SUM: &str = "FROM lineitem |> AGGREGATE SUM(l_quantity) AS total GROUP BY l_returnflag";
const GROUPED: &str = "FROM lineitem |> AGGREGATE SUM(l_quantity) AS qs, AVG(l_quantity) AS qa, SUM(l_extendedprice) AS ps, AVG(l_extendedprice) AS pa, SUM(l_discount) AS ds, AVG(l_discount) AS da, SUM(l_tax) AS ts, COUNT(*) AS n GROUP AND ORDER BY l_returnflag, l_linestatus";
fn doubles(values: &[f64]) -> Vec<Vec<String>> {
    values.iter().map(|v| vec![double(*v)]).collect()
}

pub(super) fn columns(queries: &mut Queries) -> Result<()> {
    for (label, sql, expected, database) in [
        (
            "extend-original-range",
            "FROM lineitem AS t |> EXTEND t.l_quantity+1 AS x |> SELECT t.l_quantity, x",
            [10.0, 20.0, 90.0]
                .into_iter()
                .map(|v| vec![double(v), double(v + 1.0)])
                .collect(),
            "repeated",
        ),
        (
            "extend-separate-aliases",
            "FROM lineitem |> EXTEND l_quantity+1 x |> EXTEND x*2 y |> SELECT y",
            doubles(&[22.0, 42.0, 182.0]),
            "repeated",
        ),
        (
            "extend-group-order",
            "FROM lineitem |> AGGREGATE SUM(l_quantity) AS s GROUP AND ORDER BY l_returnflag |> EXTEND s+1 AS x |> SELECT x",
            doubles(&[31.0, 91.0]),
            "repeated",
        ),
        (
            "extend-hidden-overflow",
            "FROM lineitem |> EXTEND l_quantity*1e308 AS x |> SELECT l_quantity",
            doubles(&[10.0, 20.0, 90.0]),
            "repeated",
        ),
        (
            "set-original-range",
            "FROM lineitem AS t |> SET l_quantity=l_quantity+1 |> SELECT l_quantity, t.l_quantity",
            [10.0, 20.0, 90.0]
                .into_iter()
                .map(|v| vec![double(v + 1.0), double(v)])
                .collect(),
            "repeated",
        ),
        (
            "set-changed-type",
            "FROM lineitem |> SET l_quantity=l_returnflag |> SELECT l_quantity",
            vec![
                vec!["string:41".into()],
                vec!["string:41".into()],
                vec!["string:42".into()],
            ],
            "repeated",
        ),
        (
            "drop-qualified-input",
            "FROM lineitem AS t |> DROP l_quantity |> WHERE t.l_quantity>10 |> SELECT t.l_quantity",
            doubles(&[20.0, 90.0]),
            "repeated",
        ),
        (
            "rename-original-range",
            "FROM lineitem AS t |> RENAME l_quantity AS quantity |> SELECT quantity, t.l_quantity",
            [10.0, 20.0, 90.0]
                .into_iter()
                .map(|v| vec![double(v), double(v)])
                .collect(),
            "repeated",
        ),
        (
            "set-pruned-overflow",
            "FROM lineitem |> SET l_quantity=l_quantity*1e308 |> SET l_quantity=1 |> SELECT l_quantity",
            vec![vec![integer(1)]; 3],
            "repeated",
        ),
        (
            "extend-empty",
            "FROM lineitem |> EXTEND 9223372036854775807+1 AS x",
            vec![],
            "empty",
        ),
    ] {
        queries.check(label, sql, expected, database, true)?;
    }
    for expression in ["l_quantity AS x, x+1 AS y", "SUM(l_quantity)", "*"] {
        queries.reject(
            &format!("FROM lineitem |> EXTEND {expression}"),
            "repeated",
            Failure::Preparation,
        )?;
    }
    Ok(())
}

pub(super) fn repeated(queries: &mut Queries) -> Result<()> {
    for (label, sql, expected, database) in [
        (
            "average-of-group-sums",
            format!("{GROUP_SUM} |> AGGREGATE AVG(total) AS mean"),
            vec![double(60.0)],
            "repeated",
        ),
        (
            "computed-group-input",
            format!("{GROUP_SUM} |> SELECT total*2 AS total |> AGGREGATE AVG(total) AS mean"),
            vec![double(120.0)],
            "repeated",
        ),
        (
            "three-aggregate-stages",
            format!(
                "{GROUP_SUM} |> AGGREGATE SUM(total) AS subtotal GROUP BY l_returnflag |> AGGREGATE AVG(subtotal) AS mean"
            ),
            vec![double(60.0)],
            "repeated",
        ),
        (
            "empty-group-input-to-global",
            format!(
                "{GROUP_SUM} |> WHERE total < 0 |> AGGREGATE SUM(total) AS total, COUNT(*) AS n"
            ),
            vec!["null".into(), integer(0)],
            "repeated",
        ),
        (
            "count-empty-global-output",
            format!("{GLOBAL} |> AGGREGATE COUNT(*) AS again"),
            vec![integer(1)],
            "empty",
        ),
        (
            "empty-global-input-to-global",
            "FROM lineitem |> AGGREGATE COUNT(*) AS n |> AGGREGATE SUM(n) AS n".into(),
            vec![integer(0)],
            "empty",
        ),
        (
            "transitive-undemanded-overflow",
            format!("{GLOBAL} |> AGGREGATE AVG(a) AS mean"),
            vec![double(f64::MAX)],
            "mean-only",
        ),
        (
            "filtered-intermediate-overflow",
            format!("{GLOBAL} |> WHERE n < 0 |> AGGREGATE SUM(s) AS total"),
            vec!["null".into()],
            "mean-only",
        ),
    ] {
        queries.check(label, &sql, vec![expected], database, true)?;
    }
    for suffix in [
        " |> AGGREGATE SUM(s) AS total",
        " |> SELECT s*2 AS doubled |> AGGREGATE AVG(doubled) AS mean",
    ] {
        queries.reject(&format!("{GLOBAL}{suffix}"), "mean-only", Failure::Overflow)?;
    }
    Ok(())
}

pub(super) fn demand(queries: &mut Queries, source: &[Row]) -> Result<()> {
    queries.check(
        "drop-overflowing-sum",
        &format!("{GLOBAL} |> SELECT a"),
        doubles(&[f64::MAX]),
        "mean-only",
        true,
    )?;
    queries.check(
        "swap-aggregate-aliases",
        &format!("{GLOBAL} |> SELECT a AS s, n AS a |> SELECT a, s, s"),
        vec![vec![integer(2), double(f64::MAX), double(f64::MAX)]],
        "mean-only",
        true,
    )?;
    queries.check("drop-overflowing-argument", "FROM lineitem |> AGGREGATE SUM(l_quantity*(9223372036854775807+1)) AS s, COUNT(*) AS n |> SELECT n", vec![vec![integer(source.len() as i64)]], "data", true)?;
    let mut counts = BTreeMap::new();
    for row in source {
        *counts.entry(row.keys).or_insert(0_i64) += 1;
    }
    let expected = counts
        .into_iter()
        .filter(|(_, count)| *count > 7)
        .map(|(key, count)| vec![format!("string:{:02x}", key[0]), integer(count)])
        .collect();
    queries.check("drop-key-output-retains-order", &format!("{GROUPED} |> SELECT n, l_linestatus AS status, l_returnflag AS flag |> WHERE n > 7 |> SELECT flag, n"), expected, "data", true)?;
    for (symbol, accepts_equal) in [
        ("<", false),
        ("<=", true),
        ("=", true),
        ("!=", false),
        (">=", true),
        (">", false),
    ] {
        queries.check(
            &format!("nullable-aggregate-{symbol}"),
            &format!("{GLOBAL} |> WHERE a {symbol} 0 |> SELECT n"),
            vec![],
            "empty",
            true,
        )?;
        queries.check(
            &format!("count-comparison-{symbol}"),
            &format!("{GLOBAL} |> WHERE n {symbol} 2 |> SELECT n"),
            if accepts_equal {
                vec![vec![integer(2)]]
            } else {
                vec![]
            },
            "mean-only",
            true,
        )?;
    }
    for (label, sql, expected, database) in [
        (
            "exact-int64-sum",
            "FROM lineitem |> AGGREGATE SUM(9007199254740993) AS s, AVG(9007199254740993) AS a"
                .into(),
            vec![vec![integer(18014398509481986), "4340000000000000".into()]],
            "mean-only",
        ),
        (
            "empty-int64-sum",
            "FROM lineitem |> AGGREGATE SUM(1) AS s, AVG(1) AS a, COUNT(*) AS n".into(),
            vec![vec!["null".into(), "null".into(), integer(0)]],
            "empty",
        ),
        (
            "empty-count-zero",
            format!("{GLOBAL} |> WHERE n = 0 |> SELECT a, n"),
            vec![vec!["null".into(), integer(0)]],
            "empty",
        ),
        (
            "integer-bound-folding",
            format!("{GLOBAL} |> WHERE n = 9007199254740993-9007199254740991 |> SELECT n"),
            vec![vec![integer(2)]],
            "mean-only",
        ),
        (
            "count-int64-max",
            format!("{GLOBAL} |> WHERE n < 9223372036854775807 |> SELECT n"),
            vec![vec![integer(2)]],
            "mean-only",
        ),
        (
            "count-double-coercion",
            format!("{GLOBAL} |> WHERE n > 1.5 |> SELECT n"),
            vec![vec![integer(2)]],
            "mean-only",
        ),
        (
            "post-between-and",
            format!("{GLOBAL} |> WHERE n BETWEEN 1 AND 3 AND a > 0 |> SELECT a, n"),
            vec![vec![double(f64::MAX), integer(2)]],
            "mean-only",
        ),
        (
            "all-groups-filtered",
            format!("{GROUPED} |> WHERE n < 0 |> SELECT qs"),
            vec![],
            "data",
        ),
        (
            "overflowing-result-filtered-out",
            format!("{GLOBAL} |> WHERE n < 0 |> SELECT s"),
            vec![],
            "mean-only",
        ),
        (
            "overflowing-sum-filter-skipped",
            format!("{GLOBAL} |> WHERE n < 0 |> WHERE s > 0 |> SELECT n"),
            vec![],
            "mean-only",
        ),
        (
            "shared-average-filters-overflowing-sum",
            format!("{GLOBAL} |> WHERE a < 0 |> SELECT s"),
            vec![],
            "mean-only",
        ),
        (
            "empty-group-projection",
            format!("{GROUPED} |> SELECT n"),
            vec![],
            "empty",
        ),
        (
            "source-alias-replaced-by-aggregate",
            "FROM lineitem |> SELECT l_quantity AS n |> AGGREGATE AVG(n) AS n |> SELECT n".into(),
            doubles(&[f64::MAX]),
            "mean-only",
        ),
    ] {
        queries.check(label, &sql, expected, database, true)?;
    }
    for suffix in [
        " |> SELECT s",
        " |> WHERE s > 0 |> SELECT n",
        " |> WHERE s > 0 AND n < 0 |> SELECT n",
    ] {
        queries.reject(&format!("{GLOBAL}{suffix}"), "mean-only", Failure::Overflow)?;
    }
    for sql in [
        format!("{GLOBAL} |> SELECT l_quantity"),
        format!("{GLOBAL} |> SELECT n AS a, a AS a |> WHERE a > 0"),
        format!("{GLOBAL} |> SELECT a AS x, a AS x |> SELECT x"),
        format!("{GLOBAL} |> SELECT n |> WHERE a > 0"),
        format!("{GLOBAL} |> WHERE n < DATE '2000-01-01' |> SELECT a"),
        "FROM lineitem |> AGGREGATE SUM(missing) AS s, COUNT(*) AS n |> SELECT n".into(),
        "FROM lineitem |> AGGREGATE SUM(l_shipdate) AS s, COUNT(*) AS n |> SELECT n".into(),
    ] {
        queries.reject(&sql, "empty", Failure::Preparation)?;
    }
    Ok(())
}
