//! Run the maintained campaigns sequentially against one frozen source tree.
//!
//! Each campaign keeps its own expected answers and process deadlines. This owner
//! chooses their order, retains their logs and checks that tested inputs did not
//! change. Nightly instrumentation and Docker-owner controls have separate commands.
//!
//! The outer runner requires a source-matched completion record from the inner
//! sequence as well as a successful exit. Fresh small and above-memory analytical
//! workflows run last. A failed campaign stops the sequence and retains its inputs;
//! only complete success permits removal of the frozen source copy.

use crate::{Result, workspace};
use std::{fs, path::PathBuf, process::Command, time::Duration};

pub const TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);

pub fn run() -> Result<()> {
    let mut run = workspace::Run::new(workspace::root()?, "suite")?;
    let source = run.directory.join("source");
    let identity = workspace::freeze(&run.root, &source)?;
    fs::copy(
        source.join(".pipesql-source.json"),
        run.directory.join("source.json"),
    )?;
    eprintln!("frozen source: {identity}");
    let before = workspace::tree_contents(&source)?;
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| run.root.join("target"));
    let target = run.root.join(target).join("suite");
    let results = run.directory.join("results");
    fs::create_dir(&results)?;
    let mut command = Command::new("cargo");
    command.args([
        "run",
        "--offline",
        "--locked",
        "-p",
        "pipesql-dev",
        "-j",
        "1",
    ]);
    // Workspace tests also build pipesql-dev. Keep the running executable in a
    // separate target: Linux current_exe otherwise names an unlinked binary, and
    // a later campaign cannot launch this runner as its catalog inspector.
    command
        .arg("--target-dir")
        .arg(target.with_file_name("suite-runner"))
        .args(["--", "suite-inner"]);
    command
        .env("CARGO_TARGET_DIR", target)
        .env("PIPESQL_RUN_ROOT", &results);
    run.root = source.clone();
    let output = run.command(&mut command, None, TIMEOUT)?;
    print!("{}", String::from_utf8_lossy(&output.stdout.bytes));
    eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
    output.require_success()?;
    let complete: serde_json::Value =
        serde_json::from_slice(&fs::read(results.join("suite.json"))?)?;
    if complete != serde_json::json!({"source": identity, "success": true}) {
        return Err("missing or mismatched suite completion".into());
    }
    if workspace::tree_contents(&source)? != before {
        return Err("suite changed its frozen source".into());
    }
    fs::remove_dir_all(source)?;
    fs::write(
        run.directory.join("complete.json"),
        serde_json::to_vec_pretty(&complete)?,
    )?;
    println!(
        "Maintained suite passed; results: {}",
        run.directory.display()
    );
    // Keep the nested campaign logs; Run::finish would remove their directories.
    Ok(())
}

pub fn inner() -> Result<()> {
    crate::tidy::run()?;
    let root = workspace::root()?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join(".pipesql-source.json"))?)?;
    let results = PathBuf::from(
        std::env::var_os("PIPESQL_RUN_ROOT").ok_or("missing suite result directory")?,
    );
    let mut run = workspace::Run::new(root, "suite-build")?;
    for arguments in [
        vec!["fmt", "--all", "--check"],
        vec![
            "clippy",
            "--offline",
            "--locked",
            "--release",
            "--workspace",
            "--all-targets",
            "-j",
            "1",
            "--",
            "-D",
            "warnings",
        ],
    ] {
        eprintln!("suite: cargo {}", arguments.join(" "));
        let output = run.command(
            Command::new("cargo").args(arguments),
            None,
            Duration::from_secs(600),
        )?;
        if output.require_success().is_err() {
            eprint!("{}", String::from_utf8_lossy(&output.stderr.bytes));
        }
        output.require_success()?;
    }
    run.finish()?;
    super::rust_tests()?;
    super::documentation()?;
    super::platform::run()?;
    if cfg!(target_os = "linux") {
        super::identity::run(None)?;
    }
    super::model::run()?;
    for case in ["catalog", "snapshot", "rejected", "rounding"] {
        eprintln!("suite: cargo dev test codec --case {case}");
        super::codec::run(case)?;
    }
    super::aggregate::run()?;
    for case in [
        "grouping",
        "expressions",
        "dates",
        "numeric",
        "derived",
        "filters",
        "columns",
        "repeated",
        "demand",
        "storage",
        "unions",
        "report",
    ] {
        eprintln!("suite: cargo dev test composition --case {case}");
        super::composition::run(case)?;
    }
    super::catalog::run()?;
    for case in [
        "foundation",
        "csv",
        "import",
        "parquet",
        "catalog",
        "recovery",
        "legacy",
        "concurrent",
        "shapes",
        "reports",
        "joins",
    ] {
        eprintln!("suite: cargo dev test allocation --case {case}");
        super::allocation::run(case)?;
    }
    for case in [
        "arguments",
        "database",
        "receipts",
        "streams",
        "publication",
        "import",
        "export",
        "parquet",
    ] {
        eprintln!("suite: cargo dev test cli --case {case}");
        super::cli::run(case)?;
    }
    super::initialization::run()?;
    super::sync::run(false)?;
    for case in ["base", "repeated", "derived"] {
        eprintln!("suite: cargo dev test io --case {case}");
        super::io::run(case)?;
    }
    // All-events includes the focused append-boundaries selection.
    for case in ["all-events", "report", "images", "report-images"] {
        eprintln!("suite: cargo dev test recovery --case {case}");
        super::recovery::run(case)?;
    }
    // Workflows get fresh databases after the lower-level campaigns finish.
    for case in ["small", "scaled"] {
        eprintln!("suite: cargo dev test analytics --case {case}");
        super::analytics::run(case)?;
    }
    fs::write(
        results.join("suite.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"source": manifest["sha256"], "success": true}),
        )?,
    )?;
    Ok(())
}
