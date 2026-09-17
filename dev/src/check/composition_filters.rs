//! Check predicate composition with text, NULL membership and short-circuiting.
//!
//! Literal totals distinguish IN from NOT IN when NULL is present, and cover text
//! comparisons and predicates composed with Boolean operators. Deliberately failing
//! expressions establish which branches are demanded: NULL does not automatically
//! permit skipping an error. Other cases require an unused computation to remain
//! unevaluated. Success and expected failures use separate complete-output checks.

use super::*;
fn optional(value: Option<f64>) -> String {
    value.map_or_else(|| "null".into(), double)
}

pub(super) fn run(queries: &mut Queries) -> Result<()> {
    for (predicate, total) in [
        ("l_quantity IN (10, 90, 10)", Some(100.0)),
        ("NOT l_quantity IN (10, NULL)", None),
        ("l_quantity IN (NULL)", None),
        ("l_returnflag IN ('B', NULL)", Some(90.0)),
        ("NOT l_returnflag IN ('A')", Some(90.0)),
        ("l_shipdate IN (NULL, DATE '1970-01-01')", Some(120.0)),
        ("NOT l_shipdate IN (NULL)", None),
    ] {
        queries.check(
            "legacy-membership-filter",
            &format!("FROM lineitem |> WHERE {predicate} |> AGGREGATE SUM(l_quantity) AS total"),
            vec![vec![optional(total)]],
            "repeated",
            true,
        )?;
    }
    // UTF-8 byte ordering agrees with scalar ordering for these literals. The
    // expected sum comes directly from the three input pairs, without parsing SQL.
    type Compare = fn(&str, &str) -> bool;
    for (symbol, compare) in [
        ("<", (|a, b| a < b) as Compare),
        ("<=", |a, b| a <= b),
        ("=", |a, b| a == b),
        ("!=", |a, b| a != b),
        (">=", |a, b| a >= b),
        (">", |a, b| a > b),
    ] {
        for (literal, target) in [
            ("'A'", "A"),
            (r"'\x41'", "A"),
            ("\"A\"", "A"),
            ("'é'", "é"),
            ("''", ""),
        ] {
            let mut total = None;
            for (value, key) in [(10.0, "A"), (20.0, "A"), (90.0, "B")] {
                if compare(key, target) {
                    total = Some(total.unwrap_or(0.0) + value);
                }
            }
            queries.check("stored-text-filter", &format!("FROM lineitem |> WHERE l_returnflag {symbol} {literal} |> AGGREGATE SUM(l_quantity) AS total"), vec![vec![optional(total)]], "repeated", true)?;
        }
    }
    queries.check("grouped-text-filter", "FROM lineitem |> AGGREGATE SUM(l_quantity) AS total GROUP AND ORDER BY l_returnflag |> WHERE l_returnflag = 'B' |> SELECT total", vec![vec![double(90.0)]], "repeated", true)?;
    queries.check(
        "escaped-date-filter",
        r"FROM lineitem |> WHERE l_shipdate = DATE '\x31970-01-01' |> AGGREGATE COUNT(*) AS n",
        vec![vec![integer(3)]],
        "repeated",
        true,
    )?;
    for column in ["l_quantity", "l_returnflag", "l_shipdate"] {
        for (test, count) in [("IS NULL", 0), ("IS NOT NULL", 3)] {
            queries.check(
                "legacy-null-predicate",
                &format!("FROM lineitem |> WHERE {column} {test} |> AGGREGATE COUNT(*) AS n"),
                vec![vec![integer(count)]],
                "repeated",
                true,
            )?;
        }
    }
    for (test, expected) in [
        ("IS NULL", vec![vec!["null".into()]]),
        ("IS NOT NULL", vec![]),
    ] {
        queries.check(
            "empty-sum-null-predicate",
            &format!("FROM lineitem |> AGGREGATE SUM(l_quantity) AS x |> WHERE x {test}"),
            expected,
            "empty",
            true,
        )?;
    }
    for test in ["IS NULL", "IS NOT NULL"] {
        let output = queries.execute(
            &format!("FROM lineitem |> SELECT l_quantity*1e308 AS x |> WHERE x {test}"),
            "repeated",
        )?;
        failed(&output)?;
        if !std::str::from_utf8(&output.stderr.bytes)?.contains("arithmetic overflow")
            || std::str::from_utf8(&output.stdout.bytes)?
                .lines()
                .any(|line| line.starts_with("row=") || line == "status=queried")
        {
            return Err("NULL predicate hid a demanded overflow".into());
        }
        queries.count += 1;
    }
    for (predicate, expected) in [
        ("NOT l_quantity < 20", 110.0),
        ("l_quantity<15 OR l_quantity>50", 100.0),
        ("NOT (l_quantity<15 OR l_quantity>50)", 20.0),
        ("l_returnflag='A' OR l_quantity>50", 120.0),
        ("NOT l_returnflag='A'", 90.0),
        ("NOT l_shipdate < DATE '1970-01-01'", 120.0),
        ("l_quantity<15 OR l_quantity>50 AND l_returnflag='A'", 10.0),
        (
            "(l_quantity<15 OR l_quantity>50) AND l_returnflag='A'",
            10.0,
        ),
    ] {
        queries.check(
            "legacy-boolean-filter",
            &format!("FROM lineitem |> WHERE {predicate} |> AGGREGATE SUM(l_quantity) AS total"),
            vec![vec![double(expected)]],
            "repeated",
            true,
        )?;
    }
    for (predicate, expected) in [
        ("l_quantity<0 AND x>0", vec![]),
        (
            "l_quantity>=0 OR x>0",
            vec![vec![double(10.0)], vec![double(20.0)], vec![double(90.0)]],
        ),
    ] {
        queries.check("boolean-skipped-computation", &format!("FROM lineitem |> SELECT l_quantity, l_quantity*1e308 AS x |> WHERE {predicate} |> SELECT l_quantity"), expected, "repeated", true)?;
    }
    Ok(())
}
