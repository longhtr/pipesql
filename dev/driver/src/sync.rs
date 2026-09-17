//! Check database outcomes when a native synchronization call returns an error.
//!
//! Setup finishes before the observer is armed. The selected operation then runs
//! with a chosen failure position; observation stops before reopening or retrying.
//! File and directory primitives must make one attempt. Database operations must
//! propagate the refusal without substituting weaker synchronization, and a query
//! must perform no synchronization at all.
//!
//! With faults disabled, creation must be retryable after removing partial state.
//! Loading and recovery must retain a visible commit token or allow a fresh load
//! when no token was returned. These are same-machine reopen checks: the caller
//! does not simulate lost cached writes. The development runner supplies fresh
//! processes, records artifact identities and removes each case directory.

use pipesql::{
    CancellationToken, CommitResolution, Config, Database, Error, QueryResult, QueryStep,
};
use std::path::PathBuf;
mod sync_observer;
use sync_observer::*;
fn consume(mut rows: QueryResult<'_, '_>) -> Result<(), Error> {
    for _ in 0..10_000 {
        match rows.step() {
            QueryStep::Rows(batch) => assert!(!batch.is_empty()),
            QueryStep::Progress => (),
            QueryStep::Finished => return Ok(()),
            QueryStep::Failed(_) => return Err(rows.into_error().expect("terminal query error")),
        }
    }
    panic!("query exceeded finite fixture step allowance");
}
fn main() {
    sync_observer::load();
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 6);
    let root = PathBuf::from(&args[1]);
    let mode = args[2].as_str();
    let at = args[3].parse::<u32>().unwrap();
    let burst = args[4].parse::<u32>().unwrap();
    let error = args[5].parse::<i32>().unwrap();
    let db_path = root.join("database");
    let input = root.join("input.tbl");
    std::fs::write(
        &input,
        b"1|2|3|4|17.00|21168.23|0.04|8|R|F|1996-03-13|12|13|14|15|16|\n",
    )
    .unwrap();
    let config = Config::new(2_000_000, 1_000_000).unwrap();
    let mut database = if matches!(mode, "load" | "recover" | "query") {
        Some(Database::create(&db_path, config).unwrap())
    } else {
        None
    };
    let mut token = None;
    if matches!(mode, "recover" | "query") {
        token = Some(
            database
                .as_mut()
                .unwrap()
                .load_lineitem(&input, &CancellationToken::new())
                .unwrap()
                .transaction(),
        );
    }
    if mode == "recover" {
        drop(database.take());
        // Removing one root makes open repair the persisted replicas, giving
        // the observer synchronization work to refuse during recovery.
        std::fs::remove_file(db_path.join("ROOT.B")).unwrap();
    }
    let file = std::fs::File::create(root.join("file")).unwrap();
    let directory = std::fs::File::open(&root).unwrap();
    // SAFETY: only this thread uses the observer. The observer checks argument bounds;
    // the call passes integers and retains no pointer into Rust memory.
    unsafe { sync_probe_start(at, burst, error) };
    let result = match mode {
        "weak" => {
            use std::os::fd::AsRawFd;
            // SAFETY: the descriptor remains owned and open across the call.
            #[cfg(target_os = "linux")]
            let result = unsafe { libc::fdatasync(file.as_raw_fd()) };
            #[cfg(target_os = "macos")]
            let result = unsafe { libc::fsync(file.as_raw_fd()) };
            if result == 0 {
                Ok(())
            } else {
                Err(format!("{}", std::io::Error::last_os_error()))
            }
        }
        #[cfg(target_os = "macos")]
        "barrier" => {
            use std::os::fd::AsRawFd;
            // SAFETY: F_BARRIERFSYNC takes no variadic argument; file is live.
            let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_BARRIERFSYNC) };
            if result == 0 {
                Ok(())
            } else {
                Err(format!("{}", std::io::Error::last_os_error()))
            }
        }
        "standard" => file.sync_all().map_err(|error| format!("{error:?}")),
        "file" => pipesql_filesystem::sync_all(&file).map_err(|error| format!("{error:?}")),
        "directory" => {
            pipesql_filesystem::sync_all(&directory).map_err(|error| format!("{error:?}"))
        }
        "create" => Database::create(&db_path, config)
            .map(drop)
            .map_err(|error| format!("{error:?}")),
        "recover" => Database::open(&db_path, config)
            .map(drop)
            .map_err(|error| format!("{error:?}")),
        "load" => {
            let result = database
                .as_mut()
                .unwrap()
                .load_lineitem(&input, &CancellationToken::new());
            match &result {
                Ok(commit) => token = Some(commit.transaction()),
                Err(Error::CommitAmbiguous { transaction, .. }) => token = Some(*transaction),
                Err(_) => {}
            }
            result.map(|_| ()).map_err(|error| format!("{error:?}"))
        }
        "query" => {
            let db = database.as_ref().unwrap();
            let plan = db
                .prepare(include_str!("../../../test/data/q6.pipe.sql"))
                .unwrap();
            let cancellation = CancellationToken::new();
            db.execute(&plan, &cancellation)
                .and_then(consume)
                .map_err(|error| format!("{error:?}"))
        }
        _ => panic!("unknown native sync mode"),
    };
    // SAFETY: the synchronous operation has returned, so changing observer
    // state cannot race a call still using it.
    unsafe { sync_probe_stop() };
    let (calls, refused, weak) =
        unsafe { (sync_probe_calls(), sync_probe_refused(), sync_probe_weak()) };
    println!("calls={calls} refused={refused} weak={weak} result={result:?}");
    if matches!(mode, "weak" | "barrier") {
        assert_eq!(
            (calls, refused, weak),
            (0, 0, 1),
            "weaker call was not observed"
        );
        return;
    }
    assert_eq!(
        weak, 0,
        "full sync must not fall back to a weaker primitive"
    );
    if mode == "query" {
        assert_eq!(calls, 0, "read-only query must not synchronize or repair");
        assert!(result.is_ok());
    } else if at == 0 {
        assert!(result.is_ok());
    } else if mode != "standard" {
        assert!(result.is_err(), "native sync refusal must propagate");
    }
    if matches!(mode, "file" | "directory") {
        assert_eq!(calls, 1, "one synchronization attempt cannot retry");
        assert_eq!(refused, u32::from(at != 0));
    }
    drop(database);
    if matches!(mode, "load" | "recover" | "query") {
        let mut healed = Database::open(&db_path, config).unwrap();
        if let Some(transaction) = token {
            assert!(
                matches!(
                    healed.resolve_commit(transaction).unwrap(),
                    CommitResolution::Durable(_)
                ),
                "visible published graph or prior acknowledgement was lost"
            );
            assert_eq!(healed.generation(), 1);
        } else {
            assert_eq!(healed.generation(), 0);
            healed
                .load_lineitem(&input, &CancellationToken::new())
                .unwrap();
            assert_eq!(healed.generation(), 1);
        }
    } else if mode == "create" {
        if result.is_err() {
            assert!(
                !db_path.exists(),
                "native flush-only refusal left a partial create namespace"
            );
            Database::create(&db_path, config).unwrap().close().unwrap();
        } else {
            Database::open(&db_path, config).unwrap().close().unwrap();
        }
    }
}
