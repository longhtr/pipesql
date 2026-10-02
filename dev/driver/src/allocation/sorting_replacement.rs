//! Observe and refuse workspace replacement after a sorted run has spilled.
//!
//! Discover growth by its public reservation increase, then repeat that exact
//! execution step with each allocation prefix. Text and integer cases keep their
//! own complete answers; requested/usable observations cover coexistence and free.
//! The joined text case retains the left input while replacing the right arena.

use super::{CALLS, REFUSED, measurement::Live, transient_ownership::Observer, workload};
use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, QueryStep, Value,
};
use std::{path::Path, sync::atomic::Ordering};

const WIDTH: usize = 1024;
const STEPS: usize = 500_000;
const JOIN_WIDTH: usize = 2048;

// Caller answer storage stays still across every allocator observation. Expected
// pairs follow literal key ranges, not the engine's comparator or encoded rows.
struct Answers {
    rows: usize,
    pairs: Vec<u8>,
}

impl Answers {
    fn new(profile: Profile) -> Self {
        Self {
            rows: 0,
            pairs: if matches!(profile, Profile::JoinedBytes) {
                vec![0; (64_usize * 1025).div_ceil(8)]
            } else {
                Vec::new()
            },
        }
    }

    fn clear(&mut self) {
        self.rows = 0;
        self.pairs.fill(0);
    }

    fn joined(&mut self, step: QueryStep<'_>) -> bool {
        match step {
            QueryStep::Progress => false,
            QueryStep::Rows(batch) => {
                assert_eq!(batch.column_count(), 3);
                for row in 0..batch.len() {
                    let Some(Value::Int64(left)) = batch.value(row, 0) else {
                        panic!("join growth left type");
                    };
                    let left = usize::try_from(left).unwrap();
                    assert!(left < 64, "join growth left identity");
                    let right = match batch.value(row, 1) {
                        Some(Value::Null) => {
                            assert!(left == 0 || left >= 48, "join growth unmatched relation");
                            assert_eq!(batch.value(row, 2), Some(Value::Null));
                            1024
                        }
                        Some(Value::Int64(right)) => {
                            let right = usize::try_from(right).unwrap();
                            assert!((1..1024).contains(&right), "join growth right identity");
                            assert!(
                                (1..48).contains(&left) && right / 64 == (left - 1) / 3,
                                "join growth pair relation"
                            );
                            if right % 7 == 0 {
                                assert_eq!(batch.value(row, 2), Some(Value::Null));
                            } else {
                                let Some(Value::String(text)) = batch.value(row, 2) else {
                                    panic!("join growth text type");
                                };
                                let text = text.as_str();
                                assert_eq!(text.len(), JOIN_WIDTH);
                                assert_eq!(text[..8].parse::<usize>().unwrap(), right);
                                assert_eq!(&text[8..11], "雪");
                                assert_eq!(text.as_bytes()[11], 0);
                                assert!(text.as_bytes()[12..].iter().all(|byte| *byte == b'x'));
                            }
                            right
                        }
                        _ => panic!("join growth right type"),
                    };
                    let slot = left * 1025 + right;
                    let bit = 1 << (slot % 8);
                    assert_eq!(self.pairs[slot / 8] & bit, 0, "join growth duplicate pair");
                    self.pairs[slot / 8] |= bit;
                    self.rows += 1;
                    assert!(self.rows <= 3022);
                }
                false
            }
            QueryStep::Finished => {
                // Fifteen keys have three left rows; the last has two. The
                // NULL right key removes three pairs from the first group.
                // The NULL left key and 16 disjoint keys are unmatched.
                assert_eq!(self.rows, 47 * 64 - 3 + 17);
                true
            }
            QueryStep::Failed(error) => panic!("optional joined-byte growth failed: {error}"),
        }
    }
}

fn create_join(db: &Database, token: &CancellationToken) -> Result<(), pipesql::Error> {
    for (table, rows) in [("facts", 64), ("dimensions", 1024)] {
        let mut columns = vec![
            ColumnDeclaration {
                name: "k",
                data_type: DataType::Int64,
                nullable: true,
            },
            ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            },
        ];
        if table == "dimensions" {
            columns.push(ColumnDeclaration {
                name: "note",
                data_type: DataType::String,
                nullable: true,
            });
        }
        db.declare_table(table, &columns, token)?;
        let mut append = db.begin_append(
            table,
            AppendLimits {
                batches: (rows / 64) as u32,
                encoded_bytes: 3_000_000,
            },
            token,
        )?;
        for start in (0..rows).step_by(64) {
            let ids: [i64; 64] = std::array::from_fn(|row| (rows - 1 - start - row) as i64);
            let keys: [i64; 64] = ids.map(|id| {
                if id == 0 {
                    -999
                } else if table == "facts" && id < 48 {
                    (id - 1) / 3
                } else if table == "facts" {
                    id - 32
                } else {
                    id / 64
                }
            });
            let mut key_valid = [0_u8; 8];
            let mut note_valid = [0_u8; 8];
            for (row, id) in ids.iter().enumerate() {
                if *id != 0 {
                    key_valid[row / 8] |= 1 << (row % 8);
                }
                if id % 7 != 0 {
                    note_valid[row / 8] |= 1 << (row % 8);
                }
            }
            let mut input = vec![
                ColumnInput {
                    values: ColumnValues::Int64(&keys),
                    validity: &key_valid,
                },
                ColumnInput {
                    values: ColumnValues::Int64(&ids),
                    validity: &[255; 8],
                },
            ];
            if table == "dimensions" {
                let texts: Vec<_> = ids
                    .iter()
                    .map(|id| format!("{id:08}雪\0{}", "x".repeat(JOIN_WIDTH - 12)))
                    .collect();
                let views: Vec<_> = texts.iter().map(String::as_str).collect();
                input.push(ColumnInput {
                    values: ColumnValues::String(&views),
                    validity: &note_valid,
                });
                append.write(&input, token)?;
            } else {
                append.write(&input, token)?;
            }
        }
        append.commit(token)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) enum Profile {
    Bytes,
    Rows,
    JoinedBytes,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Control {
    Healthy,
    MissingObservation,
    WrongPairs,
    WrongAttribution,
}

impl Profile {
    fn rows(self) -> usize {
        match self {
            Self::Bytes => 1024,
            Self::Rows => 32768,
            Self::JoinedBytes => 3022,
        }
    }

    fn allocations(self) -> usize {
        match self {
            Self::Bytes | Self::JoinedBytes => 1,
            Self::Rows => 3,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Bytes => "bytes",
            Self::Rows => "rows",
            Self::JoinedBytes => "join-bytes",
        }
    }

    fn kind(self) -> &'static str {
        match self {
            Self::Bytes => "byte",
            Self::Rows => "row",
            Self::JoinedBytes => "joined-byte",
        }
    }
}

// Observe actual unlinked file lengths outside allocation observation. The Linux
// campaign requires written spill; native macOS runs retain account checks.
#[cfg(target_os = "linux")]
fn written_spill(path: &Path) -> (u64, usize) {
    use std::os::unix::fs::MetadataExt;
    let files: Vec<_> = std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|entry| {
            let descriptor = entry.unwrap().path();
            let target = match std::fs::read_link(&descriptor) {
                Ok(target) => target,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
                Err(error) => panic!("read spill descriptor: {error}"),
            };
            if !target.starts_with(path) {
                return None;
            }
            let metadata = std::fs::metadata(descriptor).unwrap();
            (metadata.is_file() && metadata.nlink() == 0).then_some(metadata.len())
        })
        .collect();
    (
        files.iter().sum(),
        files.iter().filter(|length| **length > 0).count(),
    )
}

#[cfg(not(target_os = "linux"))]
fn written_spill(_: &Path) -> (u64, usize) {
    (0, 0)
}

fn check(profile: Profile, step: QueryStep<'_>, answers: &mut Answers) -> bool {
    if matches!(profile, Profile::JoinedBytes) {
        return answers.joined(step);
    }
    let rows = &mut answers.rows;
    match step {
        QueryStep::Progress => false,
        QueryStep::Rows(batch) => {
            assert_eq!(batch.column_count(), 1);
            for row in 0..batch.len() {
                match profile {
                    Profile::Bytes => {
                        let Some(Value::String(text)) = batch.value(row, 0) else {
                            panic!("sort byte growth changed text type");
                        };
                        let text = text.as_str();
                        assert_eq!(text.len(), WIDTH);
                        assert_eq!(text[..8].parse::<usize>().unwrap(), *rows);
                        assert!(text.as_bytes()[8..].iter().all(|byte| *byte == b'x'));
                    }
                    Profile::Rows => {
                        assert_eq!(batch.value(row, 0), Some(Value::Int64(*rows as i64)))
                    }
                    Profile::JoinedBytes => unreachable!(),
                }
                *rows += 1;
                assert!(*rows <= profile.rows());
            }
            false
        }
        QueryStep::Finished => {
            assert_eq!(*rows, profile.rows());
            true
        }
        QueryStep::Failed(error) => {
            panic!("optional sort {} growth failed: {error}", profile.kind())
        }
    }
}

pub(super) fn run(
    root: &Path,
    profile: Profile,
    control: Control,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(root)?;
    let token = CancellationToken::new();
    let db = Database::create_empty(&root.join("database"), Config::new(12_000_000, 32_000_000)?)?;
    let query = if matches!(profile, Profile::JoinedBytes) {
        create_join(&db, &token)?;
        let sql = if control == Control::WrongPairs {
            "FROM facts AS l |> LEFT JOIN dimensions AS r ON l.k = r.k |> SELECT l.id AS left_id, r.id + 64 AS right_id, r.note AS note"
        } else {
            "FROM facts AS l |> LEFT JOIN dimensions AS r ON l.k = r.k |> SELECT l.id AS left_id, r.id AS right_id, r.note AS note"
        };
        let query = db.prepare(sql)?;
        let columns = [
            ("left_id", DataType::Int64, false),
            ("right_id", DataType::Int64, true),
            ("note", DataType::String, true),
        ];
        assert_eq!(query.result_column_count(), columns.len());
        for (index, (name, kind, nullable)) in columns.into_iter().enumerate() {
            let column = query.result_column(index).unwrap();
            assert_eq!(
                (column.name, column.data_type, column.nullable),
                (Some(name), kind, nullable)
            );
        }
        query
    } else {
        db.declare_table(
            "facts",
            &[ColumnDeclaration {
                name: "v",
                data_type: match profile {
                    Profile::Bytes => DataType::String,
                    Profile::Rows => DataType::Int64,
                    Profile::JoinedBytes => unreachable!(),
                },
                nullable: false,
            }],
            &token,
        )?;
        let mut append = db.begin_append(
            "facts",
            AppendLimits {
                batches: (profile.rows() / 64) as u32,
                encoded_bytes: 2_000_000,
            },
            &token,
        )?;
        for start in (0..profile.rows()).step_by(64) {
            if let Profile::Rows = profile {
                let values: [i64; 64] = std::array::from_fn(|index| (32767 - start - index) as i64);
                append.write(
                    &[ColumnInput {
                        values: ColumnValues::Int64(&values),
                        validity: &[255; 8],
                    }],
                    &token,
                )?;
                continue;
            }
            let texts: Vec<_> = (start..start + 64)
                .map(|row| format!("{:08}{}", 1023 - row, "x".repeat(WIDTH - 8)))
                .collect();
            let views: Vec<_> = texts.iter().map(String::as_str).collect();
            append.write(
                &[ColumnInput {
                    values: ColumnValues::String(&views),
                    validity: &[255; 8],
                }],
                &token,
            )?;
        }
        append.commit(&token)?;
        db.prepare("FROM facts |> ORDER BY v")?
    };
    let mut growth = Vec::with_capacity(16);
    let (kind, label, allocations) = (profile.kind(), profile.label(), profile.allocations());
    println!("optional sort {kind} growth: spilled-run replacement and fallback");
    let mut answers = Answers::new(profile);
    let prepared = Live::now();
    let memory = db.reserved_memory_bytes();
    let mut result = db.execute(&query, &token)?;
    answers.clear();
    let mut finished = false;
    for index in 0..STEPS {
        let before = db.reserved_memory_bytes();
        workload::arm(None, 16);
        let step = result.step();
        workload::suspend_faults();
        let after = db.reserved_memory_bytes();
        // A join also grows startup buffers. Select its large single-arena
        // increase; refusal must still complete every pair. That fallback check
        // exposes a required allocation mistakenly classified as optional.
        if index != 0
            && after > before
            && (!matches!(profile, Profile::JoinedBytes)
                || (after - before >= 1_048_576 && CALLS.load(Ordering::Relaxed) == 1))
        {
            assert_eq!(
                CALLS.load(Ordering::Relaxed),
                allocations,
                "workspace replacement allocation count"
            );
            let (written, files) = written_spill(&root.join("database"));
            if cfg!(target_os = "linux") {
                assert!(written > 0, "replacement must follow written spill");
            }
            assert!(db.reserved_temp_bytes() > 0);
            if matches!(profile, Profile::JoinedBytes) {
                if cfg!(target_os = "linux") {
                    assert!(
                        files >= 2,
                        "join replacement must retain written left input"
                    );
                }
                let descriptors = Live::now()
                    .descriptors
                    .checked_sub(prepared.descriptors)
                    .unwrap();
                assert!(descriptors >= 4, "both join inputs must remain open");
                println!(
                    "sort {label} retained event={} files={files} descriptors={descriptors}",
                    growth.len()
                );
            }
            println!(
                "sort {label} written event={} bytes={written}",
                growth.len()
            );
            assert!(growth.len() < 16);
            growth.push(index);
        }
        if check(profile, step, &mut answers) {
            finished = true;
            break;
        }
    }
    assert!(
        finished && !growth.is_empty(),
        "missing optional {kind} growth census"
    );
    if let Profile::Rows = profile {
        assert_eq!(growth.len(), 2, "both row replacements must run");
    }
    drop(result);
    assert_eq!(Live::now(), prepared);
    assert_eq!(db.reserved_memory_bytes(), memory);
    assert_eq!(db.reserved_temp_bytes(), 0);
    println!("sort {label} census events={}", growth.len());
    for (event, target) in growth.into_iter().enumerate() {
        for prefix in 0..=allocations {
            let mut result = db.execute(&query, &token)?;
            let resident = if control == Control::WrongAttribution {
                db.config().memory_limit_bytes()
            } else {
                memory
            };
            let observer = Observer::new(&db, resident, prepared.requested, prepared.usable);
            answers.clear();
            let mut finished = false;
            let mut observed = false;
            for index in 0..STEPS {
                let step = if index == target {
                    workload::arm(Some(prefix), 16);
                    let step = if control == Control::MissingObservation && prefix == allocations {
                        result.step()
                    } else {
                        observer.during(|| result.step())
                    };
                    workload::suspend_faults();
                    assert_eq!(CALLS.load(Ordering::Relaxed), (prefix + 1).min(allocations));
                    assert_eq!(
                        REFUSED.load(Ordering::Relaxed),
                        usize::from(prefix < allocations)
                    );
                    let sample = observer.samples();
                    assert_eq!(
                        sample.allocations, prefix,
                        "missing optional {kind} growth allocation events"
                    );
                    assert_eq!(
                        sample.frees, prefix,
                        "replacement must free partial new arrays or all old arrays"
                    );
                    assert!(
                        sample.requested_headroom >= 0 && sample.usable_headroom >= 0,
                        "workspace replacement exceeds reservation: {sample:?}"
                    );
                    observed = true;
                    step
                } else {
                    result.step()
                };
                if check(profile, step, &mut answers) {
                    finished = true;
                    break;
                }
            }
            assert!(finished && observed);
            drop(result);
            assert_eq!(Live::now(), prepared);
            assert_eq!(db.reserved_memory_bytes(), memory);
            assert_eq!(db.reserved_temp_bytes(), 0);
            println!(
                "sort {label} event={event} step={target} prefix={prefix} calls={} refusals={} rows={} samples={:?}",
                CALLS.load(Ordering::Relaxed),
                REFUSED.load(Ordering::Relaxed),
                answers.rows,
                observer.samples()
            );
        }
        let cancelled = CancellationToken::new();
        let mut result = db.execute(&query, &cancelled)?;
        for index in 0..=target {
            let before = db.reserved_memory_bytes();
            assert!(matches!(result.step(), QueryStep::Progress));
            if index == target {
                assert!(
                    db.reserved_memory_bytes() > before,
                    "cancellation must follow replacement"
                );
            }
        }
        cancelled.cancel();
        for _ in 0..2 {
            assert!(matches!(
                result.step(),
                QueryStep::Failed(pipesql::Error::Cancelled)
            ));
        }
        drop(result);
        assert_eq!(Live::now(), prepared);
        assert_eq!(db.reserved_memory_bytes(), memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
        let mut retry = db.execute(&query, &token)?;
        answers.clear();
        let mut finished = false;
        for _ in 0..STEPS {
            if check(profile, retry.step(), &mut answers) {
                finished = true;
                break;
            }
        }
        assert!(finished);
        drop(retry);
        assert_eq!(Live::now(), prepared);
        assert_eq!(db.reserved_memory_bytes(), memory);
        assert_eq!(db.reserved_temp_bytes(), 0);
        println!(
            "sort {label} cancellation event={event} terminal=2 retry_rows={}",
            answers.rows
        );
    }
    println!("sort {kind} growth passed: complete rows, replacement fallback and release");
    drop(query);
    db.close()?;
    Ok(())
}
