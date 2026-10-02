//! Repeat imports while keeping setup and independent answers outside API timing.
//!
//! Each process has one warm-up and five fresh seeded databases. Read sizes vary
//! against the exact same input; the two large files differ in several geometry
//! settings, so their contrast does not isolate a Parquet page-version cost.

use super::{CASES, Case, file};
use crate::{Result, workspace::Run};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

fn report(text: &str, case: &Case, fragment: usize, length: u64) -> Result<Vec<u64>> {
    let lines: Vec<_> = text.lines().collect();
    if lines.len() != 8
        || lines[0]
            != format!(
                "input rows={} fragment={fragment} bytes={length}",
                case.rows
            )
        || lines[7] != "status=measured"
    {
        return Err("missing import samples or wrong selection".into());
    }
    let mut samples = Vec::new();
    for (sample, line) in lines[1..7].iter().enumerate() {
        let prefix = format!("sample={sample} elapsed_ns=");
        let suffix = format!(
            " end_seeks=2 rows={} baseline_generation=2 durable=true released=true reopened=true",
            case.rows + 1
        );
        let line = line
            .strip_prefix(&prefix)
            .and_then(|line| line.strip_suffix(&suffix))
            .ok_or("wrong import sample order, outcome or initial state")?;
        let (elapsed, observation) = line
            .split_once(" read_calls=")
            .ok_or("missing import time")?;
        let elapsed: u64 = elapsed.parse()?;
        let (calls, bytes) = observation
            .split_once(" read_bytes=")
            .ok_or("missing import reads")?;
        let calls: u64 = calls.parse()?;
        let bytes: u64 = bytes.parse()?;
        if !(1..=300_000_000_000).contains(&elapsed)
            || bytes != length
            || calls < bytes.div_ceil(fragment as u64)
            || calls > bytes + 1
        {
            return Err("invalid import timing or incomplete source reads".into());
        }
        samples.push(elapsed);
    }
    Ok(samples)
}

fn controls(text: &str, case: &Case, fragment: usize, length: u64) -> Result<usize> {
    let valid = report(text, case, fragment, length)?;
    let wrong = [
        text.replacen(&format!("elapsed_ns={}", valid[0]), "elapsed_ns=0", 1),
        text.replacen("sample=1", "sample=0", 1),
        text.replacen("baseline_generation=2", "baseline_generation=3", 1),
        text.replacen("released=true", "released=false", 1),
        text.replacen("durable=true", "durable=false", 1),
        text.replacen(
            &format!("read_bytes={length}"),
            &format!("read_bytes={}", length - 1),
            1,
        ),
        text.replacen("status=measured", "", 1),
        format!("{text}status=measured\n"),
    ];
    for mutation in &wrong {
        if mutation == text || report(mutation, case, fragment, length).is_ok() {
            return Err("import measurement accepted a wrong report".into());
        }
    }
    Ok(wrong.len())
}

// The directory also contains catalogs, schemas, indexes and receipt history.
// Follow the independently validated live graph before classifying data files.
pub(super) fn storage(path: &Path, rows: usize) -> Result<Value> {
    let graph = crate::oracle::catalog::inspect(path)?;
    if graph["generation"] != 3
        || graph["roots_settled"] != true
        || graph["tables"].as_array().is_none_or(|v| v.len() != 1)
        || graph["tables"][0]["name"] != "facts"
        || graph["tables"][0]["rows"]
            .as_array()
            .is_none_or(|v| v.len() != rows + 1)
    {
        return Err("imported native graph differs from the completed sample".into());
    }
    let mut units = Vec::new();
    let mut total_rows = 0_u64;
    let mut total_bytes = 0_u64;
    for name in graph["reachable"]
        .as_array()
        .ok_or("missing reachable objects")?
    {
        let name = name.as_str().ok_or("invalid object name")?;
        let mut input = fs::File::open(path.join("units").join(name))?;
        let mut header = [0; 56];
        input.read_exact(&mut header)?;
        if &header[..8] != b"PSQLDATA" {
            continue;
        }
        // The offline inspector has validated this v6 header against the table
        // index and checked every column payload. These are observations only.
        let count = u32::from_le_bytes(header[52..56].try_into()?) as u64;
        let bytes = input.metadata()?.len();
        total_rows += count;
        total_bytes += bytes;
        units.push(json!({"file":name,"rows":count,"bytes":bytes}));
    }
    if total_rows != (rows + 1) as u64 || units.is_empty() {
        return Err("native data units do not cover the imported table".into());
    }
    Ok(json!({"units":units,"rows":total_rows,"bytes":total_bytes}))
}

fn storage_control(path: &Path, rows: usize, observed: &Value) -> Result<()> {
    let name = observed["units"][0]["file"]
        .as_str()
        .ok_or("missing native data unit")?;
    let unit = path.join("units").join(name);
    let mut file = fs::OpenOptions::new().read(true).write(true).open(&unit)?;
    file.seek(SeekFrom::End(-1))?;
    let mut byte = [0];
    file.read_exact(&mut byte)?;
    file.seek(SeekFrom::End(-1))?;
    file.write_all(&[byte[0] ^ 1])?;
    let rejected = storage(path, rows).is_err();
    file.seek(SeekFrom::End(-1))?;
    file.write_all(&byte)?;
    drop(file);
    if !rejected || storage(path, rows)? != *observed {
        return Err("native inspection accepted corrupt payload or failed restored retry".into());
    }
    Ok(())
}

pub(super) fn run(run: &mut Run, driver: &Path, inputs: &Path) -> Result<()> {
    let mut selection = vec![(0, 4096, 1), (1, 4096, 1)];
    for id in [2, 3] {
        for fragment in [127, 4096, 65536] {
            selection.push((id, fragment, 3));
        }
    }
    for id in [4, 5] {
        selection.push((id, 4096, 3));
    }
    fs::write(
        run.directory.join("selection.json"),
        serde_json::to_vec_pretty(&json!({
            "case":"parquet-imports", "warmups":1, "samples":5,
            "timing":"import_parquet API including publication; setup, source open, reopen, export and independent checking excluded",
            "scenarios":selection.iter().map(|(id, fragment, rounds)| json!({"input":id,"fragment":fragment,"rounds":rounds})).collect::<Vec<_>>()
        }))?,
    )?;
    let mut results = Vec::new();
    for round in 0..3 {
        let mut order: Vec<_> = (0..selection.len()).collect();
        if round % 2 != 0 {
            order.reverse();
        }
        for id in order {
            let (input_id, fragment, rounds) = selection[id];
            if round >= rounds {
                continue;
            }
            let case = &CASES[input_id];
            let input = inputs.join(format!("input-{input_id}.parquet"));
            let length = fs::metadata(&input)?.len();
            let name = format!("measure-{id}-{round}");
            let trial = run.directory.join(&name);
            let usage = run.directory.join(format!("{name}.time"));
            let mut command = Command::new("/usr/bin/time");
            command.args(["-f", "elapsed_seconds=%e\nuser_seconds=%U\nsystem_seconds=%S\nmax_rss_kib=%M\nfilesystem_inputs=%I\nfilesystem_outputs=%O\nexit=%x", "-o"])
                .arg(&usage).arg("--").arg(driver).arg(&trial).arg(&input)
                .args([case.rows.to_string(), fragment.to_string(), "--measure".into()])
                .env_remove("PIPESQL_IMPORT_CONTROL");
            eprintln!("measure import {id}-{round}: input={input_id} fragment={fragment}");
            let output = run.command(&mut command, None, Duration::from_secs(300))?;
            if output.require_success().is_err() {
                eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
            }
            output.require_success()?;
            let text = std::str::from_utf8(&output.stdout.bytes)?;
            let times = report(text, case, fragment, length)?;
            let rejections = if id == 0 && round == 0 {
                controls(text, case, fragment, length)?
            } else {
                0
            };
            let started = Instant::now();
            let files = (0..6)
                .map(|sample| {
                    file(
                        &trial.join(format!("sample-{sample}.jsonl")),
                        case.rows,
                        false,
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            let mut native = Vec::new();
            for sample in 0..6 {
                native.push(storage(
                    &trial.join(format!("database-{sample}")),
                    case.rows,
                )?);
            }
            let validation_ns = started.elapsed().as_nanos();
            let storage_rejections = if id == 0 && round == 0 {
                storage_control(&trial.join("database-0"), case.rows, &native[0])?;
                1
            } else {
                0
            };
            results.push(
                json!({"scenario":id,"round":round,"input":input_id,"fragment":fragment,
                "warmup_ns":times[0],"samples_ns":times[1..],"files":files,
                "validation_ns":validation_ns,"native":native,"storage_rejections":storage_rejections,"report_rejections":rejections}),
            );
            fs::remove_dir_all(trial)?;
        }
    }
    if results.len() != 26 {
        return Err("missing repeated import trials".into());
    }
    fs::write(
        run.directory.join("measurements.json"),
        serde_json::to_vec_pretty(&results)?,
    )?;
    println!(
        "Parquet imports measured: 26 processes, 130 samples, 156 complete independently checked files"
    );
    Ok(())
}
