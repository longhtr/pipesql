//! Check running SUM against complete literal answers, including order peers.
//!
//! Rows with equal order keys share the cumulative total through that whole
//! group. A row-by-row sum would give different answers. Final ORDER BY clauses
//! identify rows independently of the window's private sorting order.
//!
//! Cases also distinguish partition resets, NULL arguments and typed grouping keys.
//! Exact integer peer totals test overflow only when the visible result demands it.
//! Failures must release memory and temporary-space reservations, preserving the
//! same database handle for subsequent work.

use super::*;

fn integer_windows(amounts: &[i64], peers: &[i64], validity: u8) -> (Directory, Database) {
    assert!(amounts.len() <= 8 && amounts.len() == peers.len());
    let directory = Directory::new();
    let db = Database::create_empty(&directory.database(), config()).unwrap();
    let cancel = CancellationToken::new();
    db.declare_table(
        "facts",
        &["id", "peer", "amount"].map(|name| ColumnDeclaration {
            name,
            data_type: DataType::Int64,
            nullable: name == "amount",
        }),
        &cancel,
    )
    .unwrap();
    let ids: Vec<_> = (0..amounts.len() as i64).collect();
    let valid = [(255_u16 >> (8 - amounts.len())) as u8];
    let mut append = db.begin_append("facts", limits(), &cancel).unwrap();
    append
        .write(
            &[
                ColumnInput {
                    values: ColumnValues::Int64(&ids),
                    validity: &valid,
                },
                ColumnInput {
                    values: ColumnValues::Int64(peers),
                    validity: &valid,
                },
                ColumnInput {
                    values: ColumnValues::Int64(amounts),
                    validity: &[validity],
                },
            ],
            &cancel,
        )
        .unwrap();
    append.commit(&cancel).unwrap();
    (directory, db)
}

#[test]
fn running_sum_includes_all_current_peers_and_resets_each_partition() {
    let (_directory, db) = join_fixture();
    for (window, totals) in [
        ("ORDER BY k", [70, 70, 100, 40]),
        ("ORDER BY k DESC", [60, 60, 30, 100]),
        ("ORDER BY k NULLS LAST", [30, 30, 60, 100]),
        ("ORDER BY k, v DESC", [70, 60, 100, 40]),
        ("PARTITION BY k ORDER BY v", [10, 30, 30, 40]),
        ("PARTITION BY k, k ORDER BY k DESC, v", [10, 30, 30, 40]),
    ] {
        query(
            &db,
            &format!("FROM facts |> SELECT v, SUM(v) OVER ({window}) AS total |> ORDER BY v"),
            [10, 20, 30, 40]
                .into_iter()
                .zip(totals)
                .map(|(v, total)| vec![Cell::Integer(v), Cell::Integer(total)])
                .collect(),
        );
    }
    query(
        &db,
        "FROM facts |> SELECT SUM(v) OVER (ORDER BY k) AS n |> ORDER BY n",
        integers(&[40, 70, 70, 100]),
    );
    query(
        &db,
        "FROM facts |> EXTEND SUM(v) OVER (ORDER BY k) AS n |> EXTEND SUM(v) OVER (ORDER BY v) AS m |> SELECT v, n, m |> ORDER BY v",
        [(10, 70, 10), (20, 70, 30), (30, 100, 60), (40, 40, 100)]
            .map(|(v, n, m)| vec![Cell::Integer(v), Cell::Integer(n), Cell::Integer(m)])
            .to_vec(),
    );
}

#[test]
fn running_sum_ignores_null_arguments_and_groups_typed_keys() {
    let (_directory, db) = nullable_facts().unwrap();
    query(
        &db,
        "FROM facts |> SELECT id, SUM(i) OVER (ORDER BY d) AS n |> ORDER BY id",
        [16, 16, 16, 9]
            .into_iter()
            .enumerate()
            .map(|(id, n)| vec![Cell::Integer(id as i64), Cell::Integer(n)])
            .collect(),
    );
    query(
        &db,
        "FROM facts |> SELECT SUM(i) OVER (PARTITION BY s ORDER BY d) AS n, id |> ORDER BY id |> SELECT n",
        vec![
            vec![Cell::Integer(0)],
            vec![Cell::Integer(7)],
            vec![Cell::Null],
            vec![Cell::Integer(9)],
        ],
    );
    query(
        &db,
        "FROM facts |> WHERE id<0 |> SELECT SUM(i) OVER (ORDER BY d) AS n",
        vec![],
    );
}

#[test]
fn running_sum_keeps_exact_peer_totals_and_demands_only_visible_overflow() {
    let (_directory, db) =
        integer_windows(&[i64::MAX, 1, -1, i64::MIN, -1, 1], &[0, 0, 0, 1, 1, 1], 63);
    query(
        &db,
        "FROM facts |> SELECT id, SUM(amount) OVER (ORDER BY peer) AS n |> ORDER BY id |> SELECT n",
        integers(&[i64::MAX, i64::MAX, i64::MAX, -1, -1, -1]),
    );

    let (_directory, db) = integer_windows(&[i64::MAX, 1, -1, 0], &[0, 1, 2, 3], 7);
    let call = "SUM(amount) OVER (ORDER BY peer)";
    let prefix = format!("FROM facts |> SELECT id, {call} AS n");
    query(
        &db,
        &format!("{prefix} |> WHERE id!=1 |> ORDER BY id |> SELECT n"),
        integers(&[i64::MAX; 3]),
    );
    query(
        &db,
        &format!("{prefix} |> SELECT n |> LIMIT 1"),
        integers(&[i64::MAX]),
    );
    query(
        &db,
        &format!("{prefix} |> SELECT id |> ORDER BY id"),
        integers(&[0, 1, 2, 3]),
    );
    query(
        &db,
        &format!(
            "{prefix} |> SELECT id, CASE WHEN id=1 THEN 0 ELSE n END AS safe |> ORDER BY id |> SELECT safe"
        ),
        integers(&[i64::MAX, 0, i64::MAX, i64::MAX]),
    );
    query(
        &db,
        "FROM facts |> EXTEND 9223372036854775807+id AS bad |> EXTEND SUM(bad) OVER (ORDER BY peer) AS unused |> SELECT id |> ORDER BY id",
        integers(&[0, 1, 2, 3]),
    );

    for (suffix, expected_rows) in [
        (" |> WHERE id=1 |> SELECT n", 0),
        (" |> WHERE n>0 |> WHERE id!=1 |> SELECT n", 1),
    ] {
        let sql = format!("{prefix}{suffix}");
        let baseline = db.reserved_memory_bytes();
        let prepared = db.prepare(&sql).unwrap();
        let cancel = CancellationToken::new();
        let mut result = db.execute(&prepared, &cancel).unwrap();
        let mut rows = 0;
        let mut failed = false;
        for _ in 0..10_000 {
            match result.step() {
                QueryStep::Progress => (),
                QueryStep::Rows(batch) => rows += batch.len(),
                QueryStep::Failed(Error::ArithmeticOverflow { operation, span }) => {
                    assert_eq!(*operation, "SUM");
                    assert_eq!(&sql[span.start()..span.end()], call);
                    failed = true;
                    break;
                }
                _ => panic!("demanded frame must overflow"),
            }
        }
        assert!(failed);
        assert_eq!(rows, expected_rows);
        assert!(matches!(
            result.step(),
            QueryStep::Failed(Error::ArithmeticOverflow { .. })
        ));
        drop(result);
        drop(prepared);
        assert_eq!(db.reserved_memory_bytes(), baseline);
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
}
