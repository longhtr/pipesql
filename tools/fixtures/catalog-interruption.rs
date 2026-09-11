//! Stock public catalog operations with an external native termination observer.
use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues,
    CommitResolution, Config, DataType, Database, Error, QueryStep, TransactionId, Value,
};
use std::os::fd::AsRawFd;
use std::path::Path;
unsafe extern "C" {
    fn interruption_start(fd: i32, cut: u32);
    fn interruption_stop();
}
fn config() -> Config {
    Config::new(4_000_000, 8_000_000).unwrap()
}
fn limits() -> AppendLimits {
    AppendLimits {
        batches: 2,
        encoded_bytes: 100_000,
    }
}
fn token(db: &Database, sequence: u64) -> TransactionId {
    let mut bytes = [0; 24];
    bytes[..16].copy_from_slice(db.database_identity().as_bytes());
    bytes[16..].copy_from_slice(&sequence.to_le_bytes());
    TransactionId::from_bytes(bytes).unwrap()
}
fn append(db: &Database, first: i64, second: i64, sequence: u64) {
    let cancel = CancellationToken::new();
    let mut writer = db.begin_append("facts", limits(), &cancel).unwrap();
    assert_eq!(writer.transaction(), token(db, sequence));
    for (key, text, valid) in [(first, "snow 雪\0", 1), (second, "", 0)] {
        writer
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&[key]),
                        validity: &[1],
                    },
                    ColumnInput {
                        values: ColumnValues::String(&[text]),
                        validity: &[valid],
                    },
                ],
                &cancel,
            )
            .unwrap();
    }
    let committed = writer.commit(&cancel).unwrap();
    assert_eq!(committed.transaction(), token(db, sequence));
}
fn rows(db: &Database, appended: bool, retried: bool) {
    let query = db.prepare("FROM facts").unwrap();
    let cancel = CancellationToken::new();
    let mut result = db.execute(&query, &cancel).unwrap();
    let mut seen = [0_u8; 6];
    for _ in 0..10_000 {
        match result.step() {
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 2);
                for row in 0..batch.len() {
                    let Some(Value::Int64(key)) = batch.value(row, 0) else {
                        panic!("typed key");
                    };
                    let slot = [7, i64::MAX, 11, 13, 17, 19]
                        .iter()
                        .position(|&n| n == key)
                        .expect("known key");
                    if slot % 2 == 0 {
                        assert!(
                            matches!(batch.value(row, 1), Some(Value::String(text)) if text.as_str() == "snow 雪\0")
                        );
                    } else {
                        assert!(matches!(batch.value(row, 1), Some(Value::Null)));
                    }
                    seen[slot] = seen[slot].checked_add(1).unwrap();
                }
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                assert_eq!(
                    seen,
                    [
                        1,
                        1,
                        u8::from(appended),
                        u8::from(appended),
                        u8::from(retried),
                        u8::from(retried)
                    ],
                    "independent row history"
                );
                return;
            }
            QueryStep::Failed(error) => panic!("unexpected query outcome: {error:?}"),
        }
    }
    panic!("finite row check exceeded");
}
fn durable(db: &Database, attempt: u64, generation: u64) {
    let CommitResolution::Durable(commit) = db.resolve_commit(token(db, attempt)).unwrap() else {
        panic!("missing durable receipt");
    };
    assert_eq!(commit.transaction(), token(db, attempt));
    assert_eq!(commit.generation(), generation);
}
fn history(db: &Database, state: u32) {
    assert!(state <= 2);
    assert_eq!(
        db.generation(),
        if state == 2 { 3 } else { 2 },
        "independent generation history"
    );
    durable(db, 1, 1);
    durable(db, 2, 2);
    assert_eq!(
        db.resolve_commit(token(db, 3)).unwrap(),
        CommitResolution::Aborted
    );
    match state {
        0 => assert!(matches!(
            db.resolve_commit(token(db, 4)),
            Err(Error::NotFound)
        )),
        1 => assert_eq!(
            db.resolve_commit(token(db, 4)).unwrap(),
            CommitResolution::Aborted
        ),
        2 => durable(db, 4, 3),
        _ => unreachable!(),
    }
    assert!(matches!(
        db.resolve_commit(token(db, 5)),
        Err(Error::NotFound)
    ));
    rows(db, state == 2, false);
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 6);
    let path = Path::new(&args[1]);
    let mode = args[2].as_str();
    let trace = std::fs::File::create(&args[3]).unwrap();
    let cut = args[4].parse().unwrap();
    let state = args[5].parse().unwrap();
    if mode == "setup" {
        let db = Database::create_empty(path, config()).unwrap();
        db.declare_table(
            "facts",
            &[
                ColumnDeclaration {
                    name: "k",
                    data_type: DataType::Int64,
                    nullable: false,
                },
                ColumnDeclaration {
                    name: "s",
                    data_type: DataType::String,
                    nullable: true,
                },
            ],
            &CancellationToken::new(),
        )
        .unwrap();
        append(&db, 7, i64::MAX, 2);
        let aborted = db
            .begin_append("facts", limits(), &CancellationToken::new())
            .unwrap();
        assert_eq!(aborted.transaction(), token(&db, 3));
        aborted.abort().unwrap();
        history(&db, 0);
        db.close().unwrap();
    } else if mode == "append" {
        let db = Database::open(path, config()).unwrap();
        // SAFETY: single-threaded caller retains the trace descriptor; C checks
        // the cut bound and never retains a Rust pointer or changes engine bytes.
        unsafe {
            interruption_start(trace.as_raw_fd(), cut);
        }
        append(&db, 11, 13, 4);
        unsafe {
            interruption_stop();
        }
        history(&db, 2);
        db.close().unwrap();
    } else if mode == "recover" {
        // SAFETY: same exclusively owned scalar observer state as above.
        unsafe {
            interruption_start(trace.as_raw_fd(), cut);
        }
        let db = Database::open(path, config()).unwrap();
        unsafe {
            interruption_stop();
        }
        history(&db, state);
        db.close().unwrap();
    } else if mode == "wrong-rows" {
        let db = Database::open(path, config()).unwrap();
        rows(&db, false, false);
    } else if mode == "wrong-receipt" {
        let db = Database::open(path, config()).unwrap();
        durable(&db, 3, 3);
    } else if mode == "verify" {
        let db = Database::open(path, config()).unwrap();
        history(&db, state);
        let next = if state == 0 { 4 } else { 5 };
        append(&db, 17, 19, next);
        rows(&db, state == 2, true);
        durable(&db, next, if state == 2 { 4 } else { 3 });
        if state == 1 {
            assert_eq!(
                db.resolve_commit(token(&db, 4)).unwrap(),
                CommitResolution::Aborted
            );
        }
        db.close().unwrap();
        let db = Database::open(path, config()).unwrap();
        rows(&db, state == 2, true);
        durable(&db, 1, 1);
        durable(&db, 2, 2);
        assert_eq!(
            db.resolve_commit(token(&db, 3)).unwrap(),
            CommitResolution::Aborted
        );
        if state == 1 {
            assert_eq!(
                db.resolve_commit(token(&db, 4)).unwrap(),
                CommitResolution::Aborted
            );
        }
        if state == 2 {
            durable(&db, 4, 3);
        }
        durable(&db, next, if state == 2 { 4 } else { 3 });
        assert_eq!(db.reserved_temp_bytes(), 0);
        db.close().unwrap();
    } else {
        panic!("unknown fixture mode");
    }
    println!("catalog interruption {mode} passed state={state}");
}
