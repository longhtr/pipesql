//! Check composed SQL through the stock CLI against independent row calculations.
//!
//! Queries owns generated databases, command execution and result comparison; child
//! modules select related SQL cases. Expected groups are built from input rows here,
//! without the engine's planners or aggregates. Verification requires process
//! success, complete row shape and a matching completion count; ordered queries also
//! require row order. Failure cases check their expected error boundary separately,
//! and unknown selections fail before a campaign can report an empty success.

use crate::process::{Completion, Output};
use crate::{
    Result,
    oracle::snapshot::{self, Row},
    workspace::{self, Run},
};
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command, time::Duration};

fn failed(output: &Output) -> Result<()> {
    if !matches!(output.completion, Completion::Exited(status) if status.code() == Some(1))
        || output.stdout.omitted != 0
        || output.stderr.omitted != 0
    {
        return Err("query case needs an ordinary failure with complete output".into());
    }
    Ok(())
}

#[path = "composition_dates.rs"]
mod dates;
#[path = "composition_derived.rs"]
mod derived;
#[path = "composition_expressions.rs"]
mod expressions;
#[path = "composition_filters.rs"]
mod filters;
#[path = "composition_grouping.rs"]
mod grouping;
#[path = "composition_numeric.rs"]
mod numeric;
#[path = "composition_report.rs"]
mod report;
#[path = "composition_stages.rs"]
mod stages;
#[path = "composition_storage.rs"]
mod storage;

struct Queries {
    run: Run,
    cli: PathBuf,
    count: usize,
    memory: &'static str,
    temporary: &'static str,
}

enum Failure {
    Preparation,
    Overflow,
    StoredKey,
    PayloadChecksum,
}

impl Queries {
    fn reject(&mut self, sql: &str, database: &str, expected: Failure) -> Result<()> {
        let output = self.execute(sql, database)?;
        failed(&output)?;
        let text = std::str::from_utf8(&output.stdout.bytes)?;
        let error = std::str::from_utf8(&output.stderr.bytes)?;
        let matches = match expected {
            Failure::Preparation => {
                text.is_empty()
                    && (error.starts_with("database error: parse error")
                        || error.starts_with("database error: bind error"))
            }
            Failure::Overflow | Failure::StoredKey | Failure::PayloadChecksum => {
                let diagnostic = match expected {
                    Failure::Overflow => "arithmetic overflow",
                    Failure::StoredKey => "stored key is outside printable ASCII domain",
                    Failure::PayloadChecksum => "demanded unit payload checksum failed",
                    Failure::Preparation => unreachable!(),
                };
                error.contains(diagnostic)
                    && !text
                        .lines()
                        .any(|line| line.starts_with("row=") || line == "status=queried")
            }
        };
        if !matches {
            return Err(
                format!("query did not fail at the expected boundary: {sql}: {error}").into(),
            );
        }
        self.count += 1;
        Ok(())
    }
    fn seed(&self, name: &str, values: &[(f64, u8)]) -> Result<()> {
        let input: Vec<_> = values
            .iter()
            .map(|&(value, key)| Row {
                numbers: [value.to_bits(), 1.0_f64.to_bits(), 0, 0],
                keys: [key, b'F'],
                day: 0,
            })
            .collect();
        snapshot::write(
            &self.run.directory.join(name),
            &self.run.root.join("test/data/current-single-table-format"),
            &input,
        )
    }
    fn execute(&mut self, sql: &str, database: &str) -> Result<crate::process::Output> {
        let path = self.run.directory.join("query.sql");
        fs::write(&path, sql)?;
        let mut command = Command::new(&self.cli);
        command
            .arg("query")
            .arg("--database")
            .arg(self.run.directory.join(database))
            .args([
                "--memory-limit-bytes",
                self.memory,
                "--temp-limit-bytes",
                self.temporary,
                "--query-file",
            ])
            .arg(path);
        self.run
            .command(&mut command, None, Duration::from_secs(30))
    }

    fn check(
        &mut self,
        label: &str,
        sql: &str,
        expected: Vec<Vec<String>>,
        database: &str,
        ordered: bool,
    ) -> Result<()> {
        let output = self.execute(sql, database)?;
        verify(&output, &expected, ordered, None)?;
        self.count += 1;
        println!("composition {label}: {} rows passed", expected.len());
        Ok(())
    }
}

fn verify(
    output: &Output,
    expected: &[Vec<String>],
    ordered: bool,
    schema: Option<&str>,
) -> Result<()> {
    output.require_success()?;
    let text = std::str::from_utf8(&output.stdout.bytes)?;
    let mut actual = rows(text)?;
    if let Some(schema) = schema
        && text
            .lines()
            .filter_map(|line| line.strip_prefix("columns="))
            .collect::<Vec<_>>()
            != [schema]
    {
        return Err("composition result schema differs".into());
    }
    let mut expected = expected.to_vec();
    if !ordered {
        actual.sort();
        expected.sort();
    }
    if actual != expected {
        return Err(format!(
            "composition complete rows differ (actual {}, expected {})",
            actual.len(),
            expected.len()
        )
        .into());
    }
    Ok(())
}

fn rows(text: &str) -> Result<Vec<Vec<String>>> {
    let lines: Vec<_> = text.lines().collect();
    if lines.first() != Some(&"status=querying") || lines.last() != Some(&"status=queried") {
        return Err("query did not complete".into());
    }
    if lines.len() < 8
        || !lines[1].starts_with("database=")
        || !lines[2].starts_with("memory_limit_bytes=")
        || !lines[3].starts_with("temp_limit_bytes=")
        || !lines[4].starts_with("column_count=")
        || !lines[5].starts_with("columns=")
        || lines[6..lines.len() - 2]
            .iter()
            .any(|line| !line.starts_with("row="))
    {
        return Err("query headers and rows are out of order".into());
    }
    let mut rows = Vec::new();
    for line in &lines {
        if let Some(row) = line.strip_prefix("row=") {
            rows.push(
                row.split('|')
                    .map(|cell| {
                        if let Some(value) = cell.strip_prefix("double:") {
                            let (display, bits) =
                                value.split_once(':').ok_or("missing DOUBLE bits")?;
                            if bits.len() != 16 || !bits.bytes().all(|b| b.is_ascii_hexdigit()) {
                                return Err("invalid DOUBLE bits".into());
                            }
                            let number = f64::from_bits(u64::from_str_radix(bits, 16)?);
                            // NaN text cannot carry its sign or payload. Every other
                            // display must identify these exact bits, including -0.
                            let agrees = if number.is_nan() {
                                display == "NaN"
                            } else if number.is_infinite() {
                                display
                                    == if number.is_sign_negative() {
                                        "-inf"
                                    } else {
                                        "inf"
                                    }
                            } else {
                                !display.is_empty()
                                    && display.trim() == display
                                    && display.parse::<f64>()?.to_bits() == number.to_bits()
                            };
                            if !agrees {
                                return Err("DOUBLE display disagrees with bits".into());
                            }
                            Ok(bits.to_ascii_lowercase())
                        } else {
                            Ok(cell.to_owned())
                        }
                    })
                    .collect::<Result<Vec<_>>>()?,
            );
        }
    }
    let count = format!("row_count={}", rows.len());
    let completion: Vec<_> = lines
        .iter()
        .copied()
        .filter(|line| line.starts_with("row_count=") || line.starts_with("status="))
        .collect();
    if completion != ["status=querying", &count, "status=queried"]
        || !lines.ends_with(&[&count, "status=queried"])
    {
        return Err("query completion disagrees with rows".into());
    }
    let widths: Vec<_> = lines
        .iter()
        .filter_map(|line| line.strip_prefix("column_count="))
        .collect();
    if widths.len() != 1 {
        return Err("query needs one column count".into());
    }
    let width: usize = widths[0].parse()?;
    if width == 0
        || lines[5]["columns=".len()..].split('|').count() != width
        || rows.iter().any(|row| row.len() != width)
    {
        return Err("query row shape differs from header".into());
    }
    Ok(rows)
}

fn double(number: f64) -> String {
    format!("{:016x}", number.to_bits())
}
fn integer(number: i64) -> String {
    format!("int64:{number}")
}
fn quantity(row: &Row) -> f64 {
    f64::from_bits(row.numbers[0])
}
fn price(row: &Row) -> f64 {
    f64::from_bits(row.numbers[1])
}
fn discount(row: &Row) -> f64 {
    f64::from_bits(row.numbers[2])
}
fn tax(row: &Row) -> f64 {
    f64::from_bits(row.numbers[3])
}

enum Aggregate {
    Sum(fn(&Row) -> f64),
    Average(fn(&Row) -> f64),
    Count,
}
fn groups(rows: &[&Row], keys: &[usize], entries: &[Aggregate]) -> Vec<Vec<String>> {
    let mut groups: BTreeMap<Vec<u8>, Vec<&Row>> = BTreeMap::new();
    if keys.is_empty() {
        groups.insert(Vec::new(), Vec::new());
    }
    for row in rows {
        groups
            .entry(keys.iter().map(|key| row.keys[*key]).collect())
            .or_default()
            .push(row);
    }
    groups
        .into_iter()
        .map(|(key, rows)| {
            let mut result: Vec<_> = key
                .into_iter()
                .map(|byte| format!("string:{byte:02x}"))
                .collect();
            for entry in entries {
                result.push(match entry {
                    Aggregate::Count => integer(rows.len() as i64),
                    Aggregate::Sum(value) | Aggregate::Average(value) => {
                        if rows.is_empty() {
                            "null".into()
                        } else {
                            // Corpus values have small exact binary sums. Starting with
                            // the first value also preserves a sum of negative zeroes.
                            let mut sum = value(rows[0]);
                            for row in &rows[1..] {
                                sum += value(row);
                            }
                            if matches!(entry, Aggregate::Average(_)) {
                                sum /= rows.len() as f64;
                            }
                            double(sum)
                        }
                    }
                });
            }
            result
        })
        .collect()
}

const DATES: [(i32, i32, i32); 15] = [
    (1, 1, 1),
    (1900, 2, 28),
    (1969, 12, 31),
    (1970, 1, 1),
    (1993, 12, 31),
    (1994, 1, 1),
    (1994, 6, 15),
    (1994, 12, 31),
    (1995, 1, 1),
    (1998, 9, 17),
    (1998, 9, 18),
    (1998, 9, 19),
    (2020, 2, 29),
    (2021, 2, 28),
    (9999, 12, 31),
];
fn day_offset(year: i32, month: i32, day: i32) -> i32 {
    let prior = year - 1;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let before_month = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334][month as usize - 1];
    365 * prior + prior / 4 - prior / 100
        + prior / 400
        + before_month
        + i32::from(leap && month > 2)
        + day
        - 1
        - 719162
}
fn source_rows() -> Vec<Row> {
    let keys: Vec<_> = (32_u8..=126).filter(|byte| *byte != b'|').collect();
    (0..65537)
        .map(|index| {
            let (year, month, day) = DATES[index % DATES.len()];
            Row {
                numbers: [
                    (index % 31) as f64,
                    (index % 97 + 1) as f64,
                    (index % 17) as f64 / 128.0,
                    (index % 9) as f64 / 4.0,
                ]
                .map(f64::to_bits),
                keys: [keys[index % 94], keys[(index / 94) % 94]],
                day: day_offset(year, month, day),
            }
        })
        .collect()
}

pub fn run(case: &str) -> Result<()> {
    if ![
        "grouping",
        "expressions",
        "dates",
        "numeric",
        "derived",
        "filters",
        "columns",
        "repeated",
        "demand",
        "storage",
        "unions",
        "report",
    ]
    .contains(&case)
    {
        return Err("unknown composition case".into());
    }
    let mut run = Run::new(workspace::root()?, "composition")?;
    let cli = run.build("pipesql", "--bin", "pipesql")?;
    let source = if matches!(
        case,
        "derived" | "filters" | "columns" | "repeated" | "storage" | "unions" | "report"
    ) {
        Vec::new()
    } else {
        source_rows()
    };
    let retained = run.root.join("test/data/current-single-table-format");
    if !source.is_empty() {
        snapshot::write(&run.directory.join("data"), &retained, &source)?;
    }
    let mut create = Command::new(&cli);
    create
        .arg("create")
        .arg("--database")
        .arg(run.directory.join("empty"))
        .args([
            "--memory-limit-bytes",
            "2000000",
            "--temp-limit-bytes",
            "1000000",
        ]);
    run.command(&mut create, None, Duration::from_secs(30))?
        .require_success()?;
    let memory = match case {
        "unions" => "4000000",
        "report" => "16777216",
        _ => "2000000",
    };
    let mut queries = Queries {
        run,
        cli,
        count: 0,
        memory,
        temporary: if case == "report" {
            "8388608"
        } else {
            "1000000"
        },
    };
    if matches!(case, "derived" | "filters" | "columns" | "repeated") {
        queries.seed("repeated", &[(10.0, b'A'), (20.0, b'A'), (90.0, b'B')])?;
    }
    if matches!(case, "repeated" | "demand") {
        queries.seed("mean-only", &[(f64::MAX, b'A'), (f64::MAX, b'A')])?;
    }
    match case {
        "grouping" => grouping::run(&mut queries, &source)?,
        "expressions" => expressions::run(&mut queries, &source)?,
        "dates" => dates::run(&mut queries, &source)?,
        "numeric" => numeric::run(&mut queries)?,
        "derived" => derived::run(&mut queries)?,
        "filters" => filters::run(&mut queries)?,
        "columns" => stages::columns(&mut queries)?,
        "repeated" => stages::repeated(&mut queries)?,
        "demand" => stages::demand(&mut queries, &source)?,
        "storage" => storage::run(&mut queries)?,
        "unions" => storage::unions(&mut queries)?,
        "report" => report::run(&mut queries)?,
        _ => unreachable!(),
    }
    println!(
        "Composition {case}: {} independent checks passed",
        queries.count
    );
    queries.run.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::Capture;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn complete_answer_checks_schema_order_values_and_process_outcome() {
        let text = "status=querying\ndatabase=/test\nmemory_limit_bytes=2000000\ntemp_limit_bytes=1000000\ncolumn_count=1\ncolumns=n:int64:required\nrow=int64:1\nrow=int64:2\nrow_count=2\nstatus=queried\n";
        let mut output = Output {
            completion: Completion::Exited(std::process::ExitStatus::from_raw(0)),
            stdout: Capture {
                bytes: text.as_bytes().to_vec(),
                omitted: 0,
            },
            stderr: Capture::default(),
        };
        let expected = vec![vec![integer(1)], vec![integer(2)]];
        verify(&output, &expected, true, Some("n:int64:required")).unwrap();
        assert!(verify(&output, &expected, true, Some("n:int64:nullable")).is_err());
        let reversed = vec![vec![integer(2)], vec![integer(1)]];
        assert!(verify(&output, &reversed, true, None).is_err());
        verify(&output, &reversed, false, None).unwrap();
        assert!(verify(&output, &[vec![integer(1)], vec![integer(3)]], true, None).is_err());
        output.stdout.omitted = 1;
        assert!(verify(&output, &expected, true, None).is_err());
        output.stdout.omitted = 0;
        output.completion = Completion::TimedOut;
        assert!(verify(&output, &expected, true, None).is_err());
    }

    #[test]
    fn grouped_answers_distinguish_empty_global_keys_and_signed_zero() {
        let row = Row {
            numbers: [(-0.0_f64).to_bits(); 4],
            keys: *b"AF",
            day: 0,
        };
        assert_eq!(
            groups(&[], &[], &[Aggregate::Sum(quantity), Aggregate::Count]),
            vec![vec!["null", "int64:0"]]
        );
        assert!(groups(&[], &[0], &[Aggregate::Count]).is_empty());
        assert_eq!(
            groups(&[&row], &[0], &[Aggregate::Sum(quantity)]),
            vec![vec!["string:41", "8000000000000000"]]
        );
        assert_eq!(day_offset(1970, 1, 1), 0);
        assert_eq!(day_offset(1, 1, 1), -719162);
        assert_eq!(day_offset(9999, 12, 31), 2932896);
        assert_eq!(day_offset(2000, 3, 1) - day_offset(2000, 2, 28), 2);
        assert_eq!(day_offset(1900, 3, 1) - day_offset(1900, 2, 28), 1);
    }
    #[test]
    fn row_reader_rejects_missing_completion_wrong_count_and_shape() {
        let valid = "status=querying\ndatabase=/test\nmemory_limit_bytes=2000000\ntemp_limit_bytes=1000000\ncolumn_count=1\ncolumns=value:double:required\nrow=double:1:3ff0000000000000\nrow_count=1\nstatus=queried\n";
        assert_eq!(rows(valid).unwrap(), vec![vec!["3ff0000000000000"]]);
        for bad in [
            valid.replace("status=queried\n", ""),
            valid.replace("row_count=1", "row_count=2"),
            valid.replace("column_count=1", "column_count=2"),
            valid.replace("column_count=1", "column_count=1\ncolumn_count=1"),
            valid.replace("3ff0000000000000", "+3ff0000000000000"),
            valid.replace("double:1:", "double:2:"),
            valid.replace("double:1:", "double:NaN:"),
            valid.replace("double:1:", "double::"),
            valid.replace("columns=value:double:required\n", ""),
            valid.replace("row=double", "unexpected\nrow=double"),
            valid.replace(
                "columns=value:double:required",
                "columns=a:double:required|b:double:required",
            ),
        ] {
            assert!(rows(&bad).is_err());
        }
    }

    #[test]
    fn double_displays_preserve_signed_zero_infinity_and_nan_class() {
        for (display, bits, accepted) in [
            ("-0", "8000000000000000", true),
            ("0", "8000000000000000", false),
            ("inf", "7ff0000000000000", true),
            ("-inf", "fff0000000000000", true),
            ("inf", "fff0000000000000", false),
            ("NaN", "fff8000000001234", true),
            ("NaN", "7ff0000000000001", true),
            ("nan", "7ff8000000000000", false),
        ] {
            let text = format!(
                "status=querying\ndatabase=/test\nmemory_limit_bytes=2000000\ntemp_limit_bytes=1000000\ncolumn_count=1\ncolumns=n:double:nullable\nrow=double:{display}:{bits}\nrow_count=1\nstatus=queried\n"
            );
            assert_eq!(rows(&text).is_ok(), accepted, "{display}:{bits}");
        }
    }

    #[test]
    fn composition_input_covers_every_printable_key_pair() {
        let rows = source_rows();
        let keys: std::collections::BTreeSet<_> = rows.iter().map(|row| row.keys).collect();
        assert_eq!(keys.len(), 8_836);
        for first in b' '..=b'~' {
            for second in b' '..=b'~' {
                assert_eq!(
                    keys.contains(&[first, second]),
                    first != b'|' && second != b'|'
                );
            }
        }
    }
}
