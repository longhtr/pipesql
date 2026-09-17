//! Distinguish numeric preparation errors, demanded scalar errors and aggregation.
//!
//! Invalid syntax or types must fail before execution. Overflowing expressions are
//! placed where they are either demanded or unused to check that evaluation follows
//! the query. Exceptional aggregate values require successful completion and exact
//! shape before numeric comparison, including the appropriate NaN or infinity rule.
//! An error line or a plausible first value cannot stand in for a complete answer.

use super::*;
const OVERFLOW: &str = "l_quantity*(9223372036854775807+1)";

fn overflow(queries: &mut Queries, database: &str, sql: &str) -> Result<()> {
    let output = queries.execute(sql, database)?;
    failed(&output)?;
    let text = std::str::from_utf8(&output.stdout.bytes)?;
    if !std::str::from_utf8(&output.stderr.bytes)?.contains("arithmetic overflow")
        || text
            .lines()
            .any(|line| line.starts_with("row=") || line == "status=queried")
    {
        return Err(format!("numeric case did not preserve demanded overflow: {sql}").into());
    }
    queries.count += 1;
    Ok(())
}
fn invalid(queries: &mut Queries, sql: &str) -> Result<()> {
    let output = queries.execute(sql, "empty")?;
    failed(&output)?;
    let error = std::str::from_utf8(&output.stderr.bytes)?;
    if !output.stdout.bytes.is_empty()
        || !(error.starts_with("database error: parse error")
            || error.starts_with("database error: bind error"))
    {
        return Err(format!("numeric case did not fail preparation: {sql}: {error}").into());
    }
    queries.count += 1;
    Ok(())
}

pub(super) fn run(queries: &mut Queries) -> Result<()> {
    for (label, sql, database) in [
        (
            "filtered-argument-not-evaluated",
            format!(
                "FROM lineitem |> WHERE l_quantity < 0.0 |> AGGREGATE SUM({OVERFLOW}) AS value"
            ),
            "data",
        ),
        (
            "empty-argument-not-evaluated",
            format!("FROM lineitem |> AGGREGATE SUM({OVERFLOW}) AS value"),
            "empty",
        ),
    ] {
        queries.check(label, &sql, vec![vec!["null".into()]], database, true)?;
    }
    for expression in [
        OVERFLOW,
        "l_quantity*(-(-9223372036854775808))",
        "CAST(9223372036854775807 + 1 AS DOUBLE)",
    ] {
        overflow(
            queries,
            "data",
            &format!("FROM lineitem |> AGGREGATE SUM({expression}) AS value"),
        )?;
    }
    for expression in [
        "l_quantity+9223372036854775808".into(),
        "l_quantity+-9223372036854775809".into(),
        "l_quantity+".into(),
        format!("{}l_quantity{}", "(".repeat(33), ")".repeat(33)),
        ["l_quantity"; 17].join("+"),
    ] {
        invalid(
            queries,
            &format!("FROM lineitem |> AGGREGATE SUM({expression}) AS value"),
        )?;
    }
    for (label, values, mode, expected) in [
        ("mean-only", vec![f64::MAX, f64::MAX], "AVG", f64::MAX),
        (
            "sum-cancel",
            vec![f64::MAX, f64::MAX, -f64::MAX],
            "SUM",
            f64::MAX,
        ),
        (
            "late-nan",
            vec![f64::MAX, f64::MAX, f64::NAN],
            "SUM",
            f64::NAN,
        ),
        (
            "late-inf",
            vec![f64::MAX, f64::MAX, f64::INFINITY],
            "SUM",
            f64::INFINITY,
        ),
    ] {
        let input: Vec<_> = values
            .into_iter()
            .map(|value| Row {
                numbers: [value.to_bits(), 1.0_f64.to_bits(), 0, 0],
                keys: *b"AF",
                day: 0,
            })
            .collect();
        snapshot::write(
            &queries.run.directory.join(label),
            &queries
                .run
                .root
                .join("test/data/current-single-table-format"),
            &input,
        )?;
        let output = queries.execute(
            &format!("FROM lineitem |> AGGREGATE {mode}(l_quantity) AS value"),
            label,
        )?;
        exceptional(&output, expected)?;
        queries.count += 1;
    }
    overflow(
        queries,
        "late-nan",
        "FROM lineitem |> AGGREGATE SUM(l_quantity*2.0) AS value",
    )?;
    for sql in [
        "FROM lineitem |> AGGREGATE SUM(l_quantity) AS value",
        "FROM lineitem |> AGGREGATE AVG(l_quantity) AS mean, SUM(l_quantity) AS total",
    ] {
        overflow(queries, "mean-only", sql)?;
    }
    for sql in [
        "FROM lineitem |> AGGREGATE SUM(l_shipdate) AS x",
        "FROM lineitem |> AGGREGATE COUNT(*) AS n GROUP BY l_quantity",
        "FROM lineitem |> AGGREGATE COUNT(DISTINCT l_quantity) AS n",
    ] {
        invalid(queries, sql)?;
    }
    for (database, count) in [("empty", 0), ("mean-only", 2), ("late-nan", 3)] {
        queries.check(&format!("count-typed-arguments-{database}"), "FROM lineitem |> AGGREGATE COUNT(l_quantity) AS n, COUNT(l_returnflag) AS flags, COUNT(l_shipdate) AS dates", vec![vec![integer(count);3]], database, true)?;
    }
    Ok(())
}

fn exceptional(output: &Output, expected: f64) -> Result<()> {
    output.require_success()?;
    let actual = rows(std::str::from_utf8(&output.stdout.bytes)?)?;
    if actual.len() != 1 || actual[0].len() != 1 {
        return Err("exceptional aggregate has wrong shape".into());
    }
    let bits = u64::from_str_radix(&actual[0][0], 16)?;
    if if expected.is_nan() {
        !f64::from_bits(bits).is_nan()
    } else {
        bits != expected.to_bits()
    } {
        return Err("exceptional aggregate has incorrect value".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::Capture;
    use std::os::unix::process::ExitStatusExt;
    #[test]
    fn exceptional_answer_requires_success_shape_and_complete_bits() {
        let make = |text: String| Output {
            completion: Completion::Exited(std::process::ExitStatus::from_raw(0)),
            stdout: Capture {
                bytes: text.into_bytes(),
                omitted: 0,
            },
            stderr: Capture::default(),
        };
        let text = "status=querying\ndatabase=/test\nmemory_limit_bytes=2000000\ntemp_limit_bytes=1000000\ncolumn_count=1\ncolumns=n:double:nullable\nrow=double:NaN:7ff8000000000000\nrow_count=1\nstatus=queried\n";
        exceptional(&make(text.into()), f64::NAN).unwrap();
        for bad in [
            text.replace("status=queried\n", ""),
            text.replace("7ff8000000000000", "7ff0000000000000"),
            text.replace("7ff8000000000000", "+7ff8000000000000"),
            text.replace("column_count=1", "column_count=2"),
            text.replace("row_count=1", "row_count=0"),
        ] {
            assert!(exceptional(&make(bad), f64::NAN).is_err());
        }
        let mut failure = make(text.into());
        failure.completion = Completion::TimedOut;
        assert!(exceptional(&failure, f64::NAN).is_err());
    }
}
