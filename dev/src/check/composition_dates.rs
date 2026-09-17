//! Check date comparisons and evaluation demand across composed query stages.
//!
//! Reference day offsets and predicates select expected rows independently of the
//! engine. Calendar constants test accepted boundaries and invalid dates. Stored
//! out-of-range dates distinguish a demanded failure from a branch that must not
//! read the value. Each query goes through the common complete-result checker;
//! error cases additionally require the intended diagnostic and failure boundary.

use super::*;

pub(super) fn run(queries: &mut Queries, source: &[Row]) -> Result<()> {
    let bound = day_offset(1994, 1, 1);
    type Compare = fn(i32, i32) -> bool;
    for (symbol, compare) in [
        ("<", (|a, b| a < b) as Compare),
        ("<=", |a, b| a <= b),
        ("=", |a, b| a == b),
        ("!=", |a, b| a != b),
        (">=", |a, b| a >= b),
        (">", |a, b| a > b),
    ] {
        let count = source.iter().filter(|r| compare(r.day, bound)).count();
        queries.check(&format!("date-{symbol}"), &format!("FROM lineitem |> WHERE l_shipdate {symbol} DATE '1994-01-01' |> AGGREGATE COUNT(*) AS n"), vec![vec![integer(count as i64)]], "data", true)?;
    }
    let nested = format!(
        "{}DATE '1994-01-01'{}",
        "DATE_ADD(".repeat(8),
        ", INTERVAL 0 DAY)".repeat(8)
    );
    for (literal, (year, month, day)) in [
        (
            "DATE_ADD(DATE '2020-01-31', INTERVAL 1 MONTH)",
            (2020, 2, 29),
        ),
        (
            "DATE_ADD(DATE '2020-02-29', INTERVAL 1 YEAR)",
            (2021, 2, 28),
        ),
        (
            "DATE_SUB(DATE '1995-01-01', INTERVAL 1 DAY)",
            (1994, 12, 31),
        ),
        (
            "DATE_ADD(DATE '1995-01-01', INTERVAL -1 DAY)",
            (1994, 12, 31),
        ),
        ("DATE_SUB(DATE '1993-12-31', INTERVAL -1 DAY)", (1994, 1, 1)),
        (
            "DATE_SUB(DATE_ADD(DATE '2020-02-29', INTERVAL 1 YEAR), INTERVAL 1 YEAR)",
            (2020, 2, 28),
        ),
        ("DATE '0001-01-01'", (1, 1, 1)),
        ("DATE '9999-12-31'", (9999, 12, 31)),
        (&nested, (1994, 1, 1)),
    ] {
        let bound = day_offset(year, month, day);
        let count = source.iter().filter(|r| r.day <= bound).count();
        queries.check(
            &format!("date-fold-{literal}"),
            &format!("FROM lineitem |> WHERE l_shipdate <= {literal} |> AGGREGATE COUNT(*) AS n"),
            vec![vec![integer(count as i64)]],
            "data",
            true,
        )?;
    }
    for (label, sql, count) in [
        (
            "date-alias",
            "FROM lineitem |> SELECT l_shipdate AS ship |> WHERE ship >= DATE '1994-01-01' AND ship < DATE '1995-01-01' |> AGGREGATE COUNT(*) AS n",
            source
                .iter()
                .filter(|r| (8766..9131).contains(&r.day))
                .count(),
        ),
        (
            "date-between",
            "FROM lineitem |> WHERE l_shipdate BETWEEN DATE '1994-01-01' AND DATE '1994-12-31' |> AGGREGATE COUNT(*) AS n",
            source
                .iter()
                .filter(|r| (8766..=9130).contains(&r.day))
                .count(),
        ),
        (
            "reversed-between",
            "FROM lineitem |> WHERE l_quantity BETWEEN 25 AND 2 |> AGGREGATE COUNT(*) AS n",
            0,
        ),
    ] {
        queries.check(label, sql, vec![vec![integer(count as i64)]], "data", true)?;
    }
    let nested = format!(
        "{}DATE '1994-01-01'{}",
        "DATE_ADD(".repeat(9),
        ", INTERVAL 0 DAY)".repeat(9)
    );
    let mut predicates: Vec<String> = [
        "DATE '1900-02-29'",
        "DATE '0000-01-01'",
        "DATE '9999-13-01'",
        "DATE_ADD(DATE '9999-12-31', INTERVAL 1 DAY)",
        "DATE_SUB(DATE '0001-01-01', INTERVAL 1 MONTH)",
        "DATE_ADD(DATE '1970-01-01', INTERVAL 9223372036854775807 YEAR)",
        "DATE_ADD(DATE '1970-01-01', INTERVAL 1.5 DAY)",
        "DATE_ADD(DATE '1970-01-01', INTERVAL 1 HOUR)",
        &nested,
    ]
    .iter()
    .map(|literal| format!("l_shipdate < {literal}"))
    .collect();
    predicates.extend([
        "l_quantity < DATE '1994-01-01'".into(),
        "l_shipdate < 1994".into(),
        ["l_quantity > 0"; 17].join(" AND "),
    ]);
    for predicate in predicates {
        let output = queries.execute(
            &format!("FROM lineitem |> WHERE {predicate} |> AGGREGATE COUNT(*) AS n"),
            "empty",
        )?;
        failed(&output)?;
        let error = std::str::from_utf8(&output.stderr.bytes)?;
        if !output.stdout.bytes.is_empty()
            || !(error.starts_with("database error: parse error")
                || error.starts_with("database error: bind error"))
        {
            return Err(format!(
                "invalid date/predicate did not fail preparation: {predicate}: {error}"
            )
            .into());
        }
        queries.count += 1;
    }
    let predicate = ["l_quantity > 0"; 15].join(" AND ");
    let count = source.iter().filter(|r| quantity(r) > 0.0).count();
    queries.check(
        "normalized-stage-bound",
        &format!("FROM lineitem |> WHERE {predicate} |> AGGREGATE COUNT(*) AS n"),
        vec![vec![integer(count as i64)]],
        "data",
        true,
    )?;
    let mut corrupt: Vec<_> = source
        .iter()
        .map(|r| Row {
            numbers: r.numbers,
            keys: r.keys,
            day: r.day,
        })
        .collect();
    corrupt.last_mut().unwrap().day = 2932897;
    snapshot::write(
        &queries.run.directory.join("bad-date"),
        &queries
            .run
            .root
            .join("test/data/current-single-table-format"),
        &corrupt,
    )?;
    for (mode, sql) in [
        (
            "scan",
            "FROM lineitem |> WHERE l_shipdate >= DATE '0001-01-01' |> SELECT l_quantity",
        ),
        (
            "aggregate",
            "FROM lineitem |> WHERE l_shipdate >= DATE '0001-01-01' |> AGGREGATE COUNT(*) AS n",
        ),
    ] {
        let output = queries.execute(sql, "bad-date")?;
        failed(&output)?;
        let text = std::str::from_utf8(&output.stdout.bytes)?;
        if !std::str::from_utf8(&output.stderr.bytes)?.contains("stored DATE")
            || text.lines().any(|line| line == "status=queried")
            || text.lines().any(|line| line.starts_with("row=")) != (mode == "scan")
        {
            return Err(format!("invalid stored date lost its {mode} failure boundary").into());
        }
        queries.count += 1;
    }
    queries.check("undemanded-stored-date", "FROM lineitem |> WHERE l_quantity < 0 AND l_shipdate >= DATE '0001-01-01' |> AGGREGATE COUNT(*) AS n", vec![vec![integer(0)]], "bad-date", true)?;
    Ok(())
}
