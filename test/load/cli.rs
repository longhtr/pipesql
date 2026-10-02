//! Test legacy loading and transaction resolution through the built CLI.
//!
//! One case creates a database, loads a row and passes the printed transaction
//! token to a new process. That process must report the same durable commit.
//! Another case installs independently encoded files containing an aborted
//! attempt and a committed attempt, then checks both resolutions.
//!
//! Resolution must repair a missing root during open. Unknown tokens, an occupied
//! database lock and corrupt roots must fail with no success output. Malformed
//! token text must produce the distinct command-usage exit code.

use super::{ROW, TempDir, config};
use pipesql::Database;
use std::{fs, path::PathBuf, process::Command};

#[test]
fn stock_cli_loads_and_reopens_generation_one() {
    let temp = TempDir::new();
    let database = temp.0.join("database");
    let input = temp.0.join("lineitem.tbl");
    fs::write(&input, ROW).expect("write input");
    let binary = env!("CARGO_BIN_EXE_pipesql");
    let limits = [
        "--memory-limit-bytes",
        "2000000",
        "--temp-limit-bytes",
        "1000000",
    ];
    let create = Command::new(binary)
        .arg("create")
        .arg("--database")
        .arg(&database)
        .args(limits)
        .output()
        .expect("create CLI");
    assert!(
        create.status.success(),
        "{}",
        String::from_utf8_lossy(&create.stderr)
    );
    let load = Command::new(binary)
        .arg("load")
        .arg("--database")
        .arg(&database)
        .arg("--input")
        .arg(&input)
        .args(limits)
        .output()
        .expect("load CLI");
    assert!(
        load.status.success(),
        "{}",
        String::from_utf8_lossy(&load.stderr)
    );
    let output = String::from_utf8(load.stdout).expect("UTF-8 load output");
    assert!(output.contains("status=loaded"));
    assert!(output.contains("generation=1"));
    assert!(output.contains("transaction="));
    let transaction = output
        .lines()
        .find_map(|line| line.strip_prefix("transaction="))
        .unwrap();
    let resolved = Command::new(binary)
        .args(["resolve", "--database"])
        .arg(&database)
        .args(["--transaction", transaction])
        .args(limits)
        .output()
        .expect("resolve printed token");
    assert!(
        resolved.status.success(),
        "{}",
        String::from_utf8_lossy(&resolved.stderr)
    );
    let resolved = String::from_utf8(resolved.stdout).unwrap();
    assert!(resolved.lines().any(|line| line == "resolution=durable"));
    assert!(resolved.lines().any(|line| line == "generation=1"));
    assert!(
        resolved
            .lines()
            .any(|line| line == format!("transaction={transaction}"))
    );
    Database::open(&database, config())
        .expect("reopen CLI database")
        .close()
        .expect("close CLI database");
}

#[test]
fn stock_cli_resolves_independent_history_and_refuses_unsettled_state() {
    let temp = TempDir::new();
    let database = temp.0.join("database");
    fs::create_dir(&database).unwrap();
    fs::create_dir(database.join("private")).unwrap();
    fs::create_dir(database.join("units")).unwrap();
    fs::write(database.join("LOCK"), []).unwrap();
    // These retained bytes come from the independent fixture encoder. Attempt
    // 1 aborted; attempt 2 committed generation 1. Building this history through
    // the engine would let matching writer and reader bugs conceal an error.
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test/data/current-single-table-format");
    for name in ["CONTROL", "ROOT.A", "ROOT.B", "WAL"] {
        fs::write(database.join(name), fs::read(fixture.join(name)).unwrap()).unwrap();
    }
    fs::write(
        database.join("units/0000000000000001.unit"),
        fs::read(fixture.join("UNIT")).unwrap(),
    )
    .unwrap();
    let resolve = |token: &str| {
        Command::new(env!("CARGO_BIN_EXE_pipesql"))
            .args(["resolve", "--database"])
            .arg(&database)
            .args([
                "--transaction",
                token,
                "--memory-limit-bytes",
                "2000000",
                "--temp-limit-bytes",
                "1000000",
            ])
            .output()
            .unwrap()
    };
    let aborted = "000102030405060708090a0b0c0d0e0f0100000000000000";
    let durable = "000102030405060708090a0b0c0d0e0f0200000000000000";
    // Removing one root requires open to repair the database before resolution.
    // Compare the repaired bytes below, not just the command's exit status.
    fs::remove_file(database.join("ROOT.B")).unwrap();
    for (token, expected) in [
        (aborted, "aborted"),
        (durable, "durable"),
        (aborted, "aborted"),
    ] {
        let result = resolve(token);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8(result.stdout).unwrap();
        assert!(
            output
                .lines()
                .any(|line| line == format!("transaction={token}"))
        );
        assert!(
            output
                .lines()
                .any(|line| line == format!("resolution={expected}"))
        );
        assert_eq!(
            output.lines().any(|line| line == "generation=1"),
            expected == "durable"
        );
    }
    assert_eq!(
        fs::read(database.join("ROOT.B")).unwrap(),
        fs::read(fixture.join("ROOT.B")).unwrap()
    );
    for token in [
        "000102030405060708090a0b0c0d0e0f0300000000000000",
        "100102030405060708090a0b0c0d0e0f0200000000000000",
    ] {
        let result = resolve(token);
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        assert!(String::from_utf8_lossy(&result.stderr).contains("transaction was not found"));
    }
    let held = Database::open(&database, config()).unwrap();
    let locked = resolve(durable);
    assert_eq!(locked.status.code(), Some(1));
    assert!(locked.stdout.is_empty());
    assert!(String::from_utf8_lossy(&locked.stderr).contains("already open"));
    held.close().unwrap();
    for name in ["ROOT.A", "ROOT.B"] {
        fs::write(database.join(name), [0; 4096]).unwrap();
    }
    let corrupt = resolve(durable);
    assert_eq!(corrupt.status.code(), Some(1));
    assert!(corrupt.stdout.is_empty());
    let invalid = resolve("not-a-token");
    assert_eq!(invalid.status.code(), Some(2));
    assert!(invalid.stdout.is_empty());
}
