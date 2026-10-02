//! Check native JSON Lines refusal against independent complete typed answers.
//!
//! The external fixture's literal values are repeated into 64 positional columns.
//! Retry must match every value before its bytes can serve as an exact prefix
//! reference for an interrupted export. Final flush failure still leaves a complete
//! stream, so its typed API outcome is checked separately from file completion.

use super::*;
use serde_json::{Value, json};

fn expected() -> Vec<Value> {
    let columns = (0..64)
        .map(|column| {
            let (name, kind) = [
                ("id", "int64"),
                ("amount", "int64"),
                ("number", "double"),
                ("day", "date"),
                ("note", "string"),
            ][column % 5];
            json!({"name":name,"type":kind,"nullable":column % 5 != 0})
        })
        .collect::<Vec<_>>();
    let mut records = vec![json!({"format":"pipesql-jsonl","version":1,"columns":columns})];
    let integers = [
        Some("-9223372036854775808"),
        Some("9223372036854775807"),
        None,
        Some("-1"),
        Some("0"),
        Some("1"),
        Some("42"),
        Some("-42"),
    ];
    let bits = [
        Some("0000000000000000"),
        Some("8000000000000000"),
        Some("7ff0000000000000"),
        Some("fff0000000000000"),
        Some("7ff8000000001234"),
        Some("fff0000000000001"),
        Some("0000000000000001"),
        None,
    ];
    let dates = [
        Some("0001-01-01"),
        Some("9999-12-31"),
        None,
        Some("1969-12-31"),
        Some("1970-01-01"),
        Some("1970-01-02"),
        Some("2000-02-29"),
        Some("2020-01-01"),
    ];
    let maximum = "x".repeat(65_536);
    let text = [
        Some(""),
        None,
        Some("é🙂"),
        Some("a,\n\"\\\0"),
        Some("\\N"),
        Some(maximum.as_str()),
        Some("tail"),
        Some("end"),
    ];
    for row in 0..8 {
        let values = [
            json!((row + 1).to_string()),
            json!(integers[row]),
            json!(bits[row]),
            json!(dates[row]),
            json!(text[row]),
        ];
        records.push(
            json!({"row":(0..64).map(|column| values[column % 5].clone()).collect::<Vec<_>>()}),
        );
    }
    records.push(json!({"complete":true,"rows":8}));
    records
}

fn complete(bytes: &[u8], expected: &[Value]) -> Result<()> {
    if bytes.len() > 1_000_000 || !bytes.ends_with(b"\n") {
        return Err("JSONL export is oversized or lacks its final newline".into());
    }
    let records = std::str::from_utf8(bytes)?
        .split_terminator('\n')
        .map(serde_json::from_str::<Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if records != expected {
        return Err("JSONL export differs from complete typed answers".into());
    }
    Ok(())
}

fn controls(bytes: &[u8], expected: &[Value]) -> Result<usize> {
    complete(bytes, expected)?;
    let valid = std::str::from_utf8(bytes)?
        .split_terminator('\n')
        .map(serde_json::from_str::<Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for case in 0..4 {
        let mut wrong = valid.clone();
        match case {
            0 => wrong[1]["row"][0] = wrong[1]["row"][1].clone(),
            1 => wrong[1]["row"][2] = json!("8000000000000000"),
            2 => wrong[2]["row"][4] = json!(""),
            _ => {
                wrong.pop();
            }
        }
        let mut bytes = Vec::new();
        for record in wrong {
            bytes.extend(serde_json::to_vec(&record)?);
            bytes.push(b'\n');
        }
        if complete(&bytes, expected).is_ok() {
            return Err(format!("JSONL answer control {case} was accepted").into());
        }
    }
    let mut prefix = bytes[..1024].to_vec();
    streams(&prefix, bytes, "refused", expected)?;
    prefix[0] ^= 1;
    if streams(&prefix, bytes, "refused", expected).is_ok()
        || streams(bytes, bytes, "refused", expected).is_ok()
    {
        return Err("JSONL incorrect or complete refusal prefix was accepted".into());
    }
    Ok(6)
}

fn streams(initial: &[u8], retry: &[u8], outcome: &str, expected: &[Value]) -> Result<()> {
    complete(retry, expected)?;
    if matches!(outcome, "exported" | "flush-failed") {
        complete(initial, expected)?;
        if initial != retry {
            return Err("JSONL complete retry bytes differ".into());
        }
    } else if !matches!(outcome, "refused" | "recovery")
        || initial.len() >= retry.len()
        || !retry.starts_with(initial)
    {
        return Err("JSONL refusal did not preserve an incomplete valid prefix".into());
    }
    Ok(())
}

pub(super) fn verify_files(root: &Path, output: &str, check_controls: bool) -> Result<Value> {
    let expected = expected();
    let initial = fs::read(root.join("initial.jsonl"))?;
    let retry = fs::read(root.join("retry.jsonl"))?;
    let outcome = record(output, "JSONL export outcome=")?;
    if number(output, "JSONL export bytes=")? != initial.len() {
        return Err("JSONL export byte report differs from its file".into());
    }
    streams(&initial, &retry, outcome, &expected)?;
    let blocked = root.join("blocked.jsonl");
    let blocked_receipt = if outcome == "recovery" {
        let bytes = fs::read(&blocked)?;
        streams(&bytes, &retry, "recovery", &expected)?;
        json!({"bytes":bytes.len(),"sha256":workspace::hash(&blocked)?})
    } else {
        if blocked.exists() {
            return Err("unexpected blocked JSONL attempt".into());
        }
        Value::Null
    };
    let rejected = if check_controls {
        controls(&retry, &expected)?
    } else {
        0
    };
    Ok(
        json!({"outcome":outcome,"initial_bytes":initial.len(),"retry_bytes":retry.len(),
        "initial_sha256":workspace::hash(&root.join("initial.jsonl"))?,
        "retry_sha256":workspace::hash(&root.join("retry.jsonl"))?,
        "rows":8,"columns":64,"independent_answers":true,"answer_controls":rejected,"blocked":blocked_receipt}),
    )
}

fn prefix_report(output: &str, prefix: usize, count: usize) -> Result<(&str, usize)> {
    require_line(output, "JSONL export release and complete retry passed")?;
    let outcome = record(output, "JSONL export outcome=")?;
    if !matches!(outcome, "exported" | "refused" | "recovery")
        || (outcome == "exported") != (prefix == count)
        || (number(output, "jsonl-export refusals=")? > 0) != (prefix < count)
    {
        return Err("JSONL export refusal or outcome differs from its prefix".into());
    }
    Ok((outcome, number(output, "JSONL export bytes=")?))
}

impl Campaign {
    pub(super) fn jsonl_export(&mut self) -> Result<()> {
        let healthy = self.execute("jsonl-export", None)?;
        let count = number(&healthy, "jsonl-export allocations=")?;
        if !(1..=1024).contains(&count) {
            return Err("invalid JSONL export census".into());
        }
        let (_, complete_bytes) = prefix_report(&healthy, count, count)?;
        if complete_bytes == 0 || prefix_report(&healthy, 0, count).is_ok() {
            return Err("disabled JSONL export refusal accepted".into());
        }
        let (mut before, mut partial, mut recovery) = (0, 0, 0);
        for prefix in std::iter::once(count).chain(0..count) {
            let output = self.execute(&format!("jsonl-export-{prefix}"), None)?;
            let (outcome, bytes) = prefix_report(&output, prefix, count)?;
            recovery += usize::from(outcome == "recovery");
            if prefix == count {
                if bytes != complete_bytes {
                    return Err("JSONL full-prefix length changed".into());
                }
            } else if bytes == 0 {
                before += 1;
            } else if bytes < complete_bytes {
                partial += 1;
            } else {
                return Err("JSONL refused export is complete".into());
            }
        }
        if before == 0 || partial == 0 || recovery == 0 {
            return Err("JSONL export missed an output refusal boundary".into());
        }
        let flush = self.execute("jsonl-export-flush", None)?;
        require_line(&flush, "JSONL export release and complete retry passed")?;
        if record(&flush, "JSONL export outcome=")? != "flush-failed"
            || number(&flush, "JSONL export bytes=")? != complete_bytes
            || number(&flush, "jsonl-export refusals=")? == 0
        {
            return Err("JSONL final writer failure under denial missing".into());
        }
        fs::write(
            self.run.directory.join("jsonl-export.json"),
            serde_json::to_vec_pretty(&json!({
                "refusal_prefixes":count,"before_output":before,"incomplete_output":partial,
                "recovery_required":recovery,"complete_bytes":complete_bytes,
                "full_prefix_passed":true,"disabled_refusal_rejected":true,
                "flush_failure_under_denial_passed":true,"answer_controls":6
            }))?,
        )?;
        println!(
            "JSONL export: {count} refusal prefixes, independent typed answers, complete retry and final flush failure under denial passed"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_jsonl_answers_reject_column_bits_null_and_missing_completion() {
        let expected = expected();
        let mut bytes = Vec::new();
        for record in &expected {
            bytes.extend(serde_json::to_vec(record).unwrap());
            bytes.push(b'\n');
        }
        assert_eq!(controls(&bytes, &expected).unwrap(), 6);
        bytes.pop();
        assert!(complete(&bytes, &expected).is_err());
    }

    #[test]
    fn jsonl_prefix_reports_require_refusal_and_complete_retry() {
        let healthy = "jsonl-export refusals=0\nJSONL export outcome=exported\nJSONL export bytes=900000\nJSONL export release and complete retry passed\n";
        assert_eq!(
            prefix_report(healthy, 10, 10).unwrap(),
            ("exported", 900000)
        );
        assert!(prefix_report(healthy, 0, 10).is_err());
        let refused = healthy
            .replace("refusals=0", "refusals=1")
            .replace("outcome=exported", "outcome=refused")
            .replace("bytes=900000", "bytes=2048");
        assert_eq!(prefix_report(&refused, 2, 10).unwrap(), ("refused", 2048));
        assert_eq!(
            prefix_report(
                &refused.replace("outcome=refused", "outcome=recovery"),
                2,
                10
            )
            .unwrap(),
            ("recovery", 2048)
        );
        for wrong in [
            refused.replace("refusals=1", "refusals=0"),
            refused.replace("outcome=refused", "outcome=flush-failed"),
            refused.replace("JSONL export release and complete retry passed", ""),
            refused.replace("bytes=2048", "bytes=oops"),
            format!("{refused}JSONL export bytes=2048\n"),
        ] {
            assert!(prefix_report(&wrong, 2, 10).is_err());
        }
        assert!(prefix_report(&refused, 10, 10).is_err());
        assert!(prefix_report("", 0, 10).is_err());
    }
}
