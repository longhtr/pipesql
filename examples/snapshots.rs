//! Show that a prepared query keeps its rows while later appends change the table.
//!
//! Start with amounts 10 and 20 and prepare a query. After appending 30, the old
//! query still returns two rows; a newly prepared query returns three. Reclaiming
//! obsolete files must preserve both views until the corresponding queries drop.
//!
//! Transaction outcomes have a different lifetime. A commit receipt records what
//! succeeded; copying it does not keep an old table view alive. The database can
//! still resolve both committed and aborted transactions after reclamation and
//! reopening. This example checks those outcomes alongside the visible rows.
//! Run `cargo run --release --offline --locked --example snapshots --
//! /absolute/new-sales`. Require successful exit; the caller owns the remaining
//! database. Declaration creates generation 1; the two successful appends create
//! generations 2 and 3. The aborted attempt consumes no data generation.

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues, Commit,
    CommitResolution, Config, DataType, Database, QueryResult, QueryStep, TransactionId, Value,
};
use std::path::PathBuf;

const QUERY: &str = "FROM sales |> ORDER BY amount |> SELECT amount";
const APPEND_LIMITS: AppendLimits = AppendLimits {
    batches: 1,
    encoded_bytes: 1_024,
};

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
    let first = append_amounts(&db, &[10, 20], &[0b11], &cancel)?;

    // Preparing retains this table version. Finishing one execution frees its
    // buffers, but the prepared query can run again against the same rows.
    let old = db.prepare(QUERY)?;
    verify_rows(db.execute(&old, &cancel)?, "old before append", &[10, 20])?;

    // Beginning an append assigns a transaction ID before any rows are written.
    // Abort records its failure now; simply dropping it would leave recovery
    // to settle the unfinished transaction when the database is reopened.
    let pending = db.begin_append("sales", APPEND_LIMITS, &cancel)?;
    let aborted = pending.transaction();
    pending.abort()?;
    if db.resolve_commit(aborted)? != CommitResolution::Aborted {
        return Err("empty append did not resolve as aborted".into());
    }
    let second = append_amounts(&db, &[30], &[0b1], &cancel)?;
    // Keep copies of both receipts for later comparison. These copies retain
    // no table files; the database stores the history needed to resolve them.
    let commits = [first, second];
    let current = db.prepare(QUERY)?;

    // Reclamation must keep the files used by either prepared query as well as
    // the history needed to resolve transactions. Check both obligations below.
    db.reclaim(&cancel)?;
    verify_rows(db.execute(&old, &cancel)?, "old after reclaim", &[10, 20])?;
    verify_rows(
        db.execute(&current, &cancel)?,
        "new after reclaim",
        &[10, 20, 30],
    )?;
    verify_outcomes(&db, &commits, aborted, "after reclaim")?;

    drop(old);
    db.reclaim(&cancel)?;
    verify_rows(
        db.execute(&current, &cancel)?,
        "new after old plan drops",
        &[10, 20, 30],
    )?;
    verify_outcomes(&db, &commits, aborted, "after old plan drops")?;
    // A prepared query borrows its database. Drop it before closing the handle.
    drop(current);
    db.close()?;

    let db = Database::open(&path, config)?;
    let reopened = db.prepare(QUERY)?;
    verify_rows(db.execute(&reopened, &cancel)?, "reopened", &[10, 20, 30])?;
    verify_outcomes(&db, &commits, aborted, "reopened")?;
    drop(reopened);
    db.close()?;
    Ok(())
}

fn append_amounts(
    db: &Database,
    values: &[i64],
    validity: &[u8],
    cancel: &CancellationToken,
) -> Result<Commit, pipesql::Error> {
    let mut append = db.begin_append("sales", APPEND_LIMITS, cancel)?;
    append.write(
        &[ColumnInput {
            values: ColumnValues::Int64(values),
            validity,
        }],
        cancel,
    )?;
    append.commit(cancel)
}

fn verify_outcomes(
    db: &Database,
    commits: &[Commit; 2],
    aborted: TransactionId,
    label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // A generation numbers a published database state. Declaration creates state
    // 1 and the successful appends create states 2 and 3; abort adds no state.
    let generations = commits.map(Commit::generation);
    if generations != [2, 3] {
        return Err("unexpected append generations".into());
    }
    for commit in commits {
        if db.resolve_commit(commit.transaction())? != CommitResolution::Durable(*commit) {
            return Err("durable receipt changed".into());
        }
    }
    if db.resolve_commit(aborted)? != CommitResolution::Aborted {
        return Err("later publication changed the aborted outcome".into());
    }
    println!("receipts {label}: durable generations={generations:?}, aborted=Aborted");
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
