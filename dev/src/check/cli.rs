//! Supervise CLI allocation and process-behavior checks in fresh child processes.
//!
//! Campaign builds the stock CLI and its allocation-observing counterpart, then
//! passes each selection to its case module. Result readers separate user output
//! from allocation census records and reject missing, ambiguous or truncated data.
//! A timeout cannot stand in for an expected failing exit. Argument capture,
//! database operations and typed transfers keep their own expected behavior while
//! sharing process launch, output decoding and temporary-directory ownership.

use crate::{
    Result,
    process::{Completion, Output},
    workspace::{self, Run},
};
use std::os::unix::process::CommandExt;
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[path = "cli_arguments.rs"]
mod arguments;
#[path = "cli_database.rs"]
mod database;
#[path = "cli_parquet.rs"]
mod parquet;
#[path = "cli_transfer.rs"]
mod transfer;

struct Campaign {
    run: Run,
    stock: PathBuf,
    driver: PathBuf,
}

impl Campaign {
    fn command(&mut self, args: &[OsString], mode: Option<&str>, argv0: Option<&OsStr>) -> Command {
        let mut command = Command::new(if mode.is_some() {
            &self.driver
        } else {
            &self.stock
        });
        command.args(args);
        if let Some(mode) = mode {
            command.env("PIPESQL_CLI_PROBE", mode);
        }
        if let Some(argv0) = argv0 {
            command.arg0(argv0);
        }
        // SAFETY: the post-fork callback makes only synchronous libc calls and
        // constructs an OS error on failure; it allocates nothing and takes no lock.
        unsafe {
            command.pre_exec(|| {
                let core = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                let descriptors = libc::rlimit {
                    rlim_cur: 128,
                    rlim_max: 128,
                };
                if libc::setrlimit(libc::RLIMIT_CORE, &core) != 0
                    || libc::setrlimit(libc::RLIMIT_NOFILE, &descriptors) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command
    }

    fn execute(
        &mut self,
        args: &[OsString],
        mode: Option<&str>,
        argv0: Option<&OsStr>,
    ) -> Result<Output> {
        let mut command = self.command(args, mode, argv0);
        self.run
            .command(&mut command, None, Duration::from_secs(20))
    }
}

fn broken_output(campaign: &mut Campaign, args: &[OsString]) -> Result<()> {
    use std::os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::process::CommandExt,
    };
    let mut descriptors = [-1; 2];
    // SAFETY: pipe initializes exactly two descriptors on success. Each gets one
    // owner immediately; closing the reader ensures writes fail before launch.
    if unsafe { libc::pipe(descriptors.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let (reader, writer) = unsafe {
        (
            OwnedFd::from_raw_fd(descriptors[0]),
            OwnedFd::from_raw_fd(descriptors[1]),
        )
    };
    drop(reader);
    // SAFETY: writer is live. Keep only its stdout duplicate across exec.
    if unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut command = campaign.command(args, None, None);
    // SAFETY: after fork this only duplicates an owned descriptor. The command
    // retains the writer through spawn; normal supervision still owns the child.
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(writer.as_raw_fd(), libc::STDOUT_FILENO) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let result = campaign
        .run
        .command(&mut command, None, std::time::Duration::from_secs(20))?;
    status(&result, 1)?;
    if !std::str::from_utf8(&result.stderr.bytes)?.contains("broken pipe") {
        return Err("broken output pipe lacked its write diagnostic".into());
    }
    Ok(())
}

fn status(output: &Output, code: i32) -> Result<()> {
    if !matches!(output.completion, Completion::Exited(status) if status.code() == Some(code))
        || output.stdout.omitted != 0
        || output.stderr.omitted != 0
    {
        return Err(format!(
            "CLI expected status {code} with complete output: {:?}",
            output.completion
        )
        .into());
    }
    Ok(())
}

fn census_record(output: &Output) -> Result<(&[u8], &str)> {
    let bytes = &output.stdout.bytes;
    let marker = b"cli allocations=";
    let start = bytes
        .windows(marker.len())
        .rposition(|part| part == marker)
        .ok_or("missing CLI census")?;
    Ok((&bytes[..start], std::str::from_utf8(&bytes[start..])?))
}

fn payload(output: &Output) -> Result<&[u8]> {
    census(output)?;
    Ok(census_record(output)?.0)
}

fn census(output: &Output) -> Result<(usize, usize)> {
    let (_, text) = census_record(output)?;
    let line = text.strip_suffix('\n').ok_or("missing CLI census")?;
    let (calls, refused) = line
        .strip_prefix("cli allocations=")
        .and_then(|text| text.split_once(" refusals="))
        .ok_or("invalid CLI census")?;
    if [calls, refused]
        .iter()
        .any(|text| text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err("invalid CLI allocation count".into());
    }
    let calls: usize = calls.parse()?;
    let refused: usize = refused.parse()?;
    if calls > 256 || refused > calls {
        return Err("CLI census outside bounds".into());
    }
    Ok((calls, refused))
}

fn options(database: &Path) -> Vec<OsString> {
    vec![
        "--database".into(),
        database.into(),
        "--memory-limit-bytes".into(),
        "2000000".into(),
        "--temp-limit-bytes".into(),
        "1000000".into(),
    ]
}

pub fn run(case: &str) -> Result<()> {
    let mut run = Run::new(workspace::root()?, "cli-allocation")?;
    let stock = run.build("pipesql", "--bin", "pipesql")?;
    let driver = run.build("pipesql-driver", "--bin", "cli-allocation")?;
    let mut campaign = Campaign { run, stock, driver };
    match case {
        "arguments" => arguments::run(&mut campaign)?,
        "publication" => database::publication(&mut campaign)?,
        "streams" => database::streams(&mut campaign)?,
        "import" => transfer::import(&mut campaign)?,
        "export" => transfer::export(&mut campaign)?,
        "parquet" => parquet::run(&mut campaign)?,
        "database" | "receipts" => database::run(&mut campaign, case)?,
        _ => return Err("unknown CLI allocation case".into()),
    }
    println!(
        "CLI {case} passed; logs: {}",
        campaign.run.directory.display()
    );
    campaign.run.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::Capture;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn census_requires_a_complete_bounded_final_record() {
        let output = |text: &str| Output {
            completion: Completion::Exited(std::process::ExitStatus::from_raw(0)),
            stdout: Capture {
                bytes: text.as_bytes().to_vec(),
                omitted: 0,
            },
            stderr: Capture::default(),
        };
        assert_eq!(
            census(&output("status=opened\ncli allocations=12 refusals=3\n")).unwrap(),
            (12, 3)
        );
        let mut binary = output("");
        binary.stdout.bytes = b"PAR1\xff\0cli allocations=7 refusals=0\n".to_vec();
        assert_eq!(census(&binary).unwrap(), (7, 0));
        assert_eq!(payload(&binary).unwrap(), b"PAR1\xff\0");
        for text in [
            "",
            "cli allocations=1 refusals=0",
            "cli allocations=1 refusals=0\nextra\n",
            "cli allocations=257 refusals=0\n",
            "cli allocations=1 refusals=2\n",
            "cli allocations=+1 refusals=0\n",
        ] {
            assert!(census(&output(text)).is_err(), "accepted {text}");
        }
        let mut truncated = output("cli allocations=1 refusals=0\n");
        truncated.stdout.omitted = 1;
        assert!(status(&truncated, 0).is_err());
        let mut timed_out = output("cli allocations=1 refusals=0\n");
        timed_out.completion = Completion::TimedOut;
        assert!(status(&timed_out, 0).is_err());
    }
}
