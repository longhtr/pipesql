use super::{ROW, TempDir, config};
use pipesql::{CancellationToken, Config, Database};
use std::{fs, path::PathBuf, process::Command};

fn assert_reported_stack_bound(limit: usize) {
    let reported = pipesql_filesystem::test_current_thread_stack_bytes();
    assert!(
        reported > 0 && reported <= limit,
        "reported thread stack {reported} outside 1..={limit}"
    );
}

#[test]
#[cfg_attr(
    all(target_os = "linux", target_arch = "aarch64", target_env = "gnu"),
    ignore = "GNU aarch64 has a 128-KiB pthread minimum; this test requires at most 64 KiB"
)]
fn public_load_and_queries_fit_declared_stack_headroom() {
    const CHILD: &str = "PIPESQL_LOAD_STACK_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let path = PathBuf::from(std::env::var_os("PIPESQL_STACK_DATABASE").unwrap());
        let input = PathBuf::from(std::env::var_os("PIPESQL_STACK_INPUT").unwrap());
        // Open and the caller-owned handle live outside the measured load stack.
        let mut database =
            Box::new(Database::open(&path, Config::new(2_000_000, 8_000_000).unwrap()).unwrap());
        let resident = database.reserved_memory_bytes();
        let database = std::thread::Builder::new()
            .name("bounded-public-load".into())
            // Native allocation can exceed the requested size. Leave one host
            // page of margin, then verify the original 64-KiB ceiling inside.
            .stack_size(49_152)
            .spawn(move || {
                assert_reported_stack_bound(65_536);
                let commit = database
                    .load_lineitem(&input, &CancellationToken::new())
                    .unwrap();
                assert_eq!(commit.generation(), 1);
                assert_eq!(database.reserved_memory_bytes(), resident);
                assert_eq!(database.reserved_temp_bytes(), 0);
                database
            })
            .unwrap()
            .join()
            .unwrap();
        for sql in [
            include_str!("../fixtures/q6.pipe.sql"),
            include_str!("../fixtures/upstream/q1-upstream.pipe.sql"),
        ] {
            let prepared = database.prepare(sql).unwrap();
            std::thread::scope(|scope| {
                std::thread::Builder::new()
                    .name("bounded-public-query".into())
                    .stack_size(49_152)
                    .spawn_scoped(scope, || {
                        assert_reported_stack_bound(65_536);
                        let cancellation = CancellationToken::new();
                        let mut result = database.execute(&prepared, &cancellation).unwrap();
                        let mut finished = false;
                        for _ in 0..10_000 {
                            match result.step() {
                                pipesql::QueryStep::Rows(batch) => assert!(!batch.is_empty()),
                                pipesql::QueryStep::Progress => (),
                                pipesql::QueryStep::Finished => {
                                    finished = true;
                                    break;
                                }
                                pipesql::QueryStep::Failed(error) => {
                                    panic!("public query failed: {error:?}")
                                }
                            }
                        }
                        assert!(finished, "query exceeds finite fixture step allowance");
                    })
                    .unwrap()
                    .join()
                    .unwrap();
            });
        }
        database.close().unwrap();
        eprintln!("bounded load child completed");
        return;
    }
    // Empty metadata, a nonempty unit, and both DOUBLE/date block boundaries.
    for rows in [0, 1, 65_537] {
        let temp = TempDir::new();
        let path = temp.0.join("database");
        let input = temp.0.join("input.tbl");
        fs::write(&input, ROW.repeat(rows)).unwrap();
        Database::create(&path, config()).unwrap().close().unwrap();
        let diagnostics = temp.0.join("child.err");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "stack::public_load_and_queries_fit_declared_stack_headroom",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("PIPESQL_STACK_DATABASE", &path)
            .env("PIPESQL_STACK_INPUT", &input)
            .stdout(std::process::Stdio::null())
            .stderr(fs::File::create(&diagnostics).unwrap())
            .spawn()
            .unwrap();
        let mut status = None;
        for _ in 0..1_000 {
            status = child.try_wait().unwrap();
            if status.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if status.is_none() {
            child.kill().unwrap();
        }
        let reaped = child.wait().unwrap();
        assert!(fs::metadata(&diagnostics).unwrap().len() <= 4_096);
        let diagnostics = fs::read_to_string(diagnostics).unwrap();
        assert!(
            status.is_some() && reaped.success(),
            "public load exceeds its declared stack headroom or failed: {}",
            diagnostics
        );
        assert!(
            diagnostics.contains("bounded load child completed"),
            "{rows} rows: selected child did not complete the stack scenario"
        );
    }
}
