//! Check exceptional aggregate boundaries through the ordinary CLI.
//!
//! Retained cases supply literal DOUBLE input bits and expected answers. The
//! independent snapshot writer builds each database without using the engine's
//! encoder, then the campaign runs the selected aggregate query. Comparison checks
//! every typed cell, the schema and completion. The separate rational oracle
//! supplies finite sums and bounded AVG answers; retained literals still constrain
//! their selected exceptional outcomes. Malformed case input and an unknown query fail the campaign
//! instead of silently reducing its selection.

use crate::{
    Result,
    oracle::snapshot::{self, Row},
    process::Completion,
    workspace::{self, Run},
};
use std::{fs, process::Command, time::Duration};

const Q1_SCHEMA: &str = "l_returnflag:string:required|l_linestatus:string:required|sum_qty:double:nullable|sum_base_price:double:nullable|sum_disc_price:double:nullable|sum_charge:double:nullable|avg_qty:double:nullable|avg_price:double:nullable|avg_disc:double:nullable|count_order:int64:required";
const Q6_SCHEMA: &str = "revenue:double:nullable";

enum Expected {
    Row(Answer),
    Overflow(String),
}

struct Answer {
    schema: &'static str,
    cells: Vec<String>,
}

fn answer(query: &str, rows: &[Row], column: usize, literal: &str) -> Result<Answer> {
    use crate::oracle::rounding::{average, sum};

    if rows.is_empty() || rows.iter().any(|row| row.keys != *b"AF" || row.day != 0) {
        return Err("aggregate fixture needs one nonempty AF group dated 1970-01-01".into());
    }
    let numbers: Vec<_> = rows
        .iter()
        .map(|row| row.numbers.map(f64::from_bits))
        .collect();
    let (schema, mut cells) = match query {
        "q1" => {
            let quantities: Vec<_> = numbers.iter().map(|n| n[0]).collect();
            let prices: Vec<_> = numbers.iter().map(|n| n[1]).collect();
            let discounts: Vec<_> = numbers.iter().map(|n| n[2]).collect();
            let discounted: Vec<_> = numbers.iter().map(|n| n[1] * (1.0 - n[2])).collect();
            let charged: Vec<_> = numbers
                .iter()
                .map(|n| n[1] * (1.0 - n[2]) * (1.0 + n[3]))
                .collect();
            (
                Q1_SCHEMA,
                vec![
                    "string:41".into(),
                    "string:46".into(),
                    sum(&quantities),
                    sum(&prices),
                    sum(&discounted),
                    sum(&charged),
                    average(&quantities),
                    average(&prices),
                    average(&discounts),
                    format!("int64:{}", rows.len()),
                ],
            )
        }
        "q6" => {
            // These retained inputs all pass the modified date, discount and
            // quantity predicate. Refuse a changed fixture instead of silently
            // calculating a sum over rows the query would discard.
            if numbers.iter().any(|n| n[0] != 1.0 || n[2] != 1.0) {
                return Err("Q6 aggregate fixture left its predicate profile".into());
            }
            (
                Q6_SCHEMA,
                vec![sum(&numbers
                    .iter()
                    .map(|n| n[1] * n[2])
                    .collect::<Vec<_>>())],
            )
        }
        _ => return Err("unknown aggregate query".into()),
    };
    // The retained literal remains stronger than a general AVG error interval.
    *cells
        .get_mut(column)
        .ok_or("aggregate selected column outside result")? = literal.into();
    Ok(Answer { schema, cells })
}

fn cell_matches(actual: &str, expected: &str) -> bool {
    if expected.starts_with("string:") || expected.starts_with("int64:") {
        return actual == expected;
    }
    let bits = |text: &str| {
        (text.len() == 16 && text.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| u64::from_str_radix(text, 16).ok())
            .flatten()
    };
    let Some(actual) = bits(actual) else {
        return false;
    };
    let expected = if let Some((low, high)) = expected.split_once(',') {
        if low == high {
            low
        } else {
            let (Some(low), Some(high)) = (bits(low), bits(high)) else {
                return false;
            };
            let (value, low, high) = (
                f64::from_bits(actual),
                f64::from_bits(low),
                f64::from_bits(high),
            );
            return value.is_finite()
                && low.is_finite()
                && high.is_finite()
                && low <= value
                && value <= high;
        }
    } else {
        expected
    };
    match expected {
        "nan" => f64::from_bits(actual).is_nan(),
        "infinity" => actual == f64::INFINITY.to_bits(),
        _ => bits(expected) == Some(actual),
    }
}

fn matches(expected: &Expected, code: Option<i32>, output: &str, errors: &str) -> bool {
    let answer = match expected {
        Expected::Row(answer) => answer,
        Expected::Overflow(expected) => {
            return super::composition::matches_overflow(code, output, errors, expected);
        }
    };
    if code != Some(0) || !errors.is_empty() {
        return false;
    }
    let Ok(rows) = super::composition::rows(output) else {
        return false;
    };
    let schemas: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_prefix("columns="))
        .collect();
    schemas == [answer.schema]
        && rows.len() == 1
        && rows[0].len() == answer.cells.len()
        && rows[0]
            .iter()
            .zip(&answer.cells)
            .all(|(actual, expected)| cell_matches(actual, expected))
}

pub fn run() -> Result<()> {
    let mut run = Run::new(workspace::root()?, "aggregate-boundaries")?;
    let cli = run.build("pipesql", "--bin", "pipesql")?;
    let data = run.root.join("test/data");
    let cases: serde_json::Value =
        serde_json::from_slice(&fs::read(data.join("aggregate-semantics/cases.json"))?)?;
    let cases = cases.as_array().ok_or("aggregate cases must be an array")?;
    if cases.len() != 24 {
        return Err("aggregate case inventory changed".into());
    }
    for case in cases {
        let name = case["name"].as_str().ok_or("missing aggregate case name")?;
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Err("invalid aggregate case name".into());
        }
        let expected = case["expected"].as_str().ok_or("missing expected answer")?;
        let column = if expected == "overflow" {
            0
        } else {
            case["column"].as_u64().ok_or("missing aggregate column")? as usize
        };
        let values = case["rows"]
            .as_array()
            .ok_or("missing aggregate input rows")?;
        if !(1..=4).contains(&values.len()) {
            return Err("aggregate input row count outside case bounds".into());
        }
        let mut rows = Vec::new();
        for row in values {
            let values = row.as_array().ok_or("invalid aggregate row")?;
            if values.len() != 4 {
                return Err("aggregate input needs four DOUBLEs".into());
            }
            let mut numbers = [0; 4];
            for (slot, value) in numbers.iter_mut().zip(values) {
                let value = value.as_str().ok_or("invalid DOUBLE bits")?;
                if value.len() != 16 {
                    return Err("DOUBLE bits need sixteen digits".into());
                }
                *slot = u64::from_str_radix(value, 16)?;
            }
            rows.push(Row {
                numbers,
                keys: *b"AF",
                day: 0,
            });
        }
        let database = run.directory.join(name);
        snapshot::write(&database, &data.join("current-single-table-format"), &rows)?;
        let query_name = case["query"].as_str().ok_or("missing aggregate query")?;
        let mut query = match query_name {
            "q1" => fs::read_to_string(data.join("upstream/q1-upstream.pipe.sql"))?,
            "q6" => fs::read_to_string(data.join("q6.pipe.sql"))?
                .replace("1994-01-01", "1970-01-01")
                .replace("0.08", "1.0"),
            _ => return Err("unknown aggregate query".into()),
        };
        let expected_answer = if expected == "overflow" {
            let (operation, call) = match name {
                "q1_true_sum_overflow" => ("SUM", "sum(l_quantity)"),
                "q6_true_sum_overflow" => ("SUM", "sum(l_extendedprice * l_discount)"),
                "q1_demanded_product_overflow" => {
                    ("multiplication", "sum(l_extendedprice * (1 - l_discount))")
                }
                _ => return Err("aggregate overflow needs an explicit operation and call".into()),
            };
            Expected::Overflow(super::composition::overflow_error(&query, operation, call)?)
        } else {
            Expected::Row(answer(query_name, &rows, column, expected)?)
        };
        query.push('\n');
        let path = run.directory.join(format!("{name}.sql"));
        fs::write(&path, query)?;
        let mut command = Command::new(&cli);
        command
            .arg("query")
            .arg("--database")
            .arg(&database)
            .args([
                "--memory-limit-bytes",
                "2000000",
                "--temp-limit-bytes",
                "1000000",
                "--query-file",
            ])
            .arg(path);
        let output = run.command(&mut command, None, Duration::from_secs(30))?;
        let code = match output.completion {
            Completion::Exited(status) => status.code(),
            _ => None,
        };
        if output.stdout.omitted != 0
            || output.stderr.omitted != 0
            || !matches(
                &expected_answer,
                code,
                std::str::from_utf8(&output.stdout.bytes)?,
                std::str::from_utf8(&output.stderr.bytes)?,
            )
        {
            return Err(
                format!("aggregate boundary {name} has a wrong or incomplete result").into(),
            );
        }
        println!("aggregate {name}: {expected} passed");
    }
    println!("Aggregate boundaries: 24 cases passed with complete typed rows or demanded overflow");
    run.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(schema: &str, cells: &[&str]) -> String {
        format!(
            "status=querying\ndatabase=/test\nmemory_limit_bytes=2000000\ntemp_limit_bytes=1000000\ncolumn_count={}\ncolumns={schema}\nrow={}\nrow_count=1\nstatus=queried\n",
            cells.len(),
            cells.join("|")
        )
    }

    #[test]
    fn exceptional_values_and_bits_remain_distinct() {
        for (expected, bits, accepted) in [
            ("nan", "7ff0000000000001", true),
            ("nan", "fff8000000001234", true),
            ("nan", "7ff0000000000000", false),
            ("infinity", "7ff0000000000000", true),
            ("infinity", "fff0000000000000", false),
            ("8000000000000000", "0000000000000000", false),
            (
                "8000000000000000,8000000000000000",
                "0000000000000000",
                false,
            ),
            (
                "3ff0000000000000,4000000000000000",
                "3ff8000000000000",
                true,
            ),
            (
                "3ff0000000000000,4000000000000000",
                "4000000000000001",
                false,
            ),
            (
                "3ff0000000000000,4000000000000000",
                "7ff8000000000000",
                false,
            ),
            (
                "3ff0000000000000,4000000000000000",
                "7ff0000000000000",
                false,
            ),
        ] {
            assert_eq!(cell_matches(bits, expected), accepted);
        }
        for bits in [
            "17ff0000000000001",
            "7ff000000000001",
            "+7ff000000000001",
            "7ff000000000000g",
        ] {
            assert!(!cell_matches(bits, "nan"));
        }
    }

    #[test]
    fn every_q1_cell_is_checked_even_when_the_selected_nan_is_correct() {
        let rows: Vec<_> = [f64::NAN, f64::MAX, f64::MAX]
            .map(|quantity| Row {
                numbers: [quantity.to_bits(), 0, 0, 0],
                keys: *b"AF",
                day: 0,
            })
            .into();
        let expected = Expected::Row(answer("q1", &rows, 2, "nan").unwrap());
        let cells = [
            "string:41",
            "string:46",
            "double:NaN:7ff8000000000000",
            "double:0:0000000000000000",
            "double:0:0000000000000000",
            "double:0:0000000000000000",
            "double:NaN:7ff8000000000000",
            "double:0:0000000000000000",
            "double:0:0000000000000000",
            "int64:3",
        ];
        assert!(matches(&expected, Some(0), &output(Q1_SCHEMA, &cells), ""));
        for column in 0..cells.len() {
            let mut wrong = cells;
            wrong[column] = match column {
                0 | 1 => "string:5a",
                9 => "int64:4",
                _ => "double:1:3ff0000000000000",
            };
            assert!(
                !matches(&expected, Some(0), &output(Q1_SCHEMA, &wrong), ""),
                "column {column}"
            );
        }
        for schema in [
            Q1_SCHEMA.replace("avg_price", "wrong"),
            Q1_SCHEMA.replace("count_order:int64", "count_order:double"),
        ] {
            assert!(!matches(&expected, Some(0), &output(&schema, &cells), ""));
        }
        let text = output(Q1_SCHEMA, &cells);
        for wrong in [
            text.replace("double:0:", "double:1:"),
            text.replace("double:NaN:", "double:inf:"),
            text.replace("double:0:", "double::"),
            text.replace("column_count=10", "column_count=9"),
            text.replace("columns=", "missing="),
        ] {
            assert!(!matches(&expected, Some(0), &wrong, ""));
        }
    }

    #[test]
    fn full_expectations_keep_negative_zero_counts_and_fixture_profile() {
        let rows: Vec<_> = (0..3)
            .map(|_| Row {
                numbers: [1.0_f64.to_bits(), 0, f64::MAX.to_bits(), 0],
                keys: *b"AF",
                day: 0,
            })
            .collect();
        let expected = Expected::Row(answer("q1", &rows, 8, "7fefffffffffffff").unwrap());
        let cells = [
            "string:41",
            "string:46",
            "double:3:4008000000000000",
            "double:0:0000000000000000",
            "double:-0:8000000000000000",
            "double:-0:8000000000000000",
            "double:1:3ff0000000000000",
            "double:0:0000000000000000",
            "double:1.7976931348623157e308:7fefffffffffffff",
            "int64:3",
        ];
        assert!(matches(&expected, Some(0), &output(Q1_SCHEMA, &cells), ""));
        let wrong = output(Q1_SCHEMA, &cells)
            .replace("double:-0:8000000000000000", "double:0:0000000000000000");
        assert!(!matches(&expected, Some(0), &wrong, ""));
        assert!(answer("q1", &rows, 10, "nan").is_err());
        assert!(answer("q6", &rows, 0, "nan").is_err());
        assert!(answer("unknown", &rows, 0, "nan").is_err());
        assert!(answer("q1", &[], 0, "nan").is_err());
    }

    #[test]
    fn value_requires_success_complete_shape_and_one_row() {
        let expected = Expected::Row(Answer {
            schema: Q6_SCHEMA,
            cells: vec!["3ff0000000000000".into()],
        });
        let text = output(Q6_SCHEMA, &["double:1:3ff0000000000000"]);
        assert!(matches(&expected, Some(0), &text, ""));
        assert!(!matches(
            &expected,
            Some(0),
            &text,
            "database error: unexpected diagnostic\n"
        ));
        for (code, text) in [
            (Some(1), text.clone()),
            (None, text.clone()),
            (Some(0), text.replace("status=querying\n", "")),
            (Some(0), text.replace("status=queried\n", "")),
            (Some(0), text.replace("row_count=1", "row_count=0")),
            (
                Some(0),
                text.replace("row_count=1", "row_count=1\nrow_count=1"),
            ),
            (Some(0), format!("{text}{text}")),
            (Some(0), format!("{text}extra\n")),
        ] {
            assert!(!matches(&expected, code, &text, ""));
        }
    }

    #[test]
    fn overflow_requires_its_error_without_published_results() {
        let diagnostic = "database error: arithmetic overflow during SUM at bytes 25..40\n";
        let expected = Expected::Overflow(diagnostic.into());
        assert!(matches(&expected, Some(1), "status=querying\n", diagnostic));
        for (code, output, error) in [
            (Some(0), "", diagnostic),
            (Some(1), "", "I/O failure"),
            (Some(1), "row=x\n", diagnostic),
            (Some(1), "row_count=0\n", diagnostic),
            (Some(1), "status=queried\n", diagnostic),
            (
                Some(1),
                "",
                "database error: arithmetic overflow during addition at bytes 25..40\n",
            ),
        ] {
            assert!(!matches(&expected, code, output, error));
        }
    }
}
