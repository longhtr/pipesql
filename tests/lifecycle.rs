use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use pipesql::{Config, Database, Error};

const WAIT_STEPS: usize = 500;

mod support;
use support::Directory as TempDir;

fn config() -> Config {
    Config::new(1_048_576, 1_048_576).expect("test config")
}

fn wait_for(path: &Path) {
    for _ in 0..WAIT_STEPS {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for child marker");
}

// The lease child starts no subprocesses. Reap it before its database directory
// is removed, including when a parent assertion fails before sending release.
struct LeaseChild(std::process::Child);

impl Drop for LeaseChild {
    fn drop(&mut self) {
        let cleanup = (|| -> std::io::Result<()> {
            if self.0.try_wait()?.is_none() {
                self.0.kill()?;
                self.0.wait()?;
            }
            Ok(())
        })();
        if let Err(error) = cleanup
            && !std::thread::panicking()
        {
            panic!("reap lease child: {error}");
        }
    }
}

#[test]
fn separate_process_excludes_canonical_and_alias_opens() {
    const CHILD: &str = "PIPESQL_LOCK_CHILD_DATABASE";
    if let Some(database) = std::env::var_os(CHILD) {
        let ready = PathBuf::from(std::env::var_os("PIPESQL_LOCK_CHILD_READY").unwrap());
        let release = PathBuf::from(std::env::var_os("PIPESQL_LOCK_CHILD_RELEASE").unwrap());
        let database = Database::open(Path::new(&database), config()).unwrap();
        fs::write(&ready, []).expect("publish child readiness");
        wait_for(&release);
        database.close().expect("child closes database");
        return;
    }

    let temp = TempDir::new();
    let database_path = temp.0.join("database");
    let alias_path = temp.0.join("alias");
    Database::create(&database_path, config())
        .expect("create")
        .close()
        .expect("close");
    std::os::unix::fs::symlink(&database_path, &alias_path).expect("create alias");

    for release_normally in [true, false] {
        let ready = temp.0.join(format!("ready-{release_normally}"));
        let release = temp.0.join(format!("release-{release_normally}"));
        let mut child = LeaseChild(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "separate_process_excludes_canonical_and_alias_opens",
                    "--nocapture",
                ])
                .env(CHILD, &database_path)
                .env("PIPESQL_LOCK_CHILD_READY", &ready)
                .env("PIPESQL_LOCK_CHILD_RELEASE", &release)
                .spawn()
                .expect("spawn lock child"),
        );
        for _ in 0..WAIT_STEPS {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "lease child exited before release"
            );
            if ready.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            ready.exists(),
            "lease child never reached the selected test"
        );
        for path in [&database_path, &alias_path] {
            assert!(matches!(Database::open(path, config()), Err(Error::Locked)));
        }
        if release_normally {
            fs::write(&release, []).expect("release child");
            let mut status = None;
            for _ in 0..WAIT_STEPS {
                status = child.0.try_wait().unwrap();
                if status.is_some() {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            assert!(status.expect("lease child did not finish").success());
        }
        drop(child);
        // Also proves early teardown released the lease without a release marker.
        Database::open(&alias_path, config())
            .expect("open after child cleanup")
            .close()
            .expect("close after child cleanup");
    }
}

#[test]
fn public_api_prepares_exact_q6_and_releases_memory() {
    let temp = TempDir::new();
    let database_path = temp.0.join("database");
    let database = Database::create(&database_path, config()).expect("create database");
    let resident = database.reserved_memory_bytes();
    let prepared = database
        .prepare(include_str!("fixtures/q6.pipe.sql"))
        .expect("prepare exact Q6");
    assert_eq!(prepared.result_column_count(), 1);
    assert_eq!(prepared.result_column(0).unwrap().name, Some("revenue"));
    assert_eq!(prepared.result_column(1), None);
    assert_eq!(
        database.reserved_memory_bytes(),
        resident + prepared.accounted_memory_bytes()
    );
    drop(prepared);
    assert_eq!(database.reserved_memory_bytes(), resident);
    database.close().expect("close database");
}

#[test]
fn stock_cli_preserves_usage_exit_when_stderr_is_broken() {
    let (writer, reader) = std::os::unix::net::UnixStream::pair().unwrap();
    drop(reader);
    let stderr: std::os::fd::OwnedFd = writer.into();
    let status = Command::new(env!("CARGO_BIN_EXE_pipesql"))
        .stderr(stderr)
        .status()
        .unwrap();
    assert_eq!(
        status.code(),
        Some(2),
        "diagnostic I/O must not turn usage failure into a panic"
    );
}

#[test]
fn stock_cli_creates_and_reopens_database() {
    let temp = TempDir::new();
    let database = temp.0.join("database");
    let binary = env!("CARGO_BIN_EXE_pipesql");
    let common = [
        "--database",
        database.to_str().expect("UTF-8 test path"),
        "--memory-limit-bytes",
        "1048576",
        "--temp-limit-bytes",
        "1048576",
    ];
    let create = Command::new(binary)
        .arg("create")
        .args(common)
        .output()
        .expect("run create CLI");
    assert!(
        create.status.success(),
        "{}",
        String::from_utf8_lossy(&create.stderr)
    );
    assert!(
        String::from_utf8(create.stdout)
            .expect("UTF-8 create output")
            .contains("status=created")
    );
    let open = Command::new(binary)
        .arg("open")
        .args(common)
        .output()
        .expect("run open CLI");
    assert!(
        open.status.success(),
        "{}",
        String::from_utf8_lossy(&open.stderr)
    );
    assert!(
        String::from_utf8(open.stdout)
            .expect("UTF-8 open output")
            .contains("status=opened")
    );
}

// Measure the ordinary library handle, independently of test-only instrumentation.
#[test]
fn stock_database_handle_retains_its_fixed_bound() {
    let bytes = std::mem::size_of::<Database>();
    assert!(bytes <= 4096 + 80, "database handle: {bytes} bytes");
}

#[test]
fn database_path_capacity_is_admitted_before_creation_and_retained_until_close() {
    let temp = TempDir::new();
    let path = temp.0.join("database");
    assert!(matches!(
        Database::create(&path, Config::new(4095, 1_000_000).unwrap()),
        Err(pipesql::Error::Resource {
            owner: "database pathname",
            required: 4096,
            limit: 4095
        })
    ));
    assert!(!path.exists());
    let database = Database::create(&path, config()).unwrap();
    let resident = database.reserved_memory_bytes();
    assert!(resident >= database.path().as_os_str().len() as u64);
    assert!(resident <= 4096);
    println!(
        "database_handle_bytes={} resident_path_bytes={resident}",
        std::mem::size_of::<Database>()
    );
    database.close().unwrap();
    let database = Database::open(&path, config()).unwrap();
    assert_eq!(database.reserved_memory_bytes(), resident);
    database.close().unwrap();
}

#[test]
fn fixture_cleanup_preserves_failure_context() {
    const CHILD: &str = "PIPESQL_PUBLIC_DIRECTORY_UNWIND";
    if std::env::var_os(CHILD).is_some() {
        let directory = TempDir::new();
        let path = directory.0.clone();
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, b"not a directory").unwrap();
        let original = std::panic::catch_unwind(|| {
            let _directory = directory;
            panic!("original scenario failure");
        })
        .unwrap_err();
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            original.downcast_ref::<&str>(),
            Some(&"original scenario failure")
        );
        std::process::exit(74);
    }

    let directory = TempDir::new();
    let path = directory.0.clone();
    drop(directory);
    assert!(!path.exists());

    let missing = TempDir::new();
    std::fs::remove_dir(&missing.0).unwrap();
    drop(missing);

    let directory = TempDir::new();
    let path = directory.0.clone();
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, b"not a directory").unwrap();
    let failure = std::panic::catch_unwind(|| drop(directory)).unwrap_err();
    std::fs::remove_file(path).unwrap();
    assert!(
        failure
            .downcast_ref::<String>()
            .unwrap()
            .contains("remove test directory")
    );

    // A cleanup panic during unwinding would abort the child, not this harness.
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "fixture_cleanup_preserves_failure_context"])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert_eq!(
        child.status.code(),
        Some(74),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
}
