//! Export duplicate matches and unmatched rows under native I/O faults.
//!
//! The right text group crosses both platforms' transfer buffers; its full input
//! exceeds the initial sort arena. Fixed-key numeric projections exercise batched
//! matching; STRING keys use scalar matching. The supervisor derives pair answers
//! independently from input ranges.
//! Typed key fixtures roundtrip original bits, days or text bytes before native
//! export.

use super::io_capture::{Output, descriptors, export, released};
use super::*;
use pipesql::{DateValue, ExportLimits, PreparedQuery};
use std::fmt::Write as _;
use std::path::Path;

const LEFT: usize = 40;
const RIGHT: usize = 130;
const BYTES: u64 = 4_000_000;

struct Fixture {
    kind: DataType,
    text: bool,
    outer: bool,
}
impl Fixture {
    fn new(mode: &str) -> Self {
        let kind = match mode {
            "join-inner" | "join-left" | "join-numeric" => DataType::Int64,
            "join-double" | "join-double-numeric" => DataType::Double,
            "join-date" | "join-date-numeric" => DataType::Date,
            "join-string" | "join-string-numeric" => DataType::String,
            _ => panic!("unknown native join fixture"),
        };
        Self {
            kind,
            text: !mode.ends_with("numeric"),
            outer: mode != "join-inner",
        }
    }
    fn right_rows(&self) -> usize {
        if self.kind == DataType::Int64 {
            RIGHT
        } else if self.kind == DataType::String && !self.text {
            767
        } else if self.text {
            131
        } else {
            4096
        }
    }
    fn rows(&self) -> u64 {
        if self.kind == DataType::Int64 {
            if self.outer { 1574 } else { 1568 }
        } else if self.text {
            1190
        } else if self.kind == DataType::String {
            1316
        } else {
            6178
        }
    }
    fn cancel_bytes(&self) -> usize {
        match (self.kind, self.text) {
            (DataType::Int64, _) => 120_000,
            (DataType::Double, true) => 400_000,
            (DataType::Date, true) => 800_000,
            (DataType::Double, false) => 160_000,
            (DataType::Date, false) => 190_000,
            (DataType::String, true) => 350_000,
            (DataType::String, false) => 35_000,
        }
    }
    // These categories construct inputs only. The supervisor has separate
    // literal pair ranges and checks the stored source before judging output.
    fn group(&self, left: bool, id: usize) -> Option<u8> {
        if left {
            match id {
                4..20 if self.text => Some(0),
                20..28 if self.text => Some(1),
                28..36 if self.text => Some(2),
                4..6 if !self.text => Some(0),
                20 if !self.text => Some(1),
                28 if !self.text => Some(2),
                36..38 => Some(3),
                _ => None,
            }
        } else if self.text {
            match id {
                3..51 => Some(0),
                51..75 => Some(1),
                75..99 => Some(2),
                99..115 => Some(3),
                _ => None,
            }
        } else if self.kind == DataType::String {
            match id {
                3..516 => Some(0),
                516..644 => Some(1),
                644..708 => Some(2),
                708..740 => Some(3),
                _ => None,
            }
        } else {
            match id {
                3..2051 => Some(0),
                2051..3075 => Some(1),
                3075..3587 => Some(2),
                3587..3843 => Some(3),
                _ => None,
            }
        }
    }
    fn double(&self, left: bool, id: usize) -> Option<f64> {
        if id == 0 {
            return None;
        }
        if id == 1 {
            return Some(f64::from_bits(0x7ff8_0000_0000_0042));
        }
        if id == 2 {
            return Some(f64::from_bits(if left {
                0x7ff8_0000_0000_0099
            } else {
                0xfff8_0000_0000_0007
            }));
        }
        let negative_zero = if left {
            if self.text {
                id.is_multiple_of(3)
            } else {
                id == 5
            }
        } else {
            id.is_multiple_of(5)
        };
        Some(match self.group(left, id) {
            // Vary signs independently; matching must cross both signs.
            Some(0) if negative_zero => -0.0,
            Some(0) => 0.0,
            Some(1) => f64::INFINITY,
            Some(2) => f64::MAX,
            Some(3) => f64::NEG_INFINITY,
            _ if left && id == 3 => f64::from_bits(1),
            _ if left => 1.0,
            _ if id == self.right_rows() - 1 => f64::from_bits(0x8000_0000_0000_0001),
            _ if id == if self.text { 115 } else { 3843 } => -f64::MAX,
            _ => 2.0,
        })
    }
    fn day(&self, left: bool, id: usize) -> Option<i32> {
        if id < 2 {
            return None;
        }
        Some(match self.group(left, id) {
            Some(0) => 0,
            Some(1) => -719_162,
            Some(2) => 2_932_896,
            Some(3) => -1,
            _ if left => 1,
            _ => 2,
        })
    }

    fn string(&self, left: bool, id: usize) -> Option<String> {
        if id == 0 {
            return None;
        }
        if id == 1 {
            return Some(String::new());
        }
        if id == 2 {
            return Some("a\0b".into());
        }
        let width = if self.text { 3072 } else { 512 };
        let wide = |suffix: &str| format!("K\0{}{suffix}", "x".repeat(width - 3));
        let tail = if self.text { 115 } else { 740 };
        Some(match self.group(left, id) {
            Some(0) => wide("A"),
            Some(1) => wide("B"),
            Some(2) => "é".into(),
            Some(3) => "e\u{301}".into(),
            _ if left && id == 3 => "a\0c".into(),
            _ if left && id == 38 => "K".into(),
            _ if left && id == 39 => wide("C"),
            _ if left => "unmatched-left".into(),
            _ if id == tail => wide(""),
            _ if id == tail + 1 => wide("D"),
            _ if id == tail + 2 => "a\0bZ".into(),
            _ if id == tail + 3 => "null".into(),
            _ => "unmatched-right".into(),
        })
    }
}

fn key(left: bool, id: usize) -> Option<i64> {
    if left {
        match id {
            0..2 => None,
            2..4 => Some(-1),
            4..20 => Some(0),
            20..36 => Some(1),
            36..38 => Some(2),
            _ => Some(3),
        }
    } else {
        match id {
            0..2 => None,
            2..50 => Some(0),
            50..98 => Some(1),
            98..114 => Some(2),
            _ => Some(4),
        }
    }
}

fn create(path: &Path, fixture: &Fixture) {
    let db = Database::create_empty(path, Config::new(12_000_000, 8_000_000).unwrap()).unwrap();
    let cancel = CancellationToken::new();
    // Larger numeric inputs use fewer stored units to keep native source reads
    // within the sweep bound. Their unused notes stay short for the append limit.
    let batch = if fixture.kind == DataType::String {
        // Keep each stored STRING batch within the typed scan's 64 KiB owner.
        if fixture.text { 16 } else { 128 }
    } else if !fixture.text && fixture.kind != DataType::Int64 {
        256
    } else {
        32
    };
    for (left, name, count) in [
        (true, "left_rows", LEFT),
        (false, "right_rows", fixture.right_rows()),
    ] {
        let mut schema = ["id", "k", "v", "note"].map(|name| ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: matches!(name, "k" | "v"),
        });
        schema[3].data_type = DataType::String;
        schema[1].data_type = fixture.kind;
        db.declare_table(name, &schema, &cancel).unwrap();
        let mut append = db
            .begin_append(
                name,
                AppendLimits {
                    batches: count.div_ceil(batch) as u32,
                    encoded_bytes: 1_000_000,
                },
                &cancel,
            )
            .unwrap();
        for start in (0..count).step_by(batch) {
            let length = (count - start).min(batch);
            let ids: Vec<_> = (start..start + length)
                .map(|row| {
                    if fixture.kind == DataType::String {
                        let (factor, offset) = if left { (7, 11) } else { (37, 17) };
                        ((row * factor + offset) % count) as i64
                    } else {
                        (count - 1 - row) as i64
                    }
                })
                .collect();
            let mut keys = Vec::new();
            let mut doubles = Vec::new();
            let mut days = Vec::new();
            let mut strings = Vec::new();
            match fixture.kind {
                DataType::Int64 => keys.reserve_exact(length),
                DataType::Double => doubles.reserve_exact(length),
                DataType::Date => days.reserve_exact(length),
                DataType::String => strings.reserve_exact(length),
            }
            let mut values = Vec::with_capacity(length);
            let mut notes = Vec::with_capacity(length);
            let mut present = [[0_u8; 32]; 4];
            for (row, &id) in ids.iter().enumerate() {
                let key_present = match fixture.kind {
                    DataType::Int64 => {
                        let key = key(left, id as usize);
                        keys.push(key.unwrap_or(0));
                        key.is_some()
                    }
                    DataType::Double => {
                        let key = fixture.double(left, id as usize);
                        doubles.push(key.unwrap_or(0.0));
                        key.is_some()
                    }
                    DataType::Date => {
                        let key = fixture.day(left, id as usize);
                        days.push(DateValue::from_days_since_unix_epoch(key.unwrap_or(0)).unwrap());
                        key.is_some()
                    }
                    DataType::String => {
                        let key = fixture.string(left, id as usize);
                        let present = key.is_some();
                        strings.push(key.unwrap_or_default());
                        present
                    }
                };
                let valid = id % if left { 3 } else { 5 } != 0;
                values.push(if left { id * 7 - 30 } else { id - 70 });
                notes.push(format!(
                    "{}{id:04}雪\0{}",
                    if left { 'L' } else { 'R' },
                    "x".repeat(
                        if left
                            || fixture.kind == DataType::String
                            || (!fixture.text && fixture.kind != DataType::Int64)
                        {
                            7
                        } else {
                            2039
                        }
                    )
                ));
                for (column, valid) in [true, key_present, valid, true].into_iter().enumerate() {
                    if valid {
                        present[column][row / 8] |= 1 << (row % 8);
                    }
                }
            }
            let text: Vec<_> = notes.iter().map(String::as_str).collect();
            let string_keys: Vec<_> = strings.iter().map(String::as_str).collect();
            let key_values = match fixture.kind {
                DataType::Int64 => ColumnValues::Int64(&keys),
                DataType::Double => ColumnValues::Double(&doubles),
                DataType::Date => ColumnValues::Date(&days),
                DataType::String => ColumnValues::String(&string_keys),
            };
            append
                .write(
                    &[
                        ColumnInput {
                            values: ColumnValues::Int64(&ids),
                            validity: &present[0][..length.div_ceil(8)],
                        },
                        ColumnInput {
                            values: key_values,
                            validity: &present[1][..length.div_ceil(8)],
                        },
                        ColumnInput {
                            values: ColumnValues::Int64(&values),
                            validity: &present[2][..length.div_ceil(8)],
                        },
                        ColumnInput {
                            values: ColumnValues::String(&text),
                            validity: &present[3][..length.div_ceil(8)],
                        },
                    ],
                    &cancel,
                )
                .unwrap();
        }
        append.commit(&cancel).unwrap();
    }
    db.close().unwrap();
}

// Read the committed source through its public typed API before arming faults.
// The roundtrip checks original bits, days or bytes, without calculating pairs.
// Save observed cells for the supervisor's separate source and relation checks.
fn source_keys(db: &Database, fixture: &Fixture, root: &Path) {
    let memory = db.reserved_memory_bytes();
    let files = descriptors();
    let cancel = CancellationToken::new();
    let mut bytes = String::new();
    for (left, name, count) in [
        (true, "left_rows", LEFT),
        (false, "right_rows", fixture.right_rows()),
    ] {
        let query = db.prepare(&format!("FROM {name} |> SELECT id, k")).unwrap();
        let column = query.result_column(1).unwrap();
        assert_eq!(column.data_type, fixture.kind);
        assert!(column.nullable);
        writeln!(
            bytes,
            "source {name} type={:?} rows={count}",
            column.data_type
        )
        .unwrap();
        let mut seen = vec![false; count];
        let mut result = db.execute(&query, &cancel).unwrap();
        loop {
            match result.step() {
                QueryStep::Rows(batch) => {
                    for row in 0..batch.len() {
                        let Some(Value::Int64(id)) = batch.value(row, 0) else {
                            panic!("source identity type");
                        };
                        let id = usize::try_from(id).unwrap();
                        assert!(!std::mem::replace(&mut seen[id], true));
                        let value = batch.value(row, 1).unwrap();
                        match fixture.kind {
                            DataType::Double => match (value, fixture.double(left, id)) {
                                (Value::Null, None) => writeln!(bytes, "{name} {id} null").unwrap(),
                                (Value::Double(actual), Some(expected)) => {
                                    assert_eq!(actual.to_bits(), expected.to_bits());
                                    writeln!(bytes, "{name} {id} {:016x}", actual.to_bits())
                                        .unwrap();
                                }
                                _ => panic!("source DOUBLE type/NULL differs"),
                            },
                            DataType::Date => match (value, fixture.day(left, id)) {
                                (Value::Null, None) => writeln!(bytes, "{name} {id} null").unwrap(),
                                (Value::Date(actual), Some(expected)) => {
                                    assert_eq!(actual.days_since_unix_epoch(), expected);
                                    writeln!(
                                        bytes,
                                        "{name} {id} {}",
                                        actual.days_since_unix_epoch()
                                    )
                                    .unwrap();
                                }
                                _ => panic!("source DATE type/NULL differs"),
                            },
                            DataType::String => match (value, fixture.string(left, id)) {
                                (Value::Null, None) => writeln!(bytes, "{name} {id} null").unwrap(),
                                (Value::String(actual), Some(expected)) => {
                                    assert_eq!(actual.as_str(), expected);
                                    write!(bytes, "{name} {id} hex=").unwrap();
                                    for byte in actual.as_str().as_bytes() {
                                        write!(bytes, "{byte:02x}").unwrap();
                                    }
                                    bytes.push('\n');
                                }
                                _ => panic!("source STRING type/NULL differs"),
                            },
                            _ => unreachable!(),
                        }
                    }
                }
                QueryStep::Progress => {}
                QueryStep::Finished => break,
                QueryStep::Failed(error) => panic!("source preflight: {error}"),
            }
        }
        assert!(seen.iter().all(|value| *value));
    }
    released(db, memory, files);
    assert!(bytes.len() <= 1_000_000, "source key preflight byte limit");
    std::fs::write(root.join("source-keys.txt"), bytes).unwrap();
}

struct Cancel<'a, 'c> {
    output: Output<'a>,
    token: &'c CancellationToken,
    threshold: usize,
}
impl Write for Cancel<'_, '_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let written = self.output.write(bytes)?;
        // Thresholds reach group replay before cancellation. The supervisor
        // checks the accepted prefix and each fixture's required traversals.
        if self.output.bytes.len() >= self.threshold {
            self.token.cancel();
        }
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}

fn cancelled(
    db: &Database,
    query: &PreparedQuery<'_>,
    path: &Path,
    limits: ExportLimits,
    threshold: usize,
) -> Vec<u8> {
    let memory = db.reserved_memory_bytes();
    let files = descriptors();
    let token = CancellationToken::new();
    let mut sink = Cancel {
        output: Output::new(db, path, BYTES),
        token: &token,
        threshold,
    };
    let result = db.export_jsonl(query, &mut sink, limits, &token);
    assert!(
        matches!(result, Err(Error::Cancelled)),
        "join cancellation did not reach the API: {result:?}"
    );
    assert!(sink.output.temporary > 0);
    if cfg!(target_os = "linux") {
        assert!(sink.output.spill > 0);
    }
    released(db, memory, files);
    sink.output.bytes
}

pub(super) fn run(root: &Path, kind: u32, at: u32, burst: u32, error: i32, mode: &str) {
    let fixture = Fixture::new(mode);
    let rows = fixture.rows();
    let limits = ExportLimits { rows, bytes: BYTES };
    let sql = format!(
        "FROM left_rows AS l |> {} right_rows AS r ON l.k=r.k |> SELECT l.id AS left_id, r.id AS right_id, l.v AS left_value, r.v AS right_value{}",
        if fixture.outer { "LEFT JOIN" } else { "JOIN" },
        if fixture.text && fixture.kind == DataType::String {
            ", l.note AS left_note, r.k AS right_key"
        } else if fixture.text {
            ", l.note AS left_note, r.note AS right_note"
        } else {
            ""
        }
    );
    let path = root.join("database");
    let before = descriptors();
    create(&path, &fixture);
    let config = Config::new(6_000_000, 8_000_000).unwrap();
    let db = Database::open(&path, config).unwrap();
    if fixture.kind != DataType::Int64 {
        source_keys(&db, &fixture, root);
    }
    for name in ["CONTROL", "ROOT.A", "ROOT.B", "WAL"] {
        std::fs::copy(path.join(name), root.join(name)).unwrap();
    }
    let query = db.prepare(&sql).unwrap();
    let memory = db.reserved_memory_bytes();
    let files = descriptors();
    let generation = db.generation();
    // SAFETY: the single-threaded export owns this interval. Its caller sink
    // performs no native writes, including when query disposal follows failure.
    unsafe {
        io_probe_start(kind, at, burst, error);
    }
    let (initial, result) = export(&db, &query, &path, limits);
    unsafe {
        io_probe_stop();
    }
    let (calls, refused, partial, requested, transferred) = unsafe {
        (
            io_probe_calls(),
            io_probe_refused(),
            io_probe_partial(),
            io_probe_requested_bytes(),
            io_probe_transferred_bytes(),
        )
    };
    let original = format!("{result:?}");
    println!("calls={calls} refused={refused} partial={partial} result={original}");
    println!("join transfers requested={requested} transferred={transferred}");
    if at == 0 || error == 0 {
        assert!(matches!(result, Ok(count) if count == rows));
        assert!(initial.temporary > 0);
        if cfg!(target_os = "linux") {
            assert!(initial.spill > 0, "join I/O did not write spill");
        }
    } else {
        assert!(
            result
                .as_ref()
                .is_err_and(|source| injected_error(source, error.abs())),
            "join I/O lost its original OS error: {result:?}"
        );
    }
    released(&db, memory, files);
    assert_eq!(db.generation(), generation);
    println!(
        "join initial outcome={} bytes={} spill={} temp={}",
        if result.is_ok() { "finished" } else { "failed" },
        initial.bytes.len(),
        initial.spill,
        initial.temporary
    );
    std::fs::write(root.join("initial.jsonl"), &initial.bytes).unwrap();
    if at == 0 && (fixture.text || fixture.kind != DataType::Int64) {
        let bytes = cancelled(&db, &query, &path, limits, fixture.cancel_bytes());
        println!("join cancelled bytes={}", bytes.len());
        std::fs::write(root.join("cancelled.jsonl"), bytes).unwrap();
    }
    let (retry, retried) = export(&db, &query, &path, limits);
    assert!(matches!(retried, Ok(count) if count == rows));
    assert!(retry.temporary > 0);
    if cfg!(target_os = "linux") {
        assert!(retry.spill > 0);
    }
    released(&db, memory, files);
    println!(
        "join retry rows={} spill={} temp={}",
        retried.unwrap(),
        retry.spill,
        retry.temporary
    );
    std::fs::write(root.join("retry.jsonl"), &retry.bytes).unwrap();
    drop(query);
    db.close().unwrap();
    let db = Database::open(&path, config).unwrap();
    assert_eq!(db.generation(), generation);
    let query = db.prepare(&sql).unwrap();
    let memory = db.reserved_memory_bytes();
    let files = descriptors();
    let (reopened, retried) = export(&db, &query, &path, limits);
    assert!(matches!(retried, Ok(count) if count == rows));
    assert!(reopened.temporary > 0);
    if cfg!(target_os = "linux") {
        assert!(reopened.spill > 0);
    }
    released(&db, memory, files);
    println!(
        "join reopened rows={} spill={} temp={}",
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
    println!("join I/O release and complete retries passed");
}
