//! Check memory charges as successive queries change their aggregate layout.
//!
//! Two rows share a NULL key; the other keys are "a" and "b", giving three groups.
//! Queries vary the number and mix of floating sums, integer sums and minima. Running them in one
//! process retains allocator history between layouts instead of starting each
//! shape with a fresh heap.
//!
//! After preparation and execution setup, compare changes in live requested and
//! allocator-reported usable bytes with query charges. Consume every group and
//! check answers derived from the four rows, then drop both query owners and
//! require counters to return to baseline. Temporary-storage samples must remain
//! zero. This small workload checks these layouts and sampling points; it does
//! not establish a whole-process memory bound.

use super::workload::live;
use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, QueryStep, Value,
};
use std::path::Path;

#[derive(Clone, Copy, Debug)]
enum Family {
    FloatSum,
    IntegerSum,
    IntegerMinimum,
    Mixed,
}
impl Family {
    fn at(self, column: usize) -> Self {
        match self {
            Self::Mixed => [Self::FloatSum, Self::IntegerSum, Self::IntegerMinimum][column % 3],
            other => other,
        }
    }
}

pub(super) fn run(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(root)?;
    let cancel = CancellationToken::new();
    let mut deficits = 0;
    for budget in [4_000_000, 16_000_000] {
        let db = Database::create_empty(
            &root.join(budget.to_string()),
            Config::new(budget, 32_000_000)?,
        )?;
        db.declare_table(
            "facts",
            &[
                ColumnDeclaration {
                    name: "note",
                    data_type: DataType::String,
                    nullable: true,
                },
                ColumnDeclaration {
                    name: "amount",
                    data_type: DataType::Int64,
                    nullable: false,
                },
                ColumnDeclaration {
                    name: "measure",
                    data_type: DataType::Double,
                    nullable: true,
                },
            ],
            &cancel,
        )?;
        let mut append = db.begin_append(
            "facts",
            AppendLimits {
                batches: 1,
                encoded_bytes: 100_000,
            },
            &cancel,
        )?;
        append.write(
            &[
                ColumnInput {
                    values: ColumnValues::String(&["", "", "a", "b"]),
                    validity: &[12],
                },
                ColumnInput {
                    values: ColumnValues::Int64(&[2, 4, 6, 8]),
                    validity: &[15],
                },
                ColumnInput {
                    values: ColumnValues::Double(&[1.5, 0., 3.5, 5.5]),
                    validity: &[13],
                },
            ],
            &cancel,
        )?;
        append.commit(&cancel)?;
        for width in [1, 3, 5, 7, 9] {
            for family in [
                Family::FloatSum,
                Family::IntegerSum,
                Family::IntegerMinimum,
                Family::Mixed,
            ] {
                let expressions: Vec<_> = (0..width)
                    .map(|i| match family.at(i) {
                        Family::FloatSum => format!("SUM(measure+{i}.0) AS a{i}"),
                        Family::IntegerSum => format!("SUM(amount+{i}) AS a{i}"),
                        Family::IntegerMinimum => format!("MIN(amount+{i}) AS a{i}"),
                        Family::Mixed => unreachable!("mixed state selected by column"),
                    })
                    .collect();
                let sql = format!(
                    "FROM facts |> AGGREGATE {} GROUP AND ORDER BY note",
                    expressions.join(", ")
                );
                println!("mixed SQL budget={budget}: {sql}");
                // Build the SQL first so its caller-owned strings are present
                // on both sides of the engine allocation measurement.
                let before = live();
                let memory = db.reserved_memory_bytes();
                let prepared = db.prepare(&sql)?;
                let mut rows = db.execute(&prepared, &cancel)?;
                let after = live();
                let requested = after.requested.checked_sub(before.requested).unwrap();
                let usable = after.usable.checked_sub(before.usable).unwrap();
                let charge = prepared.accounted_memory_bytes() + rows.accounted_memory_bytes();
                assert_eq!(db.reserved_memory_bytes(), memory + charge);
                assert!(requested as u64 <= charge);
                deficits += usize::from(usable as u64 > charge);
                println!(
                    "mixed held budget={budget} width={width} family={family:?} charge={charge} requested={} usable={}",
                    requested, usable
                );
                let mut seen = 0;
                let mut finished = false;
                let mut peak_temp = 0;
                for _ in 0..200_000 {
                    match rows.step() {
                        QueryStep::Progress => (),
                        QueryStep::Failed(error) => panic!("mixed rows: {error:?}"),
                        QueryStep::Finished => {
                            finished = true;
                            break;
                        }
                        QueryStep::Rows(batch) => {
                            assert_eq!(batch.column_count(), width + 1);
                            for row in 0..batch.len() {
                                assert!(seen < 3);
                                match (seen, batch.value(row, 0)) {
                                    (0, Some(Value::Null)) => (),
                                    (1, Some(Value::String(text))) => {
                                        assert_eq!(text.as_str(), "a")
                                    }
                                    (2, Some(Value::String(text))) => {
                                        assert_eq!(text.as_str(), "b")
                                    }
                                    other => panic!("mixed key: {other:?}"),
                                }
                                for i in 0..width {
                                    let expected = match family.at(i) {
                                        Family::FloatSum => {
                                            // The NULL-key group has one present measure;
                                            // its second row must not contribute the offset.
                                            Value::Double([1.5, 3.5, 5.5][seen] + i as f64)
                                        }
                                        Family::IntegerSum => Value::Int64(
                                            [6, 6, 8][seen] + [2, 1, 1][seen] * i as i64,
                                        ),
                                        Family::IntegerMinimum => {
                                            Value::Int64([2, 6, 8][seen] + i as i64)
                                        }
                                        Family::Mixed => {
                                            unreachable!("mixed state selected by column")
                                        }
                                    };
                                    assert_eq!(batch.value(row, i + 1), Some(expected));
                                }
                                seen += 1;
                            }
                        }
                    }
                    peak_temp = peak_temp.max(db.reserved_temp_bytes());
                }
                assert!(finished && seen == 3);
                assert_eq!(peak_temp, 0, "small groups must exercise the hash path");
                drop(rows);
                drop(prepared);
                assert_eq!(live(), before);
                assert_eq!(db.reserved_memory_bytes(), memory);
                assert_eq!(db.reserved_temp_bytes(), 0);
            }
        }
        db.close()?;
    }
    println!("mixed shapes completed: 40 cases, deficits={deficits}");
    assert_eq!(deficits, 0, "mixed usable allocation deficits");
    Ok(())
}
