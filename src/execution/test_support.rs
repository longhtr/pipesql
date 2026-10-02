//! Create repeating lineitem data and collect results from the real query executor.
//!
//! `loaded` imports a generated text file through the public loader. It returns
//! both the database and its directory guard; callers must retain the guard until
//! database and query handles have dropped. The guard also attempts cleanup if
//! setup panics.
//!
//! `drain` collects a row count and integer sum for the numeric scan fixtures.
//! `finish_query` counts arbitrary result rows and preserves the query's error.
//! Both wait for successful completion before returning totals. Tests supply
//! their own expected answers; these helpers do not derive an oracle from the
//! query plan or storage implementation.

use super::*;
use crate::Config;
pub(super) use crate::test_support::Directory;
use std::fs::File;
use std::io::Write;
pub(super) const SQL: &str =
    "FROM lineitem |> SELECT l_quantity AS q, l_extendedprice AS p |> WHERE q < 25.0 |> SELECT p";
pub(super) fn loaded(rows: usize) -> (Directory, Database) {
    let fixture = Directory::new();
    let input = fixture.0.join("lineitem.tbl");
    let mut file = std::io::BufWriter::new(File::create(&input).unwrap());
    // Different quantity and price periods expose row misalignment across
    // filters and block boundaries without storing a large fixture in Git.
    for row in 0..rows {
        writeln!(
            file,
            "1|2|3|4|{}|{}|0.08|0.1|A|F|1994-01-01|x|x|x|x|x|",
            row % 31,
            row % 97 + 1
        )
        .unwrap();
    }
    file.flush().expect("flush execution test input");
    drop(file);
    let mut database = Database::create(
        &fixture.0.join("database"),
        Config::new(2_000_000, 20_000_000).unwrap(),
    )
    .unwrap();
    database
        .load_lineitem(&input, &CancellationToken::new())
        .unwrap();
    (fixture, database)
}

// Only use this collector for the fixture's nonnegative integer-valued DOUBLE
// results: converting to u64 would hide fractions, negative values or NaNs.
// It borrows the result so fault tests can inspect its terminal state afterward.
pub(super) fn drain(
    result: &mut QueryResult<'_, '_>,
    effects: &mut Effects,
) -> Result<(usize, u64), ()> {
    let mut rows = 0;
    let mut sum = 0_u64;
    for _ in 0..100_000 {
        let before = effects.count();
        match result.step_with_effects(effects) {
            QueryStep::Rows(batch) => {
                assert!(batch.len() <= BATCH_ROWS);
                for row in 0..batch.len() {
                    rows += 1;
                    let Value::Double(value) = batch.value(row, 0).unwrap() else {
                        panic!("numeric result");
                    };
                    sum += value as u64;
                }
            }
            QueryStep::Progress => (),
            QueryStep::Finished => return Ok((rows, sum)),
            QueryStep::Failed(_) => return Err(()),
        }
        assert!(
            effects.count() - before <= 1,
            "one step can read at most one block"
        );
    }
    panic!("scan exceeded declared test step allowance");
}

// Consume the result so success and failure both release its workspace before
// returning. A yielded batch is only a prefix; count it but keep stepping.
pub(super) fn finish_query(
    mut result: QueryResult<'_, '_>,
    effects: &mut Effects,
) -> Result<usize, Error> {
    let mut rows = 0;
    for _ in 0..10_000 {
        match result.step_with_effects(effects) {
            QueryStep::Rows(batch) => rows += batch.len(),
            QueryStep::Progress => (),
            QueryStep::Finished => return Ok(rows),
            QueryStep::Failed(_) => return Err(result.into_error().expect("terminal failure")),
        }
    }
    panic!("query exceeded fixture step bound");
}
