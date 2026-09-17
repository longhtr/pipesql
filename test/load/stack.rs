//! Check that legacy loading and querying complete on ordinary and small stacks.
//!
//! Each case loads zero, one or 65,537 rows, then runs Q6 and Q1. The largest
//! input crosses the numeric and date block boundaries. Loading must release its
//! memory and temporary-file reservations; queries must finish within the step
//! allowance. These cases check completion, not the queries' numeric answers.
//!
//! The database is opened and queries are prepared outside the measured workers.
//! Each small-stack worker checks its native stack extent before calling the
//! engine. Run in the release profile: debug code has different stack needs.
//! A parent process enforces a deadline and checks both the child's exit status
//! and its completion marker, so an empty test selection cannot count as success.

use super::{ROW, TempDir, child::ChildProcess, config};
use pipesql::{CancellationToken, Config, Database};
use std::{fs, path::PathBuf, process::Command};

#[test]
fn public_load_and_queries_preserve_boundaries_and_release_owners() {
    check_load_and_queries(false);
}

#[test]
fn public_load_and_queries_fit_declared_stack_headroom() {
    check_load_and_queries(true);
}

fn check_load_and_queries(small_stack: bool) {
    const CHILD: &str = "PIPESQL_LOAD_STACK_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let path = PathBuf::from(std::env::var_os("PIPESQL_STACK_DATABASE").unwrap());
        let input = PathBuf::from(std::env::var_os("PIPESQL_STACK_INPUT").unwrap());
        // Allocate the database handle before spawning the worker; the worker
        // receives a pointer instead of moving the handle onto its small stack.
        let mut database =
            Box::new(Database::open(&path, Config::new(2_000_000, 8_000_000).unwrap()).unwrap());
        let resident = database.reserved_memory_bytes();
        let database = worker("public-load", small_stack)
            .spawn(move || {
                if small_stack {
                    pipesql_filesystem::test_assert_small_stack();
                }
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
            include_str!("../data/q6.pipe.sql"),
            include_str!("../data/upstream/q1-upstream.pipe.sql"),
        ] {
            let prepared = database.prepare(sql).unwrap();
            std::thread::scope(|scope| {
                worker("public-query", small_stack)
                    .spawn_scoped(scope, || {
                        if small_stack {
                            pipesql_filesystem::test_assert_small_stack();
                        }
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
        eprintln!("load/query child completed");
        return;
    }
    for rows in [0, 1, 65_537] {
        let temp = TempDir::new();
        let path = temp.0.join("database");
        let input = temp.0.join("input.tbl");
        fs::write(&input, ROW.repeat(rows)).unwrap();
        Database::create(&path, config()).unwrap().close().unwrap();
        let diagnostics = temp.0.join("child.err");
        let selected = if small_stack {
            "stack::public_load_and_queries_fit_declared_stack_headroom"
        } else {
            "stack::public_load_and_queries_preserve_boundaries_and_release_owners"
        };
        let mut child = ChildProcess(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", selected, "--nocapture"])
                .env(CHILD, "1")
                .env("PIPESQL_STACK_DATABASE", &path)
                .env("PIPESQL_STACK_INPUT", &input)
                .stdout(std::process::Stdio::null())
                .stderr(fs::File::create(&diagnostics).unwrap())
                .spawn()
                .unwrap(),
        );
        let mut status = None;
        for _ in 0..1_000 {
            status = child.0.try_wait().unwrap();
            if status.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if status.is_none() {
            child.0.kill().unwrap();
        }
        let exit_status = child.0.wait().unwrap();
        assert!(fs::metadata(&diagnostics).unwrap().len() <= 4_096);
        let diagnostics = fs::read_to_string(diagnostics).unwrap();
        assert!(
            status.is_some() && exit_status.success(),
            "load/query child failed or exceeded its deadline: {}",
            diagnostics
        );
        assert!(
            diagnostics.contains("load/query child completed"),
            "{rows} rows: selected child did not complete the load/query scenario"
        );
    }
}

fn worker(name: &str, small_stack: bool) -> std::thread::Builder {
    let thread = std::thread::Builder::new().name(name.into());
    if small_stack {
        // The worker checks the actual extent because the OS may round this
        // requested size up to a larger stack.
        thread.stack_size(pipesql_filesystem::TEST_SMALL_STACK_REQUEST_BYTES)
    } else {
        thread
    }
}
