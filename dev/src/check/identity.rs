//! Supervise the GNU/Linux witness for pathname and open-file identity.
//!
//! A caller-supplied new directory tests its actual mount and remains available for
//! inspection. The campaign selection instead creates controlled paths and requires
//! the witness's complete observations, including deliberate replacement and failed
//! observation cases. Timeouts, truncated output or a missing diagnostic fail the
//! check. Matching this bounded run does not explain an intermittent mismatch on
//! another filesystem or establish storage synchronization strength.

use crate::{
    Result,
    process::Completion,
    workspace::{self, Run},
};
use std::{path::Path, process::Command, time::Duration};

pub fn run(path: Option<&Path>) -> Result<()> {
    if !cfg!(target_os = "linux") {
        return Err(
            "identity witness requires GNU/Linux; use cargo dev linux test identity for controls"
                .into(),
        );
    }
    if path.is_some_and(|path| !path.is_absolute() || path.exists()) {
        return Err("identity witness needs a new absolute directory".into());
    }
    let mut run = Run::new(workspace::root()?, "identity")?;
    let driver = run.build("pipesql-driver", "--bin", "identity")?;
    if let Some(path) = path {
        // The user chooses the filesystem and owns this path even on failure.
        let output = run.command(
            Command::new(&driver).arg(path).arg("healthy"),
            None,
            Duration::from_secs(60),
        )?;
        print!("{}", String::from_utf8_lossy(&output.stdout.bytes));
        eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
        output.require_success()?;
        if output.stdout.bytes != b"400 file replacements checked\n" {
            return Err("identity witness did not complete all replacements".into());
        }
    } else {
        for (mode, code) in [("healthy", 0), ("replace", 1), ("missing", 2)] {
            let path = run.directory.join(mode);
            let output = run.command(
                Command::new(&driver).arg(&path).arg(mode),
                None,
                Duration::from_secs(60),
            )?;
            if output.stdout.omitted != 0 || output.stderr.omitted != 0 {
                return Err("identity control output was truncated".into());
            }
            if !matches!(output.completion, Completion::Exited(status) if status.code() == Some(code))
            {
                return Err(format!(
                    "identity {mode}: expected exit {code}, got {:?}; stderr: {}",
                    output.completion,
                    String::from_utf8_lossy(&output.stderr.bytes)
                )
                .into());
            }
            let report = std::str::from_utf8(&output.stdout.bytes)?;
            let correct = match mode {
                "healthy" => report == "400 file replacements checked\n",
                "replace" => {
                    report.lines().count() == 1
                        && report.starts_with("mismatch iteration=1 file=ROOT.A ")
                        && [
                            " before=",
                            " path_statx=",
                            " fstat=",
                            " statx=",
                            " after=",
                            " after_statx=",
                            " content=1\n",
                        ]
                        .iter()
                        .all(|field| report.contains(field))
                }
                "missing" => {
                    report.is_empty()
                        && std::str::from_utf8(&output.stderr.bytes)?
                            .contains("identity observation failed:")
                }
                _ => unreachable!(),
            };
            if !correct {
                return Err(format!("identity {mode}: missing or invalid observation").into());
            }
        }
        println!(
            "identity controls passed: 400 replacements, detected replacement, failed observation"
        );
    }
    run.finish()
}
