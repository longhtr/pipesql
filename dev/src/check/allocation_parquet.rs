//! Require native Parquet refusal prefixes and complete import recovery or export retry.

use super::*;
use std::collections::BTreeSet;

fn prefix_report(output: &str, prefix: usize, count: usize) -> Result<&str> {
    require_line(output, "Parquet import recovery and complete rows passed")?;
    let outcome = record(output, "Parquet import outcome=")?;
    if !matches!(
        outcome,
        "committed" | "refused" | "cleanup" | "recovery" | "ambiguous"
    ) || (outcome == "committed") != (prefix == count)
    {
        return Err(format!("Parquet import prefix {prefix}: unexpected {outcome}").into());
    }
    let refusals = number(output, "parquet-import refusals=")?;
    if (refusals > 0) != (prefix < count) {
        return Err("Parquet import refusal missing".into());
    }
    Ok(outcome)
}

fn export_prefix_report(output: &str, prefix: usize, count: usize) -> Result<(&str, usize)> {
    require_line(output, "Parquet export release and complete retry passed")?;
    let outcome = record(output, "Parquet export outcome=")?;
    if !matches!(outcome, "exported" | "refused" | "recovery")
        || (outcome == "exported") != (prefix == count)
    {
        return Err(format!("Parquet export prefix {prefix}: unexpected {outcome}").into());
    }
    if (number(output, "parquet-export refusals=")? > 0) != (prefix < count) {
        return Err("Parquet export refusal missing".into());
    }
    Ok((outcome, number(output, "Parquet export bytes=")?))
}

impl Campaign {
    pub(super) fn parquet_export(&mut self) -> Result<()> {
        let healthy = self.execute("parquet-export", None)?;
        let count = number(&healthy, "parquet-export allocations=")?;
        if !(1..=512).contains(&count) {
            return Err("invalid Parquet export census".into());
        }
        let (_, complete_bytes) = export_prefix_report(&healthy, count, count)?;
        if complete_bytes == 0 || export_prefix_report(&healthy, 0, count).is_ok() {
            return Err("disabled Parquet export refusal accepted".into());
        }
        let mut before_output = 0;
        let mut incomplete_output = 0;
        let mut recovery = 0;
        for prefix in std::iter::once(count).chain(0..count) {
            let output = self.execute(&format!("parquet-export-{prefix}"), None)?;
            let (outcome, bytes) = export_prefix_report(&output, prefix, count)?;
            recovery += usize::from(outcome == "recovery");
            if prefix == count {
                if bytes != complete_bytes {
                    return Err("full-prefix Parquet export length changed".into());
                }
            } else if bytes == 0 {
                before_output += 1;
            } else if bytes < complete_bytes {
                incomplete_output += 1;
            } else {
                return Err("refused Parquet export reported complete output".into());
            }
        }
        if before_output == 0 || incomplete_output == 0 || recovery == 0 {
            return Err("Parquet export missed an output refusal boundary".into());
        }
        let flush = self.execute("parquet-export-flush", None)?;
        require_line(&flush, "Parquet export release and complete retry passed")?;
        if record(&flush, "Parquet export outcome=")? != "flush-failed"
            || number(&flush, "Parquet export bytes=")? != complete_bytes
            || number(&flush, "parquet-export refusals=")? == 0
        {
            return Err("Parquet export missed final flush failure under denial".into());
        }
        fs::write(
            self.run.directory.join("parquet-export.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "refusal_prefixes":count, "before_output":before_output,
                "incomplete_output":incomplete_output, "complete_bytes":complete_bytes,
                "recovery_required":recovery,
                "full_prefix_passed":true, "disabled_refusal_rejected":true,
                "flush_failure_under_denial_passed":true
            }))?,
        )?;
        println!(
            "Parquet export: {count} refusal prefixes, complete retry, unchanged authority and final flush failure under denial passed"
        );
        Ok(())
    }

    pub(super) fn parquet_import(&mut self) -> Result<()> {
        let receipt = self.execute("parquet-import-receipt", None)?;
        require_line(&receipt, "Parquet import receipt cleanup passed")?;
        require_line(&receipt, "Parquet import recovery and complete rows passed")?;
        let healthy = self.execute("parquet-import", None)?;
        require_line(&healthy, "Parquet import outcome=committed")?;
        require_line(&healthy, "Parquet import recovery and complete rows passed")?;
        let count = number(&healthy, "parquet-import allocations=")?;
        if !(1..=1000).contains(&count) {
            return Err("invalid Parquet import census".into());
        }
        prefix_report(&healthy, count, count)?;
        // This actual uninjected run must fail if presented as a refused prefix.
        if prefix_report(&healthy, 0, count).is_ok() {
            return Err("disabled Parquet import refusal accepted".into());
        }
        let mut outcomes = BTreeSet::new();
        for prefix in std::iter::once(count).chain(0..count) {
            let output = self.execute(&format!("parquet-import-{prefix}"), None)?;
            outcomes.insert(prefix_report(&output, prefix, count)?.to_owned());
        }
        for required in ["refused", "cleanup", "ambiguous"] {
            if !outcomes.contains(required) {
                return Err(format!("Parquet import missed {required}").into());
            }
        }
        fs::write(
            self.run.directory.join("parquet-import.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "refusal_prefixes":count, "outcomes":outcomes, "full_prefix_passed":true,
                "disabled_refusal_rejected":true, "receipt_cleanup_passed":true
            }))?,
        )?;
        println!(
            "Parquet import: {count} refusal prefixes, full-prefix and disabled-refusal controls, receipt failure and complete recovery passed"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parquet_export_prefixes_require_refusal_and_complete_retry() {
        let healthy = "parquet-export refusals=0\nParquet export outcome=exported\nParquet export bytes=70000\nParquet export release and complete retry passed\n";
        assert_eq!(
            export_prefix_report(healthy, 10, 10).unwrap(),
            ("exported", 70000)
        );
        assert!(export_prefix_report(healthy, 0, 10).is_err());
        let failed = healthy
            .replace("refusals=0", "refusals=1")
            .replace("outcome=exported", "outcome=refused")
            .replace("bytes=70000", "bytes=4");
        assert_eq!(
            export_prefix_report(&failed, 4, 10).unwrap(),
            ("refused", 4)
        );
        let recovery = failed.replace("outcome=refused", "outcome=recovery");
        assert_eq!(
            export_prefix_report(&recovery, 4, 10).unwrap(),
            ("recovery", 4)
        );
        assert!(export_prefix_report(&recovery, 10, 10).is_err());
        assert!(export_prefix_report(&failed, 10, 10).is_err());
        for wrong in [
            failed.replace("refusals=1", "refusals=0"),
            failed.replace("outcome=refused", "outcome=flush-failed"),
            failed.replace("Parquet export release and complete retry passed", ""),
            failed.replace("bytes=4", "bytes=oops"),
            format!("{failed}Parquet export bytes=4\n"),
            format!("{failed}Parquet export outcome=refused\n"),
        ] {
            assert!(
                export_prefix_report(&wrong, 4, 10).is_err(),
                "accepted {wrong}"
            );
        }
        assert!(export_prefix_report("", 0, 10).is_err());
    }

    #[test]
    fn parquet_import_prefixes_require_refusal_outcomes_and_complete_recovery() {
        let healthy = "parquet-import refusals=0\nParquet import outcome=committed\nParquet import recovery and complete rows passed\n";
        assert_eq!(prefix_report(healthy, 10, 10).unwrap(), "committed");
        assert!(prefix_report(healthy, 0, 10).is_err());
        for outcome in ["refused", "cleanup", "recovery", "ambiguous"] {
            let failed = healthy
                .replace("refusals=0", "refusals=2")
                .replace("outcome=committed", &format!("outcome={outcome}"));
            assert_eq!(prefix_report(&failed, 4, 10).unwrap(), outcome);
            for wrong in [
                failed.replace("refusals=2", "refusals=0"),
                failed.replace("refusals=2", "refusals=oops"),
                failed.replace("Parquet import recovery and complete rows passed", ""),
                failed.replace(&format!("outcome={outcome}"), "outcome=unknown"),
                format!("{failed}Parquet import outcome={outcome}\n"),
            ] {
                assert!(prefix_report(&wrong, 4, 10).is_err(), "accepted {wrong}");
            }
            assert!(prefix_report(&failed, 10, 10).is_err());
        }
        assert!(prefix_report("", 0, 10).is_err());
    }
}
