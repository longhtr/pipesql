//! Supervise native ABI, mutex, stack and observer-mode checks.
//!
//! The platform driver compares installed C headers with Rust bindings and must
//! emit its completion marker. An oversized-thread control must fail the stack
//! ceiling. Each ordered pair of observer modes then runs in fresh processes:
//! overlap must fail, while stopping the first must permit the second. Exact status
//! and markers distinguish the intended rejection from a missing observer or an
//! unrelated crash. These checks cover platform premises, not engine stack peaks.

use crate::{
    Result,
    workspace::{self, Run},
};
use std::{process::Command, time::Duration};

pub fn run() -> Result<()> {
    let mut run = Run::new(workspace::root()?, "platform")?;
    let driver = run.build("pipesql-driver", "--bin", "platform")?;
    let output = run.command(&mut Command::new(&driver), None, Duration::from_secs(20))?;
    print!("{}", String::from_utf8_lossy(&output.stdout.bytes));
    eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
    output.require_success()?;
    if !std::str::from_utf8(&output.stdout.bytes)?
        .lines()
        .any(|line| line == "platform checks complete")
    {
        return Err("native platform checks did not complete".into());
    }
    let observer = run.build("pipesql-native", "--lib", "pipesql_native")?;
    let modes = [
        "interruption",
        "publication",
        "initialization",
        "io",
        "sync",
    ];
    for first in modes {
        for second in modes {
            for release in [false, true] {
                let mut command = Command::new(&driver);
                command.args([
                    "observer-modes",
                    first,
                    second,
                    if release { "release" } else { "overlap" },
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
                    &observer,
                );
                let output = run.command(&mut command, None, Duration::from_secs(10))?;
                let expected = if release { 0 } else { 90 };
                if !matches!(output.completion, crate::process::Completion::Exited(status) if status.code() == Some(expected))
                    || output.stdout.omitted != 0
                    || output.stderr.omitted != 0
                    || output.stdout.bytes
                        != if release {
                            b"observer modes entered\nobserver modes released\n".as_slice()
                        } else {
                            b"observer modes entered\n".as_slice()
                        }
                {
                    return Err(format!(
                        "observer mode control failed: {first}, {second}, release={release}"
                    )
                    .into());
                }
            }
        }
    }
    println!(
        "platform checks passed: 25 mode overlaps rejected and 25 released transitions accepted; logs: {}",
        run.directory.display()
    );
    run.finish()
}
