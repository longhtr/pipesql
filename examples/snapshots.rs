//! A prepared query keeps its snapshot across append and reclamation.
//! Supply a new absolute database path; the example leaves that database there.
use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, QueryResult, QueryStep, Value,
};
use std::path::PathBuf;

const QUERY: &str = "FROM sales |> ORDER BY amount |> SELECT amount";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(args.next().ok_or("supply a new absolute database path")?);
    if args.next().is_some() || !path.is_absolute() || path.try_exists()? {
        return Err("expected one new absolute database path".into());
    }
    let config = Config::new(16_000_000, 8_000_000)?;
    let cancel = CancellationToken::new();
    let db = Database::create_empty(&path, config)?;
    db.declare_table(
        "sales",
        &[ColumnDeclaration {
            name: "amount",
            data_type: DataType::Int64,
            nullable: false,
        }],
        &cancel,
    )?;
    append_amounts(&db, &[10, 20], &[0b11], &cancel)?;

    // Preparation owns the pin. Finishing an execution releases its buffers,
    // but this plan can still execute against the same immutable generation.
    let old = db.prepare(QUERY)?;
    verify_rows(db.execute(&old, &cancel)?, "old before append", &[10, 20])?;
    append_amounts(&db, &[30], &[0b1], &cancel)?;
    let current = db.prepare(QUERY)?;

    // Reclamation protects both pinned generations and committed receipts.
    // Its return value counts removed names, not bytes freed or rows removed.
    db.reclaim(&cancel)?;
    verify_rows(db.execute(&old, &cancel)?, "old after reclaim", &[10, 20])?;
    verify_rows(
        db.execute(&current, &cancel)?,
        "new after reclaim",
        &[10, 20, 30],
    )?;

    drop(old);
    db.reclaim(&cancel)?;
    verify_rows(
        db.execute(&current, &cancel)?,
        "new after old plan drops",
        &[10, 20, 30],
    )?;
    // Plans borrow the database; release the remaining pin before closing it.
    drop(current);
    db.close()?;

    let db = Database::open(&path, config)?;
    let reopened = db.prepare(QUERY)?;
    verify_rows(db.execute(&reopened, &cancel)?, "reopened", &[10, 20, 30])?;
    drop(reopened);
    db.close()?;
    Ok(())
}

fn append_amounts(
    db: &Database,
    values: &[i64],
    validity: &[u8],
    cancel: &CancellationToken,
) -> Result<(), pipesql::Error> {
    let mut append = db.begin_append(
        "sales",
        AppendLimits {
            batches: 1,
            encoded_bytes: 1_024,
        },
        cancel,
    )?;
    append.write(
        &[ColumnInput {
            values: ColumnValues::Int64(values),
            validity,
        }],
        cancel,
    )?;
    append.commit(cancel)?;
    Ok(())
}

fn verify_rows(
    mut result: QueryResult<'_, '_>,
    label: &str,
    expected: &[i64],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut seen = 0;
    loop {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                if batch.column_count() != 1 {
                    return Err("unexpected result schema".into());
                }
                for row in 0..batch.len() {
                    if seen >= expected.len()
                        || batch.value(row, 0) != Some(Value::Int64(expected[seen]))
                    {
                        return Err("unexpected snapshot row".into());
                    }
                    seen += 1;
                }
            }
            QueryStep::Finished => break,
            QueryStep::Failed(_) => {
                return Err(result.into_error().ok_or("missing query error")?.into());
            }
        }
    }
    if seen != expected.len() {
        return Err("missing snapshot rows".into());
    }
    println!("{label}: {expected:?}");
    Ok(())
}
