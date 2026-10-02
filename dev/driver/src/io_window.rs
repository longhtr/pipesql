//! Export spilled running-SUM peers while native transfers are short or fail.
//!
//! The caller sink holds bytes in memory, so observed writes belong to database
//! scratch files. Setup, saved output and healthy retries run outside observation.
//! Complete typed answers and prefixes belong to the independent supervisor.

use super::io_capture::{descriptors, export, released};
use super::*;
use pipesql::ExportLimits;
use std::path::Path;

const ROWS: usize = 1539;
const SQL: &str = "FROM facts |> EXTEND SUM(amount) OVER (PARTITION BY part ORDER BY peer) AS total |> SELECT id, part, peer, total, note";
const LIMITS: ExportLimits = ExportLimits {
    rows: ROWS as u64,
    bytes: 2_000_000,
};

fn create(path: &Path) {
    let db = Database::create_empty(path, Config::new(8_000_000, 8_000_000).unwrap()).unwrap();
    let cancel = CancellationToken::new();
    let mut schema = ["id", "part", "peer", "amount", "note"].map(|name| ColumnDeclaration {
        name,
        data_type: DataType::Int64,
        nullable: name != "id" && name != "note",
    });
    schema[4].data_type = DataType::String;
    db.declare_table("facts", &schema, &cancel).unwrap();
    let mut append = db
        .begin_append(
            "facts",
            AppendLimits {
                batches: 6,
                encoded_bytes: 1_000_000,
            },
            &cancel,
        )
        .unwrap();
    for start in (0..ROWS).step_by(257) {
        let length = (ROWS - start).min(257);
        let ids: Vec<_> = (start..start + length)
            .map(|row| (ROWS - 1 - row) as i64)
            .collect();
        let mut part = Vec::with_capacity(length);
        let mut peer = Vec::with_capacity(length);
        let mut amount = Vec::with_capacity(length);
        let mut note = Vec::with_capacity(length);
        let mut present = [[0_u8; 33]; 5];
        for (row, &id) in ids.iter().enumerate() {
            let group = id as usize / 513;
            let within = id as usize % 513;
            part.push(1);
            peer.push(1);
            let value = match (group, within) {
                (0, 0..256) => Some(i64::MAX),
                (0, 256..512) => Some(-i64::MAX),
                (0, 512) => Some(7),
                (1, 0) => Some(9),
                _ => None,
            };
            amount.push(value.unwrap_or(0));
            note.push(format!("{id:04}雪\0{}", "x".repeat(504)));
            for (column, valid) in [true, group == 2, group == 1, value.is_some(), true]
                .into_iter()
                .enumerate()
            {
                if valid {
                    present[column][row / 8] |= 1 << (row % 8);
                }
            }
        }
        let text: Vec<_> = note.iter().map(String::as_str).collect();
        append
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&ids),
                        validity: &present[0][..length.div_ceil(8)],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&part),
                        validity: &present[1][..length.div_ceil(8)],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&peer),
                        validity: &present[2][..length.div_ceil(8)],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&amount),
                        validity: &present[3][..length.div_ceil(8)],
                    },
                    ColumnInput {
                        values: ColumnValues::String(&text),
                        validity: &present[4][..length.div_ceil(8)],
                    },
                ],
                &cancel,
            )
            .unwrap();
    }
    append.commit(&cancel).unwrap();
    db.close().unwrap();
}

pub(super) fn run(root: &Path, kind: u32, at: u32, burst: u32, error: i32) {
    let path = root.join("database");
    let before = descriptors();
    create(&path);
    let config = Config::new(3_000_000, 8_000_000).unwrap();
    let db = Database::open(&path, config).unwrap();
    for name in ["CONTROL", "ROOT.A", "ROOT.B", "WAL"] {
        std::fs::copy(path.join(name), root.join(name)).unwrap();
    }
    let query = db.prepare(SQL).unwrap();
    let memory = db.reserved_memory_bytes();
    let files = descriptors();
    let generation = db.generation();
    // SAFETY: setup and preparation have finished. The synchronous export owns
    // this observer interval, including query disposal after an I/O error.
    unsafe {
        io_probe_start(kind, at, burst, error);
    }
    let (initial, result) = export(&db, &query, &path, LIMITS);
    unsafe {
        io_probe_stop();
    }
    let (calls, refused, partial) =
        unsafe { (io_probe_calls(), io_probe_refused(), io_probe_partial()) };
    let original = format!("{result:?}");
    println!("calls={calls} refused={refused} partial={partial} result={original}");
    if at == 0 || error == 0 {
        assert!(matches!(result, Ok(rows) if rows == ROWS as u64));
        assert!(initial.temporary > 0);
        if cfg!(target_os = "linux") {
            assert!(initial.spill > 0, "window I/O did not write spill");
        }
    } else {
        assert!(
            result
                .as_ref()
                .is_err_and(|source| injected_error(source, error.abs())),
            "window I/O lost the original OS error: {result:?}"
        );
    }
    // Preserve the original error until after releasing query owners and
    // formatting the report; saving caller bytes does not complete the query.
    released(&db, memory, files);
    assert_eq!(db.generation(), generation);
    println!(
        "window initial outcome={} bytes={} spill={} temp={}",
        if result.is_ok() { "finished" } else { "failed" },
        initial.bytes.len(),
        initial.spill,
        initial.temporary
    );
    std::fs::write(root.join("initial.jsonl"), &initial.bytes).unwrap();
    let (retry, retried) = export(&db, &query, &path, LIMITS);
    assert!(matches!(retried, Ok(rows) if rows == ROWS as u64));
    assert!(retry.temporary > 0);
    if cfg!(target_os = "linux") {
        assert!(retry.spill > 0);
    }
    released(&db, memory, files);
    println!(
        "window retry rows={} spill={} temp={}",
        retried.unwrap(),
        retry.spill,
        retry.temporary
    );
    std::fs::write(root.join("retry.jsonl"), &retry.bytes).unwrap();
    drop(query);
    db.close().unwrap();
    let db = Database::open(&path, config).unwrap();
    assert_eq!(db.generation(), generation);
    let query = db.prepare(SQL).unwrap();
    let memory = db.reserved_memory_bytes();
    let files = descriptors();
    let (reopened, retried) = export(&db, &query, &path, LIMITS);
    assert!(matches!(retried, Ok(rows) if rows == ROWS as u64));
    assert!(reopened.temporary > 0);
    if cfg!(target_os = "linux") {
        assert!(reopened.spill > 0);
    }
    released(&db, memory, files);
    println!(
        "window reopened rows={} spill={} temp={}",
        retried.unwrap(),
        reopened.spill,
        reopened.temporary
    );
    std::fs::write(root.join("reopened.jsonl"), &reopened.bytes).unwrap();
    drop(query);
    db.close().unwrap();
    assert_eq!(format!("{result:?}"), original);
    assert_eq!(descriptors(), before);
    for name in ["CONTROL", "ROOT.A", "ROOT.B", "WAL"] {
        assert_eq!(
            std::fs::read(path.join(name)).unwrap(),
            std::fs::read(root.join(name)).unwrap()
        );
    }
    println!("window I/O release and complete retries passed");
}
