//! Qualify Parquet transfers with independent typed expectations.
//!
//! PyArrow generates inputs and checks wide exports. A stock-library driver owns
//! API outcomes; this supervisor judges reopened JSON Lines for imports and
//! requires complete external-reader receipts for exports. Neither value check
//! relies on an engine encode/decode round trip.

use crate::{
    Result,
    workspace::{self, Run},
};
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Cursor},
    path::Path,
    process::Command,
    time::Duration,
};

mod measure;
mod wide;
mod wide_import;
mod wide_measure;

const ORACLE: &str = "test/data/parquet/interop.py";

fn stack_report(text: &str) -> Result<(Value, &str)> {
    let (line, remaining) = text
        .split_once('\n')
        .ok_or("missing transfer stack report")?;
    let (requested, rest) = line
        .strip_prefix("stack requested=")
        .and_then(|line| line.split_once(" reported="))
        .ok_or("invalid transfer stack report")?;
    let (reported, limit) = rest
        .split_once(" limit=")
        .ok_or("missing transfer stack limit")?;
    let (requested, reported, limit): (u64, u64, u64) =
        (requested.parse()?, reported.parse()?, limit.parse()?);
    // This is the established native-test ceiling, not a child-selected bound.
    let ceiling = if cfg!(all(
        target_os = "linux",
        target_arch = "aarch64",
        target_env = "gnu"
    )) {
        144 * 1024
    } else {
        64 * 1024
    };
    if requested != 48 * 1024 || limit != ceiling || !(requested..=ceiling).contains(&reported) {
        return Err("transfer stack extent differs from its bound".into());
    }
    Ok((
        json!({"requested":requested, "reported":reported, "limit":limit}),
        remaining,
    ))
}

fn stack_control(run: &mut Run, driver: &Path) -> Result<()> {
    let trial = run.directory.join("oversized-transfer-stack");
    let output = run.command(
        Command::new(driver)
            .arg(&trial)
            .arg("--wide-export")
            .env("PIPESQL_TRANSFER_STACK_CONTROL", "oversized")
            .env_remove("PIPESQL_IMPORT_CONTROL"),
        None,
        Duration::from_secs(30),
    )?;
    if output.require_success().is_ok()
        || !String::from_utf8_lossy(&output.stderr.bytes).contains("wide transfer stack extent")
        || !output.stdout.bytes.is_empty()
        || trial.exists()
    {
        return Err("oversized transfer stack was not rejected before engine work".into());
    }
    fs::write(
        run.directory.join("stack-control.json"),
        serde_json::to_vec_pretty(&json!({"oversized_rejected":true, "engine_not_entered":true}))?,
    )?;
    Ok(())
}

struct Case {
    version: &'static str,
    rows: usize,
    group: usize,
    page: usize,
    batch: usize,
    dense: bool,
}

const CASES: &[Case] = &[
    Case {
        version: "1.0",
        rows: 777,
        group: 389,
        page: 1024,
        batch: 37,
        dense: false,
    },
    Case {
        version: "2.0",
        rows: 777,
        group: 257,
        page: 4096,
        batch: 113,
        dense: false,
    },
    Case {
        version: "1.0",
        rows: 8192,
        group: 257,
        page: 4096,
        batch: 37,
        dense: false,
    },
    Case {
        version: "2.0",
        rows: 8192,
        group: 389,
        page: 1024,
        batch: 113,
        dense: false,
    },
    Case {
        version: "1.0",
        rows: 8192,
        group: 255,
        page: 4096,
        batch: 37,
        dense: false,
    },
    Case {
        version: "1.0",
        rows: 8192,
        group: 256,
        page: 4096,
        batch: 37,
        dense: false,
    },
    Case {
        version: "1.0",
        rows: 777,
        group: 513,
        page: 1024,
        batch: 37,
        dense: true,
    },
    Case {
        version: "2.0",
        rows: 777,
        group: 513,
        page: 4096,
        batch: 113,
        dense: true,
    },
];

fn python() -> Command {
    let mut command = Command::new("python3");
    command.args(["-I", "-B"]);
    command
}

fn manifest(value: &Value, directory: &Path) -> Result<()> {
    if value["status"] != "generated"
        || value["reader"] != "22.0.0"
        || value["row_order"] != "descending"
    {
        return Err("external Parquet input generation did not complete".into());
    }
    let inputs = value["inputs"]
        .as_array()
        .filter(|v| v.len() == CASES.len())
        .ok_or("external generator omitted or duplicated inputs")?;
    for (id, (input, case)) in inputs.iter().zip(CASES).enumerate() {
        let name = format!("input-{id}.parquet");
        let path = directory.join(&name);
        let groups: Vec<_> = (0..case.rows)
            .step_by(case.group)
            .map(|start| case.group.min(case.rows - start))
            .collect();
        if input["file"] != name
            || input["version"] != case.version
            || input["rows"] != case.rows
            || input["group_rows"] != case.group
            || input["page_target"] != case.page
            || input["write_batch"] != case.batch
            || input["dense"] != case.dense
            || input["groups"] != json!(groups)
            || input["bytes"].as_u64() != Some(fs::metadata(&path)?.len())
            || input["bytes"]
                .as_u64()
                .is_none_or(|n| !(12..=2_000_000).contains(&n))
            || input["sha256"] != workspace::hash(&path)?
        {
            return Err("external Parquet input differs from the selection or file".into());
        }
    }
    Ok(())
}

fn schema() -> Value {
    json!({"format":"pipesql-jsonl", "version":1, "columns":[
        {"name":"id", "type":"int64", "nullable":false},
        {"name":"amount", "type":"int64", "nullable":true},
        {"name":"number", "type":"double", "nullable":true},
        {"name":"day", "type":"date", "nullable":true},
        {"name":"note", "type":"string", "nullable":true}
    ]})
}

fn row(id: usize, dense: bool) -> Value {
    let amounts = [
        "-9223372036854775808",
        "9223372036854775807",
        "-1",
        "0",
        "1",
        "42",
        "-42",
    ];
    let bits = [
        "0000000000000000",
        "8000000000000000",
        "7ff0000000000000",
        "fff0000000000000",
        "7ff8000000001234",
        "fff0000000000001",
        "0000000000000001",
    ];
    let dates = [
        "0001-01-01",
        "9999-12-31",
        "1969-12-31",
        "1970-01-01",
        "1970-01-02",
        "2000-02-29",
        "2020-01-01",
    ];
    let note = if dense && id >= 768 {
        json!("x".repeat(65_536))
    } else if id % 7 == 2 {
        Value::Null
    } else if matches!(id, 255 | 645) {
        json!("x".repeat(65_536))
    } else if id % 17 == 4 {
        json!("")
    } else {
        json!(format!("{id}:é🙂\n\0{}", "q".repeat(id % 101)))
    };
    json!({"row":[id.to_string(), (id % 11 != 3).then_some(amounts[id % 7]),
        (id % 13 != 5).then_some(bits[id % 7]), (id % 19 != 7).then_some(dates[id % 7]), note]})
}

fn validate(reader: impl BufRead, rows: usize, dense: bool) -> Result<()> {
    let mut lines = reader.lines();
    let mut next = || -> Result<Value> {
        Ok(serde_json::from_str(
            &lines.next().ok_or("missing imported output record")??,
        )?)
    };
    if next()? != schema() || next()? != json!({"row":["-1", null, null, null, "existing"]}) {
        return Err("imported schema or existing sentinel differs".into());
    }
    for id in 0..rows {
        if next()? != row(id, dense) {
            return Err(format!("imported row {id} differs").into());
        }
    }
    if next()? != json!({"complete":true, "rows":rows + 1}) || lines.next().is_some() {
        return Err("imported completion or trailing data differs".into());
    }
    Ok(())
}

fn file(path: &Path, rows: usize, dense: bool) -> Result<Value> {
    let length = fs::metadata(path)?.len();
    if length > 8_000_000 {
        return Err("imported output exceeds its byte bound".into());
    }
    validate(BufReader::new(File::open(path)?), rows, dense)?;
    Ok(
        json!({"file":path.file_name().and_then(|n| n.to_str()).ok_or("non-UTF-8 filename")?,
              "rows":rows + 1, "bytes":length, "sha256":workspace::hash(path)?}),
    )
}

fn counts(line: &str, prefix: &str, suffix: &str) -> Result<[u64; 3]> {
    let line = line
        .strip_prefix(prefix)
        .and_then(|s| s.strip_suffix(suffix))
        .ok_or("missing import outcome or release")?;
    let mut words = line.split_whitespace();
    let mut values = [0; 3];
    for (value, key) in values
        .iter_mut()
        .zip(["read_calls=", "read_bytes=", "end_seeks="])
    {
        *value = words
            .next()
            .and_then(|word| word.strip_prefix(key))
            .ok_or("missing input observation")?
            .parse()?;
    }
    if words.next().is_some() {
        return Err("extra import observation".into());
    }
    Ok(values)
}

fn report(text: &str, case: &Case, fragment: usize, length: u64) -> Result<()> {
    let lines: Vec<_> = text.lines().collect();
    if lines.len() != 6
        || lines[0]
            != format!(
                "input rows={} fragment={} bytes={length}",
                case.rows, fragment
            )
        || lines[5] != "status=imported"
    {
        return Err("import selection or completion differs".into());
    }
    for (index, name) in ["receipt", "source", "cancel", "complete"]
        .iter()
        .enumerate()
    {
        let (prefix, suffix) = if *name == "complete" {
            (
                format!("complete rows={} ", case.rows + 1),
                " durable=true released=true reopened=true",
            )
        } else {
            (
                format!("failure={name} "),
                " aborted=true released=true reopened=true",
            )
        };
        let [calls, bytes, seeks] = counts(lines[index + 1], &prefix, suffix)?;
        if bytes == 0
            || calls < bytes.div_ceil(fragment as u64)
            || calls > bytes + 1
            || (*name == "receipt" && (bytes <= 12 || bytes >= length || seeks != 1))
            || (*name != "receipt" && (bytes != length || seeks != 2))
        {
            return Err("import did not exercise the selected source boundary".into());
        }
    }
    Ok(())
}

fn reader_controls(path: &Path, rows: usize) -> Result<usize> {
    let original = fs::read_to_string(path)?;
    let records: Vec<String> = original.lines().map(str::to_owned).collect();
    let mut mutations = Vec::new();
    let mut wrong = records.clone();
    let mut value: Value = serde_json::from_str(&wrong[3])?;
    value["row"][1] = json!("0");
    wrong[3] = value.to_string();
    mutations.push(wrong);
    let mut wrong = records.clone();
    wrong[3] = wrong[2].clone();
    mutations.push(wrong);
    let mut wrong = records.clone();
    wrong.remove(3);
    mutations.push(wrong);
    let mut wrong = records.clone();
    wrong.swap(2, 3);
    mutations.push(wrong);
    let mut wrong = records.clone();
    wrong.pop();
    mutations.push(wrong);
    let mut wrong = records.clone();
    wrong.truncate(3);
    mutations.push(wrong);
    let mut wrong = records.clone();
    wrong.push("{}".into());
    mutations.push(wrong);
    let mut wrong = records.clone();
    let mut header = schema();
    header["columns"][2]["type"] = json!("int64");
    wrong[0] = header.to_string();
    mutations.push(wrong);
    for wrong in &mutations {
        let wrong = wrong.join("\n");
        if validate(Cursor::new(wrong), rows, false).is_ok() {
            return Err("import reader accepted a wrong or incomplete result".into());
        }
    }
    if mutations.len() != 8 {
        return Err("import reader controls missing".into());
    }
    Ok(mutations.len())
}

fn report_controls(text: &str, case: &Case, fragment: usize, length: u64) -> Result<usize> {
    let mut mutations = vec![
        text.replacen("released=true", "released=false", 1),
        text.replacen("failure=source", "failure=receipt", 1),
        text.replacen("end_seeks=2", "end_seeks=1", 1),
        text.replacen("complete rows=", "complete invalid=", 1),
        text.replacen("status=imported", "", 1),
        format!("{text}status=imported\n"),
    ];
    mutations.push(text.replacen(
        &format!("input rows={}", case.rows),
        &format!("input rows={}", case.rows + 1),
        1,
    ));
    mutations.push(text.replacen(
        &format!("read_bytes={length}"),
        &format!("read_bytes={}", length - 1),
        1,
    ));
    for wrong in &mutations {
        if wrong == text || report(wrong, case, fragment, length).is_ok() {
            return Err("import report checker accepted a missing or wrong outcome".into());
        }
    }
    Ok(mutations.len())
}

fn dense_layout(native: &Value) -> Result<()> {
    let rows: Vec<_> = native["units"]
        .as_array()
        .ok_or("missing native units")?
        .iter()
        .map(|unit| unit["rows"].as_u64())
        .collect();
    // Sentinel, seven maximum strings, the rest of the 512-row lend, its group
    // remainder, then the next group. This observes the current split policy.
    if rows != [Some(1), Some(7), Some(505), Some(1), Some(264)] {
        return Err("dense input did not exercise both lending and native column cuts".into());
    }
    Ok(())
}

fn boundary_controls(path: &Path, native: &Value) -> Result<usize> {
    let original = fs::read_to_string(path)?;
    let lines: Vec<_> = original.lines().map(str::to_owned).collect();
    for (id, column, wrong) in [
        (264, 1, json!("0")),
        (265, 2, json!("0000000000000000")),
        (769, 4, json!("x".repeat(65_535))),
        (770, 4, json!("x".repeat(65_535))),
    ] {
        let mut changed = lines.clone();
        let mut record: Value = serde_json::from_str(&changed[id + 2])?;
        if record["row"][column] == wrong {
            return Err("boundary mutation changed nothing".into());
        }
        record["row"][column] = wrong;
        changed[id + 2] = record.to_string();
        if validate(Cursor::new(changed.join("\n")), 777, true).is_ok() {
            return Err("dense reader accepted wrong values at a split".into());
        }
    }
    let mut wrong = native.clone();
    wrong["units"][1]["rows"] = json!(8);
    wrong["units"][2]["rows"] = json!(504);
    if dense_layout(&wrong).is_ok() {
        return Err("dense layout accepted a wrong cut with the same total rows".into());
    }
    Ok(5)
}

fn reader(run: &mut Run) -> Result<()> {
    let environment = run.command(
        python().arg(ORACLE).arg("--describe"),
        None,
        Duration::from_secs(30),
    )?;
    environment.require_success()?;
    let environment: Value = serde_json::from_slice(&environment.stdout.bytes)?;
    if environment["pyarrow"] != "22.0.0" {
        return Err("wrong external Parquet reader".into());
    }
    fs::write(
        run.directory.join("reader.json"),
        serde_json::to_vec_pretty(&environment)?,
    )?;
    let disabled = run.command(
        python().arg("-O").arg(ORACLE).arg("--describe"),
        None,
        Duration::from_secs(30),
    )?;
    if disabled.require_success().is_ok()
        || !std::str::from_utf8(&disabled.stderr.bytes)?
            .contains("fixture checks require Python assertions")
    {
        return Err("disabled external assertions accepted".into());
    }
    Ok(())
}

pub fn measure_wide() -> Result<()> {
    let mut run = Run::new(workspace::root()?, "parquet-wide-measure")?;
    let before = workspace::tree_contents(&run.root)?;
    reader(&mut run)?;
    let driver = run.build("pipesql-driver", "--bin", "parquet-transfer")?;
    stack_control(&mut run, &driver)?;
    wide::run(&mut run, &driver)?;
    wide_measure::run(&mut run, &driver)?;
    if workspace::tree_contents(&run.root)? != before {
        return Err("wide export measurement changed its source".into());
    }
    run.finish()
}

pub fn run(measuring: bool) -> Result<()> {
    let mut run = Run::new(workspace::root()?, "parquet-transfer")?;
    let before = workspace::tree_contents(&run.root)?;
    reader(&mut run)?;
    let directory = run.directory.join("inputs");
    let output = run.command(
        python().arg(ORACLE).arg("--write-imports").arg(&directory),
        None,
        Duration::from_secs(60),
    )?;
    if output.require_success().is_err() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
    }
    output.require_success()?;
    let inputs: Value = serde_json::from_slice(&output.stdout.bytes)?;
    manifest(&inputs, &directory)?;
    fs::write(
        run.directory.join("inputs.json"),
        serde_json::to_vec_pretty(&inputs)?,
    )?;
    let missing = run.command(
        python().args(["-c", "print('{}')"]),
        None,
        Duration::from_secs(30),
    )?;
    missing.require_success()?;
    if manifest(&serde_json::from_slice(&missing.stdout.bytes)?, &directory).is_ok() {
        return Err("missing external generation was accepted".into());
    }
    let driver = run.build("pipesql-driver", "--bin", "parquet-transfer")?;
    let control = run.directory.join("missing-source-fault");
    let output = run.command(
        Command::new(&driver)
            .arg(&control)
            .arg(directory.join("input-0.parquet"))
            .args(["777", "7"])
            .env("PIPESQL_IMPORT_CONTROL", "skip-source-fault"),
        None,
        Duration::from_secs(300),
    )?;
    if output.require_success().is_ok()
        || !std::str::from_utf8(&output.stderr.bytes)?.contains("import fault must fail")
    {
        return Err("disabled source fault was not detected".into());
    }
    let verified = file(&control.join("unexpected-success.jsonl"), 777, false)?;
    fs::write(
        run.directory.join("controls.json"),
        serde_json::to_vec_pretty(&json!({
            "assertions_required":true, "missing_generation_rejected":true,
            "disabled_source_fault_rejected":true, "complete_control":verified
        }))?,
    )?;
    fs::remove_dir_all(control)?;
    let mut verified = Vec::new();
    let mut boundary_checked = false;
    for (id, case) in CASES.iter().enumerate() {
        let input = directory.join(format!("input-{id}.parquet"));
        for fragment in [7, 127] {
            let trial = run.directory.join(format!("case-{id}-{fragment}"));
            let mut command = Command::new(&driver);
            command
                .arg(&trial)
                .arg(&input)
                .args([case.rows.to_string(), fragment.to_string()])
                .env_remove("PIPESQL_IMPORT_CONTROL");
            if case.dense {
                command.arg("--dense");
            }
            let output = run.command(&mut command, None, Duration::from_secs(300))?;
            if output.require_success().is_err() {
                eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
            }
            output.require_success()?;
            let text = std::str::from_utf8(&output.stdout.bytes)?;
            let length = fs::metadata(&input)?.len();
            report(text, case, fragment, length)?;
            let mut files = Vec::new();
            for name in ["receipt", "source", "cancel", "complete"] {
                files.push(file(
                    &trial.join(format!("{name}.jsonl")),
                    if name == "complete" { case.rows } else { 0 },
                    case.dense,
                )?);
            }
            let controls = if id == 0 && fragment == 7 {
                reader_controls(&trial.join("complete.jsonl"), case.rows)?
            } else {
                0
            };
            let reports = if id == 0 && fragment == 7 {
                report_controls(text, case, fragment, length)?
            } else {
                0
            };
            let native = if case.dense {
                let native = measure::storage(&trial.join("database"), case.rows)?;
                dense_layout(&native)?;
                Some(native)
            } else {
                None
            };
            let boundary_rejections = if let Some(native) = &native
                && !boundary_checked
            {
                let count = boundary_controls(&trial.join("complete.jsonl"), native)?;
                boundary_checked = true;
                count
            } else {
                0
            };
            verified.push(
                json!({"case":id, "fragment":fragment, "files":files, "reader_rejections":controls,
                             "report_rejections":reports, "native":native, "boundary_rejections":boundary_rejections}),
            );
            fs::remove_dir_all(trial)?;
        }
    }
    if verified.len() != CASES.len() * 2 || !boundary_checked {
        return Err("import qualification cases did not complete".into());
    }
    stack_control(&mut run, &driver)?;
    wide::run(&mut run, &driver)?;
    wide_import::run(&mut run, &driver)?;
    if measuring {
        measure::run(&mut run, &driver, &directory)?;
    }
    manifest(&inputs, &directory)?;
    if workspace::tree_contents(&run.root)? != before {
        return Err("import qualification changed its source".into());
    }
    fs::write(
        run.directory.join("verification.json"),
        serde_json::to_vec_pretty(&verified)?,
    )?;
    fs::remove_dir_all(directory)?;
    println!(
        "Parquet transfer qualified: eight fresh inputs, sixteen read-fragment cases, complete reopened values and aborted-prefix controls"
    );
    run.finish()
}

#[cfg(test)]
mod stack_tests {
    use super::*;

    #[test]
    fn transfer_stack_reports_require_observed_extent_and_fixed_limits() {
        let limit = if cfg!(all(
            target_os = "linux",
            target_arch = "aarch64",
            target_env = "gnu"
        )) {
            147456
        } else {
            65536
        };
        let report =
            format!("stack requested=49152 reported=49152 limit={limit}\nstatus=finished\n");
        let (stack, remaining) = stack_report(&report).unwrap();
        assert_eq!(
            stack,
            json!({"requested":49152, "reported":49152, "limit":limit})
        );
        assert_eq!(remaining, "status=finished\n");
        for wrong in [
            report.replace("requested=49152", "requested=2097152"),
            report.replace("reported=49152", "reported=0"),
            report.replace("reported=49152", "reported=49151"),
            report.replace("reported=49152", &format!("reported={}", limit + 1)),
            report.replace(&format!("limit={limit}"), "limit=2097152"),
            report.replace(" reported=49152", ""),
            report.replace("stack ", ""),
            report.replace('\n', ""),
            format!("status=finished\n{report}"),
        ] {
            assert!(stack_report(&wrong).is_err(), "accepted {wrong}");
        }
        assert!(stack_report("").is_err());
    }
}
