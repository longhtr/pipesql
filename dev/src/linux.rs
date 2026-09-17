//! Run the Rust checks on a frozen tree with container-native database storage.
//!
//! Docker supplies PID 1 handling. The source mount is read-only; database files
//! live inside the container. Cargo's artifact cache uses a separate host mount.
//! These are Linux regression checks,
//! not the former full-synchronization VM's durability qualification.

use crate::{
    Result, process,
    workspace::{self, Run},
};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const IMAGE: &str = "sha256:520be9ff830f944e49a3319cbf6f8ccfb2c1f21631947de50290efb98038e282";
const DIAGNOSTIC_IMAGE: &str = "pipesql-diagnostic-rust:nightly-2026-09-06-std";

struct Container {
    name: String,
    removed: bool,
}
impl Container {
    fn remove(&mut self) -> Result<()> {
        if self.removed {
            return Ok(());
        }
        let output = process::run_cleanup(
            Command::new("docker").args(["rm", "--force", &self.name]),
            Duration::from_secs(30),
        )?;
        output.require_success()?;
        self.removed = true;
        Ok(())
    }
}
impl Drop for Container {
    fn drop(&mut self) {
        if let Err(error) = self.remove() {
            eprintln!("container cleanup failed: {error}");
        }
    }
}

pub fn run(arguments: &[String]) -> Result<()> {
    let mut arguments = arguments.to_vec();
    let root = workspace::root()?;
    let mut run = Run::new(root, "linux")?;
    let source = run.directory.join("source");
    let identity = workspace::freeze(&run.root, &source)?;
    fs::copy(
        source.join(".pipesql-source.json"),
        run.directory.join("source.json"),
    )?;
    eprintln!("frozen source: {identity}");
    let diagnostic = arguments.first().is_some_and(|value| value == "test")
        && arguments.get(1).is_some_and(|value| value == "sanitizer");
    let image = if diagnostic {
        let mut inspect = Command::new("docker");
        inspect.args(["image", "inspect", "--format", "{{.Id}}", DIAGNOSTIC_IMAGE]);
        let output = run.command(&mut inspect, None, Duration::from_secs(30))?;
        output.require_success()?;
        let identity = std::str::from_utf8(&output.stdout.bytes)?.trim();
        if identity.len() != 71
            || !identity.starts_with("sha256:")
            || !identity[7..].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("diagnostic image did not resolve to a content identity".into());
        }
        identity.to_owned()
    } else {
        IMAGE.to_owned()
    };
    let name = run
        .directory
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let target = run.root.join("target/dev-linux");
    fs::create_dir_all(&target)?;
    let mut container = Container {
        name: name.clone(),
        removed: true,
    };
    let mut create = Command::new("docker");
    create.args([
        "create",
        "--name",
        &name,
        "--init",
        "--network",
        "none",
        "--cpus",
        "1",
        "--memory",
        "2g",
        "--memory-swap",
        "2g",
        "--user",
        "1000:1000",
        "--workdir",
        "/tmp",
        "--env",
        "CARGO_BUILD_JOBS=1",
        "--env",
        "CARGO_TARGET_DIR=/tmp/pipesql-target",
        "--env",
        "PIPESQL_RUN_ROOT=/tmp/pipesql-results",
        "--mount",
    ]);
    create.arg(format!(
        "type=bind,src={},dst=/input,readonly",
        source.display()
    ));
    create.arg("--mount").arg(format!(
        "type=bind,src={},dst=/tmp/pipesql-target",
        target.display()
    ));
    if diagnostic
        && let Some(index) = arguments
            .iter()
            .position(|value| value == "--standard-library-vendor")
    {
        let supplied = arguments
            .get(index + 1)
            .ok_or("standard-library dependencies need a directory")?;
        let supplied = PathBuf::from(supplied).canonicalize()?;
        if !supplied.is_dir() {
            return Err("standard-library dependencies are not a directory".into());
        }
        create.arg("--mount").arg(format!(
            "type=bind,src={},dst=/pipesql-standard-vendor,readonly",
            supplied.display()
        ));
        arguments[index + 1] = "/pipesql-standard-vendor".into();
    }
    create.args([&image, "/bin/sh", "-c", "set -eu\nmkdir /tmp/pipesql-source /tmp/pipesql-results\ncp -R /input/. /tmp/pipesql-source/\ncd /tmp/pipesql-source\ncargo run --offline --locked -p pipesql-dev -- linux-inner \"$@\"", "pipesql-linux"]);
    create.args(&arguments);
    container.removed = false;
    let created = run.command(&mut create, None, Duration::from_secs(30))?;
    created.require_success()?;
    let execution = (|| -> Result<()> {
        let mut start = Command::new("docker");
        start.args(["start", "--attach", &name]);
        let timeout = if arguments == ["test", "all"] {
            crate::check::suite::TIMEOUT + Duration::from_secs(120)
        } else {
            Duration::from_secs(3600)
        };
        let output = run.command(&mut start, None, timeout)?;
        print!("{}", String::from_utf8_lossy(&output.stdout.bytes));
        eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
        if matches!(
            output.completion,
            process::Completion::Interrupted(_) | process::Completion::TimedOut
        ) {
            // Stop writers before copying their diagnostics. Cleanup commands
            // remain available after the supervisor has recorded cancellation.
            let mut stop = Command::new("docker");
            stop.args(["stop", "--time", "5", &name]);
            run.cleanup_command(&mut stop, Duration::from_secs(15))?
                .require_success()?;
        }
        // Copy results even when the command failed; normal container exit alone
        // cannot certify that the selected Rust command completed.
        let mut copy = Command::new("docker");
        copy.args(["cp", &format!("{name}:/tmp/pipesql-results")])
            .arg(run.directory.join("results"));
        run.cleanup_command(&mut copy, Duration::from_secs(30))?
            .require_success()?;
        output.require_success()?;
        let completion = fs::read(run.directory.join("results/complete.json"))
            .map_err(|error| format!("missing Linux completion: {error}"))?;
        let result: serde_json::Value = serde_json::from_slice(&completion)?;
        if result["source"] != identity
            || result["arguments"] != serde_json::json!(arguments)
            || result["success"] != true
        {
            return Err("missing or mismatched Linux completion".into());
        }
        Ok(())
    })();
    let cleanup = container.remove();
    match (execution, cleanup) {
        (Ok(()), Ok(())) => {
            fs::remove_dir_all(&source)?;
            println!("Linux passed; results: {}", run.directory.display());
            Ok(())
        }
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(format!("{error}; cleanup failed: {cleanup}").into()),
    }
}

pub fn completed(arguments: &[String]) -> Result<()> {
    let source: serde_json::Value = serde_json::from_slice(&fs::read(".pipesql-source.json")?)?;
    let destination = PathBuf::from(
        std::env::var_os("PIPESQL_RUN_ROOT").ok_or("missing Linux result directory")?,
    );
    fs::write(
        destination.join("complete.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"source": source["sha256"], "arguments": arguments, "success": true}),
        )?,
    )?;
    Ok(())
}

/// Isolated failure controls run inside the ordinary container command path.
pub fn probe(mode: &str) -> Result<()> {
    let directory =
        PathBuf::from(std::env::var_os("PIPESQL_RUN_ROOT").ok_or("probe outside Linux run")?);
    fs::write(directory.join("probe-entered"), mode)?;
    match mode {
        "success" => Ok(()),
        "failure" => Err("deliberate Linux probe failure".into()),
        "missing" => std::process::exit(0),
        "mismatch" => {
            fs::write(
                directory.join("complete.json"),
                br#"{"source":"wrong","arguments":[],"success":true}"#,
            )?;
            std::process::exit(0)
        }
        "interrupt" => {
            // The outside test waits for probe-entered before interrupting the
            // owner. A finite fallback prevents an abandoned probe running forever.
            std::thread::sleep(Duration::from_secs(60));
            Err("Linux interruption control was not interrupted".into())
        }
        _ => Err("unknown Linux probe".into()),
    }
}
