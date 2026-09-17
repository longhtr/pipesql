//! Check the real container owner from a Docker-capable host.
//!
//! A supervised child runs success, failure, missing-result, mismatched-result and
//! interruption controls. The parent checks saved diagnostics and independently asks
//! Docker whether the named container remains. Interruption waits for a marker from
//! inside the container before signaling its owner. A fallback guard removes the
//! container if an assertion fails. This test is selected explicitly on the host;
//! ordinary tests inside the network-disabled container cannot run Docker commands.

use super::*;

struct ContainerFallback(String);

impl Drop for ContainerFallback {
    fn drop(&mut self) {
        match container_present(&self.0) {
            Ok(false) => return,
            Err(error) => eprintln!("cannot inspect test container before cleanup: {error}"),
            Ok(true) => (),
        }
        let output = run_cleanup(
            Command::new("docker").args(["rm", "--force", &self.0]),
            Duration::from_secs(30),
        );
        if let Err(error) = output.and_then(|output| output.require_success()) {
            eprintln!("test container cleanup failed: {error}");
        }
    }
}

fn container_present(name: &str) -> io::Result<bool> {
    let output = run_cleanup(
        Command::new("docker").args([
            "ps",
            "--all",
            "--filter",
            &format!("name=^/{name}$"),
            "--format",
            "{{.Names}}",
        ]),
        Duration::from_secs(10),
    )?;
    output.require_success()?;
    Ok(!output.stdout.bytes.is_empty())
}

fn output_directory(directory: &Path) -> Option<PathBuf> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("pipesql-linux-")
        })
}

#[test]
#[ignore = "requires the pinned Docker image on the host"]
fn container_outcomes_and_interruption() {
    for (mode, expected) in [
        ("success", "success"),
        ("failure", "did not succeed"),
        ("missing", "missing Linux completion"),
        ("mismatch", "mismatched Linux completion"),
        ("interrupt", "Interrupted"),
    ] {
        eprintln!("Linux owner control: {mode}");
        let directory = Directory::new();
        let mut command = fixture("linux-owner", &directory.0);
        command.current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap());
        command
            .env("PIPESQL_LINUX_PROBE", mode)
            .env("PIPESQL_RUN_ROOT", &directory.0);
        let mut owner = Process::spawn(&mut command).unwrap();
        let mut fallback = None;
        let mut interrupted = false;
        let mut deadline = Instant::now() + Duration::from_secs(150);
        loop {
            owner.drain().unwrap();
            if fallback.is_none()
                && let Some(path) = output_directory(&directory.0)
            {
                fallback = Some(ContainerFallback(
                    path.file_name().unwrap().to_str().unwrap().to_owned(),
                ));
            }
            if let Some(status) = owner.child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "owner failed: {}",
                    String::from_utf8_lossy(&owner.stderr.bytes)
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Linux owner control timed out: {mode}"
            );
            if mode == "interrupt"
                && !interrupted
                && let Some(container) = &fallback
            {
                let ready = run(
                    Command::new("docker").args([
                        "exec",
                        &container.0,
                        "test",
                        "-f",
                        "/tmp/pipesql-results/probe-entered",
                    ]),
                    Duration::from_secs(5),
                )
                .unwrap();
                if matches!(ready.completion, Completion::Exited(status) if status.success()) {
                    assert!(
                        container_present(&container.0).unwrap(),
                        "presence check missed live container"
                    );
                    signal(owner.child.id() as i32, libc::SIGTERM).unwrap();
                    interrupted = true;
                    deadline = Instant::now() + Duration::from_secs(25);
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        owner.finish().unwrap();
        let result = fs::read_to_string(directory.0.join("owner-result")).unwrap();
        if mode == "success" {
            assert_eq!(result, expected);
        } else {
            assert!(result.contains(expected), "{mode}: {result}");
        }
        let output = output_directory(&directory.0).expect("owner created output directory");
        assert_eq!(
            fs::read_to_string(output.join("results/probe-entered")).unwrap(),
            mode
        );
        assert!(
            !container_present(&fallback.as_ref().unwrap().0).unwrap(),
            "owner left its container behind"
        );
        if mode == "interrupt" {
            assert!(interrupted);
        }
        drop(fallback);
    }
}
