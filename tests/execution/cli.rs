use super::{Q1, Q6, ROW, TempDir};
use std::{
    fs,
    process::{Command, Stdio},
};

#[test]
fn stock_cli_executes_query_file() {
    let temp = TempDir::new();
    let database = temp.0.join("database");
    let input = temp.0.join("lineitem.tbl");
    let query = temp.0.join("q6.pipe.sql");
    let q1_query = temp.0.join("q1.pipe.sql");
    fs::write(&input, ROW).expect("write input");
    fs::write(&query, Q6).expect("write query");
    fs::write(&q1_query, Q1).expect("write Q1 query");
    let binary = env!("CARGO_BIN_EXE_pipesql");
    let limits = [
        "--memory-limit-bytes",
        "2000000",
        "--temp-limit-bytes",
        "1000000",
    ];
    for (operation, extra) in [
        ("create", None),
        ("load", Some(("--input", input.as_path()))),
        ("query", Some(("--query-file", query.as_path()))),
    ] {
        let mut command = Command::new(binary);
        command.arg(operation).arg("--database").arg(&database);
        if let Some((option, value)) = extra {
            command.arg(option).arg(value);
        }
        let output = command.args(limits).output().expect("CLI operation");
        assert!(
            output.status.success(),
            "{operation}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if operation == "query" {
            let stdout = String::from_utf8(output.stdout).expect("query stdout");
            assert!(stdout.contains("status=queried"));
            assert!(stdout.contains("column_count=1"));
            assert!(stdout.contains("columns=revenue:double:nullable"));
            assert!(stdout.contains("row=double:8:4020000000000000"));
            assert!(stdout.contains("row_count=1"));
        }
    }

    let q1_output = Command::new(binary)
        .arg("query")
        .arg("--database")
        .arg(&database)
        .arg("--query-file")
        .arg(&q1_query)
        .args(limits)
        .output()
        .expect("Q1 CLI operation");
    assert!(
        q1_output.status.success(),
        "Q1: {}",
        String::from_utf8_lossy(&q1_output.stderr)
    );
    let q1_stdout = String::from_utf8(q1_output.stdout).expect("Q1 stdout");
    assert!(q1_stdout.contains("column_count=10"));
    assert!(q1_stdout.contains("row=string:52|string:46|"));
    assert!(q1_stdout.contains("|int64:1\nrow_count=1"));

    // Close the sink before spawning: closing a piped reader after spawn races
    // with a child that can finish its small output before the parent runs.
    let (writer, reader) = std::os::unix::net::UnixStream::pair().unwrap();
    drop(reader);
    let stdout: std::os::fd::OwnedFd = writer.into();
    let broken = Command::new(binary)
        .arg("query")
        .arg("--database")
        .arg(&database)
        .arg("--query-file")
        .arg(&query)
        .args(limits)
        .stdout(stdout)
        .stderr(Stdio::piped())
        .output()
        .expect("run broken-output query");
    assert!(!broken.status.success());
    assert!(String::from_utf8_lossy(&broken.stderr).contains("write command status"));

    for (name, bytes) in [
        ("oversized.sql", vec![b' '; 4_097]),
        ("invalid-utf8.sql", vec![0xff]),
    ] {
        let invalid = temp.0.join(name);
        fs::write(&invalid, bytes).expect("write invalid query");
        let status = Command::new(binary)
            .arg("query")
            .arg("--database")
            .arg(&database)
            .arg("--query-file")
            .arg(invalid)
            .args(limits)
            .status()
            .expect("invalid query CLI");
        assert!(!status.success());
    }
    let alias = temp.0.join("query-link.sql");
    std::os::unix::fs::symlink(&query, &alias).expect("query symlink");
    let status = Command::new(binary)
        .arg("query")
        .arg("--database")
        .arg(&database)
        .arg("--query-file")
        .arg(alias)
        .args(limits)
        .status()
        .expect("symlink query CLI");
    assert!(!status.success());
}
