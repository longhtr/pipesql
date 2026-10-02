//! Carry independent floating answers through declared grouping and real spill.
//!
//! The retained vectors define results without calling engine arithmetic. Rows
//! from different vectors are interleaved, while each vector keeps its argument
//! sequence. Thirty-seven-row stored units split vectors across input batches;
//! low-memory runs also cross merge boundaries.
//! Memory pressure selects hashing, immediate disk work and late hash fallback;
//! live owners and nonempty scratch files establish which paths actually ran.

use super::*;
use crate::execution::Aggregation;
use crate::execution::blocking::test_support::{Directory, expected_buffer_charge};
use crate::{
    AppendLimits, ColumnDeclaration, ColumnInput, ColumnValues, Config, QueryResult, QueryStep,
};

const AVERAGE: &str = "FROM facts |> AGGREGATE SUM(v) AS unused, AVG(v) AS value, COUNT(*) AS n GROUP AND ORDER BY k |> SELECT k, value, n";
const SUM: &str = "FROM facts |> WHERE can_sum = 1 |> AGGREGATE SUM(v) AS value, COUNT(*) AS n GROUP AND ORDER BY k";
const OVERFLOW: &str = "FROM facts |> AGGREGATE AVG(v) AS unused, SUM(v) AS value, COUNT(*) AS n GROUP AND ORDER BY k |> SELECT k, value, n";
const STEPS: usize = 5_000_000;

struct Vector {
    values: Vec<f64>,
    sum: &'static str,
    average: (&'static str, &'static str),
}

fn vectors() -> Vec<Vector> {
    let vectors: Vec<_> = include_str!("../../../../test/data/aggregate-semantics/rounding.txt")
        .lines()
        .map(|line| {
            let fields: Vec<_> = line.split(';').collect();
            assert_eq!(fields.len(), 3);
            Vector {
                values: fields[0]
                    .split(',')
                    .map(|bits| f64::from_bits(u64::from_str_radix(bits, 16).unwrap()))
                    .collect(),
                sum: fields[1],
                average: fields[2].split_once(',').unwrap(),
            }
        })
        .collect();
    assert_eq!(vectors.len(), 600);
    assert_eq!(vectors.iter().map(|v| v.values.len()).sum::<usize>(), 9513);
    assert_eq!(vectors.iter().filter(|v| v.sum == "overflow").count(), 150);
    vectors
}

fn database(directory: &Directory, vectors: &[Vector]) -> Database {
    let db = Database::create_empty(
        &directory.0.join("db"),
        Config::new(16_000_000, 16_000_000).unwrap(),
    )
    .unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "facts",
        &[
            ColumnDeclaration {
                name: "k",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "v",
                data_type: DataType::Double,
                nullable: false,
            },
            ColumnDeclaration {
                name: "can_sum",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "position",
                data_type: DataType::Int64,
                nullable: false,
            },
        ],
        &cancel,
    )
    .unwrap();
    let rows: Vec<_> = (0..36)
        .flat_map(|position| {
            vectors.iter().enumerate().filter_map(move |(key, vector)| {
                vector.values.get(position).map(|&value| {
                    (
                        key as i64,
                        value,
                        i64::from(vector.sum != "overflow"),
                        position as i64,
                    )
                })
            })
        })
        .collect();
    let mut append = db
        .begin_append(
            "facts",
            AppendLimits {
                batches: rows.len().div_ceil(37) as u32,
                encoded_bytes: 1_000_000,
            },
            &cancel,
        )
        .unwrap();
    for rows in rows.chunks(37) {
        let keys: Vec<_> = rows.iter().map(|r| r.0).collect();
        let values: Vec<_> = rows.iter().map(|r| r.1).collect();
        let sums: Vec<_> = rows.iter().map(|r| r.2).collect();
        let positions: Vec<_> = rows.iter().map(|r| r.3).collect();
        let mut validity = vec![255; rows.len().div_ceil(8)];
        if rows.len() % 8 != 0 {
            *validity.last_mut().unwrap() = (1 << (rows.len() % 8)) - 1;
        }
        append
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::Int64(&keys),
                        validity: &validity,
                    },
                    ColumnInput {
                        values: ColumnValues::Double(&values),
                        validity: &validity,
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&sums),
                        validity: &validity,
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&positions),
                        validity: &validity,
                    },
                ],
                &cancel,
            )
            .unwrap();
    }
    append.commit(&cancel).unwrap();
    db
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Path {
    Memory,
    Disk,
    Fallback,
}

fn owner<'a>(result: &'a QueryResult<'_, '_>) -> &'a General<'a> {
    let Some(Aggregation::General(owner)) = result.first_aggregate() else {
        panic!("live grouping owner");
    };
    &owner[0]
}

// Subtract optional live capacities to select a path. This is not an independent
// admission-formula test; the observed path below must confirm the selection.
fn allowance(result: &QueryResult<'_, '_>, path: Path) -> u64 {
    let group = owner(result);
    let without_hash = result.accounted_memory_bytes() + crate::storage::catalog::MAX_BYTES as u64
        - group.memory.first().map_or(0, MemoryGroups::memory_bytes);
    if path == Path::Fallback {
        return without_hash
            + MemoryGroups::requirement(&group.aggregate, &group.keys, 256, 256 * 9)
                .unwrap()
                .1;
    }
    let scalar_extra = (group.aggregate.lanes - 1) * group.aggregate.scratch.len()
        / group.aggregate.lanes
        * size_of::<u64>();
    let argument_extra =
        (group.arguments.capacity - 1) * group.arguments.shape.count * size_of::<u64>();
    let record = RECORD_HEADER + group.keys.max_bytes + group.arguments.shape.count * 8;
    assert!(record <= 16_384);
    let run_extra = group
        .sort
        .run_allocation_capacities()
        .into_iter()
        .map(expected_buffer_charge)
        .sum::<usize>()
        - record
        - 2 * size_of::<RecordSpan>();
    without_hash - (scalar_extra + argument_extra + run_extra) as u64
}

type Row = (i64, u64, i64);

fn number(bits: u64, (low, high): (&str, &str)) -> bool {
    let value = f64::from_bits(bits);
    if low == "nan" {
        return high == "nan" && value.is_nan();
    }
    let (low, high) = (
        u64::from_str_radix(low, 16).unwrap(),
        u64::from_str_radix(high, 16).unwrap(),
    );
    if low == high {
        bits == low
    } else {
        value.is_finite() && value >= f64::from_bits(low) && value <= f64::from_bits(high)
    }
}

fn answers(rows: &[Row], expected: &[(usize, &Vector)], average: bool) -> bool {
    rows.len() == expected.len()
        && rows
            .iter()
            .zip(expected)
            .all(|(&(key, bits, count), &(id, vector))| {
                key == id as i64
                    && count == vector.values.len() as i64
                    && number(
                        bits,
                        if average {
                            vector.average
                        } else {
                            (vector.sum, vector.sum)
                        },
                    )
            })
}

fn execute(db: &Database, sql: &str, path: Path, overflow: bool) -> Vec<Row> {
    let cancel = CancellationToken::new();
    let query = db.prepare(sql).unwrap();
    let baseline = db.reserved_memory_bytes();
    let probe = db.execute(&query, &cancel).unwrap();
    let bytes = allowance(&probe, path);
    drop(probe);
    let pressure = (path != Path::Memory).then(|| {
        db.reserve_memory(
            db.config().memory_limit_bytes() - baseline - bytes,
            "rounding path selection",
        )
        .unwrap()
    });
    let mut result = db.execute(&query, &cancel).unwrap();
    assert!(owner(&result).hash_pending && owner(&result).memory.is_empty());
    match path {
        Path::Memory => (),
        Path::Disk => super::tests::first_hash_limits(&mut result, 0, 0),
        Path::Fallback => super::tests::first_hash_limits(&mut result, 256, 256 * 9),
    }
    let mut rows = Vec::new();
    let (mut hashed, mut written, mut merged, mut terminal) = (false, false, false, false);
    let mut effects = Effects::default();
    for _ in 0..STEPS {
        let group = owner(&result);
        hashed |=
            matches!(group.phase, Phase::Read) && group.memory.first().is_some_and(|m| m.len() > 0);
        merged |= group.sort.phase() == SortPhase::Merge
            && group.ordinal > group.sort.run_limits().1 as u64;
        if let Files::Open(files) = &group.files {
            written |= (0..2).any(|slot| files.test_file(slot).metadata().unwrap().len() > 0);
        }
        match result.step_with_effects(&mut effects) {
            QueryStep::Rows(batch) => {
                assert!(!overflow, "overflow must precede group publication");
                assert_eq!(batch.column_count(), 3);
                for row in 0..batch.len() {
                    let (
                        Some(Value::Int64(key)),
                        Some(Value::Double(value)),
                        Some(Value::Int64(count)),
                    ) = (
                        batch.value(row, 0),
                        batch.value(row, 1),
                        batch.value(row, 2),
                    )
                    else {
                        panic!("typed grouped answer");
                    };
                    rows.push((key, value.to_bits(), count));
                }
            }
            QueryStep::Progress => (),
            QueryStep::Finished => {
                assert!(!overflow);
                terminal = true;
                break;
            }
            QueryStep::Failed(Error::ArithmeticOverflow {
                operation: "SUM",
                span,
            }) if overflow => {
                assert_eq!(&sql[span.start()..span.end()], "SUM(v)");
                let before = effects.count();
                assert!(matches!(
                    result.step_with_effects(&mut effects),
                    QueryStep::Failed(Error::ArithmeticOverflow {
                        operation: "SUM",
                        ..
                    })
                ));
                assert_eq!(effects.count(), before);
                terminal = true;
                break;
            }
            QueryStep::Failed(error) => panic!("{path:?}: {error}"),
        }
    }
    assert!(terminal, "{path:?}: bounded query completion");
    assert_eq!(
        hashed,
        path != Path::Disk,
        "{path:?}: completed hash accumulation"
    );
    assert_eq!(
        written,
        path != Path::Memory,
        "{path:?}: physically written spill"
    );
    assert_eq!(
        merged,
        path != Path::Memory,
        "{path:?}: multiple sorted runs"
    );
    drop(result);
    drop(pressure);
    assert_eq!(db.reserved_memory_bytes(), baseline);
    assert_eq!(db.reserved_temp_bytes(), 0);
    rows
}

#[test]
fn independent_vectors_cross_declared_hash_and_disk_grouping() {
    let vectors = vectors();
    let directory = Directory::new();
    let db = database(&directory, &vectors);
    let averages: Vec<_> = vectors.iter().enumerate().collect();
    let sums: Vec<_> = averages
        .iter()
        .copied()
        .filter(|(_, v)| v.sum != "overflow")
        .collect();
    for path in [Path::Memory, Path::Disk, Path::Fallback] {
        let rows = execute(&db, AVERAGE, path, false);
        assert!(
            answers(&rows, &averages, true),
            "{path:?}: independent AVG answers"
        );
        assert!(!answers(&rows[..rows.len() - 1], &averages, true));
        let mut changed = rows.clone();
        // The first exact zero result must retain its sign and complete identity.
        let zero = vectors
            .iter()
            .position(|v| {
                matches!(
                    v.average,
                    ("0000000000000000", "0000000000000000")
                        | ("8000000000000000", "8000000000000000")
                )
            })
            .unwrap();
        changed[zero].1 ^= 1;
        assert!(!answers(&changed, &averages, true));
        let rows = execute(&db, SUM, path, false);
        assert!(
            answers(&rows, &sums, false),
            "{path:?}: independent SUM answers"
        );
        assert!(!answers(&rows[..rows.len() - 1], &sums, false));
        execute(&db, OVERFLOW, path, true);
        // The failed SUM cannot poison a later AVG of the same stored inputs.
        assert!(answers(
            &execute(&db, AVERAGE, path, false),
            &averages,
            true
        ));
    }
    // M, -M, smallest subnormal sums to that subnormal. Reversing it
    // rounds the small value away before cancellation, producing positive zero.
    assert_eq!(vectors[13].sum, "0000000000000001");
    assert_eq!(
        vectors[13]
            .values
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        [0x7fef_ffff_ffff_ffff, 0xffef_ffff_ffff_ffff, 1]
    );
    let forward = execute(
        &db,
        "FROM facts |> WHERE k = 13 |> AGGREGATE SUM(v) AS value, COUNT(*) AS n GROUP AND ORDER BY k",
        Path::Memory,
        false,
    );
    assert!(answers(&forward, &[(13, &vectors[13])], false));
    let reversed = execute(
        &db,
        "FROM facts |> WHERE k = 13 |> ORDER BY position DESC |> AGGREGATE SUM(v) AS value, COUNT(*) AS n GROUP AND ORDER BY k",
        Path::Memory,
        false,
    );
    assert_eq!(reversed, [(13, 0, 3)]);
    assert!(!answers(&reversed, &[(13, &vectors[13])], false));
    db.close().unwrap();
}

#[test]
fn every_overflow_vector_is_checked_after_disk_reduction() {
    let vectors = vectors();
    let directory = Directory::new();
    let db = database(&directory, &vectors);
    // One failing grouped query cannot establish later groups' outcomes. Select
    // each overflowing vector separately and require its final SUM error after
    // real spill and merging, including the original aggregate source span.
    for (key, vector) in vectors.iter().enumerate() {
        if vector.sum == "overflow" {
            let sql = OVERFLOW.replacen("FROM facts", &format!("FROM facts |> WHERE k = {key}"), 1);
            execute(&db, &sql, Path::Disk, true);
        }
    }
    let expected: Vec<_> = vectors.iter().enumerate().collect();
    assert!(answers(
        &execute(&db, AVERAGE, Path::Memory, false),
        &expected,
        true
    ));
    db.close().unwrap();
}
