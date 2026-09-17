//! Run failure controls in a separate test process with a finite deadline.
//!
//! The caller chooses the exact test, control and expected diagnostic. A log
//! confirms that one test actually ran; child exit alone could mean an empty
//! selector. The child guard stops a timed-out process before its inputs vanish.
//!
//! Each invocation owns a fresh log directory and passes the control through an
//! environment variable. The parent requires the selected test's terminal marker,
//! expected diagnostic and exit outcome before releasing those files. Expected
//! panic cases therefore cannot pass merely because an unrelated child crashed.

use crate::test_child::ChildProcess;
use crate::test_support::Directory;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub(crate) const CASE: &str = "PIPESQL_TEST_FAILURE_CASE";

pub(crate) fn run(test: &str, case: &str, success: bool, diagnostic: &str) {
    let directory = Directory::new();
    let log_path = directory.0.join("child.log");
    let log = std::fs::File::create(&log_path).unwrap();
    let mut child = ChildProcess(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                test.split_once("::").expect("qualified test name").1,
                "--nocapture",
            ])
            .env(CASE, case)
            .env("TMPDIR", &directory.0)
            .env("RUST_BACKTRACE", "0")
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "{case}: test child timed out");
        std::thread::sleep(Duration::from_millis(10));
    };
    drop(child);
    let log = std::fs::read_to_string(log_path).unwrap();
    assert_eq!(status.success(), success, "{case}: {log}");
    assert!(log.contains("running 1 test"), "{case}: {log}");
    let terminal = if success {
        "test result: ok. 1 passed; 0 failed;"
    } else {
        "test result: FAILED. 0 passed; 1 failed;"
    };
    assert!(log.contains(terminal), "{case}: {log}");
    assert!(log.contains(diagnostic), "{case}: {log}");
    assert_eq!(
        std::fs::read_dir(&directory.0).unwrap().count(),
        1,
        "{case}: child left temporary inputs behind"
    );
}
