//! Measure EXCEPT and INTERSECT with independently counted complete rows.
//!
//! The left input has 8,192 rows, the right 4,096. Half their generated classes
//! overlap; duplicate counts differ. Two classes share each integer key, while
//! text distinguishes them. Independently nullable fields can collapse several
//! classes into one all-NULL row. A standard-library map counts complete input
//! rows before timing and applies multiset subtraction or intersection.
//!
//! Pass a fresh absolute database path, operation (`except`, `except-all`,
//! `intersect`, `intersect-all`), classes (32 or 4096), text bytes (8 or 1024),
//! and query memory bytes. Setup and preparation precede one checked warm-up and
//! five samples. Every sample includes full answer validation, resource sampling
//! and cursor disposal. The checker neither requires nor measures output order.
//!
//! Linux observes nonempty unlinked spill outside timing. Cancellation after
//! spill must fail terminally and release query owners, then a fresh query must
//! succeed. Resource counters are engine accounting, not process memory. Run
//! `cargo run --release --offline --locked -j 1 --example set_cost --
//! /absolute/new-sets except-all 4096 1024 4000000`. The caller owns the remaining
//! database. Require successful exit and the final completion line.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, PreparedQuery, QueryStep, Value,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

#[path = "support/query.rs"]
mod query_support;
use query_support::{cancel, released, scratch_usage};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const TEMP: u64 = 128_000_000;
const LEFT_ROWS: usize = 8192;
const RIGHT_ROWS: usize = 4096;
const BATCH: usize = 64;

#[derive(Clone, Copy, Debug)]
enum Operation {
    Except,
    ExceptAll,
    Intersect,
    IntersectAll,
}
impl Operation {
    const ALL: [Self; 4] = [
        Self::Except,
        Self::ExceptAll,
        Self::Intersect,
        Self::IntersectAll,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Except => "except",
            Self::ExceptAll => "except-all",
            Self::Intersect => "intersect",
            Self::IntersectAll => "intersect-all",
        }
    }
    fn sql(self) -> &'static str {
        match self {
            Self::Except => "FROM left_rows |> EXCEPT DISTINCT (FROM right_rows)",
            Self::ExceptAll => "FROM left_rows |> EXCEPT ALL (FROM right_rows)",
            Self::Intersect => "FROM left_rows |> INTERSECT DISTINCT (FROM right_rows)",
            Self::IntersectAll => "FROM left_rows |> INTERSECT ALL (FROM right_rows)",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Row {
    key: Option<i64>,
    text: Option<String>,
}

fn input(rows: usize, classes: usize, shift: usize, width: usize) -> Vec<Row> {
    (0..rows)
        .map(|index| {
            let class = (rows - 1 - index) % classes + shift;
            Row {
                key: (!class.is_multiple_of(11)).then_some((class / 2) as i64),
                text: (!class.is_multiple_of(13))
                    .then(|| format!("{}{:08}", "x".repeat(width - 8), class)),
            }
        })
        .collect()
}

#[derive(Clone)]
struct Expected {
    rows: Vec<(Row, usize)>,
    total: usize,
}

fn expected(left: &[Row], right: &[Row], operation: Operation) -> Expected {
    let counts = |input: &[Row]| {
        let mut counts = BTreeMap::new();
        for row in input {
            *counts.entry(row.clone()).or_insert(0_usize) += 1;
        }
        counts
    };
    let right = counts(right);
    let rows: Vec<_> = counts(left)
        .into_iter()
        .filter_map(|(row, left)| {
            let right = right.get(&row).copied().unwrap_or(0);
            let copies = match operation {
                Operation::Except => usize::from(right == 0),
                Operation::ExceptAll => left.saturating_sub(right),
                Operation::Intersect => usize::from(right != 0),
                Operation::IntersectAll => left.min(right),
            };
            (copies != 0).then_some((row, copies))
        })
        .collect();
    let total = rows.iter().map(|(_, count)| count).sum();
    Expected { rows, total }
}

fn create(path: &Path, left: &[Row], right: &[Row]) -> Result<()> {
    let db = Database::create_empty(path, Config::new(128_000_000, TEMP)?)?;
    let token = CancellationToken::new();
    for (table, rows) in [("left_rows", left), ("right_rows", right)] {
        db.declare_table(
            table,
            &[
                ColumnDeclaration {
                    name: "k",
                    data_type: DataType::Int64,
                    nullable: true,
                },
                ColumnDeclaration {
                    name: "payload",
                    data_type: DataType::String,
                    nullable: true,
                },
            ],
            &token,
        )?;
        if rows.is_empty() {
            continue;
        }
        let mut append = db.begin_append(
            table,
            AppendLimits {
                batches: rows.len().div_ceil(BATCH) as u32,
                encoded_bytes: 32_000_000,
            },
            &token,
        )?;
        for batch in rows.chunks(BATCH) {
            let mut keys = [0; BATCH];
            let mut texts = [""; BATCH];
            let mut key_valid = [0_u8; BATCH / 8];
            let mut text_valid = [0_u8; BATCH / 8];
            for (index, row) in batch.iter().enumerate() {
                if let Some(key) = row.key {
                    keys[index] = key;
                    key_valid[index / 8] |= 1 << (index % 8);
                }
                if let Some(text) = &row.text {
                    texts[index] = text;
                    text_valid[index / 8] |= 1 << (index % 8);
                }
            }
            let rows = batch.len();
            append.write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&keys[..rows]),
                        validity: &key_valid[..rows.div_ceil(8)],
                    },
                    ColumnInput {
                        values: ColumnValues::String(&texts[..rows]),
                        validity: &text_valid[..rows.div_ceil(8)],
                    },
                ],
                &token,
            )?;
        }
        append.commit(&token)?;
    }
    db.close()?;
    Ok(())
}

struct Sample {
    ns: u128,
    rows: usize,
    batches: u64,
    progress: u64,
    memory: u64,
    temporary: u64,
    scratch: u64,
}

fn execute(
    db: &Database,
    query: &PreparedQuery<'_>,
    expected: &Expected,
    path: &Path,
    observe: bool,
) -> Result<Sample> {
    if query.result_column_count() != 2
        || [("k", DataType::Int64), ("payload", DataType::String)]
            .iter()
            .enumerate()
            .any(|(index, (name, kind))| {
                query.result_column(index).is_none_or(|column| {
                    column.name != Some(*name) || column.data_type != *kind || !column.nullable
                })
            })
    {
        return Err("unexpected set schema".into());
    }
    let mut seen = vec![0; expected.rows.len()];
    let baseline = db.reserved_memory_bytes();
    let token = CancellationToken::new();
    let start = Instant::now();
    let mut result = db.execute(query, &token)?;
    let mut sample = Sample {
        ns: 0,
        rows: 0,
        batches: 0,
        progress: 0,
        memory: db.reserved_memory_bytes(),
        temporary: 0,
        scratch: 0,
    };
    let mut finished = false;
    for _ in 0..20_000_000 {
        match result.step() {
            QueryStep::Rows(batch) => {
                sample.batches += 1;
                if batch.column_count() != 2 {
                    return Err("unexpected set width".into());
                }
                for row in 0..batch.len() {
                    let key = match batch.value(row, 0) {
                        Some(Value::Null) => None,
                        Some(Value::Int64(value)) => Some(value),
                        _ => return Err("unexpected set integer".into()),
                    };
                    let text = match batch.value(row, 1) {
                        Some(Value::Null) => None,
                        Some(Value::String(value)) => Some(value.as_str()),
                        _ => return Err("unexpected set text".into()),
                    };
                    let index = expected
                        .rows
                        .binary_search_by(|(row, _)| {
                            row.key.cmp(&key).then(row.text.as_deref().cmp(&text))
                        })
                        .map_err(|_| "unexpected set row")?;
                    seen[index] += 1;
                    if seen[index] > expected.rows[index].1 {
                        return Err("set result multiplicity differs".into());
                    }
                    sample.rows += 1;
                }
            }
            QueryStep::Progress => sample.progress += 1,
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(_) => {
                return Err(result.into_error().ok_or("missing set error")?.into());
            }
        }
        sample.memory = sample.memory.max(db.reserved_memory_bytes());
        sample.temporary = sample.temporary.max(db.reserved_temp_bytes());
        if observe && sample.temporary > 0 && sample.scratch == 0 {
            sample.scratch = scratch_usage(path)?.1;
        }
    }
    if !finished
        || sample.rows != expected.total
        || seen
            .iter()
            .zip(&expected.rows)
            .any(|(seen, (_, count))| seen != count)
    {
        return Err("incomplete set result".into());
    }
    drop(result);
    sample.ns = start.elapsed().as_nanos();
    released(db, baseline, path)?;
    Ok(sample)
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 5 {
        return Err("expected fresh path, operation, classes, text bytes and memory bytes".into());
    }
    let operation = Operation::ALL
        .into_iter()
        .find(|op| op.name() == args[1])
        .ok_or("unknown set operation")?;
    let classes = args[2].parse()?;
    let width = args[3].parse()?;
    let memory = args[4].parse()?;
    if !matches!(classes, 32 | 4096) || !matches!(width, 8 | 1024) {
        return Err("unsupported set input shape".into());
    }
    let path = Path::new(&args[0]);
    let left = input(LEFT_ROWS, classes, 0, width);
    let right = input(RIGHT_ROWS, classes, classes / 2, width);
    let expected = expected(&left, &right, operation);
    create(path, &left, &right)?;
    drop((left, right));
    let db = Database::open(path, Config::new(memory, TEMP)?)?;
    let query = db.prepare(operation.sql())?;
    let warmup = execute(&db, &query, &expected, path, true)?;
    if warmup.temporary == 0 || (cfg!(target_os = "linux") && warmup.scratch == 0) {
        return Err("set measurement did not observe spill".into());
    }
    println!(
        "input operation={} left_rows={LEFT_ROWS} right_rows={RIGHT_ROWS} classes={classes} text_bytes={width} memory_limit={memory} expected_rows={}",
        operation.name(),
        expected.total
    );
    println!(
        "warmup scratch_bytes={} file_probe={}",
        warmup.scratch,
        cfg!(target_os = "linux")
    );
    for index in 0..5 {
        let sample = execute(&db, &query, &expected, path, false)?;
        println!(
            "sample={index} elapsed_ns={} rows={} batches={} progress={} memory={} temporary={}",
            sample.ns,
            sample.rows,
            sample.batches,
            sample.progress,
            sample.memory,
            sample.temporary
        );
    }
    cancel(&db, &query, path)?;
    execute(&db, &query, &expected, path, false)?;
    drop(query);
    db.close()?;
    println!("status=finished");
    Ok(())
}

#[cfg(test)]
#[path = "../test/support/mod.rs"]
mod support;

#[cfg(test)]
mod tests {
    use super::*;

    fn literal(rows: &[Row]) -> Expected {
        let mut sorted = rows.to_vec();
        sorted.sort();
        let mut counts: Vec<(Row, usize)> = Vec::new();
        for row in sorted {
            if let Some((previous, count)) = counts.last_mut()
                && previous == &row
            {
                *count += 1;
            } else {
                counts.push((row, 1));
            }
        }
        Expected {
            rows: counts,
            total: rows.len(),
        }
    }

    #[test]
    fn complete_set_answers_spill_cancellation_and_wrong_results() {
        let directory = support::Directory::new();
        let a = Row {
            key: Some(1),
            text: Some("x".into()),
        };
        let b = Row {
            key: Some(1),
            text: Some("y".into()),
        };
        let c = Row {
            key: None,
            text: Some("雪\0".into()),
        };
        let d = Row {
            key: Some(2),
            text: None,
        };
        let e = Row {
            key: None,
            text: None,
        };
        let cases = [
            (
                vec![
                    a.clone(),
                    a.clone(),
                    b.clone(),
                    b.clone(),
                    c.clone(),
                    e.clone(),
                ],
                vec![
                    a.clone(),
                    b.clone(),
                    b.clone(),
                    d.clone(),
                    e.clone(),
                    e.clone(),
                ],
                [
                    vec![c.clone()],
                    vec![a.clone(), c.clone()],
                    vec![a.clone(), b.clone(), e.clone()],
                    vec![a.clone(), b.clone(), b.clone(), e.clone()],
                ],
            ),
            (
                vec![],
                vec![a.clone(), b.clone()],
                [vec![], vec![], vec![], vec![]],
            ),
            (
                vec![a.clone(), a.clone(), c.clone()],
                vec![],
                [
                    vec![a.clone(), c.clone()],
                    vec![a.clone(), a.clone(), c.clone()],
                    vec![],
                    vec![],
                ],
            ),
            (
                vec![a.clone(), a.clone(), c.clone()],
                vec![b.clone(), d.clone()],
                [
                    vec![a.clone(), c.clone()],
                    vec![a.clone(), a.clone(), c.clone()],
                    vec![],
                    vec![],
                ],
            ),
            (
                vec![a.clone(), b.clone(), b.clone(), e.clone()],
                vec![a.clone(), b.clone(), b.clone(), e.clone()],
                [
                    vec![],
                    vec![],
                    vec![a.clone(), b.clone(), e.clone()],
                    vec![a.clone(), b.clone(), b.clone(), e.clone()],
                ],
            ),
        ];
        for (case, (left, right, answers)) in cases.into_iter().enumerate() {
            let path = directory.0.join(format!("literal-{case}"));
            create(&path, &left, &right).unwrap();
            let db = Database::open(&path, Config::new(12_000_000, TEMP).unwrap()).unwrap();
            for (operation, rows) in Operation::ALL.into_iter().zip(answers) {
                let answer = literal(&rows);
                let model = expected(&left, &right, operation);
                assert_eq!(model.rows, answer.rows, "case={case} {operation:?}");
                assert_eq!(model.total, answer.total);
                for suffix in ["", " |> ORDER BY k DESC, payload DESC"] {
                    let query = db.prepare(&format!("{}{suffix}", operation.sql())).unwrap();
                    let baseline = db.reserved_memory_bytes();
                    execute(&db, &query, &answer, &path, false).unwrap();
                    if case == 0 && matches!(operation, Operation::IntersectAll) {
                        let mut wrong = answer.clone();
                        wrong.rows[0].1 += 1;
                        wrong.total += 1;
                        assert_eq!(
                            execute(&db, &query, &wrong, &path, false)
                                .err()
                                .unwrap()
                                .to_string(),
                            "incomplete set result"
                        );
                        released(&db, baseline, &path).unwrap();
                        let mut wrong = answer.clone();
                        wrong.rows[0].1 += 1;
                        wrong.rows[1].1 -= 1;
                        assert_eq!(
                            execute(&db, &query, &wrong, &path, false)
                                .err()
                                .unwrap()
                                .to_string(),
                            "set result multiplicity differs"
                        );
                        released(&db, baseline, &path).unwrap();
                        let mut wrong = answer.clone();
                        wrong.rows[0].0.text = Some("changed".into());
                        wrong.rows.sort_by(|a, b| a.0.cmp(&b.0));
                        assert_eq!(
                            execute(&db, &query, &wrong, &path, false)
                                .err()
                                .unwrap()
                                .to_string(),
                            "unexpected set row"
                        );
                        released(&db, baseline, &path).unwrap();
                        let prefix = db
                            .prepare(&format!("{}{suffix} |> LIMIT 1", operation.sql()))
                            .unwrap();
                        assert_eq!(
                            execute(&db, &prefix, &answer, &path, false)
                                .err()
                                .unwrap()
                                .to_string(),
                            "incomplete set result"
                        );
                        drop(prefix);
                        released(&db, baseline, &path).unwrap();
                        execute(&db, &query, &answer, &path, false).unwrap();
                    }
                }
            }
            db.close().unwrap();
        }
        let path = directory.0.join("scaled");
        let left = input(LEFT_ROWS, 32, 0, 8);
        let right = input(RIGHT_ROWS, 32, 16, 8);
        create(&path, &left, &right).unwrap();
        for memory in [4_000_000, 12_000_000] {
            let db = Database::open(&path, Config::new(memory, TEMP).unwrap()).unwrap();
            for operation in Operation::ALL {
                let answer = expected(&left, &right, operation);
                let query = db.prepare(operation.sql()).unwrap();
                let sample = execute(&db, &query, &answer, &path, true).unwrap();
                assert!(sample.temporary > 0);
                assert!(!cfg!(target_os = "linux") || sample.scratch > 0);
                cancel(&db, &query, &path).unwrap();
                execute(&db, &query, &answer, &path, false).unwrap();
            }
            db.close().unwrap();
        }
    }
}
