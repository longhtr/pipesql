//! Stop append and recovery at native filesystem events, then reopen.
//!
//! Exact native trace prefixes establish the cut. Public API checks and the
//! independent byte reader judge rows and receipts separately. These stops retain
//! OS-visible writes; they do not simulate power loss or device synchronization.
//!
//! Healthy runs establish event sequences; each interrupted case starts from its own
//! copy and must reach the exact selected prefix. Reopen checks settled roots and
//! transaction resolution, including another interruption during recovery. Workload
//! variants keep simple append and composed-report expectations separate.

use crate::{
    Result,
    oracle::catalog,
    process::Completion,
    workspace::{self, Run, copy_tree},
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[path = "recovery_images.rs"]
mod images;
#[path = "recovery_report.rs"]
mod report;

#[derive(Clone, Copy)]
enum Workload {
    Facts,
    Report,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Event {
    kind: u8,
    after: bool,
    role: u8,
}

fn trace(path: &Path) -> Result<Vec<Event>> {
    let bytes = fs::read(path)?;
    if bytes.is_empty() || !bytes.len().is_multiple_of(8) || bytes.len() > 4096 * 8 {
        return Err("missing or truncated native trace".into());
    }
    bytes
        .as_chunks::<8>()
        .0
        .iter()
        .enumerate()
        .map(|(index, record)| {
            if record[0] != 1
                || !b"wprus".contains(&record[1])
                || record[2] > 1
                || ![0, b'A', b'B'].contains(&record[3])
                || u32::from_le_bytes(record[4..8].try_into()?) as usize != index + 1
            {
                return Err("invalid native trace event".into());
            }
            Ok(Event {
                kind: record[1],
                after: record[2] != 0,
                role: record[3],
            })
        })
        .collect()
}

fn check_prefix(actual: &[Event], reference: &[Event], cut: usize) -> Result<()> {
    if cut == 0 || reference.get(..cut) != Some(actual) {
        return Err("wrong native trace prefix".into());
    }
    Ok(())
}

fn check_graph(graph: &Value, state: u32, retried: bool) -> Result<()> {
    let issued = if state == 0 { 3 } else { 4 } + u64::from(retried);
    let generation = 2 + u64::from(state == 2) + u64::from(retried);
    let mut successes = vec![1, 2];
    if state == 2 {
        successes.push(4);
    }
    if retried {
        successes.push(issued);
    }
    if graph["issued"] != issued
        || graph["generation"] != generation
        || graph["successes"] != json!(successes)
    {
        return Err("independent transaction history differs".into());
    }
    let mut rows = vec![json!([7, "snow 雪\u{0}"]), json!([i64::MAX, null])];
    if state == 2 {
        rows.extend([json!([11, "snow 雪\u{0}"]), json!([13, null])]);
    }
    if retried {
        rows.extend([json!([17, "snow 雪\u{0}"]), json!([19, null])]);
    }
    let expected = json!([{"id": 1, "name": "facts", "columns": [{"id": 1, "name": "k", "type": 1, "nullable": false}, {"id": 2, "name": "s", "type": 3, "nullable": true}], "rows": rows}]);
    if graph["tables"] != expected {
        return Err("independent stored rows differ".into());
    }
    Ok(())
}

struct Recovery<'a> {
    run: &'a mut Run,
    driver: PathBuf,
    observer: PathBuf,
    workload: Workload,
}
impl Recovery<'_> {
    fn mode(&self, mode: &str) -> String {
        match self.workload {
            Workload::Facts => mode.to_owned(),
            Workload::Report => format!("report-{mode}"),
        }
    }

    fn check_graph(&self, graph: &Value, state: u32, retried: bool) -> Result<()> {
        match self.workload {
            Workload::Facts => check_graph(graph, state, retried),
            Workload::Report => report::check_graph(graph, state, retried),
        }
    }

    fn check_answer_controls(&self, baseline: &Value) -> Result<()> {
        let mut changes = vec![
            ("/issued", json!(0)),
            ("/generation", json!(1)),
            ("/successes", json!([])),
            ("/tables", json!([])),
            ("/tables/0/id", json!(99)),
            ("/tables/0/columns/0/type", json!(4)),
            ("/tables/0/columns/0/nullable", json!(true)),
            ("/tables/0/rows/0/0", json!(-1)),
            ("/tables/0/rows/0/0", json!(true)),
        ];
        if matches!(self.workload, Workload::Report) {
            changes.extend([
                ("/tables/0/rows/0/2", json!(10957)),
                (
                    "/tables/0/rows/3/4",
                    json!({"double_bits": "0000000000000000"}),
                ),
                (
                    "/tables/1/rows",
                    json!([[1, "north"], [2, "south"], [3, ""]]),
                ),
            ]);
        }
        for (path, value) in changes {
            let mut wrong = baseline.clone();
            *wrong
                .pointer_mut(path)
                .ok_or("answer control addressed a missing field")? = value;
            if self.check_graph(&wrong, 2, false).is_ok() {
                return Err(format!("stored answer check accepted altered {path}").into());
            }
        }
        let mut prefix = baseline.clone();
        prefix["tables"][0]["rows"]
            .as_array_mut()
            .ok_or("answer control rows missing")?
            .pop();
        if self.check_graph(&prefix, 2, false).is_ok() {
            return Err("stored answer check accepted an incomplete row prefix".into());
        }
        Ok(())
    }

    fn check_recovery_cuts(&mut self, seed: &Path, state: u32) -> Result<usize> {
        let label = seed
            .file_name()
            .unwrap()
            .to_str()
            .ok_or("non-UTF-8 recovery case")?;
        let complete = self
            .run
            .directory
            .join(format!("{label}-recovery-{state}-complete"));
        copy_tree(seed, &complete)?;
        let reference = trace(&self.healthy(&complete, "recover", state)?)?;
        self.check_graph(&catalog::inspect(&complete)?, state, false)?;
        for cut in 1..=reference.len() {
            let db = self
                .run
                .directory
                .join(format!("{label}-recovery-{state}-cut-{cut}"));
            copy_tree(seed, &db)?;
            let (output, path) = self.command(&db, "recover", cut, state, true)?;
            if !matches!(output.completion, Completion::Exited(status) if status.code() == Some(86))
            {
                return Err(format!(
                    "recovery state {state} did not stop at event {cut}: {:?}",
                    output.completion
                )
                .into());
            }
            check_prefix(&trace(&path)?, &reference, cut)?;
            self.check_graph(&catalog::inspect(&db)?, state, false)?;
            self.healthy(&db, "verify", state)?;
            let graph = catalog::inspect(&db)?;
            self.check_graph(&graph, state, true)?;
            if graph["roots_settled"] != true {
                return Err("interrupted recovery left namespace unsettled".into());
            }
        }
        Ok(reference.len())
    }

    fn command(
        &mut self,
        db: &Path,
        mode: &str,
        cut: usize,
        state: u32,
        observer: bool,
    ) -> Result<(crate::process::Output, PathBuf)> {
        let mode = self.mode(mode);
        let path = self.run.directory.join(format!(
            "{}-{mode}-{cut}.trace",
            db.file_name().unwrap().to_string_lossy()
        ));
        let mut command = Command::new(&self.driver);
        command
            .arg(db)
            .arg(&mode)
            .arg(&path)
            .arg(cut.to_string())
            .arg(state.to_string());
        command
            .env_remove("LD_PRELOAD")
            .env_remove("DYLD_INSERT_LIBRARIES");
        if observer {
            command.env(
                if cfg!(target_os = "macos") {
                    "DYLD_INSERT_LIBRARIES"
                } else {
                    "LD_PRELOAD"
                },
                &self.observer,
            );
        }
        let output = self
            .run
            .command(&mut command, None, Duration::from_secs(30))?;
        Ok((output, path))
    }

    fn healthy(&mut self, db: &Path, mode: &str, state: u32) -> Result<PathBuf> {
        let (output, path) = self.command(db, mode, 0, state, true)?;
        output.require_success()?;
        let marker = format!(
            "catalog interruption {} passed state={state}",
            self.mode(mode)
        );
        if !std::str::from_utf8(&output.stdout.bytes)?
            .lines()
            .any(|line| line == marker)
        {
            return Err("driver did not complete selected mode".into());
        }
        Ok(path)
    }
}

pub fn run(case: &str) -> Result<()> {
    let mut run = Run::new(workspace::root()?, case)?;
    let driver = run.build("pipesql-driver", "--bin", "recovery")?;
    let observer = run.build("pipesql-native", "--lib", "pipesql_native")?;
    let mut recovery = Recovery {
        run: &mut run,
        driver,
        observer,
        workload: if matches!(case, "report" | "report-images") {
            Workload::Report
        } else {
            Workload::Facts
        },
    };
    let seed = recovery.run.directory.join("seed");
    recovery.healthy(&seed, "setup", 0)?;
    recovery.check_graph(&catalog::inspect(&seed)?, 0, false)?;
    let complete = recovery.run.directory.join("complete");
    copy_tree(&seed, &complete)?;
    let reference = trace(&recovery.healthy(&complete, "append", 2)?)?;
    for kind in b"wprs" {
        if !reference.iter().any(|event| event.kind == *kind) {
            return Err(format!("append trace omitted {} events", char::from(*kind)).into());
        }
    }
    let baseline = catalog::inspect(&complete)?;
    recovery.check_graph(&baseline, 2, false)?;
    recovery.check_answer_controls(&baseline)?;
    let publications: Vec<_> = reference
        .iter()
        .enumerate()
        .filter(|(_, event)| event.kind == b'r' && event.role == b'A' && event.after)
        .map(|(index, _)| index + 1)
        .collect();
    if publications.len() != 2 {
        return Err("expected issuance and data publication in native trace".into());
    }
    let image_case = matches!(case, "images" | "report-images");
    if image_case {
        let cut = reference
            .iter()
            .position(|event| event.kind == b'r' && event.role == b'B' && event.after)
            .ok_or("missing issuance replica publication")?
            + 1;
        let issued = recovery.run.directory.join("issued");
        copy_tree(&seed, &issued)?;
        let (output, path) = recovery.command(&issued, "append", cut, 1, true)?;
        if !matches!(output.completion, Completion::Exited(status) if status.code() == Some(86)) {
            return Err("issuance seed did not reach its selected interruption".into());
        }
        check_prefix(&trace(&path)?, &reference, cut)?;
        recovery.check_graph(&catalog::inspect(&issued)?, 1, false)?;
        images::run(&mut recovery, &seed, &issued, &complete)?;
    }
    let cuts: Vec<usize> = if image_case {
        Vec::new()
    } else if case != "append-boundaries" {
        (1..=reference.len()).collect()
    } else {
        publications
            .iter()
            .flat_map(|after| [after - 1, *after])
            .collect()
    };
    let mut recovery_cuts = 0;
    for cut in cuts.iter().copied() {
        let db = recovery.run.directory.join(format!("cut-{cut}"));
        copy_tree(&seed, &db)?;
        let (output, path) = recovery.command(&db, "append", cut, 0, true)?;
        if !matches!(output.completion, Completion::Exited(status) if status.code() == Some(86)) {
            return Err(format!(
                "expected interruption at event {cut}, got {:?}",
                output.completion
            )
            .into());
        }
        check_prefix(&trace(&path)?, &reference, cut)?;
        let state = if cut >= publications[1] {
            2
        } else if cut >= publications[0] {
            1
        } else {
            0
        };
        recovery.check_graph(&catalog::inspect(&db)?, state, false)?;
        // Preserve each unsettled publication image before verification opens
        // it. Recovery itself must survive interruption without losing history.
        if case != "append-boundaries" && [publications[1] - 1, publications[1]].contains(&cut) {
            recovery_cuts += recovery.check_recovery_cuts(&db, state)?;
        }
        recovery.healthy(&db, "verify", state)?;
        let graph = catalog::inspect(&db)?;
        recovery.check_graph(&graph, state, true)?;
        if graph["roots_settled"] != true {
            return Err("recovery left namespace unsettled".into());
        }
    }
    for (mode, state, reason) in [
        (
            "wrong-rows",
            2,
            if matches!(recovery.workload, Workload::Report) {
                "report group history"
            } else {
                "independent row history"
            },
        ),
        ("wrong-receipt", 2, "missing durable receipt"),
        ("refuse", 2, "expected corruption refusal"),
        (
            "recover",
            1,
            if matches!(recovery.workload, Workload::Report) {
                "independent report generation history"
            } else {
                "independent generation history"
            },
        ),
    ] {
        let (output, _) = recovery.command(&complete, mode, 0, state, true)?;
        if !matches!(output.completion, Completion::Exited(status) if !status.success())
            || !std::str::from_utf8(&output.stderr.bytes)?.contains(reason)
        {
            return Err(format!("{mode} control did not reject its wrong answer").into());
        }
    }
    let (missing, _) = recovery.command(&complete, "recover", 0, 2, false)?;
    if !matches!(missing.completion, Completion::Exited(status) if !status.success())
        || !std::str::from_utf8(&missing.stderr.bytes)?.contains("native observer missing")
    {
        return Err("missing observer control failed".into());
    }
    run.finish()?;
    if image_case {
        println!("recovery/{case}: passed");
    } else {
        println!(
            "recovery/{case}: passed ({} append cuts, {recovery_cuts} recovery cuts)",
            cuts.len()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wrong_cut_and_wrong_history_do_not_pass() {
        let event = Event {
            kind: b'r',
            after: true,
            role: b'A',
        };
        assert!(check_prefix(&[], std::slice::from_ref(&event), 1).is_err());
        assert!(
            check_prefix(
                std::slice::from_ref(&event),
                std::slice::from_ref(&event),
                2
            )
            .is_err()
        );
        assert!(
            check_graph(
                &json!({"issued":4,"generation":2,"successes":[1,2]}),
                2,
                false
            )
            .is_err()
        );
        assert!(
            check_graph(
                &json!({"issued":4,"generation":3,"successes":[1,2,4],"tables":[]}),
                2,
                false
            )
            .is_err()
        );
    }
}
