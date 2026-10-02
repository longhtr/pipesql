//! Measure complete 64-column scan exports and require independent file answers.
//!
//! One small control precedes three large profiles. Group rows and writer
//! fragments each vary against the same large baseline; setup and PyArrow checks
//! stay outside API timing. The process owner records GNU time separately.

use super::{ORACLE, python, stack_report};
use crate::{
    Result,
    workspace::{self, Run},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

#[derive(Debug, PartialEq)]
struct Sample {
    elapsed_ns: u64,
    bytes: u64,
    writes: u64,
}

fn report(text: &str, rows: usize, group_rows: usize, fragment: usize) -> Result<Vec<Sample>> {
    let lines: Vec<_> = text.lines().collect();
    if lines.len() != 8
        || lines[0]
            != format!(
                "input rows={rows} columns=64 group_rows={group_rows} fragment={fragment} memory_limit=64000000"
            )
        || lines[7] != "status=measured"
    {
        return Err("missing wide export samples or wrong selection".into());
    }
    let mut samples = Vec::new();
    for (sample, line) in lines[1..7].iter().enumerate() {
        let value = line
            .strip_prefix(&format!("sample={sample} elapsed_ns="))
            .and_then(|value| value.strip_suffix(" flushes=1 released=true handles=true"))
            .ok_or("wrong wide export sample order or outcome")?;
        let (elapsed, bytes) = value
            .split_once(&format!(" rows={rows} bytes="))
            .ok_or("missing wide export rows or bytes")?;
        let (bytes, writes) = bytes
            .split_once(" writes=")
            .ok_or("missing wide writer calls")?;
        if !writes.bytes().all(|v| v.is_ascii_digit()) {
            return Err("invalid wide writer calls".into());
        }
        let writes: u64 = writes.parse()?;
        if !elapsed.bytes().all(|v| v.is_ascii_digit())
            || !bytes.bytes().all(|v| v.is_ascii_digit())
        {
            return Err("invalid wide export measurement number".into());
        }
        let elapsed_ns: u64 = elapsed.parse()?;
        let bytes: u64 = bytes.parse()?;
        let limit = if rows == 513 { 2_000_000 } else { 16_000_000 };
        if !(1..=300_000_000_000).contains(&elapsed_ns) || !(9..=limit).contains(&bytes) {
            return Err("invalid wide export timing or byte count".into());
        }
        if writes < bytes.div_ceil(fragment as u64) || writes > bytes {
            return Err("wide writer calls do not cover output".into());
        }
        samples.push(Sample {
            elapsed_ns,
            bytes,
            writes,
        });
    }
    Ok(samples)
}

fn controls(text: &str, rows: usize, group_rows: usize, fragment: usize) -> Result<usize> {
    let valid = report(text, rows, group_rows, fragment)?;
    let wrong = [
        text.replacen(
            &format!("elapsed_ns={}", valid[0].elapsed_ns),
            "elapsed_ns=0",
            1,
        ),
        text.replacen("sample=1", "sample=0", 1),
        text.replacen("columns=64", "columns=63", 1),
        text.replacen("flushes=1", "flushes=0", 1),
        text.replacen(&format!("writes={}", valid[0].writes), "writes=0", 1),
        text.replacen("released=true", "released=false", 1),
        text.replacen("handles=true", "handles=false", 1),
        text.replacen(&format!("bytes={}", valid[0].bytes), "bytes=0", 1),
        text.replacen("status=measured", "", 1),
        format!("{text}status=measured\n"),
    ];
    for mutation in &wrong {
        if mutation == text || report(mutation, rows, group_rows, fragment).is_ok() {
            return Err("wide measurement accepted a wrong producer report".into());
        }
    }
    Ok(wrong.len())
}

pub(super) fn run(run: &mut Run, driver: &Path) -> Result<()> {
    let selection = [
        (513, 257, 127, 1),
        (8192, 257, 4096, 3),
        (8192, 1024, 4096, 3),
        (8192, 257, 127, 3),
    ];
    let source: Value = serde_json::from_slice(&fs::read(run.root.join(".pipesql-source.json"))?)?;
    fs::write(
        run.directory.join("selection.json"),
        serde_json::to_vec_pretty(&json!({
            "case":"parquet-wide", "source":source["sha256"], "artifact":driver,
            "artifact_sha256":workspace::hash(driver)?, "warmups":1, "samples":5,
            "columns":64, "memory_limit":64_000_000,
            "timing":"export_parquet API through final flush; setup, file creation, resource checks and independent reading excluded",
            "scenarios":selection.iter().map(|(rows, group, fragment, rounds)| json!({"rows":rows,"group_rows":group,"fragment":fragment,"rounds":rounds})).collect::<Vec<_>>()
        }))?,
    )?;
    let mut results = Vec::new();
    for round in 0..3 {
        let mut order: Vec<_> = (0..selection.len()).collect();
        if round % 2 != 0 {
            order.reverse();
        }
        for id in order {
            let (rows, group_rows, fragment, rounds) = selection[id];
            if round >= rounds {
                continue;
            }
            let name = format!("measure-{id}-{round}");
            let trial = run.directory.join(&name);
            let usage = run.directory.join(format!("{name}.time"));
            let mut command = Command::new("/usr/bin/time");
            command.args(["-f", "elapsed_seconds=%e\nuser_seconds=%U\nsystem_seconds=%S\nmax_rss_kib=%M\nfilesystem_inputs=%I\nfilesystem_outputs=%O\nexit=%x", "-o"])
                .arg(&usage).arg("--").arg(driver).arg(&trial)
                .args(["--measure-wide-export".to_owned(),rows.to_string(),group_rows.to_string(),fragment.to_string()])
                .env_remove("PIPESQL_TRANSFER_STACK_CONTROL").env_remove("PIPESQL_IMPORT_CONTROL");
            eprintln!(
                "measure wide export {id}-{round}: rows={rows} group_rows={group_rows} fragment={fragment}"
            );
            let output = run.command(&mut command, None, Duration::from_secs(300))?;
            if output.require_success().is_err() {
                eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
            }
            output.require_success()?;
            let (stack, text) = stack_report(std::str::from_utf8(&output.stdout.bytes)?)?;
            let samples = report(text, rows, group_rows, fragment)?;
            let rejections = if id == 0 && round == 0 {
                controls(text, rows, group_rows, fragment)?
            } else {
                0
            };
            let started = Instant::now();
            let reader = run.command(
                python()
                    .arg(ORACLE)
                    .arg("--read-wide-measure")
                    .arg(&trial)
                    .args([rows.to_string(), group_rows.to_string()]),
                None,
                Duration::from_secs(120),
            )?;
            if reader.require_success().is_err() {
                eprint!("{}", String::from_utf8_lossy(&reader.stderr.bytes));
            }
            reader.require_success()?;
            let receipt: Value = serde_json::from_slice(&reader.stdout.bytes)?;
            let groups: Vec<_> = (0..rows)
                .step_by(group_rows)
                .map(|start| group_rows.min(rows - start))
                .collect();
            if receipt["status"] != "wide-measured"
                || receipt["files"]
                    .as_array()
                    .is_none_or(|files| files.len() != 6)
                || receipt["rejected"]
                    != if rows == 513 {
                        json!(["wrong-column", "double-bits", "prefix"])
                    } else {
                        json!([])
                    }
            {
                return Err("missing independent wide measurement receipt".into());
            }
            for (sample, observed) in samples.iter().enumerate() {
                let name = format!("sample-{sample}.parquet");
                let checked = &receipt["files"][sample];
                if checked["file"] != name
                    || checked["rows"] != rows
                    || checked["columns"] != 64
                    || checked["groups"] != json!(groups)
                    || checked["sha256"] != workspace::hash(&trial.join(&name))?
                    || fs::metadata(trial.join(&name))?.len() != observed.bytes
                {
                    return Err("wide measurement receipt differs from produced file".into());
                }
            }
            results.push(json!({"scenario":id,"round":round,"rows":rows,"group_rows":group_rows,
                "fragment":fragment,"warmup_ns":samples[0].elapsed_ns,
                "samples_ns":samples[1..].iter().map(|sample| sample.elapsed_ns).collect::<Vec<_>>(),
                "bytes":samples[0].bytes,"writes":samples.iter().map(|sample|sample.writes).collect::<Vec<_>>(),"stack":stack,"reader":receipt,
                "validation_ns":started.elapsed().as_nanos(),"report_rejections":rejections}));
            fs::remove_dir_all(trial)?;
        }
    }
    if results.len() != 10 {
        return Err("missing repeated wide exports".into());
    }
    fs::write(
        run.directory.join("measurements.json"),
        serde_json::to_vec_pretty(&results)?,
    )?;
    println!(
        "Wide Parquet exports measured: 10 processes, 50 samples, 60 complete independently checked files"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_measurements_require_each_sample_and_completed_output() {
        let mut text =
            "input rows=513 columns=64 group_rows=257 fragment=127 memory_limit=64000000\n"
                .to_owned();
        for sample in 0..6 {
            text.push_str(&format!("sample={sample} elapsed_ns={} rows=513 bytes=1234 writes=20 flushes=1 released=true handles=true\n", sample+1));
        }
        text.push_str("status=measured\n");
        assert_eq!(report(&text, 513, 257, 127).unwrap().len(), 6);
        assert_eq!(controls(&text, 513, 257, 127).unwrap(), 10);
        assert!(report(&text, 8192, 257, 127).is_err());
        assert!(report(&text, 513, 1024, 127).is_err());
        assert!(report(&text, 513, 257, 4096).is_err());
        for wrong in [
            text.replace("sample=3", "sample=6"),
            text.replace("elapsed_ns=1 ", "elapsed_ns=+1 "),
            text.replace(" rows=513 bytes=", " rows=512 bytes="),
        ] {
            assert!(report(&wrong, 513, 257, 127).is_err());
        }
        assert!(report("", 513, 257, 127).is_err());
    }
}
