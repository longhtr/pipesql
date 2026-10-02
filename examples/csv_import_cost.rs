//! Measure CSV import through a committed transaction, then check every value.
//!
//! Pass a new absolute output directory, `fixed`, `text` or `long-text`, 4 or 64 columns,
//! and 64 or 256 decoder rows. Each process performs one warm-up and five samples
//! on fresh declared databases. Fixed/short text use 8,192 rows; long text uses
//! 128 rows with 65,536-byte Unicode fields. The caller owns the directory.
//!
//! Only import is timed: input construction, declaration, receipt resolution,
//! close/reopen and complete query validation are separate. CSV bytes and expected
//! answer storage are caller allocations. Reported memory is the database's
//! reservation after import, not peak allocation or process memory. Input is read
//! from resident bytes; this does not measure filesystem input throughput.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, CommitResolution, Config, CsvLimits,
    DataType, Database, DateValue, ImportLimits, QueryStep, Value,
};
use std::{fmt::Write as _, fs, path::Path, time::Instant};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const MEMORY: u64 = 32_000_000;
const TEMPORARY: u64 = 64_000_000;

#[derive(Clone, Copy, PartialEq)]
enum Profile {
    Fixed,
    Text,
    LongText,
}

#[derive(Clone, Copy)]
struct Input {
    profile: Profile,
    columns: usize,
    rows: usize,
    batch_rows: u16,
}

impl Input {
    fn profile(&self) -> &'static str {
        match self.profile {
            Profile::Fixed => "fixed",
            Profile::Text => "text",
            Profile::LongText => "long-text",
        }
    }

    fn kind(&self, column: usize) -> DataType {
        if column == 0 {
            return DataType::Int64;
        }
        match (column - 1) % 3 {
            0 => DataType::Int64,
            1 => DataType::Double,
            _ if self.profile != Profile::Fixed => DataType::String,
            _ => DataType::Date,
        }
    }

    fn names(&self) -> Vec<String> {
        (0..self.columns)
            .map(|column| {
                if column == 0 {
                    "id".into()
                } else {
                    format!("c{column}")
                }
            })
            .collect()
    }

    fn csv(&self, names: &[String]) -> String {
        let mut csv = names.iter().rev().cloned().collect::<Vec<_>>().join(",");
        csv.push('\n');
        let long = if self.profile == Profile::LongText {
            "雪🙂".repeat(9362)
        } else {
            String::new()
        };
        for row in 0..self.rows {
            for column in (0..self.columns).rev() {
                if column + 1 != self.columns {
                    csv.push(',');
                }
                if column == 0 {
                    write!(csv, "{row}").unwrap();
                    continue;
                }
                let null_mask = [0x8421_u16, 0x1248, 0xaaaa, 0x1111][column % 4];
                if null_mask & (1 << (row % 16)) != 0 {
                    csv.push_str(r"\N");
                    continue;
                }
                // Every value class spans a whole NULL-pattern period.
                match self.kind(column) {
                    DataType::Int64 => {
                        let value = match (row / 16) % 4 {
                            0 => i64::MIN + column as i64,
                            1 => i64::MAX - column as i64,
                            2 => 9_007_199_254_740_993 + column as i64,
                            _ => -9_007_199_254_740_993 - column as i64,
                        };
                        write!(csv, "{value}").unwrap();
                    }
                    DataType::Double => {
                        csv.push_str(["-0", "1.5", "-2.25", "5e-324"][(row / 16) % 4])
                    }
                    DataType::Date => csv.push_str(
                        ["0001-01-01", "1969-12-31", "1970-01-01", "9999-12-31"][(row / 16) % 4],
                    ),
                    DataType::String
                        if self.profile == Profile::LongText
                            && (row / 16) % (self.columns / 3) == column / 3 - 1 =>
                    {
                        csv.push_str(&long);
                        write!(csv, "{column:02}").unwrap();
                    }
                    DataType::String => csv.push_str(
                        [
                            "plain",
                            "\"comma,value\"",
                            "雪🙂",
                            "\"line\n\"\"quote\"\"\"",
                        ][(row / 16) % 4],
                    ),
                }
            }
            csv.push('\n');
        }
        csv
    }

    // Literal typed answers do not round-trip through CSV parsing or conversion.
    // These row positions describe the input masks independently of their bits.
    fn matches_value(&self, row: usize, column: usize, actual: Option<Value<'_>>) -> bool {
        if column == 0 {
            return actual == Some(Value::Int64(row as i64));
        }
        let null_rows: &[usize] = match column % 4 {
            0 => &[0, 5, 10, 15],
            1 => &[3, 6, 9, 12],
            2 => &[1, 3, 5, 7, 9, 11, 13, 15],
            _ => &[0, 4, 8, 12],
        };
        if null_rows.contains(&(row % 16)) {
            return actual == Some(Value::Null);
        }
        let class = (row / 16) % 4;
        match (column - 1) % 3 {
            0 => {
                actual
                    == Some(Value::Int64(match class {
                        0 => -9_223_372_036_854_775_808 + column as i64,
                        1 => 9_223_372_036_854_775_807 - column as i64,
                        2 => 9_007_199_254_740_993 + column as i64,
                        _ => -9_007_199_254_740_993 - column as i64,
                    }))
            }
            1 => {
                matches!(actual, Some(Value::Double(value)) if value.to_bits() == [-0.0, 1.5, -2.25, f64::from_bits(1)][class].to_bits())
            }
            _ if self.profile == Profile::LongText
                && column
                    == if self.columns == 4 {
                        3
                    } else {
                        [3, 6, 9, 12, 15, 18, 21, 24][row / 16]
                    } =>
            {
                let Some(Value::String(value)) = actual else {
                    return false;
                };
                let value = value.as_str();
                value.len() == 65_536
                    && value.ends_with(&format!("{column:02}"))
                    && value.as_bytes()[..65_534]
                        .as_chunks::<7>()
                        .0
                        .iter()
                        .all(|part| *part == [0xe9, 0x9b, 0xaa, 0xf0, 0x9f, 0x99, 0x82])
            }
            _ if self.profile != Profile::Fixed => {
                matches!(actual, Some(Value::String(value)) if value.as_str() == ["plain", "comma,value", "雪🙂", "line\n\"quote\""][class])
            }
            _ => {
                actual
                    == Some(Value::Date(
                        DateValue::from_days_since_unix_epoch([-719_162, -1, 0, 2_932_896][class])
                            .unwrap(),
                    ))
            }
        }
    }
}

fn verify(db: &Database, input: Input, names: &[String], sql: &str) -> Result<usize> {
    let baseline = db.reserved_memory_bytes();
    let outcome = (|| {
        let query = db.prepare(sql)?;
        if query.result_column_count() != input.columns
            || names.iter().enumerate().any(|(column, name)| {
                query.result_column(column).is_none_or(|actual| {
                    actual.name != Some(name.as_str())
                        || actual.data_type != input.kind(column)
                        || actual.nullable != (column != 0)
                })
            })
        {
            return Err("CSV import result schema differs".into());
        }
        let cancel = CancellationToken::new();
        let mut result = db.execute(&query, &cancel)?;
        let mut seen = vec![false; input.rows];
        let mut rows = 0;
        for _ in 0..2_000_000 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Rows(batch) => {
                    if batch.column_count() != input.columns {
                        return Err("CSV import result width differs".into());
                    }
                    for row in 0..batch.len() {
                        let Some(Value::Int64(id)) = batch.value(row, 0) else {
                            return Err("CSV import row identity differs".into());
                        };
                        let id = usize::try_from(id)?;
                        if id >= seen.len() || seen[id] {
                            return Err("CSV import duplicate or unexpected row".into());
                        }
                        for column in 0..input.columns {
                            if !input.matches_value(id, column, batch.value(row, column)) {
                                return Err(format!(
                                    "CSV import value differs at row {id}, column {column}"
                                )
                                .into());
                            }
                        }
                        seen[id] = true;
                        rows += 1;
                    }
                }
                QueryStep::Finished => {
                    if rows != input.rows || seen.iter().any(|seen| !seen) {
                        return Err("CSV import answer is incomplete".into());
                    }
                    return Ok(rows);
                }
                QueryStep::Failed(_) => {
                    return Err(result.into_error().expect("failed query").into());
                }
            }
        }
        Err("CSV import verification exceeded its work bound".into())
    })();
    if db.reserved_memory_bytes() != baseline || db.reserved_temp_bytes() != 0 {
        return Err("CSV import query resources remain".into());
    }
    outcome
}

struct Sample {
    elapsed_ns: u64,
    rows: usize,
    memory: u64,
}

fn sample(path: &Path, input: Input, names: &[String], csv: &[u8]) -> Result<Sample> {
    let config = Config::new(MEMORY, TEMPORARY)?;
    let cancel = CancellationToken::new();
    let db = Database::create_empty(path, config)?;
    let schema: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(column, name)| ColumnDeclaration {
            name,
            data_type: input.kind(column),
            nullable: column != 0,
        })
        .collect();
    db.declare_table("facts", &schema, &cancel)?;
    let baseline = db.reserved_memory_bytes();
    let limits = ImportLimits {
        csv: CsvLimits {
            input_bytes: csv.len() as u64,
            rows: input.rows as u64,
            record_bytes: if input.profile == Profile::LongText {
                131_072
            } else {
                8192
            },
            field_bytes: if input.profile == Profile::LongText {
                65_536
            } else {
                128
            },
            batch_rows: input.batch_rows,
            batch_text_bytes: if input.profile == Profile::LongText {
                4_194_304
            } else if input.profile == Profile::Text {
                (input.columns * usize::from(input.batch_rows) * 64) as u32
            } else {
                1
            },
        },
        append: AppendLimits {
            batches: if input.profile == Profile::LongText {
                input.rows as u32
            } else {
                input.rows.div_ceil(usize::from(input.batch_rows)) as u32
            },
            encoded_bytes: 32_000_000,
        },
    };
    let mut issued = None;
    let start = Instant::now();
    let commit = db.import_csv("facts", csv, limits, &cancel, |token| {
        issued = Some(token);
        Ok(())
    })?;
    let elapsed_ns = start.elapsed().as_nanos().try_into()?;
    if issued != Some(commit.transaction())
        || commit.generation() != 2
        || db.resolve_commit(commit.transaction())? != CommitResolution::Durable(commit)
    {
        return Err("CSV import receipt differs".into());
    }
    if db.reserved_memory_bytes() != baseline || db.reserved_temp_bytes() != 0 {
        return Err("CSV import resources remain".into());
    }
    db.close()?;
    let db = Database::open(path, config)?;
    if db.resolve_commit(commit.transaction())? != CommitResolution::Durable(commit) {
        return Err("CSV import receipt differs after reopen".into());
    }
    let rows = verify(&db, input, names, "FROM facts")?;
    db.close()?;
    Ok(Sample {
        elapsed_ns,
        rows,
        memory: baseline,
    })
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 4 {
        return Err(
            "supply new absolute directory, fixed|text|long-text, 4|64 columns and 64|256 batch rows".into(),
        );
    }
    let root = Path::new(&args[0]);
    if !root.is_absolute() {
        return Err("output directory must be absolute".into());
    }
    let profile = match args[1].to_str() {
        Some("fixed") => Profile::Fixed,
        Some("text") => Profile::Text,
        Some("long-text") => Profile::LongText,
        _ => return Err("choose fixed, text or long-text".into()),
    };
    let columns = args[2].to_str().ok_or("invalid columns")?.parse()?;
    let batch_rows = args[3].to_str().ok_or("invalid batch rows")?.parse()?;
    if ![4, 64].contains(&columns) || ![64, 256].contains(&batch_rows) {
        return Err("unsupported import geometry".into());
    }
    let input = Input {
        profile,
        columns,
        rows: if profile == Profile::LongText {
            128
        } else {
            8192
        },
        batch_rows,
    };
    let names = input.names();
    let csv = input.csv(&names);
    fs::create_dir(root)?;
    fs::write(root.join("input.csv"), &csv)?;
    println!(
        "input profile={} columns={columns} rows={} batch_rows={batch_rows} input_bytes={} memory_limit={MEMORY} temp_limit={TEMPORARY}",
        input.profile(),
        input.rows,
        csv.len()
    );
    let warm = sample(&root.join("warmup"), input, &names, csv.as_bytes())?;
    println!("warmup rows={} generation=2", warm.rows);
    for index in 0..5 {
        let checked = sample(
            &root.join(format!("sample-{index}")),
            input,
            &names,
            csv.as_bytes(),
        )?;
        println!(
            "sample={index} elapsed_ns={} rows={} generation=2 memory={} temporary=0",
            checked.elapsed_ns, checked.rows, checked.memory
        );
    }
    println!("status=finished");
    Ok(())
}

#[cfg(test)]
#[path = "../test/support/mod.rs"]
mod support;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maximum_unicode_fields_reopen_with_independent_column_bursts() {
        let directory = support::Directory::new();
        for columns in [4, 64] {
            for batch_rows in [64, 256] {
                let input = Input {
                    profile: Profile::LongText,
                    columns,
                    rows: 128,
                    batch_rows,
                };
                let names = input.names();
                let csv = input.csv(&names);
                assert!(csv.len() < 8_000_000);
                let path = directory.0.join(format!("{columns}-{batch_rows}"));
                assert_eq!(
                    sample(&path, input, &names, csv.as_bytes()).unwrap().rows,
                    128
                );
                let mut wrong = csv.clone();
                let last = wrong.rfind('雪').unwrap();
                wrong.replace_range(last..last + 3, "雨");
                let error = sample(
                    &directory.0.join(format!("wrong-{columns}-{batch_rows}")),
                    input,
                    &names,
                    wrong.as_bytes(),
                )
                .err()
                .expect("changed long text must fail the complete answer check");
                assert!(
                    error.to_string().contains("CSV import value differs"),
                    "{error}"
                );
            }
        }
    }

    #[test]
    fn complete_imports_reopen_and_reject_wrong_answers_and_late_errors() {
        let directory = support::Directory::new();
        for profile in [Profile::Fixed, Profile::Text] {
            for columns in [4, 64] {
                let input = Input {
                    profile,
                    columns,
                    rows: 257,
                    batch_rows: 64,
                };
                let names = input.names();
                let csv = input.csv(&names);
                let path = directory.0.join(format!("{}-{columns}", input.profile()));
                assert_eq!(
                    sample(&path, input, &names, csv.as_bytes()).unwrap().rows,
                    257
                );
                let db = Database::open(&path, Config::new(MEMORY, TEMPORARY).unwrap()).unwrap();
                assert_eq!(
                    verify(&db, input, &names, "FROM facts |> WHERE id < 256")
                        .unwrap_err()
                        .to_string(),
                    "CSV import answer is incomplete"
                );
                assert_eq!(
                    verify(&db, input, &names, "FROM facts |> SET id=0")
                        .unwrap_err()
                        .to_string(),
                    "CSV import duplicate or unexpected row"
                );
                let error = verify(
                    &db,
                    input,
                    &names,
                    "FROM facts |> SET c1=CASE WHEN id<256 THEN c1 ELSE DIV(1, 256-id) END",
                )
                .unwrap_err();
                assert!(
                    matches!(
                        error.downcast_ref::<pipesql::Error>(),
                        Some(pipesql::Error::DivisionByZero { .. })
                    ),
                    "{error:?}"
                );
                assert!(
                    verify(&db, input, &names, "FROM facts |> SET c1=c1+1")
                        .unwrap_err()
                        .to_string()
                        .contains("CSV import value differs")
                );
                db.close().unwrap();
            }
        }
    }
}
