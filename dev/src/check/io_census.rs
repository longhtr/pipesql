//! Observe actual query transfers separately from timed analytical samples.
//!
//! The driver reuses the operator fixture and its checked execution. Independent
//! native standard controls establish byte accounting before this census. Fresh
//! processes isolate every selected kind; source, artifacts, inputs and counters
//! remain in generated output. No fault-coverage or timing claim follows here.

use super::*;
use serde_json::json;

fn report(output: &str, rows: u64) -> Result<[u64; 3]> {
    let mut lines = output.lines();
    let mut fields = lines
        .next()
        .and_then(|line| line.strip_prefix("census "))
        .ok_or("missing query census")?
        .split_whitespace();
    let mut values = [0; 4];
    for (slot, prefix) in values
        .iter_mut()
        .zip(["rows=", "calls=", "requested=", "transferred="])
    {
        let value = fields
            .next()
            .and_then(|field| field.strip_prefix(prefix))
            .ok_or("missing query census field")?;
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err("invalid query census number".into());
        }
        *slot = value.parse::<u64>()?;
    }
    if fields.next().is_some()
        || lines.next() != Some("checked census complete")
        || lines.next().is_some()
        || values[0] != rows
        || values[1] > 65_536
        || values[3] > values[2]
        || (values[1] == 0) != (values[2] == 0)
        || (values[1] != 0 && values[3] == 0)
    {
        return Err("query census is incomplete or its observations disagree".into());
    }
    Ok([values[1], values[2], values[3]])
}

fn inputs(joins: bool) -> Vec<[String; 5]> {
    if joins {
        return crate::measure::join_replay_inputs()
            .into_iter()
            .map(|input| input.map(str::to_owned))
            .collect();
    }
    let mut inputs = Vec::new();
    for operation in [
        "window-count-payload",
        "window-sum-payload",
        "scan-payload",
        "order-payload",
    ] {
        for (rows, keys) in [
            (8192, 32),
            (8192, 256),
            (8192, 4096),
            (8320, 4096),
            (12288, 4096),
        ] {
            if matches!(operation, "scan-payload" | "order-payload") && keys != 32 {
                continue;
            }
            for width in [8, 1024] {
                inputs.push([
                    operation.to_owned(),
                    rows.to_string(),
                    keys.to_string(),
                    width.to_string(),
                    "12000000".to_owned(),
                ]);
            }
        }
    }
    inputs
}

impl Campaign {
    pub(super) fn census(&mut self, joins: bool) -> Result<()> {
        for mode in ["probe", "census"] {
            for overflow in [false, true] {
                let root = self
                    .run
                    .directory
                    .join(format!("census-control-{mode}-{overflow}"));
                fs::create_dir(&root)?;
                let mut command = Command::new(&self.driver);
                command.arg(&root).args([
                    "census-control",
                    mode,
                    if overflow { "overflow" } else { "complete" },
                ]);
                command
                    .env_remove("LD_PRELOAD")
                    .env_remove("DYLD_INSERT_LIBRARIES")
                    .env(
                        if cfg!(target_os = "macos") {
                            "DYLD_INSERT_LIBRARIES"
                        } else {
                            "LD_PRELOAD"
                        },
                        &self.observer,
                    );
                let output = self
                    .run
                    .command(&mut command, None, Duration::from_secs(20))?;
                let limit = if mode == "probe" { 4096 } else { 65_536 };
                let expected = format!(
                    "native census limit reached: {limit}\n{}",
                    if overflow {
                        ""
                    } else {
                        "native census control complete\n"
                    }
                );
                if !matches!(output.completion, crate::process::Completion::Exited(status) if status.code() == Some(if overflow { 92 } else { 0 }))
                    || output.stdout.omitted != 0
                    || output.stderr.omitted != 0
                    || output.stdout.bytes != expected.as_bytes()
                    || !output.stderr.bytes.is_empty()
                {
                    return Err(format!(
                        "native census boundary control failed: {mode}, overflow={overflow}"
                    )
                    .into());
                }
                fs::remove_dir_all(root)?;
                self.cells += 1;
            }
        }
        let mut receipts = Vec::new();
        for [operation, rows, keys, width, memory] in inputs(joins) {
            let row_count = rows.parse::<u64>()?;
            let key_count = keys.parse::<u64>()?;
            // Required answer counts belong to this supervisor, not the driver.
            let expected = match operation.as_str() {
                "join-many" => row_count * 8,
                "order-many-text" => key_count * 8,
                "order-text" => key_count,
                _ => row_count,
            };
            for kind in [2, 3] {
                let name = format!("census-{operation}-{rows}-{keys}-{width}-{memory}-{kind}");
                let root = self.run.directory.join(&name);
                fs::create_dir(&root)?;
                let mut command = Command::new(&self.driver);
                command
                    .arg(&root)
                    .arg("census")
                    .arg(kind.to_string())
                    .arg(&operation)
                    .args([&rows, &keys, &width, &memory]);
                command
                    .env_remove("LD_PRELOAD")
                    .env_remove("DYLD_INSERT_LIBRARIES")
                    .env(
                        if cfg!(target_os = "macos") {
                            "DYLD_INSERT_LIBRARIES"
                        } else {
                            "LD_PRELOAD"
                        },
                        &self.observer,
                    );
                let output = self
                    .run
                    .command(&mut command, None, Duration::from_secs(20))?;
                output.require_success()?;
                let [calls, requested, transferred] =
                    report(std::str::from_utf8(&output.stdout.bytes)?, expected)?;
                if (calls > 0)
                    != (kind == 2 || !matches!(operation.as_str(), "scan" | "scan-payload"))
                {
                    return Err("query census missed expected native transfers".into());
                }
                receipts.push(
                    json!({"operation":operation, "rows":row_count, "keys":key_count,
                    "payload_bytes":width.parse::<u64>()?, "memory_bytes":memory.parse::<u64>()?,
                    "answer_rows":expected, "kind":kind, "calls":calls, "requested_bytes":requested,
                    "transferred_bytes":transferred}),
                );
                fs::remove_dir_all(root)?;
                self.cells += 1;
            }
        }
        fs::write(
            self.run.directory.join("census.json"),
            serde_json::to_vec_pretty(
                &json!({"cases":receipts,"faults":false,"timed":false,"complete_answers_and_release_checked":true}),
            )?,
        )?;
        println!(
            "Native query census: {} isolated checked intervals; actual calls and transferred bytes",
            receipts.len()
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn join_census_uses_matching_inputs_without_changing_window_selection() {
        let joins = inputs(true);
        assert_eq!(joins.len(), 37);
        assert_eq!(
            joins
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            37
        );
        assert_eq!(inputs(false).len(), 24);
        assert!(inputs(false).iter().all(|input| input[4] == "12000000"));
        let valid =
            "census rows=32768 calls=10 requested=100 transferred=100\nchecked census complete\n";
        report(valid, 32768).unwrap();
        assert!(report(valid, 8192).is_err());
    }
    #[test]
    fn query_census_requires_complete_bounded_native_observations() {
        let valid =
            "census rows=8192 calls=10 requested=100 transferred=100\nchecked census complete\n";
        assert_eq!(report(valid, 8192).unwrap(), [10, 100, 100]);
        assert_eq!(
            report(&valid.replace("transferred=100", "transferred=99"), 8192).unwrap(),
            [10, 100, 99]
        );
        for wrong in [
            valid.replace("transferred=100", "transferred=101"),
            valid.replace("transferred=100", "transferred=0"),
            valid.replace("calls=10", "calls=65537"),
            valid.replace("rows=8192", "rows=8191"),
            valid.replace("calls=10", "calls=+10"),
            valid.replace("checked census complete\n", ""),
            format!("{valid}checked census complete\n"),
            valid.replace("calls=10", "calls=0"),
        ] {
            assert!(report(&wrong, 8192).is_err());
        }
    }
}
