//! Judge complete export files independently of the engine and their producer.
//!
//! Expected scalar strings are literal format values; text follows the declared
//! input pattern without calling an engine codec. Scan rows may arrive in any
//! order, but every ID must occur once. Ordered profiles also require ID order.
//! A complete-looking file cannot substitute for a successful API/flush report.

use super::{Scenario, integer_fields, one_line};
use crate::Result;
use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Cursor};
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use std::time::Instant;

pub(super) fn scenarios(selected: &mut Vec<Scenario>) {
    // Exercise the empty stream and reader controls before the larger trials.
    for (query, rows) in [("scan", "0"), ("order", "257")] {
        selected.push(Scenario::new(
            "export_cost",
            &[query, "escaped1024", "64", "127", rows],
            "status=exported".into(),
            1,
        ));
    }
    for query in ["scan", "order"] {
        for profile in ["plain8", "plain1024", "escaped1024"] {
            for batch in ["64", "256"] {
                for fragment in ["127", "1024"] {
                    selected.push(Scenario::new(
                        "export_cost",
                        &[query, profile, batch, fragment, "8192"],
                        "status=exported".into(),
                        3,
                    ));
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Input<'a> {
    query: &'a str,
    profile: &'a str,
    batch: usize,
    fragment: usize,
    rows: usize,
    parquet: Option<(usize, usize)>,
}

fn input(arguments: &[String]) -> Result<Input<'_>> {
    let [query, profile, batch, fragment, rows, extra @ ..] = arguments else {
        return Err("incomplete export selection".into());
    };
    let parquet = match extra {
        [] => None,
        [format, rows, text] if format == "parquet" => {
            let rows = rows.parse()?;
            let text = text.parse()?;
            if !matches!(rows, 128 | 311) || !matches!(text, 4096 | 65536 | 131072) {
                return Err("unknown Parquet output selection".into());
            }
            Some((rows, text))
        }
        _ => return Err("unknown export format selection".into()),
    };
    let value = Input {
        query,
        profile,
        batch: batch.parse()?,
        fragment: fragment.parse()?,
        rows: rows.parse()?,
        parquet,
    };
    if !matches!(value.query, "scan" | "order")
        || !matches!(value.profile, "plain8" | "plain1024" | "escaped1024")
        || !matches!(value.batch, 64 | 256)
        || !matches!(value.fragment, 127 | 1024)
        || !matches!(value.rows, 0 | 257 | 8192)
    {
        return Err("unknown export selection".into());
    }
    Ok(value)
}

const SAMPLE: &[&str] = &[
    "elapsed_ns",
    "rows",
    "bytes",
    "writes",
    "flushes",
    "memory",
    "temporary",
    "scratch",
];

pub(super) fn check_report(scenario: &Scenario, text: &str) -> Result<()> {
    let i = input(&scenario.arguments)?;
    if one_line(text, "input ")?
        != format!(
            "query={} profile={} batch_rows={} fragment={} rows={} memory_limit=4000000",
            i.query, i.profile, i.batch, i.fragment, i.rows
        )
        || one_line(text, "status=")? != "exported"
        || text.lines().last() != Some("status=exported")
    {
        return Err("export input or completion report differs".into());
    }
    if let Some((rows, bytes)) = i.parquet {
        if one_line(text, "format=")? != format!("parquet group_rows={rows} group_text={bytes}") {
            return Err("Parquet output bounds differ".into());
        }
    } else if text.lines().any(|line| line.starts_with("format=")) {
        return Err("unexpected export format report".into());
    }
    let spilled = i.query == "order" && i.rows != 0;
    let mut prefixes = vec!["warmup ".to_owned()];
    prefixes.extend((0..5).map(|sample| format!("sample={sample} ")));
    prefixes.push("retry ".to_owned());
    let mut bytes = None;
    for prefix in prefixes {
        let values = integer_fields(one_line(text, &prefix)?, SAMPLE)?;
        if values[0] == 0
            || values[1] != i.rows as u64
            || values[2] == 0
            || values[2] > 64_000_000
            || values[3] == 0
            || values[3] < values[2].div_ceil(i.fragment as u64)
            || values[3] > values[2]
            || values[4] != 1
            || values[5] == 0
            || values[5] > 4_000_000
            || (values[6] > 0) != spilled
            || (values[7] > 0) != (spilled && prefix == "warmup ")
        {
            return Err("invalid export sample or flush outcome".into());
        }
        if bytes
            .replace(values[2])
            .is_some_and(|previous| previous != values[2])
        {
            return Err("repeated export byte count changed".into());
        }
    }
    if text
        .lines()
        .filter(|line| line.starts_with("sample="))
        .count()
        != 5
    {
        return Err("export samples missing or duplicated".into());
    }
    let mut failures = if i.rows == 0 {
        0
    } else if i.rows == 8192 {
        6
    } else {
        5
    };
    if i.rows != 0 && i.parquet.is_some() {
        failures += 3;
    }
    if text
        .lines()
        .filter(|line| line.starts_with("failure="))
        .count()
        != failures
    {
        return Err("export failure controls missing or duplicated".into());
    }
    if i.rows != 0 {
        let mut names = vec!["write", "flush", "cancel", "rows", "bytes"];
        if i.parquet.is_some() {
            names.extend(["groups", "text", "metadata"]);
        }
        for name in names {
            let line = one_line(text, &format!("failure={name} "))?
                .strip_suffix(" released=true")
                .ok_or("export failure retained resources")?;
            let values = integer_fields(line, &["bytes", "writes", "flushes"])?;
            if values[0] == 0
                || values[1] == 0
                || values[2] != u64::from(name == "flush")
                || (name == "flush" && Some(values[0]) != bytes)
            {
                return Err("export failure outcome differs".into());
            }
        }
    }
    if i.rows == 8192 {
        let line = one_line(text, "failure=query ")?
            .strip_suffix(" span=36:60 released=true")
            .ok_or("late export error lost its source span")?;
        let values = integer_fields(line, &["bytes", "writes", "flushes"])?;
        if values[0] < 1024 || values[1] == 0 || values[2] != 0 {
            return Err("query failure did not follow an unflushed prefix".into());
        }
    }
    Ok(())
}

fn schema() -> Value {
    json!({"format":"pipesql-jsonl", "version":1, "columns":[
        {"name":"id", "type":"int64", "nullable":false},
        {"name":"duplicate", "type":"int64", "nullable":true},
        {"name":"duplicate", "type":"double", "nullable":true},
        {"name":"day", "type":"date", "nullable":true},
        {"name":"label", "type":"string", "nullable":true}
    ]})
}

fn expected(id: usize, profile: &str) -> Value {
    let amount = [
        "-9223372036854775808",
        "9223372036854775807",
        "-1",
        "0",
        "1",
        "-9007199254740993",
        "9007199254740993",
    ];
    let number = [
        "0000000000000000",
        "8000000000000000",
        "3ff0000000000000",
        "bff0000000000000",
        "0000000000000001",
        "7fefffffffffffff",
        "7ff0000000000000",
        "fff0000000000000",
        "7ff8000000000123",
        "fff8000000000042",
    ];
    let day = [
        "0001-01-01",
        "1969-12-31",
        "1970-01-01",
        "2000-02-29",
        "1900-03-01",
        "9999-12-31",
    ];
    let label = if id.is_multiple_of(13) {
        Value::Null
    } else if id.is_multiple_of(17) {
        json!("")
    } else {
        let width = if profile == "plain8" { 8 } else { 1024 };
        let mut text = String::new();
        if profile == "escaped1024" {
            while text.len() + 11 <= width - 8 {
                text.push_str("\u{0}\u{a}\u{9}\u{22}\u{5c}\u{e9}\u{1f642}");
            }
        }
        while text.len() < width - 8 {
            text.push('x');
        }
        text.push_str(&format!("{id:08}"));
        json!(text)
    };
    json!([
        id.to_string(),
        (!id.is_multiple_of(5)).then_some(amount[id % amount.len()]),
        (!id.is_multiple_of(7)).then_some(number[id % number.len()]),
        (!id.is_multiple_of(11)).then_some(day[id % day.len()]),
        label
    ])
}

fn validate(reader: impl BufRead, i: &Input<'_>) -> Result<()> {
    let mut lines = reader.lines();
    let mut next = || -> Result<Value> {
        Ok(serde_json::from_str(
            &lines.next().ok_or("incomplete export stream")??,
        )?)
    };
    if next()? != schema() {
        return Err("export schema differs".into());
    }
    let mut seen = vec![false; i.rows];
    for position in 0..i.rows {
        let value = next()?;
        let id = value["row"][0]
            .as_str()
            .ok_or("export ID is not typed INT64")?
            .parse::<usize>()?;
        if id >= i.rows || seen[id] || (i.query == "order" && id != position) {
            return Err("export row identity or order differs".into());
        }
        if value != json!({"row":expected(id, i.profile)}) {
            return Err(format!("export row {id} differs").into());
        }
        seen[id] = true;
    }
    if next()? != json!({"complete":true, "rows":i.rows}) || lines.next().is_some() {
        return Err("export completion or trailing data differs".into());
    }
    Ok(())
}

fn file(path: &Path, i: &Input<'_>) -> Result<Value> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("export filename is not UTF-8")?;
    let bytes = fs::metadata(path)?.len();
    if bytes > 64_000_000 {
        return Err("export exceeds selected byte bound".into());
    }
    let start = Instant::now();
    validate(BufReader::new(File::open(path)?), i)?;
    Ok(
        json!({"file":name, "bytes":bytes, "rows":i.rows, "validation_ns":start.elapsed().as_nanos()}),
    )
}

// These corruptions operate on an actual small producer output. In particular,
// duplicate IDs must fail even for a scan, whose row order is not prescribed.
fn reader_controls(path: &Path, i: &Input<'_>) -> Result<usize> {
    let text = fs::read_to_string(path)?;
    let lines: Vec<_> = text.lines().map(str::to_owned).collect();
    if lines.len() != i.rows + 2 || i.rows != 257 || i.query != "order" {
        return Err("invalid reader-control input".into());
    }
    let mut mutations = Vec::new();
    let mut wrong = lines.clone();
    let mut row: Value = serde_json::from_str(&wrong[1])?;
    row["row"][4] = json!("wrong text");
    wrong[1] = row.to_string();
    mutations.push(wrong);
    let mut wrong = lines.clone();
    wrong[2] = wrong[1].clone();
    mutations.push(wrong);
    let mut wrong = lines.clone();
    wrong.remove(1);
    mutations.push(wrong);
    let mut wrong = lines.clone();
    wrong.pop();
    mutations.push(wrong);
    let mut wrong = lines.clone();
    wrong.truncate(3);
    mutations.push(wrong);
    let mut wrong = lines.clone();
    wrong.push("{}".into());
    mutations.push(wrong);
    let mut wrong = lines.clone();
    let mut header = schema();
    header["columns"][2]["type"] = json!("int64");
    wrong[0] = header.to_string();
    mutations.push(wrong);
    let mut wrong = lines.clone();
    wrong[i.rows + 1] = "{\"complete\":true,\"rows\":1}".into();
    mutations.push(wrong);
    let mut wrong = lines.clone();
    let mut row: Value = serde_json::from_str(&wrong[2])?;
    row["row"].as_array_mut().unwrap().swap(1, 2);
    wrong[2] = row.to_string();
    mutations.push(wrong);
    let scan = Input {
        query: "scan",
        ..*i
    };
    let count = mutations.len() + 1;
    for (index, wrong) in mutations.into_iter().enumerate() {
        if validate(Cursor::new(wrong.join("\n")), &scan).is_ok() {
            return Err(format!("export reader accepted corruption {index}").into());
        }
    }
    let mut reordered = lines;
    reordered.swap(1, 2);
    let reordered = reordered.join("\n");
    validate(Cursor::new(&reordered), &scan)?;
    if validate(Cursor::new(reordered), i).is_ok() {
        return Err("ordered export accepted reversed rows".into());
    }
    Ok(count)
}

fn late_prefix(path: &Path, report: &str) -> Result<usize> {
    let line = one_line(report, "failure=query ")?
        .strip_suffix(" span=36:60 released=true")
        .ok_or("missing late query outcome")?;
    let counts = integer_fields(line, &["bytes", "writes", "flushes"])?;
    if fs::metadata(path)?.len() != counts[0] || counts[0] > 64_000_000 {
        return Err("late query prefix length differs".into());
    }
    let bytes = fs::read_to_string(path)?;
    let mut lines = bytes.split_inclusive('\n');
    let header: Value = serde_json::from_str(lines.next().ok_or("missing late query schema")?)?;
    if header
        != json!({"format":"pipesql-jsonl", "version":1, "columns":[{"name":"overflow", "type":"int64", "nullable":false}]})
    {
        return Err("late query schema differs".into());
    }
    let mut complete = 0;
    for line in lines {
        if complete >= 8191 {
            return Err("late query emitted its overflowing row".into());
        }
        let expected = json!({"row":[(9_223_372_036_854_767_617_u64 + complete).to_string()]});
        if line.ends_with('\n') {
            if serde_json::from_str::<Value>(line)? != expected {
                return Err("late query wrote a wrong row or completion".into());
            }
            complete += 1;
        } else if !expected.to_string().starts_with(line) {
            return Err("late query partial row differs".into());
        }
    }
    if complete == 0 {
        return Err("late query failed before its checked prefix".into());
    }
    Ok(complete as usize)
}

pub(super) fn verify(scenario: &Scenario, directory: &Path, report: &str) -> Result<Value> {
    check_report(scenario, report)?;
    let i = input(&scenario.arguments)?;
    if i.parquet.is_some() {
        return Err("Parquet output needs its external reader".into());
    }
    let mut results = Vec::new();
    let mut files = vec![("warmup.jsonl".to_owned(), "warmup ".to_owned())];
    files.extend((0..5).map(|sample| {
        (
            format!("sample-{sample}.jsonl"),
            format!("sample={sample} "),
        )
    }));
    files.push(("retry.jsonl".into(), "retry ".into()));
    for (name, prefix) in files {
        let mut verified = file(&directory.join(name), &i)?;
        let observed = integer_fields(one_line(report, &prefix)?, SAMPLE)?;
        if verified["bytes"].as_u64() != Some(observed[2]) {
            return Err("reported export length differs".into());
        }
        verified["api_outcome"] = json!("success");
        results.push(verified);
    }
    if i.rows != 0 {
        // All bytes, including completion, reached this writer before its flush
        // returned an error. The producer must report the API failure separately.
        let mut verified = file(&directory.join("failure-flush.jsonl"), &i)?;
        verified["api_outcome"] = json!("expected_flush_error");
        results.push(verified);
        for name in ["write", "flush", "cancel", "rows", "bytes"] {
            let line = one_line(report, &format!("failure={name} "))?
                .strip_suffix(" released=true")
                .ok_or("missing export release")?;
            let observed = integer_fields(line, &["bytes", "writes", "flushes"])?;
            let path = directory.join(format!("failure-{name}.jsonl"));
            let length = fs::metadata(&path)?.len();
            if length != observed[0] || length > 64_000_000 {
                return Err("failed export file length differs".into());
            }
            // Read failures propagate; only malformed/incomplete stream bytes
            // satisfy this control. Missing or unreadable files must fail it.
            let bytes = fs::read(path)?;
            if name != "flush" && validate(Cursor::new(bytes), &i).is_ok() {
                return Err("failed export was accepted as a complete stream".into());
            }
        }
    }
    let controls = if i.rows == 257 {
        reader_controls(&directory.join("warmup.jsonl"), &i)?
    } else {
        0
    };
    let late_rows = if i.rows == 8192 {
        late_prefix(&directory.join("failure-query.jsonl"), report)?
    } else {
        0
    };
    Ok(
        json!({"files":results, "reader_rejections":controls, "late_query_prefix_rows":late_rows, "flush_failure_file_is_complete":i.rows != 0}),
    )
}

pub(super) fn controls(run: &mut crate::workspace::Run, executable: &Path) -> Result<()> {
    let directory = run.directory.join("export-missing-flush-fault");
    let mut command = Command::new(executable);
    command
        .arg(&directory)
        .args(["order", "plain8", "64", "127", "257"])
        .env("PIPESQL_EXPORT_CONTROL", "skip-flush-fault");
    let output = run.command(&mut command, None, Duration::from_secs(300))?;
    if output.require_success().is_ok()
        || !std::str::from_utf8(&output.stderr.bytes)?.contains("export fault must fail")
    {
        return Err("disabled flush fault was not detected".into());
    }
    let i = Input {
        query: "order",
        profile: "plain8",
        batch: 64,
        fragment: 127,
        rows: 257,
        parquet: None,
    };
    let verified = file(&directory.join("failure-flush.jsonl"), &i)?;
    fs::write(
        run.directory.join("export-control.json"),
        serde_json::to_vec_pretty(&json!({
            "missing_flush_fault_rejected":true, "complete_file":verified,
        }))?,
    )?;
    fs::remove_dir_all(directory)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_scalars_keep_bits_extremes_nulls_and_empty_text() {
        assert_eq!(expected(0, "plain8"), json!(["0", null, null, null, null]));
        assert_eq!(
            expected(1, "plain8"),
            json!([
                "1",
                "9223372036854775807",
                "8000000000000000",
                "1969-12-31",
                "00000001"
            ])
        );
        assert_eq!(
            expected(7, "plain8"),
            json!(["7", "-9223372036854775808", null, "1969-12-31", "00000007"])
        );
        assert_eq!(
            expected(17, "plain8"),
            json!(["17", "0", "fff0000000000000", "9999-12-31", ""])
        );
    }

    #[test]
    fn reports_require_every_sample_success_flush_and_failure_control() {
        let scenario = Scenario::new(
            "export_cost",
            &["order", "plain8", "64", "127", "257"],
            "status=exported".into(),
            1,
        );
        let mut report = String::from(
            "input query=order profile=plain8 batch_rows=64 fragment=127 rows=257 memory_limit=4000000\n",
        );
        let sample =
            "elapsed_ns=1 rows=257 bytes=17520 writes=154 flushes=1 memory=2000000 temporary=4096";
        report.push_str(&format!("warmup {sample} scratch=4096\n"));
        for sample_id in 0..5 {
            report.push_str(&format!("sample={sample_id} {sample} scratch=0\n"));
        }
        for name in ["write", "flush", "cancel", "rows", "bytes"] {
            report.push_str(&format!(
                "failure={name} bytes={} writes=154 flushes={} released=true\n",
                if name == "flush" { 17520 } else { 1024 },
                usize::from(name == "flush")
            ));
        }
        report.push_str(&format!("retry {sample} scratch=0\nstatus=exported\n"));
        check_report(&scenario, &report).unwrap();
        for (from, to) in [
            ("sample=0 ", "sample=5 "),
            ("rows=257 bytes=17520", "rows=256 bytes=17520"),
            ("writes=154", "writes=1"),
            ("flushes=1 memory=", "flushes=0 memory="),
            ("failure=flush ", "failure=unknown "),
            ("released=true", "released=false"),
            ("status=exported", "status=failed"),
            ("scratch=4096", "scratch=0"),
        ] {
            let wrong = report.replacen(from, to, 1);
            assert_ne!(wrong, report);
            assert!(
                check_report(&scenario, &wrong).is_err(),
                "accepted {from} -> {to}"
            );
        }
        let parquet = Scenario::new(
            "export_cost",
            &[
                "order", "plain8", "64", "127", "257", "parquet", "128", "4096",
            ],
            "status=exported".into(),
            1,
        );
        // A complete ordinary report must not stand in for the extra Parquet
        // bounds and failure outcomes, even when all common samples pass.
        assert!(check_report(&parquet, &report).is_err());
        let mut parquet_report = report.replacen(
            "warmup ",
            "format=parquet group_rows=128 group_text=4096\nwarmup ",
            1,
        );
        assert!(check_report(&parquet, &parquet_report).is_err());
        for name in ["groups", "text", "metadata"] {
            parquet_report = parquet_report.replacen(
                "retry ",
                &format!("failure={name} bytes=1024 writes=154 flushes=0 released=true\nretry "),
                1,
            );
        }
        check_report(&parquet, &parquet_report).unwrap();
        assert!(check_report(&scenario, &parquet_report).is_err());
        for (from, to) in [
            ("group_rows=128", "group_rows=311"),
            ("group_text=4096", "group_text=65536"),
            ("failure=groups ", "failure=bytes "),
            ("failure=text ", "failure=bytes "),
            ("failure=metadata ", "failure=bytes "),
        ] {
            let wrong = parquet_report.replacen(from, to, 1);
            assert_ne!(wrong, parquet_report);
            assert!(
                check_report(&parquet, &wrong).is_err(),
                "accepted {from} -> {to}"
            );
        }
        assert!(check_report(&scenario, &format!("{report}status=exported\n")).is_err());
        assert!(
            check_report(&scenario, report.strip_suffix("status=exported\n").unwrap()).is_err()
        );
    }
}
