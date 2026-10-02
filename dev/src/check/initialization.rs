//! Supervise native pathname failures and overlapping database creators.
//!
//! Fresh paths and symlink spellings exercise each selected lookup site and error.
//! The preloaded observer pauses one creator while the other reaches its intended
//! operation; driver completion confirms that the schedule actually occurred.
//! A coordinator-panic control must release and join the waiting creator promptly.
//! Running without the observer must also fail, preventing uninstrumented success
//! from being mistaken for coverage of the requested fault.

use crate::{
    Result,
    process::Completion,
    workspace::{self, Run},
};
use std::{
    fs,
    os::unix::{fs::symlink, process::CommandExt},
    process::Command,
    time::Duration,
};

pub fn run() -> Result<()> {
    let mut run = Run::new(workspace::root()?, "initialization")?;
    let driver = run.build("pipesql-driver", "--bin", "initialization")?;
    let observer = run.build("pipesql-native", "--lib", "pipesql_native")?;
    let spellings = if cfg!(target_os = "macos") {
        ["private", "data-mount", "symlink33"]
    } else {
        ["private", "symlink33", "suffix40"]
    };
    let mut count = 0;
    for spelling in spellings {
        let sites: &[&str] = if cfg!(target_os = "macos") {
            &["root"]
        } else if spelling == "private" {
            &["root", "component"]
        } else {
            &["root", "component", "symlink"]
        };
        for site in sites {
            for mode in 1..=4 {
                let errors: &[i32] = if mode == 1 || mode == 3 {
                    &[libc::EIO]
                } else {
                    &[libc::EINTR, libc::EIO, libc::ENOMEM, libc::EACCES]
                };
                for error in errors {
                    let cell = run
                        .directory
                        .join(format!("{spelling}-{site}-{mode}-{error}"));
                    fs::create_dir(&cell)?;
                    let cell = cell.canonicalize()?;
                    let expected = if cfg!(target_os = "macos") && spelling != "private" {
                        std::path::PathBuf::from(format!("/System/Volumes/Data{}", cell.display()))
                    } else {
                        cell.clone()
                    };
                    let mut requested = expected.clone();
                    if spelling == "symlink33" || spelling == "suffix40" {
                        let links = if spelling == "suffix40" { 40 } else { 33 };
                        for index in 0..links {
                            let mut target = if index + 1 < links {
                                format!("s{}", index + 1)
                            } else {
                                ".".into()
                            };
                            if spelling == "suffix40" {
                                target.push_str(&"/.".repeat(1900));
                            }
                            symlink(target, cell.join(format!("s{index}")))?;
                        }
                        requested.push("s0");
                    }
                    let mut command = Command::new(&driver);
                    command
                        .arg(&requested)
                        .arg(&expected)
                        .arg(mode.to_string())
                        .arg(error.to_string())
                        .arg(site);
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
                    // SAFETY: setrlimit is synchronous and allocates nothing after fork.
                    unsafe {
                        command.pre_exec(|| {
                            let limit = libc::rlimit {
                                rlim_cur: 0,
                                rlim_max: 0,
                            };
                            if libc::setrlimit(libc::RLIMIT_CORE, &limit) != 0 {
                                return Err(std::io::Error::last_os_error());
                            }
                            Ok(())
                        });
                    }
                    let output = run.command(&mut command, None, Duration::from_secs(30))?;
                    output.require_success()?;
                    let text = std::str::from_utf8(&output.stdout.bytes)?;
                    if !text.lines().last().is_some_and(|line| {
                        line.starts_with(&format!(
                            "mode={mode} error={error} resolutions=1,2 mounts="
                        )) && line.ends_with(" outcomes=checked")
                    }) {
                        return Err(format!("initialization case lacked completion: {spelling} {site} {mode} {error}").into());
                    }
                    count += 1;
                }
            }
        }
    }
    let panic_root = run.directory.join("coordinator-panic");
    fs::create_dir(&panic_root)?;
    let mut panic = Command::new(&driver);
    panic
        .arg(&panic_root)
        .arg(&panic_root)
        .args(["3", &libc::EIO.to_string(), "root"]);
    panic
        .env_remove("LD_PRELOAD")
        .env_remove("DYLD_INSERT_LIBRARIES");
    panic.env(
        if cfg!(target_os = "macos") {
            "DYLD_INSERT_LIBRARIES"
        } else {
            "LD_PRELOAD"
        },
        &observer,
    );
    panic
        .env("PIPESQL_INIT_PANIC", "1")
        .env("RUST_BACKTRACE", "0");
    let output = run.command(&mut panic, None, Duration::from_secs(5))?;
    let errors = std::str::from_utf8(&output.stderr.bytes)?;
    if !matches!(output.completion, Completion::Exited(status) if status.code() == Some(101))
        || output.stdout.omitted != 0
        || output.stderr.omitted != 0
        || !errors.contains("deliberate initialization coordinator panic")
        || !errors.contains("initialization panic: creator released and joined")
    {
        return Err("initialization panic did not release and join creator promptly".into());
    }
    // Missing instrumentation must fail before it can create any database.
    let mut missing = Command::new(&driver);
    missing
        .env_remove("LD_PRELOAD")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env("RUST_BACKTRACE", "0");
    let output = run.command(&mut missing, None, Duration::from_secs(10))?;
    if !matches!(output.completion, Completion::Exited(status) if status.code() == Some(101))
        || output.stdout.omitted != 0
        || output.stderr.omitted != 0
        || !std::str::from_utf8(&output.stderr.bytes)?
            .contains("native initialization observer missing")
    {
        return Err("missing initialization observer control failed".into());
    }
    println!(
        "Native initialization: {count} fresh processes checked exact errors, overlap, resolved paths and unchanged reopened bytes; missing observer rejected"
    );
    run.finish()
}
