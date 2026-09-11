use super::order::{integers, query};
use super::*;

#[test]
fn public_limit_constants_prefixes_and_predicate_boundaries() {
    let (_directory, db) = join_fixture();
    for count in [0, 1, 3, 9, i64::MAX] {
        for offset in [0, 1, 6, i64::MAX] {
            let expected = [40, 30, 20, 10]
                .into_iter()
                .enumerate()
                .filter(|(i, _)| (*i as u64) >= offset as u64)
                .take(usize::try_from(count).unwrap())
                .map(|(_, v)| v)
                .collect::<Vec<_>>();
            query(
                &db,
                &format!(
                    "FROM facts |> ORDER BY k DESC NULLS FIRST,v DESC |> SELECT v |> LIMIT {count} OFFSET {offset}"
                ),
                integers(&expected),
            );
        }
    }
    for (sql, expected) in [
        (
            "FROM facts |> ORDER BY v |> LIMIT 1 |> WHERE v > 10 |> SELECT v",
            vec![],
        ),
        (
            "FROM facts |> ORDER BY v |> WHERE v > 10 |> LIMIT 1 |> SELECT v",
            integers(&[20]),
        ),
        (
            "FROM facts |> ORDER BY v |> LIMIT (1 + 2) * 1 OFFSET +1 |> LIMIT 1 OFFSET 1 |> SELECT v",
            integers(&[30]),
        ),
        (
            "FROM facts |> LIMIT 0 |> AGGREGATE COUNT(*) AS n",
            integers(&[0]),
        ),
        ("FROM facts |> AGGREGATE COUNT(*) AS n |> LIMIT 0", vec![]),
        (
            "FROM facts |> ORDER BY v |> LIMIT 2 OFFSET 1 |> AGGREGATE SUM(v) AS n |> LIMIT 1",
            integers(&[50]),
        ),
        (
            "FROM facts |> LIMIT 0 |> AGGREGATE COUNT(*) AS n GROUP BY k",
            vec![],
        ),
        (
            "FROM facts AS f |> JOIN dimensions AS d ON f.k = d.k |> ORDER BY f.v DESC,d.label |> LIMIT 2 OFFSET 1 |> SELECT f.v",
            integers(&[20, 20]),
        ),
        (
            "FROM facts |> ORDER BY v |> LIMIT 2 |> AS f |> JOIN dimensions AS d ON f.k = d.k |> ORDER BY f.v,d.label |> SELECT f.v",
            integers(&[10, 10, 20, 20]),
        ),
    ] {
        query(&db, sql, expected);
    }
    let baseline = db.reserved_memory_bytes();
    for suffix in [
        "-1",
        "1.0",
        "NULL",
        "ALL",
        "v",
        "9223372036854775808",
        "9223372036854775807 + 1",
        "0 OFFSET -1",
        "0 OFFSET 1.0",
        "0 OFFSET v",
        "0 OFFSET 9223372036854775807 * 2",
        "1 OFFSET",
        "",
        "1 OFFSET 0 OFFSET 0",
    ] {
        assert!(
            db.prepare(&format!("FROM facts |> LIMIT {suffix}"))
                .is_err(),
            "{suffix}"
        );
        assert_eq!(db.reserved_memory_bytes(), baseline, "{suffix}");
    }
    db.close().unwrap();
}

#[test]
fn public_limit_preserves_demanded_failures_and_releases_early_owners() {
    let (_directory, db) = join_fixture();
    let baseline = db.reserved_memory_bytes();
    for (count, offset, fails) in [(0, 0, false), (1, 0, true), (0, 1, true)] {
        let sql = format!(
            "FROM facts |> AGGREGATE SUM(v * 9223372036854775807) AS doomed,COUNT(*) AS n |> ORDER BY doomed |> LIMIT {count} OFFSET {offset} |> SELECT n"
        );
        let query = db.prepare(&sql).unwrap();
        let cancel = CancellationToken::new();
        let mut result = db.execute(&query, &cancel).unwrap();
        let mut terminal = false;
        for _ in 0..2000 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Finished => {
                    assert!(!fails);
                    terminal = true;
                    break;
                }
                QueryStep::Failed(_) => {
                    assert!(fails);
                    terminal = true;
                    break;
                }
                QueryStep::Rows(_) => panic!("zero limit or demanded arithmetic failure"),
            }
        }
        assert!(terminal);
        drop(result);
        drop(query);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    for sql in [
        "FROM facts |> LIMIT 1",
        "FROM facts |> ORDER BY v |> LIMIT 1",
        "FROM facts AS f |> JOIN dimensions AS d ON f.k = d.k |> LIMIT 1",
    ] {
        for cancel_after in [0, 3, 12, usize::MAX] {
            let query = db.prepare(sql).unwrap();
            let cancel = CancellationToken::new();
            let mut result = db.execute(&query, &cancel).unwrap();
            let mut terminal = false;
            for step in 0..3000 {
                if step == cancel_after {
                    cancel.cancel();
                }
                match result.step() {
                    QueryStep::Progress => (),
                    QueryStep::Rows(batch) => {
                        assert_eq!(batch.len(), 1);
                        terminal = true;
                        break;
                    }
                    QueryStep::Failed(error) => {
                        assert!(matches!(error, pipesql::Error::Cancelled), "{error}");
                        terminal = true;
                        break;
                    }
                    QueryStep::Finished => panic!("expected one prefix row"),
                }
            }
            assert!(terminal);
            // Abandon a returned prefix before another poll. This must release
            // source pins, sorter files and batches without cancelling siblings.
            drop(result);
            drop(query);
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
            if cancel_after == usize::MAX {
                assert!(!cancel.is_cancelled());
            }
        }
    }
    db.close().unwrap();
}
