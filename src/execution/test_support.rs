//! Shared execution-test input construction and bounded result draining.
use super::*;
use crate::Config;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) static NEXT: AtomicU64 = AtomicU64::new(1);
pub(super) const SQL: &str =
    "FROM lineitem |> SELECT l_quantity AS q, l_extendedprice AS p |> WHERE q < 25.0 |> SELECT p";
pub(super) struct Fixture(pub(super) PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

pub(super) fn loaded(rows: usize) -> (Fixture, Database) {
    let path = std::env::temp_dir().join(format!(
        "pipesql-stream-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path).unwrap();
    let input = path.join("lineitem.tbl");
    let mut file = std::io::BufWriter::new(File::create(&input).unwrap());
    for row in 0..rows {
        writeln!(
            file,
            "1|2|3|4|{}|{}|0.08|0.1|A|F|1994-01-01|x|x|x|x|x|",
            row % 31,
            row % 97 + 1
        )
        .unwrap();
    }
    drop(file);
    let mut database = Database::create(
        &path.join("database"),
        Config::new(2_000_000, 20_000_000).unwrap(),
    )
    .unwrap();
    database
        .load_lineitem(&input, &CancellationToken::new())
        .unwrap();
    (Fixture(path), database)
}

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
