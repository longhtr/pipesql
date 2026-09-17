//! Check composed versions of the public analytical report with literal answers.
//!
//! The expected groups retain duplicate dimension matches, unmatched labels, NULLs,
//! empty strings and calendar boundaries. Query variants rename or rearrange values
//! while preserving their identities, and DOUBLE projections compare encoded bits.
//! Binding failures and demanded arithmetic errors require the specified diagnostic
//! and byte span. Input construction is shared with the example; the expected report
//! rows remain independently stated here.

use super::*;

const SCHEMA: &str = "calendar_year:int64:nullable|label:string:nullable|entries:int64:required|present:int64:required|total:int64:nullable";
// Dimension 2 matches south and 南. Unmatched dimensions retain a NULL label.
type Group = (Option<i64>, Option<&'static str>, i64, i64, Option<i64>);
const GROUPS: [Group; 13] = [
    (None, None, 1, 1, Some(11)),
    (None, Some(""), 1, 1, Some(5)),
    (None, Some("north"), 1, 1, Some(7)),
    (None, Some("south"), 1, 0, None),
    (None, Some("南"), 1, 0, None),
    (Some(1999), Some("north"), 2, 2, Some(8)),
    (Some(2000), None, 3, 2, Some(33)),
    (Some(2000), Some(""), 2, 2, Some(9)),
    (Some(2000), Some("south"), 2, 2, Some(15)),
    (Some(2000), Some("南"), 2, 2, Some(15)),
    (Some(2001), Some("north"), 2, 1, Some(13)),
    (Some(2001), Some("south"), 1, 1, Some(40)),
    (Some(2001), Some("南"), 1, 1, Some(40)),
];
fn optional(value: Option<i64>) -> String {
    value.map_or_else(|| "null".into(), integer)
}
fn check(queries: &mut Queries, label: &str, sql: &str, expected: &[Vec<String>]) -> Result<()> {
    let output = queries.execute(sql, "report")?;
    verify(&output, expected, true, Some(SCHEMA))?;
    queries.count += 1;
    println!("composition report-{label}: {} rows passed", expected.len());
    Ok(())
}

pub(super) fn run(queries: &mut Queries) -> Result<()> {
    let example = queries.run.build("pipesql", "--example", "event_report")?;
    let mut command = Command::new(example);
    command.arg(queries.run.directory.join("report"));
    let output = queries
        .run
        .command(&mut command, None, Duration::from_secs(60))?;
    output.require_success()?;
    if !output.stderr.bytes.is_empty()
        || !std::str::from_utf8(&output.stdout.bytes)?
            .lines()
            .any(|line| line == "status=finished")
    {
        return Err("event report example did not finish cleanly".into());
    }
    let source = fs::read_to_string(queries.run.root.join("examples/event_report.sql"))?;
    let report = source.trim().trim_end_matches(';');
    let expected: Vec<Vec<String>> = GROUPS
        .iter()
        .map(|&(year, label, entries, present, total)| {
            vec![
                optional(year),
                label.map_or_else(
                    || "null".into(),
                    |s| {
                        format!(
                            "string:{}",
                            s.as_bytes()
                                .iter()
                                .map(|b| format!("{b:02x}"))
                                .collect::<String>()
                        )
                    },
                ),
                integer(entries),
                integer(present),
                optional(total),
            ]
        })
        .collect();
    let projection = " |> SELECT calendar_year, label, entries, present, total";
    let renamed = format!("{report} |> RENAME total AS amount_sum |> RENAME amount_sum AS total");
    let output = queries.execute(&renamed, "report")?;
    verify(&output, &expected, true, Some(SCHEMA))?;
    queries.count += 1;
    let mut wrong = expected.clone();
    wrong[5][4] = integer(9);
    if verify(&output, &wrong, true, Some(SCHEMA)).is_ok() {
        return Err("report checker accepted a wrong total".into());
    }
    queries.count += 1;
    for (label, sql) in [
        ("literal", report.to_owned()),
        ("projection", format!("{report}{projection}")),
        (
            "derived-result",
            format!(
                "FROM ({report}) AS r{projection} |> ORDER BY calendar_year NULLS FIRST, label NULLS FIRST"
            ),
        ),
        (
            "derived-source",
            report.replace(
                "FROM events AS f",
                "FROM (FROM events |> SELECT id, dimension_id, happened, amount, measurement) AS f",
            ),
        ),
        (
            "derived-dimension",
            report.replace(
                "JOIN dimensions AS d",
                "JOIN (FROM dimensions |> SELECT id, label) AS d",
            ),
        ),
        (
            "null-partition",
            format!("{report} |> WHERE total IS NULL OR total IS NOT NULL"),
        ),
        (
            "boolean-short-circuit",
            format!(
                "{report} |> EXTEND SQRT(-entries) AS bad |> WHERE entries > 0 OR bad > 0{projection}"
            ),
        ),
        (
            "derived-pruned-error",
            format!(
                "FROM ({report} |> EXTEND SQRT(-1) AS unused) AS r{projection} |> ORDER BY calendar_year NULLS FIRST, label NULLS FIRST"
            ),
        ),
        (
            "pruned-domain-error",
            format!("{report} |> EXTEND SQRT(-1) AS unused{projection}"),
        ),
    ] {
        check(queries, label, &sql, &expected)?;
    }
    for (label, predicate, selected) in [
        ("null-total", "total IS NULL", vec![3, 4]),
        ("null-label", "label IS NULL", vec![0, 6]),
        ("null-year", "calendar_year IS NULL", vec![0, 1, 2, 3, 4]),
        ("empty-label", "label = ''", vec![1, 7]),
    ] {
        check(
            queries,
            label,
            &format!("{report} |> WHERE {predicate}"),
            &selected
                .iter()
                .map(|&i| expected[i].clone())
                .collect::<Vec<_>>(),
        )?;
    }
    for (label, transform, ordinary, increment) in [
        ("rename-range", "RENAME amount AS adjusted", "adjusted", 0),
        ("set-range", "SET amount = amount + 1", "amount", 1),
    ] {
        let amounts = [
            Some(10),
            Some(20),
            Some(30),
            None,
            Some(5),
            None,
            Some(-5),
            Some(7),
            Some(-2),
            Some(40),
            Some(3),
            Some(11),
            Some(0),
            Some(13),
            None,
            Some(9),
        ];
        queries.check(
            label,
            &format!(
                "FROM events AS f |> {transform} |> ORDER BY f.id |> SELECT {ordinary}, f.amount"
            ),
            amounts
                .iter()
                .map(|&v| vec![optional(v.map(|n| n + increment)), optional(v)])
                .collect(),
            "report",
            true,
        )?;
    }
    queries.check(
        "report-exact-integer-consumer",
        &format!("{report} |> SELECT COALESCE(total, 0) + 9007199254740993 AS shifted"),
        GROUPS
            .iter()
            .map(|g| vec![integer(g.4.unwrap_or(0) + 9007199254740993)])
            .collect(),
        "report",
        true,
    )?;
    queries.check(
        "report-coalesce-skips-error",
        &format!("{report} |> SELECT COALESCE(entries, SQRT(-1)) AS count_value"),
        GROUPS.iter().map(|g| vec![double(g.2 as f64)]).collect(),
        "report",
        true,
    )?;
    let bits = [
        "3fe0000000000000",
        "3ff8000000000000",
        "null",
        "8000000000000000",
        "4000000000000000",
        "4004000000000000",
        "4008000000000000",
        "bff0000000000000",
        "4010000000000000",
        "4012000000000000",
        "4014000000000000",
        "null",
        "4018000000000000",
        "401a000000000000",
        "401c000000000000",
        "c000000000000000",
    ];
    queries.check("report-renamed-double-bits", "FROM (FROM events |> RENAME measurement AS reading) AS f |> ORDER BY id |> SELECT reading, f.reading", bits.iter().map(|b| vec![(*b).into(), (*b).into()]).collect(), "report", true)?;
    for (sql, token, message) in [
        (
            "FROM events AS f |> SELECT amount |> SELECT f.amount",
            "f.amount",
            "table alias is not visible",
        ),
        (
            "FROM events |> SELECT amount AS v, amount AS v |> SELECT v",
            "v",
            "ambiguous column name",
        ),
    ] {
        let start = sql.rfind(token).unwrap();
        let output = queries.execute(sql, "report")?;
        failed(&output)?;
        if !output.stdout.bytes.is_empty()
            || output.stderr.bytes
                != format!(
                    "database error: bind error at bytes {start}..{}: {message}\n",
                    start + token.len()
                )
                .as_bytes()
        {
            return Err("report binding diagnostic or span differs".into());
        }
        queries.count += 1;
    }
    for expression in [
        "SAFE_DIVIDE(SQRT(-amount), 0)",
        "COALESCE(SAFE_DIVIDE(1, 0), SQRT(-amount))",
    ] {
        let sql = format!("FROM events |> WHERE id = 0 |> SELECT {expression} AS bad");
        let start = sql.find(expression).unwrap();
        let output = queries.execute(&sql, "report")?;
        failed(&output)?;
        if std::str::from_utf8(&output.stdout.bytes)?.lines().any(|line| line.starts_with("row=") || line == "status=queried") || output.stderr.bytes != format!("database error: arithmetic domain error during square root at bytes {start}..{}\n", start + expression.len()).as_bytes() { return Err("report demanded error or span differs".into()); }
        queries.count += 1;
    }
    Ok(())
}
