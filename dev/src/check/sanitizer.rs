//! Establish sanitizer detection before relying on instrumented engine tests.
//!
//! A clean control must finish, and a deliberate fault must produce the selected
//! sanitizer's report and exit status. Test selection separately requires every
//! named case to be discovered and completed; an empty selection cannot pass.
//!
//! Runs compare the pinned compiler with plain nightly and the instrumented build
//! on frozen inputs. Native-boundary selections and broader engine selections remain
//! distinct; the latter can rebuild the standard library from supplied dependencies.
//! A passing selected run does not establish instrumentation of every runtime library
//! or coverage of all engine paths.

use crate::{
    Result,
    process::{Completion, Output},
    workspace::{self, Run},
};
use std::{fs, process::Command, time::Duration};

const ADDRESS_OPTIONS: &str = "halt_on_error=1:abort_on_error=0:exitcode=86:detect_leaks=1";
const THREAD_OPTIONS: &str = "halt_on_error=1:abort_on_error=0:exitcode=86";

fn environment(command: &mut Command) {
    for key in [
        "RUSTC",
        "RUSTDOC",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "RUSTC_BOOTSTRAP",
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTFLAGS",
        "RUSTDOCFLAGS",
        "ASAN_OPTIONS",
        "LSAN_OPTIONS",
        "TSAN_OPTIONS",
        "LD_PRELOAD",
        "DYLD_INSERT_LIBRARIES",
        "LD_LIBRARY_PATH",
        "DYLD_LIBRARY_PATH",
    ] {
        command.env_remove(key);
    }
    command
        .env("ASAN_OPTIONS", ADDRESS_OPTIONS)
        .env("TSAN_OPTIONS", THREAD_OPTIONS);
}

fn control(output: &Output, fault: bool, thread: bool) -> Result<()> {
    if output.stdout.omitted != 0 || output.stderr.omitted != 0 {
        return Err("sanitizer control output was truncated".into());
    }
    let code = if fault { 86 } else { 0 };
    if !matches!(output.completion, Completion::Exited(status) if status.code() == Some(code)) {
        return Err("sanitizer control exit does not establish detection".into());
    }
    let text = std::str::from_utf8(&output.stdout.bytes)?;
    let error = std::str::from_utf8(&output.stderr.bytes)?;
    let kind = if thread { "thread" } else { "address" };
    if fault {
        let diagnostic = if thread {
            "WARNING: ThreadSanitizer: data race"
        } else {
            "ERROR: AddressSanitizer: heap-buffer-overflow"
        };
        if !error.contains(diagnostic) {
            return Err("deliberate fault did not produce its sanitizer report".into());
        }
    } else if text.trim() != format!("{kind} control clean") || error.contains("Sanitizer") {
        return Err("clean sanitizer control failed".into());
    }
    Ok(())
}

fn selection(text: &str, required: &[&str], completed: bool) -> Result<()> {
    let expected: std::collections::BTreeSet<_> = required.iter().copied().collect();
    if expected.is_empty() || expected.len() != required.len() {
        return Err("sanitizer selection is empty or duplicated".into());
    }
    let found: Vec<_> = text
        .lines()
        .filter_map(|line| {
            if completed {
                line.strip_prefix("test ")?.strip_suffix(" ... ok")
            } else {
                line.strip_suffix(": test")
            }
        })
        .collect();
    if found.len() != expected.len()
        || found.into_iter().collect::<std::collections::BTreeSet<_>>() != expected
    {
        return Err("sanitizer test selection differs from required scope".into());
    }
    if completed
        && !text.lines().any(|line| {
            line.starts_with(&format!(
                "test result: ok. {} passed; 0 failed; 0 ignored;",
                required.len()
            ))
        })
    {
        return Err("sanitizer test summary does not establish completion".into());
    }
    Ok(())
}

#[path = "sanitizer_boundary.rs"]
mod boundary;
#[path = "sanitizer_cases.rs"]
mod cases;
#[path = "sanitizer_standard.rs"]
mod standard;
#[path = "sanitizer_vendor.rs"]
mod vendor;

fn libraries(run: &mut Run, selector: &str, diagnostic: bool, thread: bool) -> Result<()> {
    let mut command = Command::new("rustc");
    command.args([&format!("+{selector}"), "--print", "target-libdir"]);
    environment(&mut command);
    let output = run.command(&mut command, None, Duration::from_secs(30))?;
    output.require_success()?;
    let directory = std::path::Path::new(std::str::from_utf8(&output.stdout.bytes)?.trim());
    let files: Vec<_> = fs::read_dir(directory)?.collect::<std::io::Result<_>>()?;
    let mut records = Vec::new();
    for (kind, required) in [
        ("standard-library", true),
        (
            if thread {
                "thread-runtime"
            } else {
                "address-runtime"
            },
            diagnostic,
        ),
    ] {
        if !required {
            continue;
        }
        let matches: Vec<_> = files
            .iter()
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if kind == "standard-library" {
                    name.starts_with("libstd-") && name.ends_with(".rlib")
                } else {
                    name.contains(if thread { "rt.tsan." } else { "rt.asan." })
                }
            })
            .collect();
        if matches.len() != 1 {
            return Err(format!("expected one {kind} for {selector}").into());
        }
        let path = matches[0].path();
        records.push(
            serde_json::json!({"kind": kind, "path": path, "sha256": workspace::hash(&path)?}),
        );
    }
    fs::write(
        run.directory.join(if diagnostic {
            "diagnostic-libraries.json"
        } else {
            "stock-libraries.json"
        }),
        serde_json::to_vec_pretty(
            &serde_json::json!({"selector": selector, "standard_library": "prebuilt", "artifacts": records}),
        )?,
    )?;
    Ok(())
}

pub fn run(case: &str, toolchain: &str, supplied: Option<&std::path::Path>) -> Result<()> {
    if !["controls", "mutex", "pathname", "memory", "concurrency"].contains(&case) {
        return Err("unknown sanitizer case".into());
    }
    if toolchain.is_empty() || toolchain.starts_with('-') || toolchain.contains('/') {
        return Err("expected an installed nightly selector".into());
    }
    if supplied.is_some_and(|path| !path.is_absolute()) {
        return Err("standard-library dependencies need an absolute directory".into());
    }
    if matches!(case, "memory" | "concurrency") && supplied.is_none() {
        return Err("memory and concurrency scopes require standard-library dependencies".into());
    }
    let mut run = Run::new(workspace::root()?, &format!("sanitizer-{case}"))?;
    let source = run.directory.join("source");
    let identity = workspace::freeze(&run.root, &source)?;
    fs::write(run.directory.join("source.sha256"), &identity)?;
    println!("frozen source: {identity}");
    run.root = source;
    let mut native = Command::new("cc");
    native.arg("--version");
    environment(&mut native);
    run.command(&mut native, None, Duration::from_secs(30))?
        .require_success()?;
    let mut info = Command::new("rustc");
    info.args([&format!("+{toolchain}"), "-vV"]);
    environment(&mut info);
    let output = run.command(&mut info, None, Duration::from_secs(30))?;
    output.require_success()?;
    let compiler = std::str::from_utf8(&output.stdout.bytes)?;
    if !compiler
        .lines()
        .any(|line| line.starts_with("release: ") && line.contains("nightly"))
    {
        return Err("sanitizer compiler is not nightly".into());
    }
    let host = compiler
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or("compiler did not identify its host")?;
    let thread = case == "concurrency";
    libraries(&mut run, toolchain, true, thread)?;
    let standard = supplied
        .map(|path| standard::StandardLibrary::prepare(&mut run, toolchain, path))
        .transpose()?;
    let build = run.directory.join("build");
    fs::create_dir(&build)?;
    let executable = build.join("address-control");
    let mut command = Command::new("rustc");
    command
        .args([
            &format!("+{toolchain}"),
            "--edition=2024",
            "--target",
            host,
            "-Zsanitizer=address",
            "-Copt-level=1",
            "-g",
            "-Dwarnings",
        ])
        .arg(run.root.join("dev/driver/src/address_control.rs"))
        .arg("-o")
        .arg(&executable);
    environment(&mut command);
    let executable = if let Some(standard) = &standard {
        standard.control(
            &mut run,
            toolchain,
            host,
            &build,
            if thread { "thread" } else { "address" },
        )?
    } else {
        run.command(&mut command, None, Duration::from_secs(90))?
            .require_success()?;
        executable
    };
    fs::write(
        run.directory.join("control.sha256"),
        workspace::hash(&executable)?,
    )?;
    for fault in [false, true] {
        let mut command = Command::new(&executable);
        command.arg(if fault { "fault" } else { "clean" });
        environment(&mut command);
        let output = run.command(&mut command, None, Duration::from_secs(20))?;
        control(&output, fault, thread)?;
    }
    println!(
        "{} clean and fault controls passed",
        if thread {
            "ThreadSanitizer"
        } else {
            "AddressSanitizer"
        }
    );
    if case != "controls" {
        boundary::run(&mut run, toolchain, host, case, standard.as_ref())?;
    }
    if let Some(standard) = &standard {
        standard.unchanged()?;
    }
    run.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::Capture;
    use std::os::unix::process::ExitStatusExt;
    #[test]
    fn controls_require_the_fault_report_and_exact_exit() {
        let mut output = Output {
            completion: Completion::Exited(std::process::ExitStatus::from_raw(86 << 8)),
            stdout: Capture::default(),
            stderr: Capture {
                bytes: b"ERROR: AddressSanitizer: heap-buffer-overflow".to_vec(),
                omitted: 0,
            },
        };
        control(&output, true, false).unwrap();
        assert!(control(&output, true, true).is_err());
        output.stderr.bytes = b"WARNING: ThreadSanitizer: data race".to_vec();
        control(&output, true, true).unwrap();
        output.stderr.bytes = b"panicked: ordinary assertion".to_vec();
        assert!(control(&output, true, false).is_err());
        output.stderr.bytes = b"ERROR: AddressSanitizer: heap-buffer-overflow".to_vec();
        output.completion = Completion::TimedOut;
        assert!(control(&output, true, false).is_err());
        output.completion = Completion::Exited(std::process::ExitStatus::from_raw(0));
        output.stdout.bytes = b"address control clean\n".to_vec();
        assert!(control(&output, false, false).is_err());
        output.stderr.bytes.clear();
        control(&output, false, false).unwrap();
        output.stdout.omitted = 1;
        assert!(control(&output, false, false).is_err());
    }
    #[test]
    fn selection_requires_each_named_test_once_and_successful_completion() {
        selection("alpha: test\nbeta: test\n", &["alpha", "beta"], false).unwrap();
        let text = "test alpha ... ok\ntest beta ... ok\ntest result: ok. 2 passed; 0 failed; 0 ignored; 9 filtered out\n";
        selection(text, &["alpha", "beta"], true).unwrap();
        for bad in [
            text.replace("beta ... ok", "alpha ... ok"),
            text.replace("beta ... ok", "beta ... ignored"),
            text.replace("2 passed", "1 passed"),
            text.replace("test beta ... ok\n", ""),
        ] {
            assert!(selection(&bad, &["alpha", "beta"], true).is_err());
        }
        assert!(selection("", &[], false).is_err());
        assert!(selection("alpha: test\n", &["alpha", "alpha"], false).is_err());
    }
}
