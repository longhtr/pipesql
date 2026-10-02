//! Run checked analytical examples with fixed artifacts and recorded inputs.
//!
//! Examples define their timing boundaries. Most check answers during execution;
//! export files are parsed independently here after their timed production. This
//! supervisor chooses a bounded input matrix, builds each release artifact once,
//! records whole-process GNU time separately, and requires the expected reports.
//! Each selection records its fresh-process rounds and query sample counts;
//! alternate rounds reverse scenario order. Results are observations, not
//! performance thresholds or whole-process memory promises.
//!
//! Use the Linux owner to freeze source and record the image and environment.
//! Each trial receives a fresh database path. Failed trials retain their inputs;
//! only checked success permits removing a trial's database. Raw query samples,
//! process measurements and artifact hashes remain in the generated run output.

use crate::{
    Result,
    workspace::{self, Run},
};
use serde_json::json;
use std::{collections::BTreeMap, fs, process::Command, time::Duration};

mod export;
mod parquet;

struct Scenario {
    example: &'static str,
    arguments: Vec<String>,
    marker: String,
    rounds: usize,
}

impl Scenario {
    fn new(example: &'static str, arguments: &[&str], marker: String, rounds: usize) -> Self {
        Self {
            example,
            arguments: arguments.iter().map(|s| (*s).to_owned()).collect(),
            marker,
            rounds,
        }
    }
}

// Native byte observation uses these same immutable inputs, in separate runs.
// Each supervisor still derives its required output counts independently.
pub(crate) fn join_replay_inputs() -> Vec<[&'static str; 5]> {
    let mut inputs = Vec::new();
    for operation in ["join", "join-many", "order-text", "order-many-text"] {
        for keys in ["32", "4096"] {
            for width in ["8", "1024", "2048"] {
                for memory in ["2600000", "12000000"] {
                    // The ORDER controls read exactly the matching dimensions.
                    // One admitted budget suffices; they do not replay groups.
                    if operation.starts_with("order-") && memory != "12000000" {
                        continue;
                    }
                    inputs.push([operation, "8192", keys, width, memory]);
                }
            }
        }
    }
    inputs.push(["scan", "8192", "32", "8", "12000000"]);
    inputs
}

fn scenarios(case: &str) -> Result<Vec<Scenario>> {
    if !matches!(
        case,
        "all"
            | "grouping"
            | "numeric-grouping"
            | "composed-grouping"
            | "filtered-grouping"
            | "string-grouping"
            | "sets"
            | "scalar"
            | "scans"
            | "computed"
            | "operators"
            | "windows"
            | "window-payloads"
            | "joins"
            | "join-replay"
            | "numeric-joins"
            | "scaled-numeric-joins"
            | "report"
            | "exports"
            | "parquet"
            | "csv-imports"
    ) {
        return Err(
            "choose grouping, composed-grouping, numeric-grouping, filtered-grouping, string-grouping, sets, scalar, scans, computed, operators, windows, window-payloads, joins, join-replay, numeric-joins, scaled-numeric-joins, report, exports, parquet, parquet-imports, parquet-wide, csv-imports or all"
                .into(),
        );
    }
    let mut result = Vec::new();
    if case == "join-replay" {
        for input in join_replay_inputs() {
            result.push(Scenario::new(
                "operator_cost",
                &input,
                "status=finished".into(),
                3,
            ));
        }
    }
    if matches!(case, "all" | "window-payloads") {
        for operation in [
            "window-count-payload",
            "window-sum-payload",
            "scan-payload",
            "order-payload",
        ] {
            for rows in ["8192", "65536"] {
                for keys in ["32", "4096"] {
                    for width in ["8", "1024"] {
                        for memory in ["4000000", "12000000"] {
                            // Scan and ORDER controls need only one key count
                            // and budget: neither groups rows by this unused key.
                            if matches!(operation, "scan-payload" | "order-payload")
                                && (keys != "32" || memory != "12000000")
                            {
                                continue;
                            }
                            result.push(Scenario::new(
                                "operator_cost",
                                &[operation, rows, keys, width, memory],
                                "status=finished".into(),
                                3,
                            ));
                        }
                    }
                }
            }
        }
    }
    if case == "parquet" {
        parquet::scenarios(&mut result);
    }
    if matches!(case, "all" | "csv-imports") {
        for profile in ["fixed", "text", "long-text"] {
            for columns in ["4", "64"] {
                for batch in ["64", "256"] {
                    result.push(Scenario::new(
                        "csv_import_cost",
                        &[profile, columns, batch],
                        "status=finished".into(),
                        3,
                    ));
                }
            }
        }
    }
    if matches!(case, "all" | "exports") {
        export::scenarios(&mut result);
    }
    if matches!(case, "all" | "grouping") {
        for groups in ["32", "4096"] {
            for distribution in ["even", "skewed"] {
                for memory in ["1200000", "2000000"] {
                    result.push(Scenario::new(
                        "grouping",
                        &[memory, groups, distribution],
                        format!(
                            "verified profile=extrema groups={groups} rows=8192 skewed={} memory_limit={memory}",
                            distribution == "skewed"
                        ),
                        3,
                    ));
                }
            }
        }
    }
    if matches!(case, "all" | "grouping" | "composed-grouping") {
        for memory in ["2200000", "12000000"] {
            result.push(Scenario::new("composed", &[memory], "verified 4096 descending groups: four joined pairs per key, nullable counts and sums".into(), 3));
        }
    }
    if matches!(case, "all" | "grouping" | "string-grouping") {
        for profile in ["extrema", "keys"] {
            for groups in ["4", "256"] {
                for width in ["8", "65536"] {
                    for memory in ["4000000", "80000000"] {
                        result.push(Scenario::new("string_grouping", &[groups, width, memory, "4", profile], format!("verified profile={profile} groups={groups} text_bytes={width} memory_limit={memory} batch_rows=4"), 3));
                    }
                }
            }
        }
    }
    if matches!(case, "all" | "numeric-grouping") {
        for profile in ["integer", "double"] {
            for groups in ["32", "4096"] {
                for distribution in ["even", "skewed"] {
                    for memory in ["1200000", "2000000"] {
                        result.push(Scenario::new(
                            "grouping", &[memory, groups, distribution, profile],
                            format!("verified profile={profile} groups={groups} rows=8192 skewed={} memory_limit={memory}", distribution == "skewed"), 3,
                        ));
                    }
                }
            }
        }
    }
    if matches!(case, "all" | "filtered-grouping") {
        for groups in ["32", "4096"] {
            for distribution in ["even", "skewed"] {
                for memory in ["1200000", "2000000"] {
                    result.push(Scenario::new(
                        "grouping", &[memory, groups, distribution, "filtered"],
                        format!("verified profile=filtered groups={groups} rows=8192 skewed={} memory_limit={memory}", distribution == "skewed"), 3,
                    ));
                }
            }
        }
    }
    if matches!(case, "all" | "sets") {
        for operation in ["except", "except-all", "intersect", "intersect-all"] {
            for classes in ["32", "4096"] {
                for width in ["8", "1024"] {
                    for memory in ["4000000", "12000000"] {
                        result.push(Scenario::new(
                            "set_cost",
                            &[operation, classes, width, memory],
                            "status=finished".into(),
                            3,
                        ));
                    }
                }
            }
        }
    }
    if matches!(case, "all" | "scalar") {
        result.push(Scenario::new(
            "projection_cost",
            &[],
            "verified rows=32768 total=1817536 samples=10 executions_per_sample=100 warmups=20"
                .into(),
            1,
        ));
        result.push(Scenario::new("text_cost", &[], "verified rows=4096 present=3584 byte_total=393216 scalar_total=246784 samples=10 executions_per_sample=50 warmups=10".into(), 1));
    }
    if matches!(case, "all" | "scans") {
        for rows in ["8192", "65536"] {
            for operation in [
                "scan",
                "filter-none",
                "filter-sparse",
                "filter-half",
                "filter-all",
            ] {
                for fact_batch in ["64", "512", "4096"] {
                    result.push(Scenario::new(
                        "operator_cost",
                        &[operation, rows, "32", "8", "8000000", fact_batch],
                        "status=finished".into(),
                        3,
                    ));
                }
            }
        }
    }
    if matches!(case, "all" | "operators" | "windows") {
        for rows in ["8192", "65536"] {
            for operation in [
                "scan",
                "order",
                "order-text",
                "distinct",
                "join",
                "window-count",
                "window-sum",
            ] {
                let window = matches!(operation, "window-count" | "window-sum");
                if case == "windows" && !window {
                    continue;
                }
                for keys in ["32", "4096"] {
                    for width in ["8", "1024"] {
                        // COUNT and SUM have different admission minima. These
                        // lower budgets admit each owner but constrain optional
                        // run growth; 2 MB permits the full initial run in both.
                        let limits = match operation {
                            "window-count" => ["1000000", "2000000"],
                            "window-sum" => ["1200000", "2000000"],
                            _ => ["8000000", "32000000"],
                        };
                        for memory in limits {
                            // Scan is a control for the same fact layout. Text
                            // width varies for joins and dimension sorting. The latter
                            // reads only dimensions, so one fact row count suffices.
                            if (!matches!(operation, "join" | "order-text") && width != "8")
                                || (operation == "order-text" && rows != "8192")
                                || (operation == "scan"
                                    && (case == "all" || keys != "32" || memory != "8000000"))
                            {
                                continue;
                            }
                            result.push(Scenario::new(
                                "operator_cost",
                                &[operation, rows, keys, width, memory],
                                "status=finished".into(),
                                if operation == "scan" { 1 } else { 3 },
                            ));
                        }
                    }
                }
            }
        }
    }
    if matches!(case, "all" | "joins") {
        // Fix left input size while independently varying group count, one or
        // eight right matches, text width and the budget available for run growth.
        for operation in ["join", "join-many"] {
            for keys in ["32", "4096"] {
                for width in ["8", "1024"] {
                    for memory in ["2600000", "12000000"] {
                        result.push(Scenario::new(
                            "operator_cost",
                            &[operation, "8192", keys, width, memory],
                            "status=finished".into(),
                            3,
                        ));
                    }
                }
            }
        }
    }
    if matches!(case, "all" | "numeric-joins" | "scaled-numeric-joins") {
        for size in ["8192", "262144"] {
            if (case == "numeric-joins" && size != "8192")
                || (case == "scaled-numeric-joins" && size != "262144")
            {
                continue;
            }
            for keys in ["32", "4096"] {
                for matches in ["1", "8"] {
                    for kind in ["inner", "left"] {
                        for memory in ["2600000", "12000000"] {
                            let rows = numeric_join_rows(size, keys, matches, kind)?;
                            result.push(Scenario::new("numeric_join", &[memory, keys, matches, kind, size],
                                format!("verified kind={kind} keys={keys} matches={matches} rows={rows} memory_limit={memory}"), 3));
                        }
                    }
                }
            }
        }
    }
    if matches!(case, "all" | "computed") {
        for operation in ["order-computed", "join-computed", "join-many-computed"] {
            for rows in ["8192", "65536"] {
                for memory in ["4000000", "12000000"] {
                    result.push(Scenario::new(
                        "operator_cost",
                        &[operation, rows, "32", "8", memory],
                        "status=finished".into(),
                        3,
                    ));
                }
            }
        }
    }
    if matches!(case, "all" | "report") {
        for profile in ["even", "skewed"] {
            for memory in ["8000000", "32000000"] {
                result.push(Scenario::new(
                    "scaled_report",
                    &[memory, profile, "--measure"],
                    "status=finished".into(),
                    3,
                ));
            }
        }
    }
    Ok(result)
}

fn one_line<'a>(text: &'a str, prefix: &str) -> Result<&'a str> {
    let mut found = text.lines().filter_map(|line| line.strip_prefix(prefix));
    let line = found
        .next()
        .ok_or_else(|| format!("missing measurement: {prefix}"))?;
    if found.next().is_some() {
        return Err(format!("duplicate measurement: {prefix}").into());
    }
    Ok(line)
}

fn integer_fields(text: &str, names: &[&str]) -> Result<Vec<u64>> {
    let fields: Vec<_> = text.split_whitespace().collect();
    if fields.len() != names.len() {
        return Err("incomplete measurement fields".into());
    }
    fields
        .iter()
        .zip(names)
        .map(|(field, name)| {
            let (key, value) = field.split_once('=').ok_or("invalid measurement field")?;
            if key != *name || value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err("invalid measurement field".into());
            }
            Ok(value.parse()?)
        })
        .collect()
}

fn positive_seconds(value: &str) -> Result<()> {
    let seconds: f64 = value.parse()?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err("invalid execution time".into());
    }
    Ok(())
}

// Complete-row counts for the fixed overlapping input ranges. At 4,096
// classes, multiples of both 11 and 13 collapse into one all-NULL row;
// counting class IDs or only integer keys would give different answers.
fn set_rows(operation: &str, classes: &str) -> Result<u64> {
    match (classes, operation) {
        ("32", "except" | "intersect") => Ok(16),
        ("32", "except-all") => Ok(6144),
        ("32", "intersect-all") => Ok(2048),
        ("4096", "except") => Ok(2033),
        ("4096", "except-all") => Ok(6130),
        ("4096", "intersect") => Ok(2035),
        ("4096", "intersect-all") => Ok(2062),
        _ => Err("unknown set measurement shape".into()),
    }
}

// Each key occurs 256 or 8192 times at 32 keys, and 2 or 64 times
// at 4096 keys. Exclude right-key multiples of 11 and left-ID multiples
// of 13, counting their overlap once. These literal totals do not use
// the example's row generator or its expected-pair calculation.
fn numeric_join_rows(size: &str, keys: &str, matches: &str, kind: &str) -> Result<u64> {
    let eligible = match (size, keys) {
        ("8192", "32") => 6852,
        ("8192", "4096") => 6873,
        ("262144", "32") => 219294,
        ("262144", "4096") => 219943,
        _ => return Err("invalid numeric join size or keys".into()),
    };
    let multiplicity = match matches {
        "1" => 1,
        "8" => 8,
        _ => return Err("invalid numeric join multiplicity".into()),
    };
    match kind {
        "inner" => Ok(eligible * multiplicity),
        "left" => Ok(size.parse::<u64>()? + eligible * (multiplicity - 1)),
        _ => Err("invalid numeric join kind".into()),
    }
}

fn check_report(scenario: &Scenario, stdout: &str, stderr: &str) -> Result<()> {
    if stdout
        .lines()
        .filter(|line| *line == scenario.marker)
        .count()
        != 1
    {
        return Err("missing or duplicate checked completion report".into());
    }
    match scenario.example {
        "export_cost" => export::check_report(scenario, stdout)?,
        "csv_import_cost" => {
            let [profile, columns, batch] = scenario.arguments.as_slice() else {
                return Err("invalid CSV import selection".into());
            };
            if !matches!(profile.as_str(), "fixed" | "text" | "long-text")
                || !matches!(columns.as_str(), "4" | "64")
                || !matches!(batch.as_str(), "64" | "256")
            {
                return Err("invalid CSV import geometry".into());
            }
            let rows = if profile == "long-text" { 128 } else { 8192 };
            let prefix =
                format!("profile={profile} columns={columns} rows={rows} batch_rows={batch} ");
            let input = integer_fields(
                one_line(stdout, "input ")?
                    .strip_prefix(&prefix)
                    .ok_or("CSV import input differs from selection")?,
                &["input_bytes", "memory_limit", "temp_limit"],
            )?;
            if input[0] == 0
                || input[0] > 32_000_000
                || input[1] != 32_000_000
                || input[2] != 64_000_000
            {
                return Err("invalid CSV import input bounds".into());
            }
            if one_line(stdout, "warmup ")? != format!("rows={rows} generation=2") {
                return Err("missing checked CSV import warmup".into());
            }
            for sample in 0..5 {
                let fields = integer_fields(
                    one_line(stdout, &format!("sample={sample} "))?,
                    &["elapsed_ns", "rows", "generation", "memory", "temporary"],
                )?;
                if fields[0] == 0
                    || fields[1] != rows
                    || fields[2] != 2
                    || !(1..=32_000_000).contains(&fields[3])
                    || fields[4] != 0
                {
                    return Err("invalid CSV import sample".into());
                }
            }
            if !stderr.is_empty()
                || stdout.lines().count() != 8
                || stdout.lines().last() != Some("status=finished")
            {
                return Err("incomplete or unexpected CSV import output".into());
            }
        }
        "string_grouping" => {
            let [groups, width, memory, batch, profile] = scenario.arguments.as_slice() else {
                return Err("invalid string grouping selection".into());
            };
            if one_line(stdout, "input ")?
                != format!(
                    "profile={profile} groups={groups} text_bytes={width} memory_limit={memory} batch_rows={batch}"
                )
            {
                return Err("string grouping input differs from selection".into());
            }
            let scratch: u64 = one_line(stdout, "warmup scratch_bytes=")?
                .strip_suffix(" file_probe=true")
                .ok_or("missing Linux string grouping spill observation")?
                .parse()?;
            if groups == "256" && width == "65536" && memory == "4000000" && scratch == 0 {
                return Err("wide string grouping must write spill".into());
            }
            for sample in 0..5 {
                let values = integer_fields(
                    one_line(stdout, &format!("sample={sample} "))?,
                    &[
                        "elapsed_ns",
                        "rows",
                        "batches",
                        "progress",
                        "memory",
                        "temporary",
                    ],
                )?;
                if values[0] == 0
                    || values[1] != groups.parse::<u64>()?
                    || values[2] == 0
                    || values[3] == 0
                    || values[4] == 0
                    || values[4] > memory.parse::<u64>()?
                    || values[5] > 128_000_000
                    || (values[5] > 0) != (scratch > 0)
                {
                    return Err("invalid string grouping measurement".into());
                }
            }
            if stdout
                .lines()
                .filter(|line| line.starts_with("sample="))
                .count()
                != 5
                || one_line(stdout, "status=")? != "finished"
                || stdout.lines().last() != Some("status=finished")
            {
                return Err("incomplete string grouping measurement".into());
            }
        }
        "numeric_join" => {
            let [memory, keys, matches, kind, size] = scenario.arguments.as_slice() else {
                return Err("invalid numeric join arguments".into());
            };
            let rows = numeric_join_rows(size, keys, matches, kind)?;
            if one_line(stdout, "input ")?
                != format!(
                    "kind={kind} left_rows={size} keys={keys} matches={matches} memory_limit={memory} batch_rows=256"
                )
            {
                return Err("numeric join input differs from selection".into());
            }
            let scratch: u64 = one_line(stdout, "warmup scratch_bytes=")?
                .strip_suffix(" file_probe=true")
                .ok_or("missing numeric join Linux spill observation")?
                .parse()?;
            if scratch == 0 || (size == "262144" && scratch <= memory.parse::<u64>()?) {
                return Err("numeric join must write the required spill volume".into());
            }
            for sample in 0..5 {
                let values = integer_fields(
                    one_line(stdout, &format!("sample={sample} "))?,
                    &[
                        "elapsed_ns",
                        "rows",
                        "batches",
                        "progress",
                        "memory",
                        "temporary",
                    ],
                )?;
                if values[0] == 0
                    || values[1] != rows
                    || !(1..=rows).contains(&values[2])
                    || values[3] == 0
                    || !(1..=memory.parse::<u64>()?).contains(&values[4])
                    || !(1..=if size == "8192" {
                        16_000_000
                    } else {
                        64_000_000
                    })
                        .contains(&values[5])
                {
                    return Err("invalid numeric join sample".into());
                }
            }
            if stdout
                .lines()
                .filter(|line| line.starts_with("sample="))
                .count()
                != 5
                || one_line(stdout, "status=")? != "finished"
                || stdout.lines().last() != Some("status=finished")
            {
                return Err("incomplete numeric join measurement".into());
            }
        }
        "composed" => {
            let [memory] = scenario.arguments.as_slice() else {
                return Err("invalid composed grouping selection".into());
            };
            if one_line(stdout, "input ")?
                != format!("groups=4096 rows=8192 memory_limit={memory} batch_rows=256")
            {
                return Err("composed grouping input differs from selection".into());
            }
            let bytes: u64 = one_line(stdout, "warmup scratch_bytes=")?
                .strip_suffix(" file_probe=true")
                .ok_or("missing composed Linux scratch observation")?
                .parse()?;
            if bytes == 0 {
                return Err("composed grouping must write spill".into());
            }
            for sample in 0..5 {
                let values = integer_fields(
                    one_line(stdout, &format!("sample={sample} "))?,
                    &[
                        "elapsed_ns",
                        "rows",
                        "batches",
                        "progress",
                        "memory",
                        "temporary",
                    ],
                )?;
                if values[0] == 0
                    || values[1] != 4096
                    || values[2] == 0
                    || values[2] > 4096
                    || values[3] == 0
                    || !(1..=memory.parse::<u64>()?).contains(&values[4])
                    || !(1..=8_000_000).contains(&values[5])
                {
                    return Err("invalid composed grouping sample".into());
                }
            }
            if stdout
                .lines()
                .filter(|line| line.starts_with("sample="))
                .count()
                != 5
                || one_line(stdout, "status=")? != "finished"
                || stdout.lines().last() != Some("status=finished")
            {
                return Err("incomplete composed grouping measurement".into());
            }
        }
        "grouping" => {
            let [memory, groups, distribution, rest @ ..] = scenario.arguments.as_slice() else {
                unreachable!()
            };
            let profile = rest.first().map(String::as_str).unwrap_or("extrema");
            let skewed = distribution == "skewed";
            if one_line(stdout, "input ")?
                != format!(
                    "profile={profile} groups={groups} rows=8192 skewed={skewed} memory_limit={memory} batch_rows=256"
                )
            {
                return Err("grouping input report differs from requested parameters".into());
            }
            let bytes: u64 = one_line(stdout, "warmup scratch_bytes=")?
                .strip_suffix(" file_probe=true")
                .ok_or("missing Linux scratch observation")?
                .parse()?;
            let expected_rows = groups.parse::<u64>()? / if profile == "filtered" { 2 } else { 1 };
            if profile == "filtered" && groups == "4096" && memory == "1200000" && bytes == 0 {
                return Err("filtered large grouping must write spill".into());
            }
            for sample in 0..5 {
                let values = integer_fields(
                    one_line(stdout, &format!("sample={sample} "))?,
                    &[
                        "elapsed_ns",
                        "rows",
                        "batches",
                        "progress",
                        "memory",
                        "temporary",
                    ],
                )?;
                if values[0] == 0
                    || values[1] != expected_rows
                    || values[2] == 0
                    || values[3] == 0
                    || values[4] == 0
                    || values[4] > memory.parse::<u64>()?
                    || values[5] > 8_000_000
                    || (values[5] > 0) != (bytes > 0)
                {
                    return Err("invalid grouping measurement".into());
                }
            }
            if stdout
                .lines()
                .filter(|line| line.starts_with("sample="))
                .count()
                != 5
                || one_line(stdout, "status=")? != "finished"
                || stdout.lines().last() != Some("status=finished")
            {
                return Err("incomplete grouping measurement".into());
            }
        }
        "set_cost" => {
            let [operation, classes, width, memory] = scenario.arguments.as_slice() else {
                unreachable!()
            };
            let rows = set_rows(operation, classes)?;
            if one_line(stdout, "input ")?
                != format!(
                    "operation={operation} left_rows=8192 right_rows=4096 classes={classes} text_bytes={width} memory_limit={memory} expected_rows={rows}"
                )
            {
                return Err("set input or complete-row count differs".into());
            }
            let bytes: u64 = one_line(stdout, "warmup scratch_bytes=")?
                .strip_suffix(" file_probe=true")
                .ok_or("missing set spill observation")?
                .parse()?;
            if bytes == 0 {
                return Err("set measurement did not observe spill".into());
            }
            for sample in 0..5 {
                let values = integer_fields(
                    one_line(stdout, &format!("sample={sample} "))?,
                    &[
                        "elapsed_ns",
                        "rows",
                        "batches",
                        "progress",
                        "memory",
                        "temporary",
                    ],
                )?;
                if values[0] == 0
                    || values[1] != rows
                    || values[2] == 0
                    || values[2] > rows
                    || values[3] == 0
                    || values[4] == 0
                    || values[4] > memory.parse::<u64>()?
                    || values[5] == 0
                {
                    return Err("invalid set measurement".into());
                }
            }
            if stdout
                .lines()
                .filter(|line| line.starts_with("sample="))
                .count()
                != 5
                || one_line(stdout, "status=")? != "finished"
                || stdout.lines().last() != Some("status=finished")
            {
                return Err("incomplete set measurement".into());
            }
        }
        "operator_cost" => {
            let [operation, rows, keys, width, memory] = &scenario.arguments[..5] else {
                unreachable!()
            };
            let fact_batch = scenario
                .arguments
                .get(5)
                .map(String::as_str)
                .unwrap_or("64");
            if !one_line(stdout, "input ")?.eq(&format!("operation={operation} rows={rows} keys={keys} text_bytes={width} memory_limit={memory} dimension_batch_rows=64 fact_batch_rows={fact_batch}")) {
                return Err("operator input report differs from requested parameters".into());
            }
            let blocking = !matches!(operation.as_str(), "scan" | "scan-payload")
                && !operation.starts_with("filter-");
            let warmup = one_line(stdout, "warmup scratch_bytes=")?;
            let bytes: u64 = warmup
                .strip_suffix(" file_probe=true")
                .ok_or("missing Linux scratch observation")?
                .parse()?;
            if (bytes > 0) != blocking {
                return Err("unexpected scratch observation".into());
            }
            for sample in 0..5 {
                let values = integer_fields(
                    one_line(stdout, &format!("sample={sample} "))?,
                    &[
                        "elapsed_ns",
                        "rows",
                        "batches",
                        "progress",
                        "memory",
                        "temporary",
                    ],
                )?;
                let expected_rows: u64 = if matches!(
                    operation.as_str(),
                    "distinct" | "order-text" | "order-many-text"
                ) {
                    keys
                } else {
                    rows
                }
                .parse()?;
                let expected_rows = match operation.as_str() {
                    "filter-none" => 0,
                    "filter-sparse" => (expected_rows + 50) / 101,
                    "filter-half" => expected_rows / 101 * 50 + (expected_rows % 101).min(50),
                    "join-many" | "order-many-text" => expected_rows * 8,
                    "order-computed" | "join-computed" | "join-many-computed" => {
                        let selected =
                            expected_rows / 101 * 52 + (expected_rows % 101).saturating_sub(49);
                        selected
                            * if operation == "join-many-computed" {
                                8
                            } else {
                                1
                            }
                    }
                    _ => expected_rows,
                };
                if values[0] == 0
                    || values[1] != expected_rows
                    || (values[2] == 0) != (expected_rows == 0)
                    || values[2] > expected_rows
                    || values[4] == 0
                    || values[4] > memory.parse::<u64>()?
                    || (values[5] > 0) != blocking
                    || values[5]
                        > if operation.ends_with("-payload") || width == "2048" {
                            256_000_000
                        } else {
                            128_000_000
                        }
                {
                    return Err("invalid operator measurement".into());
                }
            }
            if stdout
                .lines()
                .filter(|line| line.starts_with("sample="))
                .count()
                != 5
                || stdout.lines().last() != Some("status=finished")
            {
                return Err("unexpected operator sample count".into());
            }
        }
        "projection_cost" | "text_cost" => {
            let variants = if scenario.example == "projection_cost" {
                ["staged", "single"]
            } else {
                ["bytes", "scalars"]
            };
            for sample in 1..=10 {
                let values: Vec<_> = one_line(stdout, &format!("sample={sample} "))?
                    .split_whitespace()
                    .collect();
                if values.len() != 2 {
                    return Err("missing scalar comparison".into());
                }
                for (value, name) in values.iter().zip(variants) {
                    positive_seconds(
                        value
                            .strip_prefix(&format!("{name}_seconds="))
                            .ok_or("wrong scalar comparison")?,
                    )?;
                }
            }
            for name in variants {
                let bytes: u64 = one_line(
                    stdout,
                    &format!("{name} sampled additional logical memory="),
                )?
                .strip_suffix(" temporary=0")
                .ok_or("unexpected scalar spill")?
                .parse()?;
                if bytes == 0 {
                    return Err("missing scalar memory sample".into());
                }
            }
            if stdout
                .lines()
                .filter(|line| line.starts_with("sample="))
                .count()
                != 10
            {
                return Err("unexpected scalar sample count".into());
            }
        }
        "scaled_report" => {
            let [memory, profile, mode] = scenario.arguments.as_slice() else {
                return Err("invalid composed report selection".into());
            };
            if mode != "--measure"
                || one_line(stdout, "input ")?
                    != format!("profile={profile} memory_limit={memory} batch_rows=256 samples=5")
            {
                return Err("composed report input differs from its selection".into());
            }
            let memory: u64 = memory.parse()?;
            const TEMP_LIMIT: u64 = 64_000_000;
            let scratch: u64 = one_line(stdout, "verification scratch_bytes=")?
                .strip_suffix(" file_probe=true")
                .ok_or("missing Linux report scratch observation")?
                .parse()?;
            if scratch == 0 || stdout.lines().last() != Some("status=finished") {
                return Err("missing written report spill or final completion".into());
            }
            let result = integer_fields(
                one_line(stdout, "events=131072 ")?,
                &["groups", "sampled_temp_bytes"],
            )?;
            // Even covers four years (including NULL), five labels and three
            // amount classes. Skew ties north and the empty label to 1999 or
            // NULL, leaving 16 year/label pairs, each with all three classes.
            let expected_groups = match profile.as_str() {
                "even" => 60,
                "skewed" => 48,
                _ => return Err("unknown composed report profile".into()),
            };
            if result[0] != expected_groups || !(1..=TEMP_LIMIT).contains(&result[1]) {
                return Err("report groups or spill differ from selection".into());
            }
            for sample in 0..5 {
                let values = integer_fields(
                    one_line(stdout, &format!("sample={sample} "))?,
                    &[
                        "elapsed_ns",
                        "groups",
                        "sampled_memory_bytes",
                        "sampled_temp_bytes",
                    ],
                )?;
                if values[0] == 0
                    || values[1] != expected_groups
                    || values[2] == 0
                    || values[2] > memory
                    || values[3] == 0
                    || values[3] > TEMP_LIMIT
                {
                    return Err("invalid composed report sample".into());
                }
            }
            if stdout
                .lines()
                .filter(|line| line.starts_with("sample="))
                .count()
                != 5
            {
                return Err("unexpected composed report sample count".into());
            }
            for (phase, fields) in [
                ("model", &["elapsed_ns"][..]),
                ("ingest", &["elapsed_ns"][..]),
                ("reopen", &["elapsed_ns", "resident_bytes"][..]),
                ("prepare", &["elapsed_ns", "reserved_bytes"][..]),
                (
                    "warmup",
                    &["elapsed_ns", "sampled_memory_bytes", "sampled_temp_bytes"][..],
                ),
                ("reclaim", &["elapsed_ns", "removed_names"][..]),
            ] {
                let values = integer_fields(one_line(stderr, &format!("phase={phase} "))?, fields)?;
                if values[0] == 0 {
                    return Err("zero phase time".into());
                }
                // These fields sample the database's logical reservation
                // counters. Whole-process RSS and written scratch are separate.
                if matches!(phase, "reopen" | "prepare" | "warmup")
                    && !(1..=memory).contains(&values[1])
                {
                    return Err("invalid report phase memory reservation".into());
                }
                if phase == "warmup" && values[2] != result[1] {
                    return Err("inconsistent report spill".into());
                }
            }
        }
        _ => {
            positive_seconds(one_line(stdout, "execution and validation seconds=")?)?;
            let values = integer_fields(
                &one_line(stdout, "sampled logical bytes: ")?.replace(',', ""),
                &["memory", "temporary"],
            )?;
            if values[0] == 0 {
                return Err("missing memory measurement".into());
            }
        }
    }
    Ok(())
}

pub fn run(case: &str) -> Result<()> {
    let root = workspace::root()?;
    if !cfg!(target_os = "linux") || !root.join(".pipesql-source.json").is_file() {
        return Err("measure frozen Linux inputs with cargo dev linux measure --case NAME".into());
    }
    if case == "parquet-wide" {
        return crate::check::parquet::measure_wide();
    }
    if case == "parquet-imports" {
        return crate::check::parquet::run(true);
    }
    let scenarios = scenarios(case)?;
    let source: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join(".pipesql-source.json"))?)?;
    let before = workspace::tree_contents(&root)?;
    let mut run = Run::new(root, "measure")?;
    fs::write(
        run.directory.join("selection.json"),
        serde_json::to_vec_pretty(&json!({
            "source": source["sha256"], "case": case,
            "scenarios": scenarios.iter().enumerate().map(|(id, s)| json!({"id": id, "example": s.example, "arguments": s.arguments, "rounds": s.rounds})).collect::<Vec<_>>()
        }))?,
    )?;
    let mut artifacts = BTreeMap::new();
    for scenario in &scenarios {
        if !artifacts.contains_key(scenario.example) {
            artifacts.insert(
                scenario.example,
                run.build("pipesql", "--example", scenario.example)?,
            );
        }
    }
    for (example, control) in [
        (
            "csv_import_cost",
            "tests::complete_imports_reopen_and_reject_wrong_answers_and_late_errors",
        ),
        (
            "csv_import_cost",
            "tests::maximum_unicode_fields_reopen_with_independent_column_bursts",
        ),
        (
            "numeric_join",
            "tests::complete_pairs_nulls_spill_cancellation_and_wrong_answers",
        ),
        (
            "composed",
            "tests::complete_answers_spill_cancellation_and_wrong_results",
        ),
        (
            "scaled_report",
            "tests::spill_observation_requires_written_data_and_closed_descriptors",
        ),
        (
            "scaled_report",
            "tests::profiles_match_at_both_budgets_and_release_cancelled_work",
        ),
        (
            "scaled_report",
            "tests::refusal_and_wrong_answers_release_ownership_before_reuse",
        ),
        (
            "operator_cost",
            "tests::complete_typed_answers_and_spill_cleanup",
        ),
        (
            "operator_cost",
            "tests::scan_answers_cross_fact_unit_boundaries",
        ),
        (
            "operator_cost",
            "tests::computed_answers_require_selected_values_and_all_join_pairs",
        ),
        (
            "operator_cost",
            "tests::retained_text_windows_require_complete_values_and_release_failed_checks",
        ),
        (
            "operator_cost",
            "tests::wide_join_replay_and_matching_order_reject_wrong_complete_answers",
        ),
        (
            "grouping",
            "tests::complete_grouped_answers_spill_cancellation_and_wrong_results",
        ),
        (
            "string_grouping",
            "tests::text_profiles_require_complete_answers_spill_and_release",
        ),
        (
            "set_cost",
            "tests::complete_set_answers_spill_cancellation_and_wrong_results",
        ),
    ] {
        if !artifacts.contains_key(example) {
            continue;
        }
        let output = run.command(
            Command::new("cargo").args([
                "test",
                "--offline",
                "--locked",
                "--release",
                "-j",
                "1",
                "--example",
                example,
                control,
                "--",
                "--exact",
                "--test-threads=1",
            ]),
            None,
            Duration::from_secs(300),
        )?;
        output.require_success()?;
        if !std::str::from_utf8(&output.stdout.bytes)?
            .lines()
            .any(|line| line == format!("test {control} ... ok"))
        {
            return Err("measurement controls did not run".into());
        }
    }
    if let Some(executable) = artifacts.get("export_cost") {
        if case == "parquet" {
            parquet::controls(&mut run, executable)?;
        } else {
            export::controls(&mut run, executable)?;
        }
    }
    let mut csv_inputs = BTreeMap::new();
    for round in 0..3 {
        let mut order: Vec<_> = (0..scenarios.len()).collect();
        if round % 2 != 0 {
            order.reverse();
        }
        for id in order {
            let scenario = &scenarios[id];
            if round >= scenario.rounds {
                continue;
            }
            let name = format!("{id:02}-{round}");
            let database = run.directory.join(&name);
            let usage = run.directory.join(format!("{name}.time"));
            let mut command = Command::new("/usr/bin/time");
            command.args(["-f", "elapsed_seconds=%e\nuser_seconds=%U\nsystem_seconds=%S\nmax_rss_kib=%M\nfilesystem_inputs=%I\nfilesystem_outputs=%O\nexit=%x", "-o"]);
            command
                .arg(&usage)
                .arg("--")
                .arg(&artifacts[scenario.example])
                .arg(&database)
                .args(&scenario.arguments);
            if scenario.example == "export_cost" {
                command.env_remove("PIPESQL_EXPORT_CONTROL");
            }
            eprintln!(
                "measure {name}: {} {}",
                scenario.example,
                scenario.arguments.join(" ")
            );
            let output = run.command(&mut command, None, Duration::from_secs(300))?;
            if output.require_success().is_err() {
                eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
            }
            output.require_success()?;
            check_report(
                scenario,
                std::str::from_utf8(&output.stdout.bytes)?,
                std::str::from_utf8(&output.stderr.bytes)?,
            )?;
            if scenario.example == "csv_import_cost" {
                let input = database.join("input.csv");
                let bytes = fs::metadata(&input)?.len();
                let hash = workspace::hash(&input)?;
                fs::write(
                    run.directory.join(format!("{name}.input.json")),
                    serde_json::to_vec_pretty(
                        &json!({"sha256": hash, "bytes": bytes, "arguments": scenario.arguments}),
                    )?,
                )?;
                let report = one_line(std::str::from_utf8(&output.stdout.bytes)?, "input ")?;
                let reported: u64 = report
                    .split_whitespace()
                    .find_map(|field| field.strip_prefix("input_bytes="))
                    .ok_or("missing CSV byte count")?
                    .parse()?;
                if reported != bytes {
                    return Err("saved CSV differs from reported input size".into());
                }
                if let Some(previous) =
                    csv_inputs.insert(scenario.arguments[..2].to_vec(), hash.clone())
                    && previous != hash
                {
                    return Err("CSV input changed across matching samples".into());
                }
            }
            if scenario.example == "export_cost" {
                let report = std::str::from_utf8(&output.stdout.bytes)?;
                let verified = if case == "parquet" {
                    parquet::verify(&mut run, scenario, &database, report)?
                } else {
                    export::verify(scenario, &database, report)?
                };
                fs::write(
                    run.directory.join(format!("{name}.validation.json")),
                    serde_json::to_vec_pretty(&verified)?,
                )?;
            }
            if one_line(&fs::read_to_string(&usage)?, "exit=")? != "0" {
                return Err("missing successful process measurement".into());
            }
            fs::remove_dir_all(database)?;
        }
    }
    if workspace::tree_contents(&run.root)? != before {
        return Err("measurement changed its frozen source".into());
    }
    println!(
        "Checked measurements complete; results: {}",
        run.directory.display()
    );
    run.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selective_scans_require_complete_samples_including_empty_answers() {
        let cases = scenarios("scans").unwrap();
        assert_eq!(cases.len(), 30);
        for scenario in cases {
            assert_eq!(scenario.rounds, 3);
            let [operation, rows, keys, width, memory, fact_batch] = scenario.arguments.as_slice()
            else {
                unreachable!()
            };
            let count = match (rows.as_str(), operation.as_str()) {
                (_, "filter-none") => 0,
                ("8192", "filter-sparse") => 81,
                ("65536", "filter-sparse") => 649,
                ("8192", "filter-half") => 4061,
                ("65536", "filter-half") => 32450,
                (_, "scan" | "filter-all") => rows.parse::<usize>().unwrap(),
                _ => unreachable!(),
            };
            let batches = usize::from(count > 0);
            let mut output = format!(
                "input operation={operation} rows={rows} keys={keys} text_bytes={width} memory_limit={memory} dimension_batch_rows=64 fact_batch_rows={fact_batch}\nwarmup scratch_bytes=0 file_probe=true\n"
            );
            for sample in 0..5 {
                output.push_str(&format!("sample={sample} elapsed_ns=100 rows={count} batches={batches} progress=50 memory=100000 temporary=0\n"));
            }
            output.push_str("status=finished\n");
            check_report(&scenario, &output, "").unwrap();
            for bad in [
                output.replace("dimension_batch_rows=64", "dimension_batch_rows=512"),
                output.replace(
                    &format!("fact_batch_rows={fact_batch}"),
                    "fact_batch_rows=1",
                ),
                output.replace("sample=4 ", "sample=5 "),
                output.replace(
                    &format!(" rows={count} batches="),
                    &format!(" rows={} batches=", count + 1),
                ),
                output.replace("scratch_bytes=0", "scratch_bytes=256"),
                output.replace("temporary=0", "temporary=1"),
                output.replace("file_probe=true", "file_probe=false"),
                output.replace(
                    &format!("batches={batches}"),
                    &format!("batches={}", usize::from(batches == 0)),
                ),
                format!(
                    "{output}sample=5 elapsed_ns=100 rows={count} batches={batches} progress=50 memory=100000 temporary=0\n"
                ),
                format!("{output}extra\n"),
            ] {
                assert!(
                    check_report(&scenario, &bad, "").is_err(),
                    "{operation}: {bad}"
                );
            }
        }
    }

    #[test]
    fn computed_matrix_requires_selected_counts_and_spill() {
        let selected = scenarios("computed").unwrap();
        assert_eq!(selected.len(), 12);
        let mut unique = std::collections::BTreeSet::new();
        for scenario in selected {
            assert_eq!(scenario.rounds, 3);
            assert!(unique.insert(scenario.arguments.clone()));
            let [operation, rows, keys, width, memory] = scenario.arguments.as_slice() else {
                panic!("computed dimensions")
            };
            assert_eq!(keys, "32");
            assert_eq!(width, "8");
            // Literal counts for these two fixed fixture sizes are independent
            // of the report validator's quotient/remainder calculation.
            let count = match rows.as_str() {
                "8192" => 4212,
                "65536" => 33735,
                _ => panic!("computed rows"),
            } * if operation == "join-many-computed" {
                8
            } else {
                1
            };
            let mut output = format!(
                "input operation={operation} rows={rows} keys={keys} text_bytes={width} memory_limit={memory} dimension_batch_rows=64 fact_batch_rows=64\nwarmup scratch_bytes=256 file_probe=true\n"
            );
            for sample in 0..5 {
                output.push_str(&format!("sample={sample} elapsed_ns=100 rows={count} batches=32 progress=50 memory=100000 temporary=1000\n"));
            }
            output.push_str("status=finished\n");
            check_report(&scenario, &output, "").unwrap();
            for bad in [
                output.replace(&format!(" rows={count} batches="), " rows=1 batches="),
                output.replace("scratch_bytes=256", "scratch_bytes=0"),
                output.replace("temporary=1000", "temporary=0"),
                output.replace("status=finished\n", ""),
                output.replace("sample=4 ", "sample=5 "),
            ] {
                assert!(check_report(&scenario, &bad, "").is_err());
            }
        }
    }

    #[test]
    fn csv_import_matrix_requires_complete_samples_and_receipts() {
        let selected = scenarios("csv-imports").unwrap();
        assert_eq!(selected.len(), 12);
        let mut unique = std::collections::BTreeSet::new();
        for scenario in selected {
            assert_eq!(scenario.rounds, 3);
            assert!(unique.insert(scenario.arguments.clone()));
            let [profile, columns, batch] = scenario.arguments.as_slice() else {
                panic!("CSV import dimensions")
            };
            let rows = if profile == "long-text" { 128 } else { 8192 };
            let mut output = format!(
                "input profile={profile} columns={columns} rows={rows} batch_rows={batch} input_bytes=100000 memory_limit=32000000 temp_limit=64000000\nwarmup rows={rows} generation=2\n"
            );
            for sample in 0..5 {
                output.push_str(&format!("sample={sample} elapsed_ns=100 rows={rows} generation=2 memory=100000 temporary=0\n"));
            }
            output.push_str("status=finished\n");
            check_report(&scenario, &output, "").unwrap();
            for bad in [
                output.replace("elapsed_ns=100", "elapsed_ns=0"),
                output.replace(&format!(" rows={rows} generation="), " rows=1 generation="),
                output.replace("generation=2", "generation=3"),
                output.replace("temporary=0", "temporary=1"),
                output.replace("memory=100000", "memory=32000001"),
                output.replace("sample=4 ", "sample=5 "),
                output.replace("status=finished\n", ""),
                output.replace("input_bytes=100000", "input_bytes=0"),
                output.replace(&format!("profile={profile}"), "profile=other"),
                format!("{output}status=finished\n"),
                format!(
                    "{output}sample=0 elapsed_ns=100 rows={rows} generation=2 memory=100000 temporary=0\n"
                ),
            ] {
                assert!(check_report(&scenario, &bad, "").is_err(), "accepted {bad}");
            }
            assert!(check_report(&scenario, &output, "unexpected diagnostic").is_err());
        }
    }

    #[test]
    fn window_matrix_requires_repeated_complete_samples() {
        let selected = scenarios("windows").unwrap();
        assert_eq!(selected.len(), 16);
        let mut unique = std::collections::BTreeSet::new();
        for scenario in selected {
            assert_eq!(scenario.rounds, 3);
            let [operation, rows, keys, width, memory] = scenario.arguments.as_slice() else {
                panic!("window dimensions")
            };
            assert!(matches!(operation.as_str(), "window-count" | "window-sum"));
            assert_eq!(width, "8");
            assert!(unique.insert(scenario.arguments.clone()));
            let mut output = format!(
                "input operation={operation} rows={rows} keys={keys} text_bytes={width} memory_limit={memory} dimension_batch_rows=64 fact_batch_rows=64\nwarmup scratch_bytes=256 file_probe=true\n"
            );
            for sample in 0..5 {
                output.push_str(&format!("sample={sample} elapsed_ns=100 rows={rows} batches=32 progress=50 memory=100000 temporary=1000\n"));
            }
            output.push_str("status=finished\n");
            check_report(&scenario, &output, "").unwrap();
            for bad in [
                output.replace("status=finished\n", ""),
                output.replace("sample=4 ", "sample=5 "),
                output.replace(&format!(" rows={rows} batches="), " rows=1 batches="),
                output.replace("scratch_bytes=256", "scratch_bytes=0"),
            ] {
                assert!(check_report(&scenario, &bad, "").is_err());
            }
        }
    }

    #[test]
    fn window_payload_matrix_requires_complete_samples_spill_and_controls() {
        let matrix = scenarios("window-payloads").unwrap();
        assert_eq!(matrix.len(), 40);
        let mut unique = std::collections::BTreeSet::new();
        for scenario in matrix {
            assert_eq!(scenario.rounds, 3);
            assert!(unique.insert(scenario.arguments.clone()));
            let [operation, rows, keys, width, memory] = scenario.arguments.as_slice() else {
                panic!("window payload dimensions")
            };
            let scan = operation == "scan-payload";
            if matches!(operation.as_str(), "scan-payload" | "order-payload") {
                assert_eq!(keys, "32");
                assert_eq!(memory, "12000000");
            }
            let temporary = if scan { 0 } else { 1000 };
            let scratch = if scan { 0 } else { 256 };
            let mut output = format!(
                "input operation={operation} rows={rows} keys={keys} text_bytes={width} memory_limit={memory} dimension_batch_rows=64 fact_batch_rows=64\nwarmup scratch_bytes={scratch} file_probe=true\n"
            );
            for sample in 0..5 {
                output.push_str(&format!("sample={sample} elapsed_ns=100 rows={rows} batches=32 progress=50 memory=100000 temporary={temporary}\n"));
            }
            output.push_str("status=finished\n");
            check_report(&scenario, &output, "").unwrap();
            for bad in [
                output.replace("elapsed_ns=100", "elapsed_ns=0"),
                output.replace(&format!(" rows={rows} batches="), " rows=1 batches="),
                output.replace(&format!("temporary={temporary}"), "temporary=256000001"),
                output.replace("sample=4 ", "sample=5 "),
                output.replace("status=finished\n", ""),
                format!("{output}status=finished\n"),
            ] {
                assert!(check_report(&scenario, &bad, "").is_err());
            }
            let wrong = output.replace(
                &format!("scratch_bytes={scratch}"),
                if scan {
                    "scratch_bytes=256"
                } else {
                    "scratch_bytes=0"
                },
            );
            assert!(check_report(&scenario, &wrong, "").is_err());
        }
    }

    #[test]
    fn join_matrix_requires_complete_pair_counts_and_all_samples() {
        let cases = scenarios("joins").unwrap();
        assert_eq!(cases.len(), 16);
        let mut unique = std::collections::BTreeSet::new();
        for scenario in cases {
            assert_eq!(scenario.rounds, 3);
            assert!(unique.insert(scenario.arguments.clone()));
            let [operation, rows, keys, width, memory] = scenario.arguments.as_slice() else {
                unreachable!()
            };
            assert_eq!(rows, "8192");
            assert!(matches!(operation.as_str(), "join" | "join-many"));
            let count = if operation == "join-many" {
                65_536
            } else {
                8192
            };
            let mut output = format!(
                "input operation={operation} rows={rows} keys={keys} text_bytes={width} memory_limit={memory} dimension_batch_rows=64 fact_batch_rows=64\nwarmup scratch_bytes=256 file_probe=true\n"
            );
            for sample in 0..5 {
                output.push_str(&format!("sample={sample} elapsed_ns=100 rows={count} batches={count} progress=50 memory=100000 temporary=1000\n"));
            }
            output.push_str("status=finished\n");
            check_report(&scenario, &output, "").unwrap();
            for bad in [
                output.replace(&format!(" rows={count} batches="), " rows=8191 batches="),
                output.replace("sample=4 ", "sample=5 "),
                output.replace("scratch_bytes=256", "scratch_bytes=0"),
            ] {
                assert!(check_report(&scenario, &bad, "").is_err());
            }
        }
    }

    #[test]
    fn join_replay_requires_matching_controls_complete_samples_and_scoped_limits() {
        let matrix = scenarios("join-replay").unwrap();
        assert_eq!(matrix.len(), 37);
        let mut unique = std::collections::BTreeSet::new();
        for scenario in matrix {
            assert_eq!(scenario.rounds, 3);
            assert!(unique.insert(scenario.arguments.clone()));
            let [operation, rows, keys, width, memory] = scenario.arguments.as_slice() else {
                unreachable!()
            };
            let count: u64 = match operation.as_str() {
                "join-many" => 65536,
                "order-many-text" => keys.parse::<u64>().unwrap() * 8,
                "order-text" => keys.parse().unwrap(),
                _ => 8192,
            };
            let scan = operation == "scan";
            let temporary = if scan { 0 } else { 1000 };
            let scratch = if scan { 0 } else { 256 };
            let mut output = format!(
                "input operation={operation} rows={rows} keys={keys} text_bytes={width} memory_limit={memory} dimension_batch_rows=64 fact_batch_rows=64\nwarmup scratch_bytes={scratch} file_probe=true\n"
            );
            for sample in 0..5 {
                output.push_str(&format!("sample={sample} elapsed_ns=100 rows={count} batches={count} progress=50 memory=100000 temporary={temporary}\n"));
            }
            output.push_str("status=finished\n");
            check_report(&scenario, &output, "").unwrap();
            for wrong in [
                output.replace(&format!(" rows={count} batches="), " rows=1 batches="),
                output.replace("sample=4 ", "sample=5 "),
                output.replace("status=finished\n", ""),
                output.replace("dimension_batch_rows=64", "dimension_batch_rows=256"),
                output.replace(&format!("temporary={temporary}"), "temporary=256000001"),
            ] {
                assert!(check_report(&scenario, &wrong, "").is_err());
            }
            if width != "2048" && !scan {
                assert!(
                    check_report(
                        &scenario,
                        &output.replace("temporary=1000", "temporary=128000001"),
                        ""
                    )
                    .is_err()
                );
            }
        }
    }

    #[test]
    fn numeric_grouping_requires_complete_samples_and_observed_spill() {
        let matrix = scenarios("numeric-grouping").unwrap();
        assert_eq!(matrix.len(), 16);
        for scenario in matrix {
            assert_eq!(scenario.rounds, 3);
            let [memory, groups, distribution, profile] = scenario.arguments.as_slice() else {
                unreachable!()
            };
            for scratch in [0, 4096] {
                let mut output = format!(
                    "input profile={profile} groups={groups} rows=8192 skewed={} memory_limit={memory} batch_rows=256\nwarmup scratch_bytes={scratch} file_probe=true\n",
                    distribution == "skewed"
                );
                for sample in 0..5 {
                    output.push_str(&format!("sample={sample} elapsed_ns=100 rows={groups} batches=32 progress=50 memory=100000 temporary={scratch}\n"));
                }
                output.push_str(&format!("{}\nstatus=finished\n", scenario.marker));
                check_report(&scenario, &output, "").unwrap();
                for bad in [
                    output.replace(&format!("rows={groups} batches="), "rows=1 batches="),
                    output.replace("sample=4 ", "sample=5 "),
                    output.replace("elapsed_ns=100", "elapsed_ns=0"),
                    output.replace("file_probe=true", "file_probe=false"),
                    output.replace("status=finished\n", ""),
                    format!("{output}status=finished\n"),
                    output.replace(
                        &format!("temporary={scratch}"),
                        if scratch == 0 {
                            "temporary=1"
                        } else {
                            "temporary=0"
                        },
                    ),
                ] {
                    assert!(check_report(&scenario, &bad, "").is_err());
                }
            }
        }
    }

    #[test]
    fn filtered_grouping_requires_selected_keys_counts_and_spill() {
        let matrix = scenarios("filtered-grouping").unwrap();
        assert_eq!(matrix.len(), 8);
        for scenario in matrix {
            assert_eq!(scenario.rounds, 3);
            let [memory, groups, distribution, profile] = scenario.arguments.as_slice() else {
                unreachable!()
            };
            assert_eq!(profile, "filtered");
            let rows = groups.parse::<u64>().unwrap() / 2;
            let scratch = if groups == "4096" && memory == "1200000" {
                4096
            } else {
                0
            };
            let mut output = format!(
                "input profile=filtered groups={groups} rows=8192 skewed={} memory_limit={memory} batch_rows=256\nwarmup scratch_bytes={scratch} file_probe=true\n",
                distribution == "skewed"
            );
            for sample in 0..5 {
                output.push_str(&format!("sample={sample} elapsed_ns=100 rows={rows} batches=8 progress=50 memory=100000 temporary={scratch}\n"));
            }
            output.push_str(&format!("{}\nstatus=finished\n", scenario.marker));
            check_report(&scenario, &output, "").unwrap();
            for wrong_rows in [0, rows - 1, rows + 1, groups.parse().unwrap()] {
                let bad = output.replace(
                    &format!("rows={rows} batches="),
                    &format!("rows={wrong_rows} batches="),
                );
                assert!(check_report(&scenario, &bad, "").is_err());
            }
            for bad in [
                output.replace("sample=4 ", "sample=5 "),
                output.replace("elapsed_ns=100", "elapsed_ns=0"),
                output.replace("file_probe=true", "file_probe=false"),
                output.replace("status=finished\n", ""),
                output.replace("memory=100000", "memory=999999999"),
                output.replace(&format!("temporary={scratch}"), "temporary=8000001"),
            ] {
                assert!(check_report(&scenario, &bad, "").is_err());
            }
            if scratch > 0 {
                let bad = output
                    .replace("scratch_bytes=4096", "scratch_bytes=0")
                    .replace("temporary=4096", "temporary=0");
                assert!(check_report(&scenario, &bad, "").is_err());
            }
        }
    }

    #[test]
    fn set_matrix_requires_complete_multiplicities_samples_and_spill() {
        let matrix = scenarios("sets").unwrap();
        assert_eq!(matrix.len(), 32);
        for scenario in matrix {
            assert_eq!(scenario.rounds, 3);
            let [operation, classes, width, memory] = scenario.arguments.as_slice() else {
                unreachable!()
            };
            let rows = set_rows(operation, classes).unwrap();
            let mut output = format!(
                "input operation={operation} left_rows=8192 right_rows=4096 classes={classes} text_bytes={width} memory_limit={memory} expected_rows={rows}\nwarmup scratch_bytes=4096 file_probe=true\n"
            );
            for sample in 0..5 {
                output.push_str(&format!("sample={sample} elapsed_ns=100 rows={rows} batches={rows} progress=50 memory=100000 temporary=8192\n"));
            }
            output.push_str("status=finished\n");
            check_report(&scenario, &output, "").unwrap();
            for bad in [
                output.replace(&format!("rows={rows} batches="), "rows=1 batches="),
                output.replace(&format!("expected_rows={rows}"), "expected_rows=1"),
                output.replace("sample=4 ", "sample=5 "),
                output.replace("elapsed_ns=100", "elapsed_ns=0"),
                output.replace("memory=100000", "memory=999999999"),
                output.replace("temporary=8192", "temporary=0"),
                output.replace("scratch_bytes=4096", "scratch_bytes=0"),
                output.replace("file_probe=true", "file_probe=false"),
                output.replace("status=finished\n", ""),
                format!("{output}status=finished\n"),
            ] {
                assert!(check_report(&scenario, &bad, "").is_err());
            }
        }
    }

    #[test]
    fn composed_report_requires_written_spill_and_complete_phases() {
        let phases = "phase=model elapsed_ns=100\nphase=ingest elapsed_ns=100\nphase=reopen elapsed_ns=100 resident_bytes=10\nphase=prepare elapsed_ns=100 reserved_bytes=20\nphase=warmup elapsed_ns=100 sampled_memory_bytes=100 sampled_temp_bytes=1000\nphase=reclaim elapsed_ns=100 removed_names=0\n";
        for (profile, groups, other_profile, other_groups) in
            [("even", 60, "skewed", 48), ("skewed", 48, "even", 60)]
        {
            for memory in ["8000000", "32000000"] {
                let scenario = Scenario::new(
                    "scaled_report",
                    &[memory, profile, "--measure"],
                    "status=finished".into(),
                    3,
                );
                let mut output = format!(
                    "events=131072 groups={groups} sampled_temp_bytes=1000\nverification scratch_bytes=900 file_probe=true\ninput profile={profile} memory_limit={memory} batch_rows=256 samples=5\n"
                );
                for sample in 0..5 {
                    output.push_str(&format!("sample={sample} elapsed_ns=100 groups={groups} sampled_memory_bytes=100 sampled_temp_bytes=1000\n"));
                }
                output.push_str("status=finished\n");
                check_report(&scenario, &output, phases).unwrap();
                // Agreement between the producer's summary and samples cannot
                // substitute for the selected fixture's complete cardinality.
                for wrong_groups in [0, 1, groups - 1, groups + 1, other_groups] {
                    let wrong = output.replace(
                        &format!("groups={groups}"),
                        &format!("groups={wrong_groups}"),
                    );
                    assert!(check_report(&scenario, &wrong, phases).is_err());
                }
                for wrong in [
                    output.replace("status=finished\n", &format!("sample=5 elapsed_ns=100 groups={groups} sampled_memory_bytes=100 sampled_temp_bytes=1000\nstatus=finished\n")),
                    output.replace("sample=4 ", "sample=3 "),
                    output.replace("sample=4 ", "sample=5 "),
                    output.replace("sample=0 elapsed_ns=100", "sample=0 elapsed_ns=0"),
                    output.replace(
                        &format!("sample=0 elapsed_ns=100 groups={groups}"),
                        &format!("sample=0 elapsed_ns=100 groups={}", groups - 1),
                    ),
                    output.replace("sampled_memory_bytes=100 ", "sampled_memory_bytes=99999999 "),
                    output.replace(&format!("profile={profile}"), &format!("profile={other_profile}")),
                    output.replace("scratch_bytes=900", "scratch_bytes=0"),
                    output.replace("file_probe=true", "file_probe=false"),
                    output.replace("verification scratch_bytes=900 file_probe=true\n", ""),
                    output.replace("status=finished\n", ""),
                    format!("{output}verification scratch_bytes=900 file_probe=true\n"),
                ] {
                    assert!(check_report(&scenario, &wrong, phases).is_err());
                }
                let memory_limit = memory.parse::<u64>().unwrap();
                for (field, valid) in [
                    ("resident_bytes", 10),
                    ("reserved_bytes", 20),
                    ("sampled_memory_bytes", 100),
                ] {
                    for invalid in [0, memory_limit + 1] {
                        let wrong = phases
                            .replace(&format!("{field}={valid}"), &format!("{field}={invalid}"));
                        assert!(
                            check_report(&scenario, &output, &wrong).is_err(),
                            "phase {field}={invalid}"
                        );
                    }
                }
                let at_memory_limit = phases.replace(
                    "sampled_memory_bytes=100",
                    &format!("sampled_memory_bytes={memory_limit}"),
                );
                check_report(&scenario, &output, &at_memory_limit).unwrap();
                // Matching warm-up and summary peaks must still obey the
                // configured limit; changing every sample would conceal this
                // gap behind the existing per-sample checks.
                for (temporary, accepted) in [(64_000_000, true), (64_000_001, false)] {
                    let field = format!("sampled_temp_bytes={temporary}");
                    let changed_summary = output.replacen("sampled_temp_bytes=1000", &field, 1);
                    let changed_phase = phases.replace("sampled_temp_bytes=1000", &field);
                    assert_eq!(
                        check_report(&scenario, &changed_summary, &changed_phase).is_ok(),
                        accepted
                    );
                }
                for wrong in [
                    phases.replace("sampled_temp_bytes=1000", "sampled_temp_bytes=999"),
                    phases.replace("phase=warmup elapsed_ns=100", "phase=warmup elapsed_ns=0"),
                    phases.replace("phase=reclaim elapsed_ns=100 removed_names=0\n", ""),
                ] {
                    assert!(check_report(&scenario, &output, &wrong).is_err());
                }
            }
        }
    }

    #[test]
    fn string_grouping_requires_complete_samples_and_written_spill() {
        let matrix = scenarios("string-grouping").unwrap();
        assert_eq!(matrix.len(), 16);
        for scenario in matrix {
            assert_eq!(scenario.rounds, 3);
            let [groups, width, memory, batch, profile] = scenario.arguments.as_slice() else {
                unreachable!()
            };
            let spill = groups == "256" && width == "65536" && memory == "4000000";
            let scratch = if spill { 4096 } else { 0 };
            let mut output = format!(
                "input profile={profile} groups={groups} text_bytes={width} memory_limit={memory} batch_rows={batch}\nwarmup scratch_bytes={scratch} file_probe=true\n"
            );
            for sample in 0..5 {
                output.push_str(&format!("sample={sample} elapsed_ns=100 rows={groups} batches=4 progress=10 memory=100000 temporary={scratch}\n"));
            }
            output.push_str(&format!("{}\nstatus=finished\n", scenario.marker));
            check_report(&scenario, &output, "").unwrap();
            for bad in [
                output.replace(&format!("rows={groups} batches="), "rows=1 batches="),
                output.replace("sample=4 ", "sample=5 "),
                output.replace("elapsed_ns=100", "elapsed_ns=0"),
                output.replace("memory=100000", "memory=999999999"),
                output.replace("file_probe=true", "file_probe=false"),
                output.replace("status=finished\n", ""),
                format!("{output}status=finished\n"),
                output.replace(
                    &format!("temporary={scratch}"),
                    if spill { "temporary=0" } else { "temporary=1" },
                ),
                output.replace(&format!("text_bytes={width}"), "text_bytes=7"),
            ] {
                assert!(check_report(&scenario, &bad, "").is_err());
            }
            if spill {
                assert!(
                    check_report(&scenario, &output.replace("4096", "0"), "").is_err(),
                    "zeroing both observations must not hide required spill"
                );
            }
        }
    }

    #[test]
    fn numeric_joins_require_complete_pairs_and_written_spill() {
        let mut matrix = scenarios("numeric-joins").unwrap();
        assert_eq!(matrix.len(), 16);
        let scaled = scenarios("scaled-numeric-joins").unwrap();
        assert_eq!(scaled.len(), 16);
        matrix.extend(scaled);
        for scenario in matrix {
            let [memory, keys, matches, kind, size] = scenario.arguments.as_slice() else {
                unreachable!()
            };
            let rows = numeric_join_rows(size, keys, matches, kind).unwrap();
            let mut output = format!(
                "input kind={kind} left_rows={size} keys={keys} matches={matches} memory_limit={memory} batch_rows=256\nwarmup scratch_bytes=16000000 file_probe=true\n"
            );
            for index in 0..5 {
                output.push_str(&format!("sample={index} elapsed_ns=100 rows={rows} batches=20 progress=30 memory=2000000 temporary=100000\n"));
            }
            output.push_str(&format!("{}\nstatus=finished\n", scenario.marker));
            check_report(&scenario, &output, "").unwrap();
            if size == "262144" {
                for bytes in ["1", memory] {
                    assert!(
                        check_report(
                            &scenario,
                            &output.replace(
                                "scratch_bytes=16000000",
                                &format!("scratch_bytes={bytes}")
                            ),
                            ""
                        )
                        .is_err()
                    );
                }
                assert!(
                    check_report(
                        &scenario,
                        &output.replace("left_rows=262144", "left_rows=8192"),
                        ""
                    )
                    .is_err()
                );
            }
            for bad in [
                output.replace("scratch_bytes=16000000", "scratch_bytes=0"),
                output.replace("file_probe=true", "file_probe=false"),
                output.replace(
                    &format!("rows={rows} batches"),
                    &format!("rows={} batches", rows - 1),
                ),
                output.replace(
                    &format!("rows={rows} batches"),
                    &format!("rows={} batches", rows + 1),
                ),
                output.replace("elapsed_ns=100", "elapsed_ns=0"),
                output.replace("sample=1 ", "sample=2 "),
                output.replace("sample=1 ", "missing=1 "),
                output.replace("memory=2000000", "memory=12000001"),
                output.replace("temporary=100000", "temporary=0"),
                output.replace(
                    "temporary=100000",
                    &format!(
                        "temporary={}",
                        if size == "8192" {
                            16_000_001
                        } else {
                            64_000_001
                        }
                    ),
                ),
                output.replace("status=finished\n", ""),
                format!(
                    "{output}sample=5 elapsed_ns=100 rows={rows} batches=20 progress=30 memory=2000000 temporary=100000\n"
                ),
            ] {
                assert!(check_report(&scenario, &bad, "").is_err(), "accepted {bad}");
            }
        }
        assert!(numeric_join_rows("262145", "32", "1", "inner").is_err());
        assert!(numeric_join_rows("8192", "33", "1", "inner").is_err());
        assert!(numeric_join_rows("8192", "32", "2", "inner").is_err());
        assert!(numeric_join_rows("8192", "32", "1", "outer").is_err());
    }

    #[test]
    fn composed_grouping_requires_complete_samples_and_written_spill() {
        let matrix = scenarios("composed-grouping").unwrap();
        assert_eq!(matrix.len(), 2);
        for scenario in matrix {
            let memory = &scenario.arguments[0];
            let mut output = format!(
                "input groups=4096 rows=8192 memory_limit={memory} batch_rows=256\nwarmup scratch_bytes=4096 file_probe=true\n"
            );
            for i in 0..5 {
                output.push_str(&format!("sample={i} elapsed_ns=100 rows=4096 batches=16 progress=20 memory=100000 temporary=65536\n"));
            }
            output.push_str(&format!("{}\nstatus=finished\n", scenario.marker));
            check_report(&scenario, &output, "").unwrap();
            for bad in [
                output.replace("scratch_bytes=4096", "scratch_bytes=0"),
                output.replace("file_probe=true", "file_probe=false"),
                output.replace("rows=4096", "rows=4095"),
                output.replace("rows=4096", "rows=4097"),
                output.replace("sample=2 ", "sample=1 "),
                output.replace("sample=2 ", "ignored=2 "),
                output.replace("elapsed_ns=100", "elapsed_ns=0"),
                output.replace("memory=100000", "memory=99999999"),
                output.replace("temporary=65536", "temporary=0"),
                output.replace("temporary=65536", "temporary=8000001"),
                output.replace("status=finished\n", ""),
                format!(
                    "{output}sample=5 elapsed_ns=100 rows=4096 batches=16 progress=20 memory=100000 temporary=65536\n"
                ),
            ] {
                assert!(check_report(&scenario, &bad, "").is_err(), "accepted {bad}");
            }
        }
    }

    #[test]
    fn measurements_require_completion_samples_and_finite_time() {
        let scenario = Scenario::new("legacy_example", &[], "verified".into(), 3);
        let good = "verified\nsampled logical bytes: memory=5, temporary=7\nexecution and validation seconds=0.1\n";
        check_report(&scenario, good, "").unwrap();
        for bad in [
            String::new(),
            good.replace("verified", ""),
            format!("{good}verified\n"),
            good.replace("0.1", "NaN"),
            good.replace("0.1", "-1"),
            good.replace("execution and validation seconds=0.1\n", ""),
        ] {
            assert!(check_report(&scenario, &bad, "").is_err());
        }
        let scenario = Scenario::new("projection_cost", &[], "verified".into(), 1);
        let mut samples = "verified\n".to_owned();
        for i in 1..=10 {
            samples.push_str(&format!("sample={i} staged_seconds=1 single_seconds=2\n"));
        }
        samples.push_str("staged sampled additional logical memory=10 temporary=0\nsingle sampled additional logical memory=8 temporary=0\n");
        check_report(&scenario, &samples, "").unwrap();
        assert!(check_report(&scenario, &samples.replace("sample=7 ", "sample=8 "), "").is_err());
        assert!(check_report(&scenario, &format!("{samples}sample=11 extra\n"), "").is_err());
        assert!(
            check_report(
                &scenario,
                &samples.replace("single_seconds=2", "single_seconds=NaN"),
                ""
            )
            .is_err()
        );
        let scenario = Scenario::new(
            "operator_cost",
            &["distinct", "8192", "32", "8", "8000000"],
            "status=finished".into(),
            1,
        );
        let mut output = "input operation=distinct rows=8192 keys=32 text_bytes=8 memory_limit=8000000 dimension_batch_rows=64 fact_batch_rows=64\nwarmup scratch_bytes=256 file_probe=true\n".to_owned();
        for sample in 0..5 {
            output.push_str(&format!("sample={sample} elapsed_ns=100 rows=32 batches=32 progress=50 memory=100000 temporary=1000\n"));
        }
        output.push_str("status=finished\n");
        check_report(&scenario, &output, "").unwrap();
        for (from, to) in [
            ("rows=32", "rows=31"),
            ("scratch_bytes=256", "scratch_bytes=0"),
            ("file_probe=true", "file_probe=false"),
            ("temporary=1000", "temporary=0"),
            ("elapsed_ns=100", "elapsed_ns=NaN"),
        ] {
            assert!(check_report(&scenario, &output.replace(from, to), "").is_err());
        }
    }
}
