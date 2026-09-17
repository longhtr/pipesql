//! Check synchronization failures without accepting retries or weaker fallbacks.
//!
//! A healthy native trace supplies call counts for each operation. Fault runs select
//! a call interval and errno, then require matching attempt, refusal and weaker-call
//! counts along with the driver's database outcome. Controls challenge absent or
//! incorrect observer records. A read-only query must make no synchronization calls.
//! These are syscall-level checks; the platform's storage stack still determines
//! whether successful synchronization survives loss of power.

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
    fn cell(&mut self, mode: &str, at: u32, burst: u32, error: i32) -> Result<[u32; 3]> {
        let root = self
            .run
            .directory
            .join(format!("{mode}-{at}-{burst}-{error}"));
        fs::create_dir(&root)?;
        let mut command = Command::new(&self.driver);
        command
            .arg(&root)
            .arg(mode)
            .args([at.to_string(), burst.to_string(), error.to_string()]);
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
        if counts[2] != 0 && !matches!(mode, "weak" | "barrier") {
            return Err("operation used weaker synchronization".into());
        }
        fs::remove_dir_all(root)?;
        self.cells += 1;
        Ok(counts)
    }
}

fn counts(text: &str) -> Result<[u32; 3]> {
    let mut lines = text.lines().filter(|line| line.starts_with("calls="));
    let line = lines.next().ok_or("missing synchronization counts")?;
    if lines.next().is_some() {
        return Err("duplicate synchronization counts".into());
    }
    let mut fields = line.split_whitespace();
    let mut result = [0; 3];
    for (slot, prefix) in result.iter_mut().zip(["calls=", "refused=", "weak="]) {
        let value = fields
            .next()
            .and_then(|field| field.strip_prefix(prefix))
            .ok_or("invalid synchronization count field")?;
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("invalid synchronization count".into());
        }
        *slot = value.parse()?;
    }
    if result[0] > 4096 || result[1] > result[0] {
        return Err("synchronization counts outside observer bounds".into());
    }
    Ok(result)
}

pub fn run(controls_only: bool) -> Result<()> {
    let mut run = Run::new(workspace::root()?, "native-sync")?;
    let driver = run.build("pipesql-driver", "--bin", "sync")?;
    let observer = run.build("pipesql-native", "--lib", "pipesql_native")?;
    let mut campaign = Campaign {
        run,
        driver,
        observer,
        cells: 0,
    };
    if campaign.cell("standard", 1, 3, libc::EINTR)? != [4, 3, 0] {
        return Err("standard synchronization did not retry three intercepted EINTR calls".into());
    }
    for mode in if cfg!(target_os = "macos") {
        &["weak", "barrier"][..]
    } else {
        &["weak"][..]
    } {
        if campaign.cell(mode, 0, 0, libc::EIO)? != [0, 0, 1] {
            return Err("weaker synchronization control missed its call".into());
        }
    }
    for mode in ["file", "directory", "create", "load", "recover"] {
        let healthy = campaign.cell(mode, 0, 0, libc::EIO)?;
        if !(1..=128).contains(&healthy[0]) || healthy[1..] != [0, 0] {
            return Err(format!("invalid synchronization census: {mode} {healthy:?}").into());
        }
        if controls_only {
            continue;
        }
        for at in 1..=healthy[0] {
            for error in [libc::EINTR, libc::EIO, libc::ENOTSUP] {
                for burst in [1, 3] {
                    let observed = campaign.cell(mode, at, burst, error)?;
                    if observed[1] == 0 {
                        return Err("synchronization fault position was not reached".into());
                    }
                }
            }
        }
        println!(
            "native sync {mode}: {} observed positions passed",
            healthy[0]
        );
    }
    if campaign.cell("query", 1, 3, libc::EINTR)? != [0, 0, 0] {
        return Err("read-only query performed synchronization".into());
    }
    if controls_only {
        println!(
            "Native synchronization: {} interception and census controls passed; no full fault-coverage claim",
            campaign.cells
        );
    } else {
        println!(
            "Native synchronization: {} processes passed; single attempts, public failures, resolution/retry and no weaker fallback",
            campaign.cells
        );
    }
    campaign.run.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_preserve_weak_calls_and_reject_invalid_records() {
        assert_eq!(
            counts("calls=4 refused=3 weak=1 result=Ok(())\n").unwrap(),
            [4, 3, 1]
        );
        for text in [
            "",
            "calls=1 refused=2 weak=0",
            "calls=4097 refused=0 weak=0",
            "calls=+1 refused=0 weak=0",
            "calls=1 weak=0 refused=0",
            "calls=1 refused=0 weak=0\ncalls=1 refused=0 weak=0",
        ] {
            assert!(counts(text).is_err());
        }
    }
}
