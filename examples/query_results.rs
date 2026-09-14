//! Rows are a prefix until Finished; terminal errors can outlive their query.
//! Supply a fresh absolute database path. The example leaves it there for inspection.
use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Config,
    DataType, Database, Error, PreparedQuery, QueryStep, Value,
};
use std::path::Path;

const OVERFLOW_SQL: &str = "FROM facts
|> ORDER BY amount
|> SELECT amount + 1 AS next_amount";
const ORDERED_SQL: &str = "FROM facts |> ORDER BY amount";
const PREFIX_ROWS: usize = 256;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("supply a fresh absolute database path")?;
    let path = Path::new(&path);
    if args.next().is_some() || !path.is_absolute() || path.try_exists()? {
        return Err("expected one fresh absolute database path".into());
    }
    let db = create_facts(path)?;
    let resident = db.reserved_memory_bytes();

    // Preparation retains the expression's span, not the caller's SQL string.
    let source = OVERFLOW_SQL.to_owned();
    let overflow = db.prepare(&source)?;
    drop(source);
    let prepared = db.reserved_memory_bytes();
    let error = overflow_after_rows(&db, &overflow)?;
    require_released(&db, prepared)?;
    drop(overflow);
    require_released(&db, resident)?;
    match error {
        Error::ArithmeticOverflow {
            operation: "addition",
            span,
        } if span.start() == 40 && span.end() == 50 => (),
        _ => return Err("expected owned addition overflow at bytes 40..50".into()),
    }
    println!("overflow: prefix=256 values=1..256; addition bytes=40..50; owners released");

    let ordered = db.prepare(ORDERED_SQL)?;
    let prepared = db.reserved_memory_bytes();
    let (error, prefix) = cancel_after_rows(&db, &ordered)?;
    require_released(&db, prepared)?;
    if !matches!(error, Error::Cancelled) {
        return Err("expected owned cancellation error".into());
    }
    println!("cancelled: prefix={prefix}; execution released; plan retained");

    // Cancellation tokens are one-way. Reuse the immutable plan with a fresh token.
    finish_ordered(&db, &ordered)?;
    require_released(&db, prepared)?;
    drop(ordered);
    require_released(&db, resident)?;
    db.close()?;
    println!("finished: rows=257 last=9223372036854775807; owners released");
    Ok(())
}

fn create_facts(path: &Path) -> Result<Database, Error> {
    let db = Database::create_empty(path, Config::new(8_000_000, 4_000_000)?)?;
    let cancel = CancellationToken::new();
    db.declare_table(
        "facts",
        &[ColumnDeclaration {
            name: "amount",
            data_type: DataType::Int64,
            nullable: false,
        }],
        &cancel,
    )?;
    let mut append = db.begin_append(
        "facts",
        AppendLimits {
            batches: 2,
            encoded_bytes: 20_000,
        },
        &cancel,
    )?;
    let values = std::array::from_fn::<_, PREFIX_ROWS, _>(|index| index as i64);
    append.write(
        &[ColumnInput {
            values: ColumnValues::Int64(&values),
            validity: &[0xff; 32],
        }],
        &cancel,
    )?;
    append.write(
        &[ColumnInput {
            values: ColumnValues::Int64(&[i64::MAX]),
            validity: &[1],
        }],
        &cancel,
    )?;
    append.commit(&cancel)?;
    Ok(db)
}

fn overflow_after_rows(
    db: &Database,
    query: &PreparedQuery<'_>,
) -> Result<Error, Box<dyn std::error::Error>> {
    let cancel = CancellationToken::new();
    let mut result = db.execute(query, &cancel)?;
    let mut rows = 0;
    loop {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                if batch.column_count() != 1 {
                    return Err("unexpected result schema".into());
                }
                for row in 0..batch.len() {
                    if rows >= PREFIX_ROWS
                        || batch.value(row, 0) != Some(Value::Int64(rows as i64 + 1))
                    {
                        return Err("unexpected overflow prefix".into());
                    }
                    rows += 1;
                }
                // The batch borrow ends here; the next step can reuse its storage.
            }
            QueryStep::Finished => return Err("overflow query unexpectedly finished".into()),
            QueryStep::Failed(_) => break,
        }
    }
    if rows != PREFIX_ROWS {
        return Err("overflow did not follow the complete expected prefix".into());
    }
    if !matches!(
        result.step(),
        QueryStep::Failed(Error::ArithmeticOverflow { .. })
    ) {
        return Err("arithmetic failure did not remain terminal".into());
    }
    // Failed lends an error. Consuming the result moves that error out and drops
    // the remaining result handle, so the caller can release the prepared plan.
    result
        .into_error()
        .ok_or_else(|| "missing owned arithmetic error".into())
}

fn cancel_after_rows(
    db: &Database,
    query: &PreparedQuery<'_>,
) -> Result<(Error, usize), Box<dyn std::error::Error>> {
    let cancel = CancellationToken::new();
    let mut result = db.execute(query, &cancel)?;
    let mut rows = 0;
    loop {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                if batch.column_count() != 1 {
                    return Err("unexpected result schema".into());
                }
                if cancel.is_cancelled() {
                    return Err("rows appeared after the cancellation boundary".into());
                }
                for row in 0..batch.len() {
                    if batch.value(row, 0) != Some(Value::Int64(rows as i64)) {
                        return Err("unexpected cancellation prefix".into());
                    }
                    rows += 1;
                }
                cancel.cancel();
            }
            QueryStep::Finished => return Err("cancelled query unexpectedly finished".into()),
            QueryStep::Failed(_) => break,
        }
    }
    if rows == 0 || rows >= 257 || !matches!(result.step(), QueryStep::Failed(Error::Cancelled)) {
        return Err("expected stable cancellation after a nonempty partial result".into());
    }
    let error = result
        .into_error()
        .ok_or("missing owned cancellation error")?;
    Ok((error, rows))
}

fn finish_ordered(
    db: &Database,
    query: &PreparedQuery<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cancel = CancellationToken::new();
    let mut result = db.execute(query, &cancel)?;
    let mut rows = 0;
    loop {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                if batch.column_count() != 1 {
                    return Err("unexpected result schema".into());
                }
                for row in 0..batch.len() {
                    let expected = if rows == PREFIX_ROWS {
                        i64::MAX
                    } else {
                        rows as i64
                    };
                    if rows > PREFIX_ROWS || batch.value(row, 0) != Some(Value::Int64(expected)) {
                        return Err("unexpected complete result".into());
                    }
                    rows += 1;
                }
            }
            QueryStep::Finished => break,
            QueryStep::Failed(error) => return Err(format!("healthy query failed: {error}").into()),
        }
    }
    if rows != 257 || !matches!(result.step(), QueryStep::Finished) {
        return Err("healthy query did not complete all 257 rows".into());
    }
    Ok(())
}

fn require_released(db: &Database, baseline: u64) -> Result<(), Box<dyn std::error::Error>> {
    if db.reserved_memory_bytes() != baseline || db.reserved_temp_bytes() != 0 {
        return Err("query owners did not return to the prepared/resident baseline".into());
    }
    Ok(())
}
