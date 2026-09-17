//! Check exceptional aggregate boundaries through the ordinary CLI.
//!
//! Retained cases supply literal DOUBLE input bits and expected answers. The
//! independent snapshot writer builds each database without using the engine's
//! encoder, then the campaign runs the selected aggregate query. Comparison checks
//! row width, column position and completion as well as finite or exceptional
//! numeric values. Malformed case input and an unknown query fail the campaign
//! instead of silently reducing its selection.

use crate::{
    Result,
    oracle::snapshot::{self, Row},
    process::Completion,
    workspace::{self, Run},
};
use std::{fs, process::Command, time::Duration};

fn matches(
    expected: &str,
    column: usize,
    width: usize,
    code: Option<i32>,
    output: &str,
    errors: &str,
) -> bool {
    let lines: Vec<_> = output.lines().collect();
    if expected == "overflow" {
        return code == Some(1)
            && errors.contains("arithmetic overflow")
            && !lines.iter().any(|line| {
                line.starts_with("row=")
                    || line.starts_with("row_count=")
                    || *line == "status=queried"
            });
    }
    if code != Some(0)
        || lines.first() != Some(&"status=querying")
        || !lines.ends_with(&["row_count=1", "status=queried"])
    {
        return false;
    }
    let completion: Vec<_> = lines
        .iter()
        .copied()
        .filter(|line| line.starts_with("row_count=") || line.starts_with("status="))
        .collect();
    if completion != ["status=querying", "row_count=1", "status=queried"] {
        return false;
    }
    let rows: Vec<_> = lines
        .iter()
        .filter_map(|line| line.strip_prefix("row="))
        .collect();
    if rows.len() != 1 {
        return false;
    }
    let cells: Vec<_> = rows[0].split('|').collect();
    if cells.len() != width {
        return false;
    }
    let Some(cell) = cells.get(column) else {
        return false;
    };
    let parts: Vec<_> = cell.split(':').collect();
    if parts.len() != 3
        || parts[0] != "double"
        || parts[2].len() != 16
        || !parts[2].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return false;
    }
    let Ok(bits) = u64::from_str_radix(parts[2], 16) else {
        return false;
    };
    match expected {
        "nan" => {
            bits & 0x7ff0_0000_0000_0000 == 0x7ff0_0000_0000_0000
                && bits & 0x000f_ffff_ffff_ffff != 0
        }
        "infinity" => bits == 0x7ff0_0000_0000_0000,
        _ => u64::from_str_radix(expected, 16) == Ok(bits),
    }
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
        let (mut query, width) = match case["query"].as_str() {
            Some("q1") => (
                fs::read_to_string(data.join("upstream/q1-upstream.pipe.sql"))?,
                10,
            ),
            Some("q6") => (
                fs::read_to_string(data.join("q6.pipe.sql"))?
                    .replace("1994-01-01", "1970-01-01")
                    .replace("0.08", "1.0"),
                1,
            ),
            _ => return Err("unknown aggregate query".into()),
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
                expected,
                column,
                width,
                code,
                std::str::from_utf8(&output.stdout.bytes)?,
                std::str::from_utf8(&output.stderr.bytes)?,
            )
        {
            return Err(format!("aggregate boundary {name} failed: expected {expected}").into());
        }
        println!("aggregate {name}: {expected} passed");
    }
    println!(
        "Aggregate boundaries: 24 literal cases passed through independently encoded snapshots"
    );
    run.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn output(bits: &str) -> String {
        format!("status=querying\nrow=double:value:{bits}\nrow_count=1\nstatus=queried\n")
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
        ] {
            assert_eq!(
                matches(expected, 0, 1, Some(0), &output(bits), ""),
                accepted
            );
        }
        for bits in [
            "17ff0000000000001",
            "7ff000000000001",
            "+7ff000000000001",
            "7ff000000000000g",
        ] {
            assert!(!matches("nan", 0, 1, Some(0), &output(bits), ""));
        }
    }
    #[test]
    fn value_requires_success_complete_shape_and_one_row() {
        let text = output("3ff0000000000000");
        assert!(matches("3ff0000000000000", 0, 1, Some(0), &text, ""));
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
            (Some(0), format!("{text}{}", output("3ff0000000000000"))),
        ] {
            assert!(!matches("3ff0000000000000", 0, 1, code, &text, ""));
        }
        assert!(!matches("3ff0000000000000", 0, 2, Some(0), &text, ""));
    }
    #[test]
    fn overflow_requires_its_error_without_published_results() {
        assert!(matches(
            "overflow",
            0,
            1,
            Some(1),
            "status=querying\n",
            "arithmetic overflow"
        ));
        for (code, output, error) in [
            (Some(0), "", "arithmetic overflow"),
            (Some(1), "", "I/O failure"),
            (Some(1), "row=x\n", "arithmetic overflow"),
            (Some(1), "status=queried\n", "arithmetic overflow"),
        ] {
            assert!(!matches("overflow", 0, 1, code, output, error));
        }
    }
}
