//! Check stock CLI CSV import from both files and pipes.
//!
//! The transaction token must appear before input consumption and remain resolvable
//! after late invalid input or process interruption. Reopen verifies that private
//! batches never become a committed prefix. A closed-stdin case checks that descriptor
//! reuse cannot make the importer read database files as input. Child guards and
//! finite waits keep failures from leaving a process using temporary directories
//! after the test begins cleanup.

use super::*;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

#[path = "../support/child.rs"]
mod child;
use child::ChildProcess;

const INPUT: &[u8] = b"amount,note,number,day\n1,first,1.5,1970-01-02\n2,\\N,-0,1969-12-31\n3,\"\",Infinity,1970-01-01\n";
const BOUNDS: [&str; 20] = [
    "--memory-limit-bytes",
    "4000000",
    "--temp-limit-bytes",
    "2000000",
    "--input-limit-bytes",
    "100000",
    "--row-limit",
    "100",
    "--record-limit-bytes",
    "4096",
    "--field-limit-bytes",
    "1024",
    "--batch-rows",
    "2",
    "--batch-text-bytes",
    "4096",
    "--batch-limit",
    "8",
    "--encoded-limit-bytes",
    "100000",
];

fn create(directory: &Directory) {
    let db = Database::create_empty(&directory.database(), config()).unwrap();
    db.declare_table("facts", &declarations(), &CancellationToken::new())
        .unwrap();
    db.close().unwrap();
}

fn command(directory: &Directory, input: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pipesql"));
    command
        .args(["import", "--database"])
        .arg(directory.database())
        .args(["--table", "facts", "--input"])
        .arg(input)
        .args(BOUNDS);
    command
}

fn wait(child: &mut ChildProcess) -> std::process::ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "import child timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn run(directory: &Directory, mut command: Command, input: Option<&[u8]>) -> Output {
    let stdout = directory.0.join("stdout");
    let stderr = directory.0.join("stderr");
    command
        .stdout(std::fs::File::create(&stdout).unwrap())
        .stderr(std::fs::File::create(&stderr).unwrap())
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = ChildProcess(command.spawn().unwrap());
    if let Some(bytes) = input {
        let mut stdin = child.0.stdin.take().unwrap();
        stdin.write_all(bytes).unwrap();
    }
    let status = wait(&mut child);
    drop(child);
    Output {
        status,
        stdout: std::fs::read(stdout).unwrap(),
        stderr: std::fs::read(stderr).unwrap(),
    }
}

fn token(bytes: &[u8]) -> pipesql::TransactionId {
    let text = std::str::from_utf8(bytes).unwrap();
    let mut lines = text
        .lines()
        .filter_map(|line| line.strip_prefix("transaction="));
    let text = lines.next().expect("early transaction token");
    assert!(lines.next().is_none());
    assert_eq!(text.len(), 48);
    let mut bytes = [0; 24];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).unwrap();
    }
    pipesql::TransactionId::from_bytes(bytes).unwrap()
}

#[test]
fn file_and_stdin_import_commit_typed_rows_and_resolve_receipts() {
    for stdin in [false, true] {
        let directory = Directory::new();
        create(&directory);
        let path = directory.0.join("input.csv");
        std::fs::write(&path, INPUT).unwrap();
        let output = run(
            &directory,
            command(&directory, if stdin { Path::new("-") } else { &path }),
            stdin.then_some(INPUT),
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let receipt = token(&output.stdout);
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.starts_with("transaction="));
        assert!(text.ends_with("status=imported\ngeneration=2\n"));
        let db = Database::open(&directory.database(), config()).unwrap();
        assert!(
            matches!(db.resolve_commit(receipt).unwrap(), CommitResolution::Durable(commit) if commit.generation() == 2)
        );
        query(
            &db,
            "FROM facts |> ORDER BY amount",
            vec![
                vec![
                    Cell::Text("first".into()),
                    Cell::Integer(1),
                    Cell::Number(1.5f64.to_bits()),
                    Cell::Day(1),
                ],
                vec![
                    Cell::Null,
                    Cell::Integer(2),
                    Cell::Number((-0.0f64).to_bits()),
                    Cell::Day(-1),
                ],
                vec![
                    Cell::Text("".into()),
                    Cell::Integer(3),
                    Cell::Number(f64::INFINITY.to_bits()),
                    Cell::Day(0),
                ],
            ],
        );
    }
}

#[test]
fn late_invalid_input_keeps_early_token_and_no_committed_prefix() {
    let directory = Directory::new();
    create(&directory);
    let mut input = INPUT.to_vec();
    input.extend_from_slice(b"bad,last,1,1970-01-01\n");
    let output = run(
        &directory,
        command(&directory, Path::new("-")),
        Some(&input),
    );
    assert_eq!(output.status.code(), Some(1));
    let receipt = token(&output.stdout);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("status=imported"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid CSV INT64"));
    let db = Database::open(&directory.database(), config()).unwrap();
    assert_eq!(
        db.resolve_commit(receipt).unwrap(),
        CommitResolution::Aborted
    );
    query(&db, "FROM facts", vec![]);
}

#[test]
fn interrupted_pipe_import_removes_private_files_on_reopen() {
    let directory = Directory::new();
    create(&directory);
    let stdout = directory.0.join("receipt");
    let mut request = command(&directory, Path::new("-"));
    request
        .stdin(Stdio::piped())
        .stdout(std::fs::File::create(&stdout).unwrap())
        .stderr(Stdio::null());
    let mut child = ChildProcess(request.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited before input"
        );
        if std::fs::read(&stdout).unwrap().ends_with(b"\n") {
            break;
        }
        assert!(Instant::now() < deadline, "missing early receipt");
        std::thread::sleep(Duration::from_millis(10));
    }
    let receipt = token(&std::fs::read(&stdout).unwrap());
    let before = std::fs::read_dir(directory.database().join("units"))
        .unwrap()
        .count();
    child.0.stdin.as_mut().unwrap().write_all(INPUT).unwrap();
    loop {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited before EOF"
        );
        if std::fs::read_dir(directory.database().join("units"))
            .unwrap()
            .count()
            > before
        {
            break;
        }
        assert!(Instant::now() < deadline, "no private file created");
        std::thread::sleep(Duration::from_millis(10));
    }
    child.0.kill().unwrap();
    assert!(!wait(&mut child).success());
    drop(child);
    let db = Database::open(&directory.database(), config()).unwrap();
    assert_eq!(
        db.resolve_commit(receipt).unwrap(),
        CommitResolution::Aborted
    );
    assert_eq!(db.reserved_temp_bytes(), 0);
    query(&db, "FROM facts", vec![]);
    db.close().unwrap();
    let output = run(&directory, command(&directory, Path::new("-")), Some(INPUT));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn shell_closed_stdin_is_empty_and_cannot_read_database_files() {
    let directory = Directory::new();
    create(&directory);
    let mut request = Command::new("/bin/sh");
    request
        .args([
            "-c",
            "exec 0<&-; exec \"$@\"",
            "pipesql-test",
            env!("CARGO_BIN_EXE_pipesql"),
            "import",
            "--database",
        ])
        .arg(directory.database())
        .args(["--table", "facts", "--input", "-"])
        .args(BOUNDS);
    let output = run(&directory, request, None);
    assert_eq!(output.status.code(), Some(1));
    let receipt = token(&output.stdout);
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing CSV header"));
    let db = Database::open(&directory.database(), config()).unwrap();
    assert_eq!(db.generation(), 1);
    assert_eq!(
        db.resolve_commit(receipt).unwrap(),
        CommitResolution::Aborted
    );
    query(&db, "FROM facts", vec![]);
}
