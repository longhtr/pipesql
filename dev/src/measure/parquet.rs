//! Measure fresh Parquet output with the separately provisioned PyArrow reader.
//!
//! The stock producer owns API outcomes and lifetime checks. The external reader
//! owns independent values and group cuts; this supervisor requires its complete
//! receipt, matches files to producer reports, and owns processes and cleanup.

use super::{Scenario, integer_fields, one_line};
use crate::{
    Result,
    workspace::{self, Run},
};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, time::Duration};

const ORACLE: &str = "test/data/parquet/interop.py";
const REJECTIONS: &[&str] = &[
    "wrong_value",
    "duplicate_row",
    "missing_row",
    "prefix",
    "missing_footer",
    "footer_magic",
    "ordered_rows",
];

pub(super) fn scenarios(selected: &mut Vec<Scenario>) {
    // Empty output and the near-valid reader mutations precede the larger trials.
    for (query, profile, rows, text) in [
        ("scan", "plain8", "0", "4096"),
        ("order", "escaped1024", "257", "65536"),
    ] {
        selected.push(Scenario::new(
            "export_cost",
            &[query, profile, "64", "127", rows, "parquet", "128", text],
            "status=exported".into(),
            1,
        ));
    }
    // Each variant changes one bound from its query/text baseline. Row and text
    // capacities share the query's memory budget, so their cost includes any
    // resulting change in query execution as well as encoding.
    for query in ["scan", "order"] {
        for profile in ["plain8", "plain1024", "escaped1024"] {
            let base = [
                query,
                profile,
                "256",
                "1024",
                "8192",
                "parquet",
                "311",
                if profile == "plain8" {
                    "4096"
                } else {
                    "131072"
                },
            ];
            let mut inputs = vec![base];
            let mut rows = base;
            rows[6] = "128";
            inputs.push(rows);
            if profile != "plain8" {
                let mut text = base;
                text[7] = "65536";
                inputs.push(text);
            }
            if profile != "plain1024" {
                let mut batch = base;
                batch[2] = "64";
                inputs.push(batch);
                let mut fragment = base;
                fragment[3] = "127";
                inputs.push(fragment);
            }
            for arguments in inputs {
                selected.push(Scenario::new(
                    "export_cost",
                    &arguments,
                    "status=exported".into(),
                    3,
                ));
            }
        }
    }
}

fn python() -> Command {
    let mut command = Command::new("python3");
    // Ignore inherited Python configuration and do not write into frozen source.
    command.args(["-I", "-B"]);
    command
}

fn failure_names(rows: u64) -> Vec<&'static str> {
    let mut names = Vec::new();
    if rows != 0 {
        names.extend([
            "write", "cancel", "rows", "bytes", "groups", "text", "metadata",
        ]);
    }
    if rows == 8192 {
        names.push("query");
    }
    names
}

fn receipt(value: &Value, scenario: &Scenario, path: &Path, single: bool) -> Result<()> {
    let [query, profile, _, _, rows, format, group_rows, group_text] =
        scenario.arguments.as_slice()
    else {
        return Err("incomplete Parquet selection".into());
    };
    let rows: u64 = rows.parse()?;
    let group_rows: u64 = group_rows.parse()?;
    let group_text: u64 = group_text.parse()?;
    if format != "parquet"
        || value["status"] != "checked"
        || value["reader"] != "22.0.0"
        || value["query"] != *query
        || value["profile"] != *profile
        || value["rows"].as_u64() != Some(rows)
        || value["group_rows"].as_u64() != Some(group_rows)
        || value["group_text"].as_u64() != Some(group_text)
    {
        return Err("external Parquet reader did not check the selected input".into());
    }
    let paths = if single {
        vec![path.to_owned()]
    } else {
        let mut names = vec!["warmup".to_owned()];
        names.extend((0..5).map(|sample| format!("sample-{sample}")));
        names.push("retry".to_owned());
        if rows != 0 {
            names.push("failure-flush".to_owned());
        }
        names
            .into_iter()
            .map(|name| path.join(format!("{name}.parquet")))
            .collect()
    };
    let files = value["files"]
        .as_array()
        .filter(|files| files.len() == paths.len())
        .ok_or("external Parquet reader omitted or duplicated files")?;
    for (file, path) in files.iter().zip(paths) {
        let length = fs::metadata(&path)?.len();
        if file["file"].as_str() != path.file_name().and_then(|name| name.to_str())
            || file["rows"].as_u64() != Some(rows)
            || !(12..=64_000_000).contains(&length)
            || file["bytes"].as_u64() != Some(length)
            || file["validation_ns"]
                .as_u64()
                .is_none_or(|value| value == 0)
            || file["sha256"].as_str() != Some(workspace::hash(&path)?.as_str())
        {
            return Err("external Parquet receipt differs from the complete file".into());
        }
        let groups = file["groups"].as_array().ok_or("missing Parquet groups")?;
        let mut count = 0;
        for group in groups {
            let group = group
                .as_u64()
                .filter(|n| (1..=group_rows).contains(n))
                .ok_or("invalid Parquet group count")?;
            count += group;
        }
        if groups.len() > 256 || count != rows {
            return Err("Parquet groups do not cover the result".into());
        }
    }
    let names = if single {
        Vec::new()
    } else {
        failure_names(rows)
    };
    let failed = value["failed_exports"]
        .as_array()
        .filter(|files| files.len() == names.len())
        .ok_or("external reader omitted failed exports")?;
    for (file, name) in failed.iter().zip(names) {
        let name = format!("failure-{name}.parquet");
        let bytes = fs::metadata(path.join(&name))?.len();
        if file["file"] != name
            || file["bytes"].as_u64() != Some(bytes)
            || !(1..=64_000_000).contains(&bytes)
        {
            return Err("failed Parquet receipt differs from its prefix".into());
        }
    }
    let rejections: &[&str] = if !single && rows == 257 {
        REJECTIONS
    } else {
        &[]
    };
    if value["reader_rejections"] != json!(rejections) {
        return Err("Parquet reader controls did not complete".into());
    }
    Ok(())
}

fn read(run: &mut Run, scenario: &Scenario, path: &Path, single: bool) -> Result<Value> {
    let a = &scenario.arguments;
    let mut command = python();
    command
        .arg(ORACLE)
        .arg(if single {
            "--read-profile-file"
        } else {
            "--read-profile"
        })
        .arg(path)
        .args([&a[0], &a[1], &a[4], &a[6], &a[7]]);
    let output = run.command(&mut command, None, Duration::from_secs(120))?;
    if output.require_success().is_err() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
    }
    output.require_success()?;
    let value = serde_json::from_slice(&output.stdout.bytes)?;
    receipt(&value, scenario, path, single)?;
    Ok(value)
}

pub(super) fn controls(run: &mut Run, executable: &Path) -> Result<()> {
    let output = run.command(
        python().arg(ORACLE).arg("--describe"),
        None,
        Duration::from_secs(30),
    )?;
    output.require_success()?;
    let environment: Value = serde_json::from_slice(&output.stdout.bytes)?;
    if environment["pyarrow"] != "22.0.0"
        || environment["python"].as_str().is_none_or(str::is_empty)
        || environment["module"].as_str().is_none_or(str::is_empty)
    {
        return Err("Parquet reader environment was not identified".into());
    }
    fs::write(
        run.directory.join("parquet-reader.json"),
        serde_json::to_vec_pretty(&environment)?,
    )?;
    let output = run.command(
        python().arg("-O").arg(ORACLE).arg("--describe"),
        None,
        Duration::from_secs(30),
    )?;
    if output.require_success().is_ok()
        || !std::str::from_utf8(&output.stderr.bytes)?
            .contains("fixture checks require Python assertions")
    {
        return Err("disabled reader assertions were not rejected".into());
    }
    let scenario = Scenario::new(
        "export_cost",
        &[
            "order", "plain8", "64", "127", "257", "parquet", "128", "4096",
        ],
        "status=exported".into(),
        1,
    );
    let directory = run.directory.join("parquet-missing-flush-fault");
    let output = run.command(
        Command::new(executable)
            .arg(&directory)
            .args(&scenario.arguments)
            .env("PIPESQL_EXPORT_CONTROL", "skip-flush-fault"),
        None,
        Duration::from_secs(300),
    )?;
    if output.require_success().is_ok()
        || !std::str::from_utf8(&output.stderr.bytes)?.contains("export fault must fail")
    {
        return Err("disabled Parquet flush fault was not detected".into());
    }
    let path = directory.join("failure-flush.parquet");
    let verified = read(run, &scenario, &path, true)?;
    let output = run.command(
        python().args(["-c", "print('{}')"]),
        None,
        Duration::from_secs(30),
    )?;
    output.require_success()?;
    let missing: Value = serde_json::from_slice(&output.stdout.bytes)?;
    if receipt(&missing, &scenario, &path, true).is_ok() {
        return Err("missing Parquet checker execution was accepted".into());
    }
    for field in [
        "files",
        "failed_exports",
        "reader_rejections",
        "status",
        "rows",
    ] {
        let mut wrong = verified.clone();
        wrong.as_object_mut().unwrap().remove(field);
        if receipt(&wrong, &scenario, &path, true).is_ok() {
            return Err(format!("missing Parquet receipt field {field} accepted").into());
        }
    }
    fs::write(
        run.directory.join("parquet-controls.json"),
        serde_json::to_vec_pretty(&json!({
            "assertions_required":true, "missing_checker_rejected":true,
            "missing_flush_fault_rejected":true, "receipt_rejections":5, "complete_file":verified
        }))?,
    )?;
    fs::remove_dir_all(directory)?;
    Ok(())
}

pub(super) fn verify(
    run: &mut Run,
    scenario: &Scenario,
    directory: &Path,
    report: &str,
) -> Result<Value> {
    super::export::check_report(scenario, report)?;
    let mut value = read(run, scenario, directory, false)?;
    for file in value["files"].as_array_mut().unwrap() {
        let name = file["file"]
            .as_str()
            .unwrap()
            .strip_suffix(".parquet")
            .unwrap();
        let prefix = if name == "warmup" || name == "retry" {
            format!("{name} ")
        } else if name == "failure-flush" {
            "failure=flush ".into()
        } else {
            format!("sample={} ", name.strip_prefix("sample-").unwrap())
        };
        let line = one_line(report, &prefix)?;
        let bytes = line
            .split_whitespace()
            .find_map(|word| word.strip_prefix("bytes="))
            .ok_or("producer omitted export bytes")?
            .parse::<u64>()?;
        if file["bytes"].as_u64() != Some(bytes) {
            return Err("Parquet producer byte count differs".into());
        }
        file["api_outcome"] = json!(if name == "failure-flush" {
            "expected_flush_error"
        } else {
            "success"
        });
    }
    for file in value["failed_exports"].as_array().unwrap() {
        let name = file["file"]
            .as_str()
            .unwrap()
            .strip_prefix("failure-")
            .unwrap()
            .strip_suffix(".parquet")
            .unwrap();
        let line = one_line(report, &format!("failure={name} "))?;
        let line = line
            .strip_suffix(if name == "query" {
                " span=36:60 released=true"
            } else {
                " released=true"
            })
            .ok_or("failed Parquet outcome missing")?;
        let fields = integer_fields(line, &["bytes", "writes", "flushes"])?;
        if file["bytes"].as_u64() != Some(fields[0]) {
            return Err("Parquet failure prefix differs".into());
        }
    }
    Ok(value)
}
