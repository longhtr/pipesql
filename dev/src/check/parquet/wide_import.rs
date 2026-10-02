//! Judge repeated-type imports independently of engine conversion and output.
//!
//! PyArrow writes permuted columns; the driver declares them in reverse order.
//! Literal expected JSON cells check the final canonical order after reopen.

use super::{ORACLE, python};
use crate::{
    Result,
    workspace::{self, Run},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Cursor},
    path::Path,
    process::Command,
    time::Duration,
};

const REPORT: &str = "wide-import rows=513 columns=64 aborted=true durable=true released=true reopened=true status=finished\n";

fn schema() -> Value {
    json!({"format":"pipesql-jsonl", "version":1, "columns":(0..64).map(|column| json!({
        "name":format!("c{column:02}"),
        "type":(["int64", "double", "date", "string"][column % 4]),
        "nullable":column % 3 != 0
    })).collect::<Vec<_>>()})
}

fn row(id: Option<usize>) -> Value {
    let cells: Vec<_> = (0..64)
        .map(|column| {
            let Some(row) = id else {
                return json!(match column % 4 {
                    0 if column == 0 => "-1",
                    0 => "0",
                    1 => "0000000000000000",
                    2 => "1970-01-01",
                    _ => "existing",
                });
            };
            if column == 0 {
                return json!(row.to_string());
            }
            if column % 3 != 0 && (row + 7 * column) % 11 == 0 {
                return Value::Null;
            }
            match column % 4 {
                0 => json!(
                    match row % 7 {
                        0 => i64::MIN + column as i64,
                        1 => i64::MAX - column as i64,
                        _ => (row * 1000 + column) as i64,
                    }
                    .to_string()
                ),
                1 => {
                    let bits = match row % 8 {
                        0 => 0,
                        1 => 0x8000_0000_0000_0000,
                        2 => 0x7ff0_0000_0000_0000,
                        3 => 0xfff0_0000_0000_0000,
                        4 => 0x7ff8_0000_0000_0100 | column as u64,
                        5 => 1 + column as u64,
                        6 => ((row * 64 + column) as f64).to_bits(),
                        _ => (-((row * 64 + column) as f64)).to_bits(),
                    };
                    json!(format!("{bits:016x}"))
                }
                2 => json!(format!(
                    "{}-01-{:02}",
                    if row % 2 == 0 { 2000 } else { 9999 },
                    column / 4 + 1
                )),
                _ if row % 13 == 0 => json!(""),
                _ => json!(format!("c{column:02}:r{row}:雪\0é🙂")),
            }
        })
        .collect();
    json!({"row":cells})
}

fn validate(reader: impl BufRead, imported: bool) -> Result<()> {
    let mut lines = reader.lines();
    let mut next = || -> Result<Value> {
        Ok(serde_json::from_str(
            &lines.next().ok_or("missing wide import record")??,
        )?)
    };
    if next()? != schema() || next()? != row(None) {
        return Err("wide imported schema or sentinel differs".into());
    }
    if imported {
        for id in 0..513 {
            if next()? != row(Some(id)) {
                return Err(format!("wide imported row {id} differs").into());
            }
        }
    }
    if next()? != json!({"complete":true, "rows":if imported {514} else {1}})
        || lines.next().is_some()
    {
        return Err("wide import completion or trailing data differs".into());
    }
    Ok(())
}

fn file(path: &Path, imported: bool) -> Result<Value> {
    let bytes = fs::metadata(path)?.len();
    if bytes > 2_000_000 {
        return Err("wide import output exceeds byte bound".into());
    }
    validate(BufReader::new(fs::File::open(path)?), imported)?;
    Ok(
        json!({"file":path.file_name().unwrap().to_str().unwrap(), "bytes":bytes, "sha256":workspace::hash(path)?}),
    )
}

fn controls(text: &str) -> Result<usize> {
    validate(Cursor::new(text), true)?;
    let original: Vec<Value> = text
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    let mut mutations = Vec::new();
    let mut changed = original.clone();
    // Both strings are present here; their column identities differ.
    changed[252]["row"][7] = changed[252]["row"][11].clone();
    mutations.push(changed);
    let mut changed = original.clone();
    changed[251]["row"][1] = json!("0000000000000000"); // row 249: negative zero
    mutations.push(changed);
    let mut changed = original.clone();
    changed[11]["row"][5] = changed[11]["row"][1].clone(); // row 9: distinct NULL positions
    mutations.push(changed);
    let mut changed = original.clone();
    changed.remove(514); // omit the last data row, preserving completion
    mutations.push(changed);
    let mut changed = original.clone();
    changed.pop();
    mutations.push(changed);
    for changed in &mutations {
        if changed == &original {
            return Err("wide import control did not change its answer".into());
        }
        let text = changed
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        if validate(Cursor::new(text), true).is_ok() {
            return Err("wide import checker accepted a wrong answer".into());
        }
    }
    Ok(mutations.len())
}

fn manifest(value: &Value, root: &Path) -> Result<()> {
    if value["status"] != "wide-generated"
        || value["reader"] != "22.0.0"
        || value["inputs"]
            .as_array()
            .is_none_or(|inputs| inputs.len() != 2)
    {
        return Err("missing wide input generator receipt".into());
    }
    for (id, (version, groups, page, batch)) in [
        ("1.0", vec![513], 1024, 37),
        ("2.0", vec![257, 256], 4096, 113),
    ]
    .into_iter()
    .enumerate()
    {
        let input = &value["inputs"][id];
        let name = format!("wide-{id}.parquet");
        let path = root.join(&name);
        let bytes = fs::metadata(&path)?.len();
        if input["file"] != name
            || input["version"] != version
            || input["rows"] != 513
            || input["columns"] != 64
            || input["groups"] != json!(groups)
            || input["page_target"] != page
            || input["write_batch"] != batch
            || input["bytes"] != bytes
            || !(12..=2_000_000).contains(&bytes)
            || input["sha256"] != workspace::hash(&path)?
        {
            return Err("wide input differs from its generation receipt".into());
        }
    }
    Ok(())
}

pub(super) fn run(run: &mut Run, driver: &Path) -> Result<()> {
    let inputs = run.directory.join("wide-import-inputs");
    let output = run.command(
        python()
            .arg(ORACLE)
            .arg("--write-wide-imports")
            .arg(&inputs),
        None,
        Duration::from_secs(60),
    )?;
    if output.require_success().is_err() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
    }
    output.require_success()?;
    let receipt: Value = serde_json::from_slice(&output.stdout.bytes)?;
    manifest(&receipt, &inputs)?;
    fs::write(
        run.directory.join("wide-inputs.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    let mut verified = Vec::new();
    for id in 0..2 {
        let input = inputs.join(format!("wide-{id}.parquet"));
        let trial = run.directory.join(format!("wide-import-{id}"));
        let output = run.command(
            Command::new(driver)
                .arg(&trial)
                .arg("--wide-import")
                .arg(&input)
                .env_remove("PIPESQL_TRANSFER_STACK_CONTROL")
                .env_remove("PIPESQL_IMPORT_CONTROL"),
            None,
            Duration::from_secs(300),
        )?;
        if output.require_success().is_err() {
            eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
        }
        output.require_success()?;
        let (stack, report) = super::stack_report(std::str::from_utf8(&output.stdout.bytes)?)?;
        if report != REPORT || !output.stderr.bytes.is_empty() {
            return Err("wide import driver outcome differs".into());
        }
        let failure = file(&trial.join("source.jsonl"), false)?;
        let complete = file(&trial.join("complete.jsonl"), true)?;
        let rejected = controls(&fs::read_to_string(trial.join("complete.jsonl"))?)?;
        let native = super::measure::storage(&trial.join("database"), 513)?;
        let mut counts: Vec<_> = native["units"]
            .as_array()
            .ok_or("missing wide native units")?
            .iter()
            .map(|unit| unit["rows"].as_u64().ok_or("missing wide native row count"))
            .collect::<std::result::Result<_, _>>()?;
        counts.sort_unstable();
        if counts
            != if id == 0 {
                vec![1, 1, 512]
            } else {
                vec![1, 256, 257]
            }
        {
            return Err("wide native units differ from decoder lending boundaries".into());
        }
        verified.push(json!({"case":id, "failure":failure, "complete":complete, "wrong_answers_rejected":rejected, "native":native, "stack":stack}));
        fs::remove_dir_all(trial)?;
        if id == 0 {
            let trial = run.directory.join("wide-import-disabled-fault");
            let output = run.command(
                Command::new(driver)
                    .arg(&trial)
                    .arg("--wide-import")
                    .arg(&input)
                    .env_remove("PIPESQL_TRANSFER_STACK_CONTROL")
                    .env("PIPESQL_IMPORT_CONTROL", "skip-source-fault"),
                None,
                Duration::from_secs(300),
            )?;
            if output.require_success().is_ok()
                || !String::from_utf8_lossy(&output.stderr.bytes)
                    .contains("wide import fault must fail")
            {
                return Err("disabled wide import source fault accepted".into());
            }
            let (stack, _) = super::stack_report(std::str::from_utf8(&output.stdout.bytes)?)?;
            let control = file(&trial.join("unexpected-success.jsonl"), true)?;
            fs::write(
                run.directory.join("wide-import-control.json"),
                serde_json::to_vec_pretty(&json!({"complete":control, "stack":stack}))?,
            )?;
            fs::remove_dir_all(trial)?;
        }
    }
    manifest(&receipt, &inputs)?;
    fs::write(
        run.directory.join("wide-import-verification.json"),
        serde_json::to_vec_pretty(&verified)?,
    )?;
    fs::remove_dir_all(inputs)?;
    println!(
        "Wide Parquet imports: independent v1/v2 files, 64 reordered columns, complete reopened answers, late abort and retry passed"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_wide_import_answers_reject_column_aliases_bits_nulls_and_prefixes() {
        let mut lines = vec![schema(), row(None)];
        lines.extend((0..513).map(|id| row(Some(id))));
        lines.push(json!({"complete":true, "rows":514}));
        let text = lines
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(controls(&text).unwrap(), 5);
        assert!(validate(Cursor::new(format!("{text}\n{}", row(Some(0)))), true).is_err());
        assert!(manifest(&json!({}), Path::new("unused")).is_err());
    }
}
