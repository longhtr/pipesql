//! Provide the local build, Rust-test and documentation checks used by cargo dev.
//!
//! Specialized campaigns live in child modules; these entry points own the common
//! focused selections. Commands run through the process supervisor with recorded
//! output and deadlines. Checks that filter tests also require completion markers,
//! so a renamed test cannot quietly become an empty success. Library and CLI docs
//! use separate targets because their shared crate name would overwrite one index.
//! The full campaign sequence belongs to suite, not the fast local check.

pub mod aggregate;
pub mod allocation;
pub mod analytics;
pub mod catalog;
pub mod cli;
pub mod codec;
pub mod composition;
pub mod identity;
pub mod initialization;
pub mod io;
pub mod model;
pub mod platform;
pub mod recovery;
pub mod sanitizer;
pub mod suite;
pub mod sync;

pub fn fast() -> crate::Result<()> {
    use std::{process::Command, time::Duration};

    println!(
        "Fast selection: repository links, formatting, all-target Clippy, three CLI lifecycle checks"
    );
    crate::tidy::run()?;
    let mut run = crate::workspace::Run::new(crate::workspace::root()?, "check")?;
    for arguments in [
        vec!["fmt", "--all", "--check"],
        vec![
            "clippy",
            "--offline",
            "--locked",
            "--workspace",
            "--all-targets",
            "-j",
            "1",
            "--",
            "-D",
            "warnings",
        ],
    ] {
        let output = run.command(
            Command::new("cargo").args(arguments),
            None,
            Duration::from_secs(300),
        )?;
        if output.require_success().is_err() {
            eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
        }
        output.require_success()?;
    }
    let mut command = Command::new("cargo");
    command.args([
        "test",
        "--offline",
        "--locked",
        "-p",
        "pipesql",
        "--test",
        "lifecycle",
        "-j",
        "1",
        "stock_cli_",
        "--",
        "--test-threads=1",
    ]);
    let output = run.command(&mut command, None, Duration::from_secs(120))?;
    print!("{}", String::from_utf8_lossy(&output.stdout.bytes));
    eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
    output.require_success()?;
    let stdout = std::str::from_utf8(&output.stdout.bytes)?;
    for name in [
        "stock_cli_preserves_usage_exit_when_stderr_is_broken",
        "stock_cli_creates_and_reopens_database",
        "stock_cli_declares_whole_schemas_before_publication",
    ] {
        let expected = format!("test {name} ... ok");
        if !stdout.lines().any(|line| line == expected) {
            return Err(format!("CLI smoke test did not complete: {name}").into());
        }
    }
    run.finish()
}

pub fn documentation() -> crate::Result<()> {
    use std::{fs, path::PathBuf, process::Command, time::Duration};

    let mut run = crate::workspace::Run::new(crate::workspace::root()?, "documentation")?;
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| run.root.join("target"));
    let target = run.root.join(target);
    // Both targets are named pipesql. Keep separate warm outputs so documenting
    // the binary cannot replace the library's HTML index.
    for (selector, directory, introduction) in [
        (
            "--lib",
            "doc-library",
            "An embedded database for analytical queries over local tables.",
        ),
        (
            "--bin",
            "doc-cli",
            "Entry point for one PipeSQL command-line invocation.",
        ),
    ] {
        let destination = target.join(directory);
        let mut command = Command::new("cargo");
        command.args(["doc", "--offline", "--locked", "--no-deps", "-j", "1"]);
        if selector == "--lib" {
            command.args(["--workspace", selector]);
        } else {
            command.args(["-p", "pipesql", selector, "pipesql"]);
        }
        command.arg("--target-dir").arg(&destination);
        command.env("RUSTDOCFLAGS", "-D warnings");
        let output = run.command(&mut command, None, Duration::from_secs(300))?;
        if output.require_success().is_err() {
            eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
        }
        output.require_success()?;
        let index = destination.join("doc/pipesql/index.html");
        if !fs::read_to_string(&index)?.contains(introduction) {
            return Err(format!("missing {selector} introduction in {}", index.display()).into());
        }
    }
    let mut command = Command::new("cargo");
    command.args([
        "test",
        "--offline",
        "--locked",
        "-p",
        "pipesql",
        "-p",
        "pipesql-filesystem",
        "--doc",
        "-j",
        "1",
    ]);
    let output = run.command(&mut command, None, Duration::from_secs(300))?;
    print!("{}", String::from_utf8_lossy(&output.stdout.bytes));
    eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
    output.require_success()?;
    println!(
        "Library and CLI documentation passed; logs: {}",
        run.directory.display()
    );
    run.finish()
}

pub fn process_tests() -> crate::Result<()> {
    let mut run = crate::workspace::Run::new(crate::workspace::root()?, "process-tests")?;
    let mut command = std::process::Command::new("cargo");
    command.args([
        "test",
        "--offline",
        "--locked",
        "-p",
        "pipesql-dev",
        "-j",
        "1",
        "process::tests::",
        "--",
        "--test-threads=1",
    ]);
    let output = run.command(&mut command, None, std::time::Duration::from_secs(120))?;
    print!("{}", String::from_utf8_lossy(&output.stdout.bytes));
    eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
    output.require_success()?;
    let stdout = std::str::from_utf8(&output.stdout.bytes)?;
    // The child entry point and Docker control are deliberately excluded. Require
    // each ordinary control so a renamed module cannot silently select no tests.
    for name in [
        "exit_status_and_bounded_output",
        "timeout_stops_a_child_that_ignores_termination",
        "supervisor_return_unwind_and_interrupt_stop_descendants",
        "nested_supervisor_stops_its_separate_group",
    ] {
        let expected = format!("test process::tests::{name} ... ok");
        if !stdout.lines().any(|line| line == expected) {
            return Err(format!("process control did not complete: {name}").into());
        }
    }
    run.finish()
}

pub fn rust_tests() -> crate::Result<()> {
    let mut run = crate::workspace::Run::new(crate::workspace::root()?, "rust-tests")?;
    let mut command = std::process::Command::new("cargo");
    command.args([
        "test",
        "--offline",
        "--locked",
        "--release",
        "--workspace",
        "--all-targets",
        "--no-fail-fast",
        "-j",
        "1",
        "--",
        "--test-threads=1",
    ]);
    let output = run.command(&mut command, None, std::time::Duration::from_secs(1200))?;
    if output.require_success().is_err() {
        eprint!("{}", String::from_utf8_lossy(&output.stdout.bytes));
        eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
    }
    output.require_success()?;
    println!("Rust suite passed; logs: {}", run.directory.display());
    run.finish()
}
