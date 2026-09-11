//! Public snapshots contract tests.
use super::*;

#[test]
fn declared_tables_append_and_snapshot_queries_survive_reopen() {
    let directory = Directory::new();
    let path = directory.database();
    let db = Database::create_empty(&path, config()).unwrap();
    let cancel = CancellationToken::new();
    assert!(matches!(
        db.prepare("FROM missing"),
        Err(Error::Bind { .. })
    ));
    db.declare_table("facts", &declarations(), &cancel).unwrap();
    db.declare_table("other", &declarations(), &cancel).unwrap();
    let sql = "FROM facts |> SELECT note,amount,number,day";
    let empty = db.prepare(sql).unwrap();
    let mut text = String::from("雪");
    let integers = [9_007_199_254_740_993, i64::MAX];
    let numbers = [-0.0, f64::from_bits(0x7ff8_0000_0000_0042)];
    let dates = [
        DateValue::from_days_since_unix_epoch(-719_162).unwrap(),
        DateValue::from_days_since_unix_epoch(2_932_896).unwrap(),
    ];
    let mut append = db.begin_append("FACTS", limits(), &cancel).unwrap();
    let token = append.transaction();
    assert!(matches!(
        db.resolve_commit(token),
        Err(Error::Contention(_))
    ));
    let mut expected = Vec::new();
    for next in ["é", "caller reused its input"] {
        append
            .write(
                &[
                    ColumnInput {
                        values: ColumnValues::String(&[text.as_str(), "ignored"]),
                        validity: &[1],
                    },
                    ColumnInput {
                        values: ColumnValues::Int64(&integers),
                        validity: &[3],
                    },
                    ColumnInput {
                        values: ColumnValues::Double(&numbers),
                        validity: &[3],
                    },
                    ColumnInput {
                        values: ColumnValues::Date(&dates),
                        validity: &[3],
                    },
                ],
                &cancel,
            )
            .unwrap();
        expected.push(vec![
            Cell::Text(text.clone()),
            Cell::Integer(integers[0]),
            Cell::Number(numbers[0].to_bits()),
            Cell::Day(-719_162),
        ]);
        expected.push(vec![
            Cell::Null,
            Cell::Integer(integers[1]),
            Cell::Number(numbers[1].to_bits()),
            Cell::Day(2_932_896),
        ]);
        text.clear();
        text.push_str(next);
        assert!(collect(&mut db.execute(&empty, &cancel).unwrap()).is_empty());
    }
    expected.sort_unstable();
    let committed = append.commit(&cancel).unwrap();
    assert_eq!(committed.transaction(), token);
    let query = db.prepare(sql).unwrap();
    let mut running = db.execute(&query, &cancel).unwrap();
    // Publication to another table cannot disturb the already opened scan.
    let other = db.begin_append("other", limits(), &cancel).unwrap();
    let aborted = other.transaction();
    other.abort().unwrap();
    db.declare_table("third", &declarations(), &cancel).unwrap();
    assert_eq!(collect(&mut running), expected);
    drop(running);
    drop(query);
    drop(empty);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close().unwrap();
    let db = Database::open(&path, config()).unwrap();
    assert_eq!(
        db.resolve_commit(token).unwrap(),
        CommitResolution::Durable(committed)
    );
    assert_eq!(
        db.resolve_commit(aborted).unwrap(),
        CommitResolution::Aborted
    );
    let query = db.prepare(sql).unwrap();
    assert_eq!(collect(&mut db.execute(&query, &cancel).unwrap()), expected);
    let other = db.prepare("FROM other |> SELECT note").unwrap();
    assert!(collect(&mut db.execute(&other, &cancel).unwrap()).is_empty());
    drop(other);
    drop(query);
    db.close().unwrap();
    let source = directory.0.join("query.sql");
    std::fs::write(&source, "FROM facts |> SELECT amount").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pipesql"))
        .arg("query")
        .arg("--database")
        .arg(&path)
        .arg("--query-file")
        .arg(&source)
        .args([
            "--memory-limit-bytes",
            "4000000",
            "--temp-limit-bytes",
            "2000000",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = String::from_utf8(output.stdout).unwrap();
    assert_eq!(output.matches("9007199254740993").count(), 2);
    assert_eq!(output.matches("9223372036854775807").count(), 2);
}

#[test]
fn reclamation_preserves_all_pinned_generations_and_receipts() {
    fn append_one(db: &Database, value: i64) -> pipesql::Commit {
        let cancel = CancellationToken::new();
        let mut append = db.begin_append("facts", limits(), &cancel).unwrap();
        append
            .write(
                &[ColumnInput {
                    values: ColumnValues::Int64(&[value]),
                    validity: &[1],
                }],
                &cancel,
            )
            .unwrap();
        append.commit(&cancel).unwrap()
    }
    let directory = Directory::new();
    let path = directory.database();
    let db = Database::create_empty(&path, config()).unwrap();
    let cancel = CancellationToken::new();
    assert_eq!(db.reclaim(&cancel).unwrap(), 0);
    db.declare_table(
        "facts",
        &[ColumnDeclaration {
            name: "v",
            data_type: DataType::Int64,
            nullable: false,
        }],
        &cancel,
    )
    .unwrap();
    let mut pins = vec![db.prepare("FROM facts").unwrap()];
    let mut commits = Vec::new();
    for value in 1..=3 {
        commits.push(append_one(&db, value));
        pins.push(db.prepare("FROM facts").unwrap());
    }
    // All four slots are pinned; cleanup must not reserve a publication slot.
    assert!(matches!(
        db.begin_append("facts", limits(), &cancel),
        Err(Error::Resource {
            owner: "catalog snapshot slots",
            ..
        })
    ));
    assert_eq!(db.reclaim(&cancel).unwrap(), 3);
    for (count, query) in pins.iter().enumerate() {
        let rows = collect(&mut db.execute(query, &cancel).unwrap());
        assert_eq!(
            rows,
            (1..=count)
                .map(|n| vec![Cell::Integer(n as i64)])
                .collect::<Vec<_>>()
        );
    }
    for commit in &commits {
        assert_eq!(
            db.resolve_commit(commit.transaction()).unwrap(),
            CommitResolution::Durable(*commit)
        );
    }
    drop(pins);
    assert_eq!(db.reclaim(&cancel).unwrap(), 5);
    assert_eq!(db.reclaim(&cancel).unwrap(), 0);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close().unwrap();
    let reopened = Database::open(&path, config()).unwrap();
    for commit in &commits {
        assert_eq!(
            reopened.resolve_commit(commit.transaction()).unwrap(),
            CommitResolution::Durable(*commit)
        );
    }
    let next = append_one(&reopened, 4);
    assert_eq!(next.generation(), commits.last().unwrap().generation() + 1);
    let query = reopened.prepare("FROM facts").unwrap();
    assert_eq!(
        collect(&mut reopened.execute(&query, &cancel).unwrap()),
        (1..=4).map(|n| vec![Cell::Integer(n)]).collect::<Vec<_>>()
    );
    drop(query);
    reopened.close().unwrap();
}
