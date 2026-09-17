//! Sweep native read and write failures at positions observed in healthy runs.
//!
//! The selected base, repeated or derived workload first supplies per-kind call
//! counts. Fresh runs then inject short transfers and errors, including errors after
//! partial progress. Counters must prove that each selected kind made positive
//! partial progress where required. The supervisor owns the driver and preloaded
//! observer, validates complete records and rejects missing instrumentation or a
//! fault that never reached the intended operation.

use crate::{
    Result,
    workspace::{self, Run},
};
use std::{fs, path::PathBuf, process::Command, time::Duration};

struct Campaign {
    run: Run,
    driver: PathBuf,
    observer: PathBuf,
    cells: usize,
}
impl Campaign {
    fn cell(&mut self, mode: &str, kind: u32, at: u32, burst: u32, error: i32) -> Result<[u32; 3]> {
        let root = self
            .run
            .directory
            .join(format!("{mode}-{kind}-{at}-{burst}-{error}"));
        fs::create_dir(&root)?;
        let mut command = Command::new(&self.driver);
        command.arg(&root).arg(mode).args([
            kind.to_string(),
            at.to_string(),
            burst.to_string(),
            error.to_string(),
        ]);
        command
            .env_remove("LD_PRELOAD")
            .env_remove("DYLD_INSERT_LIBRARIES");
        command.env(
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
        let counts = counts(std::str::from_utf8(&output.stdout.bytes)?)?;
        fs::remove_dir_all(root)?;
        self.cells += 1;
        Ok(counts)
    }
}

fn counts(text: &str) -> Result<[u32; 3]> {
    let mut lines = text.lines().filter(|line| line.starts_with("calls="));
    let line = lines.next().ok_or("missing I/O counts")?;
    if lines.next().is_some() {
        return Err("duplicate I/O counts".into());
    }
    let mut fields = line.split_whitespace();
    let mut result = [0; 3];
    for (slot, prefix) in result.iter_mut().zip(["calls=", "refused=", "partial="]) {
        let value = fields
            .next()
            .and_then(|field| field.strip_prefix(prefix))
            .ok_or("invalid I/O count field")?;
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("invalid I/O count".into());
        }
        *slot = value.parse()?;
    }
    if result[0] > 4096 || result[1] > result[0] || result[2] > result[0] {
        return Err("I/O counts outside observer bounds".into());
    }
    Ok(result)
}

pub fn run(case: &str) -> Result<()> {
    let workloads: &[(&str, u32)] = match case {
        "controls" | "base" => &[
            ("create", 3),
            ("load", 15),
            ("recover", 7),
            ("q6", 5),
            ("q1", 5),
        ],
        "repeated" => &[("repeated", 12)],
        "derived" => &[("derived", 12)],
        _ => return Err("unknown native I/O case".into()),
    };
    let mut run = Run::new(workspace::root()?, "native-io")?;
    let driver = run.build("pipesql-driver", "--bin", "io")?;
    let observer = run.build("pipesql-native", "--lib", "pipesql_native")?;
    let mut campaign = Campaign {
        run,
        driver,
        observer,
        cells: 0,
    };
    for kind in 0..4 {
        if campaign.cell("standard", kind, 1, 3, libc::EINTR)? != [4, 3, 0] {
            return Err("standard I/O did not retry three intercepted EINTR calls".into());
        }
    }
    let mut partial_mask = 0;
    for &(mode, mask) in workloads {
        for kind in 0..4 {
            let healthy = campaign.cell(mode, kind, 0, 0, libc::EIO)?;
            if healthy[0] > 256
                || healthy[1..] != [0, 0]
                || (healthy[0] > 0) != (mask & (1 << kind) != 0)
            {
                return Err(format!(
                    "native I/O census missed expected calls: {mode} {kind} {healthy:?}"
                )
                .into());
            }
            println!("native I/O {mode} kind={kind} calls={}", healthy[0]);
            if case == "controls" {
                continue;
            }
            for at in 1..=healthy[0] {
                for error in [libc::EINTR, libc::EIO, 0] {
                    for burst in [1, 3] {
                        let observed = campaign.cell(mode, kind, at, burst, error)?;
                        if error != 0 && observed[1] == 0 {
                            return Err("I/O fault position was not reached".into());
                        }
                        if error == 0 && observed[2] > 0 {
                            partial_mask |= 1 << kind;
                        }
                    }
                }
            }
            if healthy[0] > 0 {
                for error in [-libc::EINTR, -libc::EIO] {
                    for burst in [1, 3] {
                        let crossed = campaign.cell(mode, kind, 1, burst, error)?;
                        if crossed[1] == 0 || crossed[2] == 0 {
                            return Err("I/O did not cross from partial progress to failure".into());
                        }
                    }
                }
            }
        }
    }
    if case != "controls" && partial_mask != if case == "base" { 15 } else { 12 } {
        return Err("not every selected I/O kind made positive partial progress".into());
    }
    println!(
        "Native I/O {case}: {} processes passed{}",
        campaign.cells,
        if case == "controls" {
            "; census only, no failure-coverage claim"
        } else {
            "; every observed position, short transfers and errors after partial progress"
        }
    );
    campaign.run.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_reject_missing_duplicate_and_impossible_observation() {
        assert_eq!(
            counts("calls=4 refused=3 partial=0 result=Ok(())\n").unwrap(),
            [4, 3, 0]
        );
        for text in [
            "",
            "calls=1 refused=2 partial=0",
            "calls=4097 refused=0 partial=0",
            "calls=+1 refused=0 partial=0",
            "calls=1 partial=0 refused=0",
            "calls=1 refused=0 partial=0\ncalls=1 refused=0 partial=0",
        ] {
            assert!(counts(text).is_err());
        }
    }
}
