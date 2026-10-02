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
    for (literal, class, message, fragment) in [
        (
            "DATE '1900-02-29'",
            "bind",
            "invalid DATE literal",
            Some("'1900-02-29'"),
        ),
        (
            "DATE '0000-01-01'",
            "bind",
            "invalid DATE literal",
            Some("'0000-01-01'"),
        ),
        (
            "DATE '9999-13-01'",
            "bind",
            "invalid DATE literal",
            Some("'9999-13-01'"),
        ),
        (
            "DATE_ADD(DATE '9999-12-31', INTERVAL 1 DAY)",
            "bind",
            "DATE is outside supported calendar",
            Some("'9999-12-31'"),
        ),
        (
            "DATE_SUB(DATE '0001-01-01', INTERVAL 1 MONTH)",
            "bind",
            "DATE is outside supported calendar",
            Some("'0001-01-01'"),
        ),
        (
            "DATE_ADD(DATE '1970-01-01', INTERVAL 9223372036854775807 YEAR)",
            "bind",
            "DATE is outside supported calendar",
            Some("'1970-01-01'"),
        ),
        (
            "DATE_ADD(DATE '1970-01-01', INTERVAL 1.5 DAY)",
            "bind",
            "INTERVAL requires an INT64 integer",
            Some("1.5"),
        ),
        (
            "DATE_ADD(DATE '1970-01-01', INTERVAL 1 HOUR)",
            "bind",
            "unsupported DATE interval unit",
            Some("HOUR"),
        ),
        (
            "DATE_ADD(DATE '1970-01-01', INTERVAL 9223372036854775808 DAY)",
            "bind",
            "INTERVAL exceeds INT64",
            Some("9223372036854775808"),
        ),
        (
            "DATE_SUB(DATE '1970-01-01', INTERVAL -9223372036854775808 DAY)",
            "bind",
            "DATE interval overflow",
            Some("9223372036854775808"),
        ),
        // The nesting diagnostic points at EOF, not at a particular DATE token.
        (&nested, "parse", "DATE nesting limit exceeded", None),
    ] {
        let sql =
            format!("FROM lineitem |> WHERE l_shipdate < {literal} |> AGGREGATE COUNT(*) AS n");
        let span = match fragment {
            Some(fragment) => source_range(&sql, fragment)?,
            None => sql.len()..sql.len(),
        };
        queries.preparation("empty", &sql, class, message, span)?;
    }
    for (predicate, column) in [
        ("l_quantity < DATE '1994-01-01'", "l_quantity"),
        ("l_shipdate < 1994", "l_shipdate"),
    ] {
        let sql = format!("FROM lineitem |> WHERE {predicate} |> AGGREGATE COUNT(*) AS n");
        queries.preparation(
            "empty",
            &sql,
            "bind",
            "comparison operand types disagree",
            source_range(&sql, column)?,
        )?;
    }
    for (comparisons, pipe, message) in [
        (16, "|> AGGREGATE", "stage limit exceeded"),
        (17, "|> WHERE", "normalized stage limit exceeded"),
    ] {
        let predicate = vec!["l_quantity > 0"; comparisons].join(" AND ");
        let sql = format!("FROM lineitem |> WHERE {predicate} |> AGGREGATE COUNT(*) AS n");
        let start = source_range(&sql, pipe)?.start;
        queries.preparation("empty", &sql, "parse", message, start..start + 2)?;
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
    // The invalid final row follows two complete 32,768-row scan blocks. The
    // scan publishes both blocks before demanding that row; aggregation publishes
    // nothing until all input has been consumed.
    let mut prefix = String::new();
    for row in &source[..source.len() - 1] {
        use std::fmt::Write;
        writeln!(
            prefix,
            "row=double:{}:{:016x}",
            quantity(row),
            row.numbers[0]
        )?;
    }
    for (sql, schema, prefix) in [
        (
            "FROM lineitem |> WHERE l_shipdate >= DATE '0001-01-01' |> SELECT l_quantity",
            "l_quantity:double:required",
            prefix.as_str(),
        ),
        (
            "FROM lineitem |> WHERE l_shipdate >= DATE '0001-01-01' |> AGGREGATE COUNT(*) AS n",
            "n:int64:required",
            "",
        ),
    ] {
        queries.corrupt(
            sql,
            "bad-date",
            "stored DATE is outside supported calendar",
            schema,
            prefix,
        )?;
    }
    queries.check("undemanded-stored-date", "FROM lineitem |> WHERE l_quantity < 0 AND l_shipdate >= DATE '0001-01-01' |> AGGREGATE COUNT(*) AS n", vec![vec![integer(0)]], "bad-date", true)?;
    Ok(())
}
