//! Show why grouping and equality joins treat the same values differently.
//!
//! Eight rows contain two zeros with different signs, two different NaNs, two
//! NULLs and two ones. GROUP BY and DISTINCT place them in four classes. An
//! equality join matches zeros and ones, but neither NaNs nor NULLs. COUNT(v)
//! counts NaNs as values while excluding NULLs.
//!
//! The program writes and reopens the rows, checks literal results for each
//! operation, and prints the comparison. It also checks the original DOUBLE bits:
//! treating two values as one group must not invent a new stored representation.
//! Pass one fresh absolute database path, which remains available afterward.
//! Run with `cargo run --release --offline --locked --example equality -- PATH`.
//! The literal expectations below distinguish grouping from ordinary equality:
//! grouping retains one original representative per class, while the self-join
//! emits four zero pairs and four one pairs. No query without ORDER BY promises
//! the printed class order; the checker arranges that report itself.
//! `docs/sql/README.md` defines the comparison rules.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, QueryStep, Value,
};
use std::io::Write;
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const NEGATIVE_ZERO: u64 = 0x8000_0000_0000_0000;
const NAN_A: u64 = 0x7ff8_0000_0000_0042;
const NAN_B: u64 = 0xfff8_0000_0000_1234;
const ONE: u64 = 0x3ff0_0000_0000_0000;
const EXPECTED_GROUP_ROWS: [i64; 4] = [2, 2, 2, 2];
const CLASSES: [&str; 4] = ["null", "nan", "zero", "one"];

// Keep DOUBLE bits so comparisons distinguish zero signs and NaN payloads.
// Comparing f64 values directly would treat the zeros as equal and NaNs as unequal.
#[derive(Debug, PartialEq, Eq)]
enum Cell {
    Null,
    Integer(i64),
    Double(u64),
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("supply a fresh absolute database path")?;
    let path = Path::new(&path);
    if args.next().is_some() || !path.is_absolute() || path.try_exists()? {
        return Err("expected one fresh absolute database path".into());
    }
    create_samples(path)?;
    let db = Database::open(path, Config::new(8_000_000, 8_000_000)?)?;
    let input = read_numeric(
        &db,
        "FROM samples |> ORDER BY id |> SELECT id, v",
        &[
            ("id", DataType::Int64, false),
            ("v", DataType::Double, true),
        ],
    )?;
    let expected = [
        Cell::Double(NEGATIVE_ZERO),
        Cell::Double(0),
        Cell::Double(NAN_A),
        Cell::Double(NAN_B),
        Cell::Null,
        Cell::Null,
        Cell::Double(ONE),
        Cell::Double(ONE),
    ]
    .into_iter()
    .enumerate()
    .map(|(id, value)| vec![Cell::Integer(id as i64), value])
    .collect::<Vec<_>>();
    if input != expected {
        return Err("reopened rows differ from the literal bit and NULL oracle".into());
    }
    let counts = read_numeric(
        &db,
        "FROM samples |> AGGREGATE COUNT(*) AS entries, COUNT(v) AS present",
        &[
            ("entries", DataType::Int64, false),
            ("present", DataType::Int64, false),
        ],
    )?;
    if counts != [vec![Cell::Integer(8), Cell::Integer(6)]] {
        return Err("COUNT must distinguish present NaNs from NULL".into());
    }
    let groups = read_numeric(
        &db,
        "FROM samples |> AGGREGATE COUNT(*) AS entries GROUP BY v",
        &[
            ("v", DataType::Double, true),
            ("entries", DataType::Int64, false),
        ],
    )?;
    let mut grouped_rows = [0; 4];
    for row in &groups {
        let [value, Cell::Integer(rows)] = row.as_slice() else {
            return Err("expected one key and one group count".into());
        };
        let class = fixture_class(value)?;
        if grouped_rows[class] != 0 || *rows <= 0 {
            return Err("group classes must be unique and nonempty".into());
        }
        grouped_rows[class] = *rows;
    }
    if grouped_rows != EXPECTED_GROUP_ROWS {
        return Err("group counts differ from the literal oracle".into());
    }
    let distinct = read_numeric(
        &db,
        "FROM samples |> SELECT v |> DISTINCT",
        &[("v", DataType::Double, true)],
    )?;
    let mut representatives = [0; 4];
    for row in &distinct {
        representatives[fixture_class(&row[0])?] += 1;
    }
    if representatives != [1, 1, 1, 1] {
        return Err("DISTINCT must retain one input representative per class".into());
    }

    let mut retained = Vec::new();
    for (predicate, expected_ids) in [
        ("v = 0", &[0, 1][..]),
        ("v != 0", &[2, 3, 6, 7][..]),
        ("v IS NULL", &[4, 5][..]),
        ("v IS DISTINCT FROM 0", &[2, 3, 4, 5, 6, 7][..]),
    ] {
        let rows = read_numeric(
            &db,
            &format!("FROM samples |> WHERE {predicate} |> ORDER BY id |> SELECT id"),
            &[("id", DataType::Int64, false)],
        )?;
        let expected = expected_ids
            .iter()
            .map(|id| vec![Cell::Integer(*id)])
            .collect::<Vec<_>>();
        if rows != expected {
            return Err(format!("unexpected retained IDs for {predicate}").into());
        }
        retained.push((predicate, rows.len()));
    }
    let pairs = read_numeric(
        &db,
        "FROM samples AS l |> JOIN samples AS r ON l.v = r.v |> ORDER BY l.id, r.id |> SELECT l.id AS left_id, r.id AS right_id",
        &[
            ("left_id", DataType::Int64, false),
            ("right_id", DataType::Int64, false),
        ],
    )?;
    let expected = [
        [0, 0],
        [0, 1],
        [1, 0],
        [1, 1],
        [6, 6],
        [6, 7],
        [7, 6],
        [7, 7],
    ]
    .map(|pair| Vec::from(pair.map(Cell::Integer)));
    if pairs != expected {
        return Err("equality join pairs differ from the literal oracle".into());
    }
    db.close()?;

    // Delay the report until all checks pass, so partial success cannot look
    // like a complete comparison.
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    for row in &input {
        let Cell::Integer(id) = row[0] else {
            unreachable!("checked input ID")
        };
        match row[1] {
            Cell::Null => writeln!(output, "input id={id} value=NULL")?,
            Cell::Double(bits) => writeln!(output, "input id={id} bits={bits:016x}")?,
            _ => unreachable!("checked input type"),
        }
    }
    writeln!(output, "counts rows=8 present=6")?;
    for (name, rows) in CLASSES.into_iter().zip(grouped_rows) {
        writeln!(output, "group class={name} rows={rows}")?;
    }
    writeln!(
        output,
        "distinct classes={} representatives=original-input",
        distinct.len()
    )?;
    for (predicate, rows) in retained {
        writeln!(output, "filter={predicate} rows={rows}")?;
    }
    for row in &pairs {
        let [Cell::Integer(left), Cell::Integer(right)] = row.as_slice() else {
            unreachable!("checked join IDs")
        };
        writeln!(output, "join left={left} right={right}")?;
    }
    writeln!(output, "status=equality-checked")?;
    output.flush()?;
    Ok(())
}

/// Classify only this example's literal values. A zero or NaN must retain one
/// of the input bit patterns; a new representation fails the check.
fn fixture_class(value: &Cell) -> Result<usize> {
    match value {
        Cell::Null => Ok(0),
        Cell::Double(NAN_A | NAN_B) => Ok(1),
        Cell::Double(0 | NEGATIVE_ZERO) => Ok(2),
        Cell::Double(ONE) => Ok(3),
        _ => Err("result is not an original fixture representative".into()),
    }
}

fn create_samples(path: &Path) -> Result<()> {
    let db = Database::create_empty(path, Config::new(8_000_000, 8_000_000)?)?;
    let cancel = CancellationToken::new();
    db.declare_table(
        "samples",
        &[
            ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "v",
                data_type: DataType::Double,
                nullable: true,
            },
        ],
        &cancel,
    )?;
    // Different numbers sit beneath the two NULLs. Clearing their validity
    // bits must hide both numbers from SQL, regardless of those stored bits.
    let values = [
        NEGATIVE_ZERO,
        0,
        NAN_A,
        NAN_B,
        0x4045_0000_0000_0000,
        0xfff0_0000_0000_0000,
        ONE,
        ONE,
    ]
    .map(f64::from_bits);
    let mut append = db.begin_append(
        "samples",
        AppendLimits {
            batches: 1,
            encoded_bytes: 100_000,
        },
        &cancel,
    )?;
    append.write(
        &[
            ColumnInput {
                values: ColumnValues::Int64(&[0, 1, 2, 3, 4, 5, 6, 7]),
                validity: &[255],
            },
            ColumnInput {
                values: ColumnValues::Double(&values),
                validity: &[0b1100_1111],
            },
        ],
        &cancel,
    )?;
    append.commit(&cancel)?;
    db.close()?;
    Ok(())
}

/// Copy a small query result after checking its schema, then verify that dropping
/// the result and prepared query releases their separate memory reservations.
fn read_numeric(
    db: &Database,
    sql: &str,
    schema: &[(&str, DataType, bool)],
) -> Result<Vec<Vec<Cell>>> {
    let resident = db.reserved_memory_bytes();
    let query = db.prepare(sql)?;
    if query.result_column_count() != schema.len() {
        return Err("unexpected column count".into());
    }
    for (index, &(name, kind, nullable)) in schema.iter().enumerate() {
        let column = query.result_column(index).ok_or("missing result column")?;
        if column.name != Some(name) || column.data_type != kind || column.nullable != nullable {
            return Err("unexpected result schema".into());
        }
    }
    let prepared = db.reserved_memory_bytes();
    let cancel = CancellationToken::new();
    let mut result = db.execute(&query, &cancel)?;
    let mut rows = Vec::new();
    let mut finished = false;
    for _ in 0..10_000 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                if batch.is_empty() || batch.column_count() != schema.len() {
                    return Err("unexpected result batch shape".into());
                }
                for row in 0..batch.len() {
                    // Even the self-join has only eight expected pairs here. This bound
                    // catches duplicate output without letting the collector grow.
                    if rows.len() == 8 {
                        return Err("query exceeds the fixture's eight-row output bound".into());
                    }
                    let mut cells = Vec::new();
                    for column in 0..batch.column_count() {
                        cells.push(match batch.value(row, column).ok_or("missing cell")? {
                            Value::Null => Cell::Null,
                            Value::Int64(value) => Cell::Integer(value),
                            Value::Double(value) => Cell::Double(value.to_bits()),
                            _ => return Err("expected numeric fixture output".into()),
                        });
                    }
                    rows.push(cells);
                }
            }
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(_) => {
                return Err(result.into_error().ok_or("missing query error")?.into());
            }
        }
    }
    if !finished {
        return Err("fixture query exceeded its step bound".into());
    }
    drop(result);
    if db.reserved_memory_bytes() != prepared || db.reserved_temp_bytes() != 0 {
        return Err("query did not release execution ownership".into());
    }
    drop(query);
    if db.reserved_memory_bytes() != resident {
        return Err("query did not release preparation ownership".into());
    }
    Ok(rows)
}
