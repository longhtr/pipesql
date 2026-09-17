//! Refuse allocation after every possible successful prefix in a fresh process.
//!
//! The driver observes requested and native usable bytes through its allocator.
//! This supervisor requires complete census records and verifies persisted files
//! independently. Neither measurement establishes a bound on total process memory.
//!
//! Campaign cases select distinct workload families. Refusal sweeps first learn the
//! successful allocation count, then require every selected prefix and its expected
//! outcome. Deliberately broken observation and attribution cases must be rejected,
//! so missing evidence cannot appear as reduced allocation.

use crate::{
    Result,
    workspace::{self, Run},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[path = "allocation_join.rs"]
mod join;
#[path = "allocation_shapes.rs"]
mod shapes;
#[path = "allocation_trace.rs"]
mod trace;

struct Campaign {
    run: Run,
    driver: PathBuf,
    cases: usize,
}

impl Campaign {
    fn execute(&mut self, mode: &str, length: Option<usize>) -> Result<String> {
        self.execute_with_threshold(mode, length, None)
    }

    fn execute_with_threshold(
        &mut self,
        mode: &str,
        length: Option<usize>,
        threshold: Option<usize>,
    ) -> Result<String> {
        self.cases += 1;
        let mut root = self.run.directory.join(format!("case-{}", self.cases));
        if let Some(length) = length {
            let parent = root.join("x".repeat(200));
            let suffix = length
                .checked_sub(parent.as_os_str().len() + 1 + "/database".len())
                .filter(|n| (1..=255).contains(n))
                .ok_or("cannot construct requested pathname length")?;
            fs::create_dir_all(&parent)?;
            root = parent.join("y".repeat(suffix));
            assert_eq!(root.join("database").as_os_str().len(), length);
        }
        let mut command = if mode == "ownership" {
            let mut command = Command::new("/usr/bin/time");
            command.arg(if cfg!(target_os = "macos") {
                "-l"
            } else {
                "-v"
            });
            command.arg(&self.driver);
            command
        } else {
            Command::new(&self.driver)
        };
        command.arg(&root).arg(mode);
        if let Some(threshold) = threshold {
            assert!(cfg!(target_os = "linux") && mode == "reader-allocation-shapes");
            command.env("MALLOC_MMAP_THRESHOLD_", threshold.to_string());
            command.env(
                "GLIBC_TUNABLES",
                format!("glibc.malloc.mmap_threshold={threshold}"),
            );
        }
        let output = self
            .run
            .command(&mut command, None, Duration::from_secs(30));
        // A timeout can leave Darwin's deletion restriction installed. Remove
        // it after the child stops, including when cancellation caused the stop.
        #[cfg(target_os = "macos")]
        if mode.starts_with("catalog-recover-permission") && root.join("database").exists() {
            let mut cleanup = Command::new("/bin/chmod");
            cleanup.arg("-N").arg(root.join("database"));
            self.run
                .cleanup_command(&mut cleanup, Duration::from_secs(5))?
                .require_success()?;
        }
        let output = output?;
        if output.require_success().is_err() {
            eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
        }
        output.require_success()?;
        if mode.starts_with("filesystem-") || mode.starts_with("open-") {
            unchanged(&root)?;
        }
        if mode.starts_with("q1-") || mode.starts_with("q6-") {
            let snapshot = if root.join("fault-active").is_file() {
                "fault"
            } else {
                "before"
            };
            for (index, name) in [
                "CONTROL",
                "ROOT.A",
                "ROOT.B",
                "WAL",
                "units/0000000000000001.unit",
            ]
            .iter()
            .enumerate()
            {
                if fs::read(root.join(format!("{snapshot}-{index}")))?
                    != fs::read(root.join("database").join(name))?
                {
                    return Err(format!("query allocation refusal changed {name}").into());
                }
            }
        }
        Ok(String::from_utf8(output.stdout.bytes)?)
    }

    fn rejected(&mut self, mode: &str, diagnostic: &str) -> Result<()> {
        self.cases += 1;
        let root = self.run.directory.join(format!("case-{}", self.cases));
        let mut command = Command::new(&self.driver);
        command.arg(root).arg(mode).env("RUST_BACKTRACE", "0");
        let output = self
            .run
            .command(&mut command, None, Duration::from_secs(20))?;
        require_rejection(&output, diagnostic)?;
        println!("{mode}: rejected at {diagnostic}");
        Ok(())
    }

    fn concurrent(&mut self) -> Result<()> {
        for length in [None, Some(384)] {
            let output = self.execute("ownership", length)?;
            require_line(
                &output,
                "ownership overlap passed: old/new rows, allocation/memory/temp refusal, cancellation, publication and owner release",
            )?;
            if output
                .lines()
                .filter(|line| line.starts_with("ownership hash-held "))
                .count()
                != 1
            {
                return Err("missing concurrent grouped-reader observation".into());
            }
        }
        for (mode, diagnostic) in [
            ("ownership-worker-panic", "injected ownership worker panic"),
            (
                "ownership-coordinator-panic",
                "injected ownership coordinator panic",
            ),
            ("ownership-negative", "complete-row oracle"),
            (
                "ownership-attribution-negative",
                "prepared ownership attribution",
            ),
        ] {
            self.rejected(mode, diagnostic)?;
        }
        Ok(())
    }

    fn completed(&mut self, mode: &str, marker: &str) -> Result<()> {
        let output = self.execute(mode, None)?;
        require_line(&output, marker)
    }

    fn csv_import(&mut self) -> Result<()> {
        let receipt = self.execute("csv-import-receipt", None)?;
        require_line(&receipt, "CSV import receipt cleanup passed")?;
        require_line(&receipt, "CSV import recovery and complete rows passed")?;
        let healthy = self.execute("csv-import", None)?;
        require_line(&healthy, "CSV import outcome=committed")?;
        require_line(&healthy, "CSV import recovery and complete rows passed")?;
        let count = number(&healthy, "csv-import allocations=")?;
        if !(1..=1000).contains(&count) {
            return Err("invalid CSV import census".into());
        }
        let mut outcomes = std::collections::BTreeSet::new();
        for prefix in std::iter::once(count).chain(0..count) {
            let output = self.execute(&format!("csv-import-{prefix}"), None)?;
            require_line(&output, "CSV import recovery and complete rows passed")?;
            let outcome = record(&output, "CSV import outcome=")?;
            if !matches!(
                outcome,
                "committed" | "refused" | "cleanup" | "recovery" | "ambiguous"
            ) || (outcome == "committed") != (prefix == count)
            {
                return Err(format!("CSV import prefix {prefix}: unexpected {outcome}").into());
            }
            let refusals = number(&output, "csv-import refusals=")?;
            if (refusals > 0) != (prefix < count) {
                return Err("CSV import refusal missing".into());
            }
            outcomes.insert(outcome.to_owned());
        }
        for required in ["refused", "cleanup", "ambiguous"] {
            if !outcomes.contains(required) {
                return Err(format!("CSV import missed {required}").into());
            }
        }
        println!(
            "CSV import: {count} refusal prefixes, full-prefix control, receipt failure and recovery passed"
        );
        Ok(())
    }

    fn prefixes(&mut self, operation: &str, length: Option<usize>) -> Result<()> {
        let observed = self.execute(&format!("{operation}-control"), length)?;
        let mut missing = required_outcomes(operation);
        observe_outcomes(&mut missing, &observed);
        let count = number(&observed, &format!("{operation} allocations="))?;
        if count > if operation == "catalog" { 1100 } else { 128 } {
            return Err("allocation census exceeds bounded sweep".into());
        }
        let healthy = if operation == "filesystem" {
            "returned not-found for unissued attempt".to_owned()
        } else if matches!(
            operation,
            "catalog-recover-corrupt"
                | "catalog-recover-permission"
                | "load-readonly"
                | "q1-corrupt"
                | "q6-corrupt"
        ) {
            format!("returned expected {operation}")
        } else {
            format!("returned healthy {operation}")
        };
        require_line(&observed, &healthy)?;
        // Arming refusal after the full census must still allow completion. Only
        // then do shorter prefixes establish failures within the real operation.
        for prefix in std::iter::once(count).chain(0..count) {
            let output = self.execute(&format!("{operation}-after-{prefix}"), length)?;
            let calls = number(&output, &format!("{operation} allocations="))?;
            let refusals = number(&output, &format!("{operation} refusals="))?;
            if (refusals > 0) != (prefix < count) || (prefix < count && calls <= prefix) {
                return Err(
                    format!("{operation} prefix {prefix}: refusal disagrees with census").into(),
                );
            }
            if prefix == count {
                require_line(&output, &healthy)?;
                if calls != count {
                    return Err("full-prefix allocation count changed".into());
                }
            }
            observe_outcomes(&mut missing, &output);
        }
        if !missing.is_empty() {
            return Err(format!("{operation} sweep missed outcomes: {missing:?}").into());
        }
        println!(
            "{operation}: pathname={length:?}, {count} refused prefixes and full-prefix control passed"
        );
        Ok(())
    }
}

fn require_rejection(output: &crate::process::Output, diagnostic: &str) -> Result<()> {
    // A timeout after the injected panic would mean a peer remained blocked.
    // Require ordinary panic termination, complete capture and the intended check.
    if !matches!(output.completion, crate::process::Completion::Exited(status) if status.code() == Some(101))
        || output.stdout.omitted != 0
        || output.stderr.omitted != 0
        || !std::str::from_utf8(&output.stderr.bytes)?.contains(diagnostic)
    {
        return Err(format!(
            "control did not terminate at {diagnostic}: {:?}",
            output.completion
        )
        .into());
    }
    Ok(())
}

// Track only required outcomes; retaining every child's text would multiply the
// per-process output bound by the number of refusal prefixes. Logs retain details.
fn observe_outcomes(missing: &mut Vec<String>, output: &str) {
    missing.retain(|expected| {
        !output.lines().any(|line| {
            line == expected
                || (expected.ends_with('=')
                    && line.strip_prefix(expected).is_some_and(|tail| {
                        !tail.is_empty() && tail.bytes().all(|byte| byte.is_ascii_digit())
                    }))
        })
    });
}

fn required_outcomes(operation: &str) -> Vec<String> {
    let mut required: Vec<String> = match operation {
        "create-expanded" | "open-expanded" => vec![
            "returned pathname scratch allocation refusal",
            "pathname scratch healed",
        ],
        "catalog" => vec![
            "returned catalog allocation refusal",
            "returned healthy catalog",
            "catalog healed rows=",
            "catalog query same-handle healed rows=4 distinct=3 writer=usable",
            "catalog query retained scan; scratch requires reopen",
        ],
        "load" => vec![
            "returned definite load allocation refusal",
            "returned load resource refusal",
            "returned load admission/issuance recovery debt",
            "returned load cleanup debt",
            "returned ambiguous load with original token",
            "returned healthy load",
        ],
        "load-readonly" => vec![
            "returned crossed permission/allocation cleanup debt",
            "returned expected load-readonly",
        ],
        operation if operation.starts_with("catalog-recover-") => vec![
            "returned catalog recovery allocation refusal",
            "catalog recovery healed generation=",
        ],
        operation if operation.starts_with("q1") || operation.starts_with("q6") => vec![
            "returned typed query allocation refusal",
            "returned query resource refusal",
        ],
        _ => vec![],
    }
    .into_iter()
    .map(str::to_owned)
    .collect();
    if operation == "catalog" {
        for phase in [
            "order-execute",
            "joined-order-execute",
            "repeated-execute",
            "repeated-step",
            "derived-prepare",
            "derived-execute",
            "derived-step",
            "union-prepare",
            "union-execute",
            "union-step",
            "union-distinct-prepare",
            "union-distinct-execute",
            "union-distinct-step",
            "except-prepare",
            "except-execute",
            "except-step",
            "count-only-prepare",
            "count-only-execute",
            "count-only-step",
            "division-prepare",
            "division-execute",
            "division-step",
            "distinct-prepare",
            "distinct-execute",
            "distinct-step",
        ] {
            required.push(format!("catalog phase={phase}"));
        }
    }
    if operation == "catalog-recover-permission" {
        required.push("catalog repair replacement was not promoted to authority".to_owned());
    }
    required
}

fn unchanged(root: &Path) -> Result<()> {
    for name in ["CONTROL", "ROOT.A", "ROOT.B", "WAL"] {
        if fs::read(root.join(name))? != fs::read(root.join("database").join(name))? {
            return Err(format!("allocation refusal changed authoritative {name}").into());
        }
    }
    Ok(())
}

fn record<'a>(output: &'a str, prefix: &str) -> Result<&'a str> {
    let mut records = output.lines().filter_map(|line| line.strip_prefix(prefix));
    let value = records
        .next()
        .ok_or_else(|| format!("missing {prefix} record"))?;
    if records.next().is_some() || value.is_empty() {
        return Err(format!("invalid or duplicate {prefix} record").into());
    }
    Ok(value)
}

fn number(output: &str, prefix: &str) -> Result<usize> {
    let value = record(output, prefix)?;
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("invalid {prefix} number").into());
    }
    Ok(value.parse()?)
}

fn require_line(output: &str, expected: &str) -> Result<()> {
    if output.lines().filter(|line| *line == expected).count() != 1 {
        return Err(format!("missing or duplicate completion: {expected}").into());
    }
    Ok(())
}

pub fn run(case: &str) -> Result<()> {
    let mut run = Run::new(workspace::root()?, "allocation")?;
    let driver = run.build("pipesql-driver", "--bin", "allocation")?;
    let mut campaign = Campaign {
        run,
        driver,
        cases: 0,
    };
    match case {
        "foundation" => {
            campaign.completed("allocation-capacity", "allocation capacity passed: 3 preflight refusals; empty, denied, exact and spare-capacity controls")?;
            campaign.completed("mutex", "native mutex contention passed without Rust allocation")?;
            for length in [None, Some(384)] {
                let output = campaign.execute("declaration-refusal", length)?;
                require_line(&output, "declaration pre-issuance refusal and same-handle retry passed")?;
            }
            for mode in ["control", "deny"] {
                campaign.completed(mode, "returned typed recovery-required outcome")?;
            }
            for mode in ["format-control", "format-deny"] {
                let output = campaign.execute(mode, None)?;
                if output.lines().filter(|line| line.starts_with("returned rendered diagnostic: ")).count() != 1 {
                    return Err("missing fixed-buffer diagnostic".into());
                }
            }
            for operation in ["filesystem", "create", "open"] {
                for length in [None, Some(384)] { campaign.prefixes(operation, length)?; }
            }
            campaign.prefixes("filesystem", Some(383))?;
            #[cfg(target_os = "linux")]
            for operation in ["create-expanded", "open-expanded"] { campaign.prefixes(operation, None)?; }
        }
        "joins" => campaign.joins()?,
        "reports" => campaign.reports()?,
        "shapes" => campaign.shapes()?,
        "concurrent" => campaign.concurrent()?,
        "legacy" => {
            for operation in ["load", "load-readonly", "q1", "q6", "q1-corrupt", "q6-corrupt"] {
                for length in [None, Some(384)] { campaign.prefixes(operation, length)?; }
            }
        }
        "catalog" => {
            for length in [None, Some(384)] { campaign.prefixes("catalog", length)?; }
        }
        "recovery" => {
            for operation in ["catalog-recover-empty", "catalog-recover-data", "catalog-recover-corrupt"] {
                for length in [None, Some(384)] { campaign.prefixes(operation, length)?; }
            }
            #[cfg(target_os = "macos")]
            for length in [None, Some(384)] { campaign.prefixes("catalog-recover-permission", length)?; }
            #[cfg(not(target_os = "macos"))]
            println!("Darwin ACL repair-refusal case is unavailable on this target");
        }
        "import" => campaign.csv_import()?,
        "parquet" => campaign.completed("parquet-ownership", "Parquet ownership passed: receipt failure, import, complete export and three output failures")?,
        "csv" => campaign.completed("csv", "CSV allocation checks passed: four construction allocations, all refusal prefixes, no decoding allocations")?,
        _ => return Err(format!("unknown allocation case: {case}").into()),
    }
    println!(
        "allocation {case} passed: {} processes; logs: {}",
        campaign.cases,
        campaign.run.directory.display()
    );
    campaign.run.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_control_requires_panic_exit_and_complete_diagnostic() {
        use crate::process::{Capture, Completion, Output};
        use std::os::unix::process::ExitStatusExt;
        let make = |completion, diagnostic: &[u8], omitted| Output {
            completion,
            stdout: Capture::default(),
            stderr: Capture {
                bytes: diagnostic.to_vec(),
                omitted,
            },
        };
        let panic = || Completion::Exited(std::process::ExitStatus::from_raw(101 << 8));
        assert!(require_rejection(&make(panic(), b"intended panic", 0), "intended panic").is_ok());
        for completion in [
            Completion::TimedOut,
            Completion::Interrupted(libc::SIGTERM),
            Completion::Exited(std::process::ExitStatus::from_raw(0)),
            Completion::Exited(std::process::ExitStatus::from_raw(libc::SIGKILL)),
        ] {
            assert!(
                require_rejection(&make(completion, b"intended panic", 0), "intended panic")
                    .is_err()
            );
        }
        assert!(
            require_rejection(&make(panic(), b"unrelated panic", 0), "intended panic").is_err()
        );
        assert!(require_rejection(&make(panic(), b"intended panic", 1), "intended panic").is_err());
    }

    #[test]
    fn outcome_tracking_keeps_missing_phases_and_rejects_partial_records() {
        let mut missing = vec![
            "catalog phase=derived-step".to_owned(),
            "catalog healed rows=".to_owned(),
        ];
        observe_outcomes(
            &mut missing,
            "catalog phase=derived-step-extra\ncatalog healed rows=4x\n",
        );
        assert_eq!(missing.len(), 2);
        observe_outcomes(
            &mut missing,
            "catalog healed rows=4\ncatalog healed rows=0\n",
        );
        assert_eq!(missing, ["catalog phase=derived-step"]);
        observe_outcomes(&mut missing, "catalog phase=derived-step\n");
        assert!(missing.is_empty());
    }

    #[test]
    fn census_and_completion_reject_missing_duplicate_and_partial_records() {
        assert_eq!(number("allocations=12\n", "allocations=").unwrap(), 12);
        for output in [
            "",
            "allocations=1\nallocations=2\n",
            "allocations=1x\n",
            "allocations=\n",
            "allocations=+1\n",
        ] {
            assert!(number(output, "allocations=").is_err());
        }
        assert!(require_line("done\n", "done").is_ok());
        for output in ["", "almost done\n", "done\ndone\n"] {
            assert!(require_line(output, "done").is_err());
        }
    }
}
