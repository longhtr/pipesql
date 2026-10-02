//! Judge native window I/O by independent typed answers and API outcomes.
//!
//! Row order is not a public promise. Complete retries are checked by identity;
//! their validated byte stream then bounds partial output from the failed attempt.
//! The final marker and successful API outcome remain separate requirements.

use super::*;
use serde_json::{Value, json};
use std::path::Path;

const ROWS: usize = 1539;

fn schema() -> Value {
    json!({"format":"pipesql-jsonl","version":1,"columns":[
        {"name":"id","type":"int64","nullable":false},
        {"name":"part","type":"int64","nullable":true},
        {"name":"peer","type":"int64","nullable":true},
        {"name":"total","type":"int64","nullable":true},
        {"name":"note","type":"string","nullable":false}
    ]})
}

fn expected(id: usize) -> Result<Value> {
    if id >= ROWS {
        return Err("window I/O returned an out-of-range identity".into());
    }
    let (part, peer, total) = [
        (None, None, Some("7")),
        (None, Some("1"), Some("16")),
        (Some("1"), None, None),
    ][id / 513];
    Ok(json!({"row":[id.to_string(),part,peer,total,format!("{id:04}雪\0{}", "x".repeat(504))]}))
}

fn records(bytes: &[u8]) -> Result<Vec<Value>> {
    if bytes.len() > 2_000_000 || !bytes.ends_with(b"\n") {
        return Err("window I/O stream is oversized or lacks its final newline".into());
    }
    Ok(std::str::from_utf8(bytes)?
        .split_terminator('\n')
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?)
}

fn complete(bytes: &[u8]) -> Result<()> {
    let records = records(bytes)?;
    if records.len() != ROWS + 2
        || records[0] != schema()
        || records[ROWS + 1] != json!({"complete":true,"rows":ROWS})
    {
        return Err("window I/O schema, row count or completion differs".into());
    }
    let mut seen = vec![false; ROWS];
    for record in &records[1..=ROWS] {
        let id = record["row"][0]
            .as_str()
            .ok_or("missing typed window identity")?
            .parse::<usize>()?;
        if *record != expected(id)? || seen[id] {
            return Err("window I/O differs from independent complete typed answers".into());
        }
        seen[id] = true;
    }
    Ok(())
}

fn streams(initial: &[u8], retry: &[u8], finished: bool) -> Result<()> {
    complete(retry)?;
    if finished {
        complete(initial)?;
    } else if initial.len() >= retry.len() {
        return Err("failed window I/O produced a complete stream".into());
    }
    // A write may stop inside a JSON record or UTF-8 character. Validate bytes
    // against a complete, independently checked retry instead of parsing a tail.
    if !retry.starts_with(initial) {
        return Err("window I/O initial bytes differ from the validated retry prefix".into());
    }
    Ok(())
}

fn record<'a>(output: &'a str, prefix: &str) -> Result<&'a str> {
    let mut matches = output.lines().filter_map(|line| line.strip_prefix(prefix));
    let line = matches
        .next()
        .ok_or_else(|| format!("missing {prefix} record"))?;
    if matches.next().is_some() || line.is_empty() {
        return Err(format!("duplicate or empty {prefix} record").into());
    }
    Ok(line)
}

fn number(field: Option<&str>, prefix: &str) -> Result<usize> {
    let value = field
        .and_then(|field| field.strip_prefix(prefix))
        .ok_or("missing window I/O numeric field")?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("invalid window I/O numeric field".into());
    }
    Ok(value.parse()?)
}

fn spill(fields: &mut std::str::SplitWhitespace<'_>) -> Result<(usize, usize)> {
    let written = number(fields.next(), "spill=")?;
    let temporary = number(fields.next(), "temp=")?;
    if fields.next().is_some() {
        return Err("extra window I/O field".into());
    }
    Ok((written, temporary))
}

fn report(output: &str, initial_bytes: usize, at: u32, error: i32) -> Result<(usize, usize)> {
    if output
        .lines()
        .filter(|line| *line == "window I/O release and complete retries passed")
        .count()
        != 1
    {
        return Err("missing or duplicate window I/O completion".into());
    }
    let observed = counts(output)?;
    let finished = at == 0 || error == 0;
    if (observed[1] > 0) == finished {
        return Err("window I/O fault was disabled or its outcome differs".into());
    }
    let mut fields = record(output, "window initial ")?.split_whitespace();
    if fields.next()
        != Some(if finished {
            "outcome=finished"
        } else {
            "outcome=failed"
        })
        || number(fields.next(), "bytes=")? != initial_bytes
    {
        return Err("window I/O outcome or byte report differs".into());
    }
    let initial_spill = spill(&mut fields)?;
    for prefix in ["window retry ", "window reopened "] {
        let mut fields = record(output, prefix)?.split_whitespace();
        if number(fields.next(), "rows=")? != ROWS {
            return Err("window I/O retry row count differs".into());
        }
        let (written, temporary) = spill(&mut fields)?;
        if temporary == 0 || (cfg!(target_os = "linux") && written == 0) {
            return Err("window I/O retry lacks spill observation".into());
        }
    }
    if finished && (initial_spill.1 == 0 || (cfg!(target_os = "linux") && initial_spill.0 == 0)) {
        return Err("healthy window I/O lacks spill observation".into());
    }
    Ok(initial_spill)
}

fn encode(records: &[Value]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for record in records {
        bytes.extend(serde_json::to_vec(record)?);
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn controls(bytes: &[u8]) -> Result<usize> {
    complete(bytes)?;
    let valid = records(bytes)?;
    for case in 0..7 {
        let mut wrong = valid.clone();
        match case {
            0 => {
                wrong
                    .iter_mut()
                    .find(|r| r["row"][3] == json!("7"))
                    .unwrap()["row"][3] = json!("8")
            }
            1 => {
                wrong
                    .iter_mut()
                    .find(|r| r.get("row").is_some() && r["row"][3].is_null())
                    .unwrap()["row"][3] = json!("0")
            }
            2 => wrong[1]["row"][4] = json!("wrong identity text"),
            3 => {
                wrong.remove(1);
            }
            4 => wrong[2] = wrong[1].clone(),
            5 => {
                wrong.pop();
            }
            _ => wrong[0]["columns"][3]["nullable"] = json!(false),
        }
        if complete(&encode(&wrong)?).is_ok() {
            return Err(format!("window I/O answer control {case} was accepted").into());
        }
    }
    let mut prefix = bytes[..1024].to_vec();
    streams(&prefix, bytes, false)?;
    prefix[0] ^= 1;
    if streams(&prefix, bytes, false).is_ok()
        || streams(bytes, bytes, false).is_ok()
        || streams(&bytes[..1024], bytes, true).is_ok()
    {
        return Err("window I/O corrupt prefix or false completion was accepted".into());
    }
    Ok(10)
}

pub(super) fn verify_files(root: &Path, output: &str, at: u32, error: i32) -> Result<Value> {
    let initial = fs::read(root.join("initial.jsonl"))?;
    let retry = fs::read(root.join("retry.jsonl"))?;
    let reopened = fs::read(root.join("reopened.jsonl"))?;
    streams(&initial, &retry, at == 0 || error == 0)?;
    complete(&reopened)?;
    let (written, temporary) = report(output, initial.len(), at, error)?;
    let mut rejected = 0;
    if at == 0 {
        rejected = controls(&retry)?;
        if report(output, initial.len(), 1, libc::EIO).is_ok() {
            return Err("window I/O disabled fault control was accepted".into());
        }
        rejected += 1;
    }
    Ok(
        json!({"rows":ROWS,"columns":5,"initial_bytes":initial.len(),
        "outcome":if at == 0 || error == 0 { "finished" } else { "failed" },
        "written_spill":written,"temporary_peak":temporary,"independent_answers":true,
        "answer_controls":rejected,"counts":counts(output)?,
        "initial_sha256":workspace::hash(&root.join("initial.jsonl"))?,
        "retry_sha256":workspace::hash(&root.join("retry.jsonl"))?,
        "reopened_sha256":workspace::hash(&root.join("reopened.jsonl"))?}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn window_io_rejects_readable_wrong_answers_and_partial_completion() {
        let mut records = vec![schema()];
        for id in (0..ROWS).rev() {
            records.push(expected(id).unwrap());
        }
        records.push(json!({"complete":true,"rows":ROWS}));
        assert_eq!(controls(&encode(&records).unwrap()).unwrap(), 10);
    }
}
