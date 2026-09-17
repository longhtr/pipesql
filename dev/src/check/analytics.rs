//! Import, query, export and reopen complete typed analytical results.
//!
//! Literal small answers check the model. Larger answers enumerate each peer
//! frame independently. Parquet round trips check composition; external codec
//! fixtures separately establish interoperability.
//!
//! Each workflow creates fresh databases and checks transaction resolution, spill,
//! cancellation and export limits along with the final report. Command and artifact
//! records retain the configuration needed to investigate failure. Expected failures
//! are explicit; an unexpected outcome stops the workflow and preserves its files.

use crate::{
    Result,
    process::Completion,
    workspace::{self, Run},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[path = "analytics_data.rs"]
mod data;
use data::{DIMENSIONS, EVENT_SCHEMA, Event, REPORT_SCHEMA};

type Schema = &'static [(&'static str, &'static str, bool)];
type Key = (Option<i64>, Option<&'static str>, i64);

fn model(events: &[Event]) -> Vec<Value> {
    let mut groups: BTreeMap<Key, Vec<Option<i64>>> = BTreeMap::new();
    for event in events {
        let labels: Vec<_> = DIMENSIONS
            .iter()
            .filter(|(key, _)| Some(*key) == event.dimension)
            .map(|(_, label)| Some(*label))
            .collect();
        let labels = if labels.is_empty() {
            vec![None]
        } else {
            labels
        };
        let year = event.day.map(|day| day[..4].parse::<i64>().unwrap());
        let category = event
            .amount
            .map_or(0, |amount| if amount < 0 { -1 } else { 1 });
        for label in labels {
            groups
                .entry((year, label, category))
                .or_default()
                .push(event.amount);
        }
    }
    groups
        .iter()
        .map(|((year, label, category), values)| {
            let present: Vec<_> = values.iter().flatten().copied().collect();
            // Enumerate the complete frame instead of imitating incremental SUM.
            let frame: Vec<_> = groups
                .iter()
                .filter(|((y, lab, _), _)| lab == label && y <= year)
                .flat_map(|(_, values)| values.iter().flatten().copied())
                .collect();
            json!([
                integer(*year),
                label,
                category.to_string(),
                values.len().to_string(),
                present.len().to_string(),
                sum(&present),
                groups
                    .keys()
                    .filter(|(_, lab, _)| lab == label)
                    .count()
                    .to_string(),
                sum(&frame)
            ])
        })
        .collect()
}

fn integer(value: Option<i64>) -> Value {
    value.map_or(Value::Null, |value| json!(value.to_string()))
}
fn sum(values: &[i64]) -> Value {
    if values.is_empty() {
        Value::Null
    } else {
        json!(values.iter().sum::<i64>().to_string())
    }
}

fn check_jsonl(path: &Path, schema: Schema, expected: &[Value]) -> Result<()> {
    let mut lines = BufReader::new(File::open(path)?).lines();
    let mut next = || -> Result<Value> {
        Ok(serde_json::from_str(
            &lines.next().ok_or("missing JSONL record")??,
        )?)
    };
    let header = json!({"format": "pipesql-jsonl", "version": 1, "columns": schema.iter().map(|(name, kind, nullable)| json!({"name": name, "type": kind, "nullable": nullable})).collect::<Vec<_>>()});
    if next()? != header {
        return Err(format!("{}: wrong schema", path.display()).into());
    }
    for (index, row) in expected.iter().enumerate() {
        let actual = next()?;
        if actual != json!({"row": row}) {
            return Err(format!(
                "{}: row {index}: expected {row}, got {actual}",
                path.display()
            )
            .into());
        }
    }
    if next()? != json!({"complete": true, "rows": expected.len()}) {
        return Err(format!("{}: wrong completion", path.display()).into());
    }
    if lines.next().is_some() {
        return Err(format!("{}: records after completion", path.display()).into());
    }
    Ok(())
}

struct Export {
    rows: usize,
    parquet: bool,
    memory: u64,
    bytes: usize,
    refusal: Option<&'static str>,
}

struct Workflow<'a> {
    run: &'a mut Run,
    cli: PathBuf,
}

impl Workflow<'_> {
    fn command(
        &mut self,
        db: &Path,
        operation: &str,
        args: &[String],
        memory: u64,
        output: Option<&Path>,
        success: bool,
    ) -> Result<crate::process::Output> {
        let mut command = Command::new(&self.cli);
        command
            .arg(operation)
            .arg("--database")
            .arg(db)
            .args([
                "--memory-limit-bytes",
                &memory.to_string(),
                "--temp-limit-bytes",
                "64000000",
            ])
            .args(args);
        let output = self
            .run
            .command(&mut command, output, Duration::from_secs(600))?;
        let Completion::Exited(status) = output.completion else {
            return Err(format!("CLI failed to finish: {:?}", output.completion).into());
        };
        if status.success() != success {
            return Err(format!(
                "{operation}: unexpected {status}: {}",
                String::from_utf8_lossy(&output.stderr.bytes)
            )
            .into());
        }
        if success {
            output.require_success()?;
        } else if output.stdout.omitted != 0 || output.stderr.omitted != 0 {
            return Err("refusal diagnostics truncated".into());
        }
        Ok(output)
    }

    fn create(&mut self, db: &Path) -> Result<()> {
        self.command(db, "create-declared", &[], 8_000_000, None, true)?;
        for name in ["events", "dimensions"] {
            self.command(
                db,
                "declare",
                &[
                    "--schema-file".into(),
                    self.run
                        .root
                        .join(format!("examples/{name}.schema"))
                        .display()
                        .to_string(),
                ],
                8_000_000,
                None,
                true,
            )?;
        }
        Ok(())
    }

    fn import(
        &mut self,
        db: &Path,
        table: &str,
        input: &Path,
        rows: usize,
        parquet: bool,
        success: bool,
    ) -> Result<()> {
        let mut args = strings(&[
            "--table",
            table,
            "--input",
            &input.display().to_string(),
            "--input-limit-bytes",
            "32000000",
            "--row-limit",
            &rows.max(1).to_string(),
            "--batch-limit",
            &(2 * rows.div_ceil(256) + 4).to_string(),
            "--encoded-limit-bytes",
            "32000000",
        ]);
        args.extend(strings(if parquet {
            &[
                "--format",
                "parquet",
                "--metadata-limit-bytes",
                "262144",
                "--row-group-limit",
                "256",
                "--row-group-rows",
                "1024",
                "--row-group-limit-bytes",
                "262144",
                "--page-limit-bytes",
                "131072",
            ]
        } else {
            &[
                "--record-limit-bytes",
                "4096",
                "--field-limit-bytes",
                "1024",
                "--batch-rows",
                "256",
                "--batch-text-bytes",
                "4096",
            ]
        }));
        let output = self.command(db, "import", &args, 32_000_000, None, success)?;
        let text = std::str::from_utf8(&output.stdout.bytes)?;
        let tokens: Vec<_> = text
            .lines()
            .filter_map(|line| line.strip_prefix("transaction="))
            .collect();
        if tokens.len() != 1 || tokens[0].len() != 48 {
            return Err("import must print one transaction token".into());
        }
        let resolution = self.command(
            db,
            "resolve",
            &strings(&["--transaction", tokens[0]]),
            8_000_000,
            None,
            true,
        )?;
        let required = if success {
            "resolution=durable"
        } else {
            "resolution=aborted"
        };
        if !std::str::from_utf8(&resolution.stdout.bytes)?
            .lines()
            .any(|line| line == required)
        {
            return Err(format!("import did not resolve as {required}").into());
        }
        if text.lines().any(|line| line == "status=imported") != success {
            return Err("wrong import completion".into());
        }
        if !success && !std::str::from_utf8(&output.stderr.bytes)?.contains("CSV") {
            return Err("missing CSV refusal".into());
        }
        Ok(())
    }

    fn export(&mut self, db: &Path, query: &Path, output: &Path, export: Export) -> Result<()> {
        let Export {
            rows,
            parquet,
            memory,
            bytes,
            refusal,
        } = export;
        let mut args = strings(&[
            "--query-file",
            &query.display().to_string(),
            "--row-limit",
            &rows.max(1).to_string(),
            "--output-limit-bytes",
            &bytes.to_string(),
        ]);
        if parquet {
            args.extend(strings(&[
                "--format",
                "parquet",
                "--metadata-limit-bytes",
                "262144",
                "--row-group-limit",
                "256",
                "--row-group-rows",
                "1024",
                "--row-group-text-bytes",
                "8192",
            ]));
        }
        let result = self.command(db, "export", &args, memory, Some(output), refusal.is_none())?;
        if let Some(required) = refusal
            && !std::str::from_utf8(&result.stderr.bytes)?.contains(required)
        {
            return Err(format!("missing {required} refusal").into());
        }
        Ok(())
    }

    fn profile(&mut self, scaled: bool) -> Result<()> {
        let work = self
            .run
            .directory
            .join(if scaled { "scaled" } else { "small" });
        fs::create_dir(&work)?;
        let events = data::events(scaled);
        let expected = if scaled {
            model(&events)
        } else {
            data::small_report()
        };
        let (first, second) = events.split_at(events.len() / 2);
        data::write_events(&work.join("first.csv"), first)?;
        data::write_events(&work.join("second.csv"), second)?;
        fs::write(
            work.join("dimensions.csv"),
            "id,label\n1,north\n2,south\n2,南\n3,\n",
        )?;
        let db = work.join("csv-db");
        self.create(&db)?;
        self.import(
            &db,
            "dimensions",
            &work.join("dimensions.csv"),
            4,
            false,
            true,
        )?;
        self.import(
            &db,
            "events",
            &work.join("first.csv"),
            first.len(),
            false,
            true,
        )?;
        let query = self.run.root.join("examples/analytics.sql");
        self.export(
            &db,
            &query,
            &work.join("before.jsonl"),
            Export {
                rows: 100,
                parquet: false,
                memory: 8_000_000,
                bytes: 32_000_000,
                refusal: None,
            },
        )?;
        check_jsonl(&work.join("before.jsonl"), REPORT_SCHEMA, &model(first))?;
        self.import(
            &db,
            "events",
            &work.join("second.csv"),
            second.len(),
            false,
            true,
        )?;
        let explain = self.command(
            &db,
            "explain",
            &strings(&["--query-file", &query.display().to_string()]),
            8_000_000,
            None,
            true,
        )?;
        let plan = std::str::from_utf8(&explain.stdout.bytes)?;
        for text in [
            "range unbounded preceding to current row",
            "count(*) over (partition by",
            "status=explained",
        ] {
            if !plan.contains(text) {
                return Err(format!("explain missing {text}").into());
            }
        }
        for memory in [32_000_000, 8_000_000] {
            let path = work.join(format!("report-{memory}.jsonl"));
            self.export(
                &db,
                &query,
                &path,
                Export {
                    rows: 100,
                    parquet: false,
                    memory,
                    bytes: 32_000_000,
                    refusal: None,
                },
            )?;
            check_jsonl(&path, REPORT_SCHEMA, &expected)?;
        }
        for parquet in [false, true] {
            self.export(
                &db,
                &query,
                &work.join(format!("refused-{parquet}")),
                Export {
                    rows: 100,
                    parquet,
                    memory: 8_000_000,
                    bytes: 32,
                    refusal: Some(if parquet {
                        "Parquet output bytes"
                    } else {
                        "result export bytes"
                    }),
                },
            )?;
        }
        let mut bad: Vec<_> = (0..256).map(|i| events[i % events.len()]).collect();
        data::write_events(&work.join("bad.csv"), &bad)?;
        bad.clear();
        writeln!(
            fs::OpenOptions::new()
                .append(true)
                .open(work.join("bad.csv"))?,
            "bad,1,2000-01-01,1,1"
        )?;
        self.import(&db, "events", &work.join("bad.csv"), 257, false, false)?;
        self.export(
            &db,
            &query,
            &work.join("after-abort.jsonl"),
            Export {
                rows: 100,
                parquet: false,
                memory: 8_000_000,
                bytes: 32_000_000,
                refusal: None,
            },
        )?;
        check_jsonl(&work.join("after-abort.jsonl"), REPORT_SCHEMA, &expected)?;
        let copied = work.join("parquet-db");
        self.create(&copied)?;
        let event_rows = events.iter().map(Event::cells).collect::<Vec<_>>();
        let mut dimension_rows: Vec<_> = DIMENSIONS
            .iter()
            .map(|(id, label)| json!([id.to_string(), label]))
            .collect();
        dimension_rows.sort_by_key(|row| row.to_string());
        for (table, schema, reference) in [
            ("events", EVENT_SCHEMA, event_rows),
            ("dimensions", data::DIMENSION_SCHEMA, dimension_rows),
        ] {
            let raw_query = work.join(format!("{table}.sql"));
            fs::write(
                &raw_query,
                format!(
                    "FROM {table} |> ORDER BY id{};\n",
                    if table == "dimensions" { ", label" } else { "" }
                ),
            )?;
            let raw = work.join(format!("{table}.jsonl"));
            self.export(
                &db,
                &raw_query,
                &raw,
                Export {
                    rows: reference.len(),
                    parquet: false,
                    memory: 8_000_000,
                    bytes: 32_000_000,
                    refusal: None,
                },
            )?;
            check_jsonl(&raw, schema, &reference)?;
            let binary = work.join(format!("{table}.parquet"));
            self.export(
                &db,
                &raw_query,
                &binary,
                Export {
                    rows: reference.len(),
                    parquet: true,
                    memory: 8_000_000,
                    bytes: 32_000_000,
                    refusal: None,
                },
            )?;
            self.import(&copied, table, &binary, reference.len(), true, true)?;
            let copied_raw = work.join(format!("{table}-copied.jsonl"));
            self.export(
                &copied,
                &raw_query,
                &copied_raw,
                Export {
                    rows: reference.len(),
                    parquet: false,
                    memory: 8_000_000,
                    bytes: 32_000_000,
                    refusal: None,
                },
            )?;
            check_jsonl(&copied_raw, schema, &reference)?;
        }
        self.export(
            &copied,
            &query,
            &work.join("parquet-report.jsonl"),
            Export {
                rows: 100,
                parquet: false,
                memory: 8_000_000,
                bytes: 32_000_000,
                refusal: None,
            },
        )?;
        check_jsonl(&work.join("parquet-report.jsonl"), REPORT_SCHEMA, &expected)?;
        let report_schema = work.join("report.schema");
        let mut file = BufWriter::new(File::create(&report_schema)?);
        writeln!(file, "table report")?;
        for (name, kind, nullable) in REPORT_SCHEMA {
            writeln!(
                file,
                "{name} {kind} {}",
                if *nullable { "nullable" } else { "required" }
            )?;
        }
        file.flush()?;
        self.command(
            &copied,
            "declare",
            &strings(&["--schema-file", &report_schema.display().to_string()]),
            8_000_000,
            None,
            true,
        )?;
        self.export(
            &db,
            &query,
            &work.join("report.parquet"),
            Export {
                rows: 100,
                parquet: true,
                memory: 8_000_000,
                bytes: 32_000_000,
                refusal: None,
            },
        )?;
        self.import(
            &copied,
            "report",
            &work.join("report.parquet"),
            expected.len(),
            true,
            true,
        )?;
        fs::write(
            work.join("report.sql"),
            "FROM report |> ORDER BY calendar_year, label, amount_class;\n",
        )?;
        self.export(
            &copied,
            &work.join("report.sql"),
            &work.join("roundtrip.jsonl"),
            Export {
                rows: 100,
                parquet: false,
                memory: 8_000_000,
                bytes: 32_000_000,
                refusal: None,
            },
        )?;
        check_jsonl(&work.join("roundtrip.jsonl"), REPORT_SCHEMA, &expected)?;
        Ok(())
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub fn run(case: &str) -> Result<()> {
    let root = workspace::root()?;
    let mut run = Run::new(root, case)?;
    let cli = run.build("pipesql", "--bin", "pipesql")?;
    let identity = workspace::hash(&cli)?;
    let mut workflow = Workflow {
        run: &mut run,
        cli: cli.clone(),
    };
    workflow.profile(case == "scaled")?;
    if case == "scaled" {
        let example = run.build("pipesql", "--example", "scaled_report")?;
        let mut command = Command::new(example);
        command
            .arg(run.directory.join("spill-db"))
            .args(["8000000", "even"]);
        run.command(&mut command, None, Duration::from_secs(600))?
            .require_success()?;
    }
    if workspace::hash(&cli)? != identity {
        return Err("CLI artifact changed during workflow".into());
    }
    run.finish()?;
    println!("analytics/{case}: passed");
    Ok(())
}

#[cfg(test)]
#[path = "analytics_tests.rs"]
mod tests;
