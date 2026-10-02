//! Check maximum-width output with an external reader and independent cells.
//!
//! The driver reports refused limits and release; PyArrow checks complete files,
//! reordered schema and raw DOUBLE bits. Both reports must actually be present.

use super::{ORACLE, python};
use crate::{
    Result,
    workspace::{self, Run},
};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, time::Duration};

fn report(text: &str) -> Result<()> {
    if text
        != "wide rows=513 columns=64 groups=2,3 query_width_refused=true rows_refused=true metadata_refused=true released=true retry=true status=exported\n"
    {
        return Err("wide export outcome or selection differs".into());
    }
    Ok(())
}

pub(super) fn run(run: &mut Run, driver: &Path) -> Result<()> {
    let trial = run.directory.join("wide-export");
    let output = run.command(
        Command::new(driver)
            .arg(&trial)
            .arg("--wide-export")
            .env_remove("PIPESQL_TRANSFER_STACK_CONTROL")
            .env_remove("PIPESQL_IMPORT_CONTROL"),
        None,
        Duration::from_secs(300),
    )?;
    if output.require_success().is_err() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
    }
    output.require_success()?;
    let (stack, text) = super::stack_report(std::str::from_utf8(&output.stdout.bytes)?)?;
    report(text)?;
    let mutations = [
        text.replacen("rows=513", "rows=512", 1),
        text.replacen("columns=64", "columns=63", 1),
        text.replacen("released=true", "released=false", 1),
        text.replacen("query_width_refused=true", "query_width_refused=false", 1),
        text.replacen("status=exported", "", 1),
        format!("{text}{text}"),
    ];
    for mutation in &mutations {
        if mutation == text || report(mutation).is_ok() {
            return Err("wide export accepted a wrong driver report".into());
        }
    }
    let output = run.command(
        python().arg(ORACLE).arg("--read-wide-export").arg(&trial),
        None,
        Duration::from_secs(60),
    )?;
    if output.require_success().is_err() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
    }
    output.require_success()?;
    let receipt: Value = serde_json::from_slice(&output.stdout.bytes)?;
    if receipt["status"] != "wide-verified"
        || receipt["rejected"] != json!(["wrong-column", "double-bits", "prefix"])
        || receipt["files"]
            .as_array()
            .is_none_or(|files| files.len() != 3)
    {
        return Err("missing independent wide output checks".into());
    }
    for (index, (name, groups)) in [
        ("wide-257.parquet", vec![257, 256]),
        ("wide-255.parquet", vec![255, 255, 3]),
        ("retry.parquet", vec![257, 256]),
    ]
    .into_iter()
    .enumerate()
    {
        let checked = &receipt["files"][index];
        if checked["file"] != name
            || checked["rows"] != 513
            || checked["columns"] != 64
            || checked["groups"] != json!(groups)
            || checked["sha256"] != workspace::hash(&trial.join(name))?
        {
            return Err("independent wide output receipt differs from its file".into());
        }
    }
    fs::write(
        run.directory.join("wide-verification.json"),
        serde_json::to_vec_pretty(&json!({
            "driver_report_rejections":mutations.len(), "reader":receipt, "stack":stack
        }))?,
    )?;
    fs::remove_dir_all(trial)?;
    println!(
        "Wide Parquet output: 64 mixed columns, 513 rows, refused limits, release/retry and independent reader controls passed"
    );
    Ok(())
}
