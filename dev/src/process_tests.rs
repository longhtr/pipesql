//! Challenge process ownership with disposable children and live heartbeats.
//!
//! A separate test process acts as the supervisor, while descendants update a file
//! that the outer test can observe. Return, panic, interruption and timeout must stop
//! owned work. Disabling cleanup must leave the heartbeat changing, proving that the
//! check detects surviving descendants. Fallback owners stop deliberate strays.
//! The ignored child entry point is invoked only with a role and readiness markers;
//! Docker-host controls live in the linux child module.

use super::*;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[path = "linux_tests.rs"]
mod linux;

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        static HANDLERS: std::sync::Once = std::sync::Once::new();
        HANDLERS.call_once(|| install_interrupt_handlers().unwrap());
        let path = std::env::temp_dir().join(format!(
            "pipesql-process-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("failed process test retained output: {}", self.0.display());
        } else {
            fs::remove_dir_all(&self.0).expect("process test directory cleanup");
        }
    }
}

fn fixture(role: &str, path: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "process::tests::child",
        "--ignored",
        "--nocapture",
    ]);
    command
        .env("PIPESQL_PROCESS_ROLE", role)
        .env("PIPESQL_PROCESS_DIRECTORY", path);
    command
}

#[test]
fn exit_status_and_bounded_output() {
    let directory = Directory::new();
    for (role, code) in [("success", 0), ("failure", 17)] {
        let output = run(&mut fixture(role, &directory.0), Duration::from_secs(5)).unwrap();
        assert!(
            matches!(output.completion, Completion::Exited(status) if status.code() == Some(code))
        );
        assert_eq!(output.require_success().is_ok(), code == 0);
        assert!(String::from_utf8_lossy(&output.stdout.bytes).contains("fixture entered"));
    }
    let output = run(
        &mut fixture("output", &directory.0),
        Duration::from_secs(15),
    )
    .unwrap();
    assert!(matches!(output.completion, Completion::Exited(status) if status.success()));
    assert_eq!(output.stdout.bytes.len(), OUTPUT_LIMIT);
    assert_eq!(output.stderr.bytes.len(), OUTPUT_LIMIT);
    assert!(output.stdout.omitted > 0 && output.stderr.omitted > 0);
    assert!(
        output
            .require_success()
            .unwrap_err()
            .to_string()
            .contains("truncated")
    );
}

#[test]
fn timeout_stops_a_child_that_ignores_termination() {
    let directory = Directory::new();
    let output = run(
        &mut fixture("heartbeat", &directory.0),
        Duration::from_millis(300),
    )
    .unwrap();
    assert_eq!(output.completion, Completion::TimedOut);
    assert_heartbeat(&directory.0, false);
}

fn assert_heartbeat(directory: &Path, changing: bool) {
    let before = fs::read(directory.join("heartbeat")).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    let after = fs::read(directory.join("heartbeat")).unwrap();
    assert_eq!(
        before != after,
        changing,
        "child heartbeat has the wrong liveness"
    );
}

struct StrandedGroup(i32);
impl Drop for StrandedGroup {
    fn drop(&mut self) {
        signal_group(self.0, libc::SIGKILL).unwrap();
    }
}

#[test]
fn supervisor_return_unwind_and_interrupt_stop_descendants() {
    for role in ["return", "unwind", "interrupt", "disabled-cleanup"] {
        let directory = Directory::new();
        let mut supervisor = Process::spawn(&mut fixture(role, &directory.0)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let group = loop {
            supervisor.drain().unwrap();
            if let Ok(pid) = fs::read_to_string(directory.0.join("group"))
                && let Ok(pid) = pid.parse::<i32>()
            {
                break StrandedGroup(pid);
            }
            assert!(
                Instant::now() < deadline,
                "supervisor did not publish child group"
            );
            std::thread::sleep(POLL);
        };
        if role == "interrupt" {
            signal(supervisor.child.id() as i32, libc::SIGTERM).unwrap();
        }
        let status = loop {
            supervisor.drain().unwrap();
            if let Some(status) = supervisor.child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "supervisor failed to stop");
            std::thread::sleep(POLL);
        };
        supervisor.finish().unwrap();
        assert_eq!(status.success(), role != "unwind");
        assert_heartbeat(&directory.0, role == "disabled-cleanup");
        drop(group);
    }
}

#[test]
fn nested_supervisor_stops_its_separate_group() {
    let directory = Directory::new();
    let mut supervisor = Process::spawn_with_stdout(
        &mut fixture("nested", &directory.0),
        Stdio::piped(),
        GRACE * 4,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let group = loop {
        supervisor.drain().unwrap();
        if let Ok(pid) = fs::read_to_string(directory.0.join("group"))
            && let Ok(pid) = pid.parse::<i32>()
        {
            break StrandedGroup(pid);
        }
        assert!(
            Instant::now() < deadline,
            "nested supervisor did not become ready"
        );
        std::thread::sleep(POLL);
    };
    supervisor.finish().unwrap();
    assert!(
        supervisor.child.wait().unwrap().success(),
        "nested supervisor was killed before cleanup finished"
    );
    assert_heartbeat(&directory.0, false);
    drop(group);
}

/// Only the parent tests launch this entry point. Each fixture prints a marker
/// so an accidental empty test selection cannot stand in for execution.
#[test]
#[ignore]
fn child() {
    let role = std::env::var("PIPESQL_PROCESS_ROLE").unwrap();
    let directory = PathBuf::from(std::env::var_os("PIPESQL_PROCESS_DIRECTORY").unwrap());
    println!("fixture entered");
    match role.as_str() {
        "linux-owner" => {
            install_interrupt_handlers().unwrap();
            let mode = std::env::var("PIPESQL_LINUX_PROBE").unwrap();
            let result = crate::linux::run(&["linux-probe".into(), mode]);
            match result {
                Ok(()) => fs::write(directory.join("owner-result"), "success").unwrap(),
                Err(error) => fs::write(directory.join("owner-result"), error.to_string()).unwrap(),
            }
        }
        "success" => {}
        "failure" => std::process::exit(17),
        "output" => {
            let bytes = [b'x'; 8192];
            for _ in 0..2048 {
                std::io::stdout().write_all(&bytes).unwrap();
                std::io::stderr().write_all(&bytes).unwrap();
            }
        }
        "heartbeat" => {
            // SAFETY: this isolated child deliberately ignores graceful shutdown.
            unsafe {
                libc::signal(libc::SIGTERM, libc::SIG_IGN);
            }
            for number in 0..1500 {
                fs::write(directory.join("heartbeat"), number.to_string()).unwrap();
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        "parent" => {
            let child = fixture("heartbeat", &directory).spawn().unwrap();
            // This process exits while its descendant remains in the same group.
            // The supervisor is responsible for that descendant.
            std::mem::forget(child);
            while !directory.join("heartbeat").exists() {
                std::thread::sleep(POLL);
            }
        }
        "nested" => {
            install_interrupt_handlers().unwrap();
            let mut child = Process::spawn(&mut fixture("heartbeat", &directory)).unwrap();
            while !directory.join("heartbeat").exists() {
                std::thread::sleep(POLL);
            }
            fs::write(directory.join("group"), child.child.id().to_string()).unwrap();
            while INTERRUPT.load(Ordering::Relaxed) == 0 {
                std::thread::sleep(POLL);
            }
            child.finish().unwrap();
        }
        "return" | "unwind" | "interrupt" | "disabled-cleanup" => {
            install_interrupt_handlers().unwrap();
            let mut process = Process::spawn(&mut fixture("parent", &directory)).unwrap();
            while !directory.join("heartbeat").exists() {
                process.drain().unwrap();
                std::thread::sleep(POLL);
            }
            // Publish only after the deliberate descendant is known to be alive.
            fs::write(directory.join("group"), process.child.id().to_string()).unwrap();
            match role.as_str() {
                "unwind" => panic!("deliberate supervisor unwind"),
                "disabled-cleanup" => std::mem::forget(process),
                "interrupt" => {
                    while INTERRUPT.load(Ordering::Relaxed) == 0 {
                        std::thread::sleep(POLL);
                    }
                    process.finish().unwrap();
                }
                _ => process.finish().unwrap(),
            }
        }
        other => panic!("unknown fixture: {other}"),
    }
}
