use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use pipesql::{Config, Database, Error};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);
const WAIT_STEPS: usize = 500;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("pipesql-public-{}-{id}", std::process::id()));
        fs::create_dir(&path).expect("create public test directory");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            // Preserve the test failure during unwinding; report cleanup failures
            // when the scenario itself completed successfully.
            if error.kind() != std::io::ErrorKind::NotFound && !std::thread::panicking() {
                panic!("remove test directory {}: {error}", self.0.display());
            }
        }
    }
}

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

#[test]
fn cross_process_lock_child() {
    let Some(database) = std::env::var_os("PIPESQL_LOCK_CHILD_DATABASE") else {
        return;
    };
    let ready = PathBuf::from(std::env::var_os("PIPESQL_LOCK_CHILD_READY").expect("ready path"));
    let release =
        PathBuf::from(std::env::var_os("PIPESQL_LOCK_CHILD_RELEASE").expect("release path"));
    let database = Database::open(Path::new(&database), config()).expect("child opens database");
    fs::write(&ready, []).expect("publish child readiness");
    wait_for(&release);
    database.close().expect("child closes database");
}

#[test]
fn separate_process_excludes_canonical_and_alias_opens() {
    let temp = TempDir::new();
    let database_path = temp.0.join("database");
    let alias_path = temp.0.join("alias");
    let ready = temp.0.join("ready");
    let release = temp.0.join("release");
    Database::create(&database_path, config())
        .expect("create")
        .close()
        .expect("close");
    std::os::unix::fs::symlink(&database_path, &alias_path).expect("create alias");

    let mut child = Command::new(std::env::current_exe().expect("test executable"))
        .arg("--exact")
        .arg("cross_process_lock_child")
        .arg("--nocapture")
        .env("PIPESQL_LOCK_CHILD_DATABASE", &database_path)
        .env("PIPESQL_LOCK_CHILD_READY", &ready)
        .env("PIPESQL_LOCK_CHILD_RELEASE", &release)
        .spawn()
        .expect("spawn lock child");
    wait_for(&ready);
    assert!(matches!(
        Database::open(&database_path, config()),
        Err(Error::Locked)
    ));
    assert!(matches!(
        Database::open(&alias_path, config()),
        Err(Error::Locked)
    ));
    fs::write(&release, []).expect("release child");
    assert!(child.wait().expect("wait for child").success());
    Database::open(&alias_path, config())
        .expect("open after child")
        .close()
        .expect("close after child");
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
