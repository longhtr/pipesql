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
    let sql = "FROM facts |> SELECT note, amount, number, day";
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

#[test]
fn concurrent_readers_keep_generations_through_reclamation_and_early_drop() {
    fn append(db: &Database, value: i64) -> pipesql::Commit {
        let cancel = CancellationToken::new();
        let mut writer = db.begin_append("facts", limits(), &cancel).unwrap();
        writer
            .write(
                &[ColumnInput {
                    values: ColumnValues::Int64(&[value]),
                    validity: &[1],
                }],
                &cancel,
            )
            .unwrap();
        writer.commit(&cancel).unwrap()
    }

    let directory = Directory::new();
    let path = directory.database();
    let db = Database::create_empty(&path, config()).unwrap();
    let cancel = CancellationToken::new();
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
    let resident = db.reserved_memory_bytes();
    let first = append(&db, 11);
    let old = db.prepare("FROM facts |> SELECT v").unwrap();
    let second = append(&db, 22);
    let middle = db.prepare("FROM facts |> SELECT v").unwrap();
    let timeout = std::time::Duration::from_secs(30);
    let (old_ready_tx, old_ready_rx) = std::sync::mpsc::sync_channel(1);
    let (old_resume_tx, old_resume_rx) = std::sync::mpsc::sync_channel(1);
    let (middle_ready_tx, middle_ready_rx) = std::sync::mpsc::sync_channel(1);
    let (middle_resume_tx, middle_resume_rx) = std::sync::mpsc::sync_channel(1);

    let third = std::thread::scope(|scope| {
        let reader_db = &db;
        let old_worker = scope.spawn(move || {
            let cancel = CancellationToken::new();
            let mut result = reader_db.execute(&old, &cancel).unwrap();
            assert!(matches!(result.step(), QueryStep::Progress));
            old_ready_tx.send(()).unwrap();
            old_resume_rx.recv_timeout(timeout).unwrap();
            // Drop an unfinished cursor, then reopen its still-pinned input.
            // An already open file alone could survive an erroneous unlink.
            drop(result);
            assert_eq!(
                collect(&mut reader_db.execute(&old, &cancel).unwrap()),
                vec![vec![Cell::Integer(11)]]
            );
        });
        old_ready_rx.recv_timeout(timeout).unwrap();
        let middle_worker = scope.spawn(move || {
            let cancel = CancellationToken::new();
            let mut result = reader_db.execute(&middle, &cancel).unwrap();
            assert!(matches!(result.step(), QueryStep::Progress));
            middle_ready_tx.send(()).unwrap();
            middle_resume_rx.recv_timeout(timeout).unwrap();
            assert_eq!(
                collect(&mut result),
                vec![vec![Cell::Integer(11)], vec![Cell::Integer(22)]]
            );
            drop(result);
            assert_eq!(
                collect(&mut reader_db.execute(&middle, &cancel).unwrap()),
                vec![vec![Cell::Integer(11)], vec![Cell::Integer(22)]]
            );
        });
        middle_ready_rx.recv_timeout(timeout).unwrap();
        let parked = db.reserved_memory_bytes();
        let third = append(&db, 33);
        assert!(db.reclaim(&cancel).unwrap() > 0);
        assert_eq!(db.reserved_memory_bytes(), parked);
        assert_eq!(db.reserved_temp_bytes(), 0);
        for commit in [first, second, third] {
            assert_eq!(
                db.resolve_commit(commit.transaction()).unwrap(),
                CommitResolution::Durable(commit)
            );
        }
        old_resume_tx.send(()).unwrap();
        old_worker.join().unwrap();
        assert!(db.reserved_memory_bytes() < parked);
        // The middle reader remains parked while the older pin is released.
        assert!(db.reclaim(&cancel).unwrap() > 0);
        middle_resume_tx.send(()).unwrap();
        middle_worker.join().unwrap();
        third
    });
    assert_eq!(db.reserved_memory_bytes(), resident);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert!(db.reclaim(&cancel).unwrap() > 0);
    assert_eq!(db.reclaim(&cancel).unwrap(), 0);
    let fourth = append(&db, 44);
    let expected = vec![
        vec![Cell::Integer(11)],
        vec![Cell::Integer(22)],
        vec![Cell::Integer(33)],
        vec![Cell::Integer(44)],
    ];
    let fresh = db.prepare("FROM facts |> SELECT v").unwrap();
    assert_eq!(collect(&mut db.execute(&fresh, &cancel).unwrap()), expected);
    drop(fresh);
    assert_eq!(db.reserved_memory_bytes(), resident);
    db.close().unwrap();
    let db = Database::open(&path, config()).unwrap();
    for commit in [first, second, third, fourth] {
        assert_eq!(
            db.resolve_commit(commit.transaction()).unwrap(),
            CommitResolution::Durable(commit)
        );
    }
    let fresh = db.prepare("FROM facts |> SELECT v").unwrap();
    assert_eq!(collect(&mut db.execute(&fresh, &cancel).unwrap()), expected);
    drop(fresh);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close().unwrap();
}

#[path = "../../examples/support/event_data.rs"]
mod event_data;

type ReportGroup = (Option<i64>, Option<&'static str>, i64, i64, Option<i64>);

// Literal answers, independent of fixture construction and production evaluation.
const OLD_REPORT: &[ReportGroup] = &[
    (None, Some(""), 1, 1, Some(5)),
    (None, Some("north"), 1, 1, Some(7)),
    (Some(1999), Some("north"), 1, 1, Some(10)),
    (Some(2000), None, 2, 1, Some(30)),
    (Some(2000), Some("south"), 2, 2, Some(15)),
    (Some(2000), Some("南"), 2, 2, Some(15)),
    (Some(2001), Some("north"), 1, 0, None),
];
const NEW_REPORT: &[ReportGroup] = &[
    (None, None, 1, 1, Some(11)),
    (None, Some(""), 1, 1, Some(5)),
    (None, Some("north"), 1, 1, Some(7)),
    (None, Some("south"), 1, 0, None),
    (None, Some("南"), 1, 0, None),
    (Some(1999), Some("north"), 2, 2, Some(8)),
    (Some(2000), None, 3, 2, Some(33)),
    (Some(2000), Some(""), 2, 2, Some(9)),
    (Some(2000), Some("south"), 2, 2, Some(15)),
    (Some(2000), Some("南"), 2, 2, Some(15)),
    (Some(2001), Some("north"), 2, 1, Some(13)),
    (Some(2001), Some("south"), 1, 1, Some(40)),
    (Some(2001), Some("南"), 1, 1, Some(40)),
];

fn append_report(db: &Database, events: &[event_data::Event]) -> pipesql::Commit {
    let cancel = CancellationToken::new();
    let mut writer = db
        .begin_append(
            "events",
            AppendLimits {
                batches: 1,
                encoded_bytes: 4096,
            },
            &cancel,
        )
        .unwrap();
    event_data::write_events(&mut writer, events, &cancel).unwrap();
    writer.commit(&cancel).unwrap()
}

fn report_rows(expected: &[ReportGroup]) -> Vec<Vec<Cell>> {
    let mut rows: Vec<_> = expected
        .iter()
        .map(|&(year, label, entries, present, total)| {
            vec![
                year.map_or(Cell::Null, Cell::Integer),
                label.map_or(Cell::Null, |label| Cell::Text(label.to_owned())),
                Cell::Integer(entries),
                Cell::Integer(present),
                total.map_or(Cell::Null, Cell::Integer),
            ]
        })
        .collect();
    rows.sort_unstable();
    rows
}

fn park_report(db: &Database, result: &mut QueryResult<'_, '_>, prior_temp: u64) -> u64 {
    for _ in 0..1024 {
        assert!(
            matches!(result.step(), QueryStep::Progress),
            "report emitted before its spill checkpoint"
        );
        if db.reserved_temp_bytes() > prior_temp {
            return db.reserved_temp_bytes() - prior_temp;
        }
    }
    panic!("report did not reach spill checkpoint");
}

fn prepare_report(db: &Database) -> pipesql::PreparedQuery<'_> {
    let query = db
        .prepare(include_str!("../../examples/event_report.sql"))
        .unwrap();
    assert_eq!(query.result_column_count(), 5);
    for (index, (name, data_type, nullable)) in [
        ("calendar_year", DataType::Int64, true),
        ("label", DataType::String, true),
        ("entries", DataType::Int64, false),
        ("present", DataType::Int64, false),
        ("total", DataType::Int64, true),
    ]
    .into_iter()
    .enumerate()
    {
        let column = query.result_column(index).unwrap();
        assert_eq!(
            (column.name, column.data_type, column.nullable),
            (Some(name), data_type, nullable)
        );
    }
    query
}

fn verify_report(
    db: &Database,
    mut result: QueryResult<'_, '_>,
    expected: &[ReportGroup],
) -> Result<(), Error> {
    let outside_memory = db.reserved_memory_bytes() - result.accounted_memory_bytes();
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    for _ in 0..1024 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    rows.push(
                        (0..batch.column_count())
                            .map(|column| owned_cell(batch.value(row, column).unwrap()))
                            .collect(),
                    );
                }
            }
            QueryStep::Finished => {
                rows.sort_unstable();
                assert_eq!(rows, report_rows(expected));
                drop(result);
                assert_eq!(db.reserved_memory_bytes(), outside_memory);
                return Ok(());
            }
            QueryStep::Failed(_) => {
                let error = result.into_error().expect("owned report error");
                assert_eq!(db.reserved_memory_bytes(), outside_memory);
                return Err(error);
            }
        }
    }
    panic!("small report exceeded bounded progress allowance");
}

// The current 24-event answer is independent of the two pinned answers. The
// third append repeats the first eight inputs; it does not replace either view.
const LATEST_REPORT: &[ReportGroup] = &[
    (None, None, 1, 1, Some(11)),
    (None, Some(""), 2, 2, Some(10)),
    (None, Some("north"), 2, 2, Some(14)),
    (None, Some("south"), 1, 0, None),
    (None, Some("南"), 1, 0, None),
    (Some(1999), Some("north"), 3, 3, Some(18)),
    (Some(2000), None, 5, 3, Some(63)),
    (Some(2000), Some(""), 2, 2, Some(9)),
    (Some(2000), Some("south"), 4, 4, Some(30)),
    (Some(2000), Some("南"), 4, 4, Some(30)),
    (Some(2001), Some("north"), 3, 1, Some(13)),
    (Some(2001), Some("south"), 1, 1, Some(40)),
    (Some(2001), Some("南"), 1, 1, Some(40)),
];

#[test]
fn overlapping_reports_preserve_pins_through_publication_and_cancellation() {
    for publish_first in [true, false] {
        for older_first in [true, false] {
            overlapping_reports(publish_first, older_first, false);
        }
    }
}

#[test]
fn overlapping_report_control_detects_unlinked_pinned_catalog() {
    overlapping_reports(true, true, true);
}

fn overlapping_reports(publish_first: bool, older_first: bool, unlink_old_catalog: bool) {
    let directory = Directory::new();
    let path = directory.database();
    let config = Config::new(64_000_000, 32_000_000).unwrap();
    let db = Database::create_empty(&path, config).unwrap();
    let cancel = CancellationToken::new();
    event_data::declare(&db, &cancel).unwrap();
    let prior_names: std::collections::BTreeSet<_> = std::fs::read_dir(path.join("units"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    let first = append_report(&db, &event_data::EVENTS[..8]);
    let old = prepare_report(&db);
    // Identify the newly published catalog by the literal format tag and
    // directory difference, independently of the engine's graph traversal.
    let catalogs: Vec<_> = std::fs::read_dir(path.join("units"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| !prior_names.contains(path))
        .filter(|path| std::fs::read(path).unwrap().starts_with(b"PSQLCATL"))
        .collect();
    assert_eq!(catalogs.len(), 1);
    let old_catalog = &catalogs[0];
    let second = append_report(&db, &event_data::EVENTS[8..]);
    let new = prepare_report(&db);
    let resident =
        db.reserved_memory_bytes() - old.accounted_memory_bytes() - new.accounted_memory_bytes();
    let timeout = std::time::Duration::from_secs(30);
    let (old_ready_tx, old_ready_rx) = std::sync::mpsc::sync_channel(1);
    let (old_resume_tx, old_resume_rx) = std::sync::mpsc::sync_channel(1);
    let (new_ready_tx, new_ready_rx) = std::sync::mpsc::sync_channel(1);
    let (new_resume_tx, new_resume_rx) = std::sync::mpsc::sync_channel(1);
    let third = std::thread::scope(|scope| {
        let reader_db = &db;
        let old_worker = scope.spawn(move || {
            let token = CancellationToken::new();
            let mut result = reader_db.execute(&old, &token).unwrap();
            let own_temp = park_report(reader_db, &mut result, 0);
            old_ready_tx.send(()).unwrap();
            old_resume_rx.recv_timeout(timeout).unwrap();
            let outside_memory =
                reader_db.reserved_memory_bytes() - result.accounted_memory_bytes();
            let outside_temp = reader_db.reserved_temp_bytes() - own_temp;
            token.cancel();
            assert!(matches!(result.step(), QueryStep::Failed(Error::Cancelled)));
            assert!(matches!(result.into_error(), Some(Error::Cancelled)));
            assert_eq!(reader_db.reserved_memory_bytes(), outside_memory);
            assert_eq!(reader_db.reserved_temp_bytes(), outside_temp);
            // A fresh cursor must reopen the pinned graph. Retaining only an
            // already open descriptor could conceal an erroneous unlink.
            let token = CancellationToken::new();
            let verified = reader_db
                .execute(&old, &token)
                .and_then(|result| verify_report(reader_db, result, OLD_REPORT));
            (old, verified)
        });
        old_ready_rx.recv_timeout(timeout).unwrap();
        let mut writer = db
            .begin_append(
                "events",
                AppendLimits {
                    batches: 1,
                    encoded_bytes: 4096,
                },
                &cancel,
            )
            .unwrap();
        event_data::write_events(&mut writer, &event_data::EVENTS[..8], &cancel).unwrap();
        // Include the writer's temporary reservation before observing the second
        // reader; its first spill must add its own temporary storage.
        let prior_temp = db.reserved_temp_bytes();
        let new_worker = scope.spawn(move || {
            let token = CancellationToken::new();
            let mut result = reader_db.execute(&new, &token).unwrap();
            let own_temp = park_report(reader_db, &mut result, prior_temp);
            new_ready_tx.send(()).unwrap();
            new_resume_rx.recv_timeout(timeout).unwrap();
            let outside_temp = reader_db.reserved_temp_bytes() - own_temp;
            verify_report(reader_db, result, NEW_REPORT).unwrap();
            assert_eq!(reader_db.reserved_temp_bytes(), outside_temp);
            verify_report(
                reader_db,
                reader_db.execute(&new, &CancellationToken::new()).unwrap(),
                NEW_REPORT,
            )
            .unwrap();
        });
        new_ready_rx.recv_timeout(timeout).unwrap();
        let parked = (db.reserved_memory_bytes(), db.reserved_temp_bytes());
        let extra = prepare_report(&db);
        match db.execute(&extra, &cancel) {
            Err(Error::Resource {
                owner,
                required,
                limit,
            }) => {
                assert_eq!(owner, "native query workspace");
                assert_eq!(limit, config.memory_limit_bytes());
                assert!(required > limit);
            }
            _ => panic!("third report must refuse shared memory admission"),
        }
        drop(extra);
        assert_eq!(
            (db.reserved_memory_bytes(), db.reserved_temp_bytes()),
            parked
        );
        assert!(matches!(
            db.reclaim(&cancel),
            Err(Error::Contention("catalog writer"))
        ));
        assert_eq!(db.generation(), second.generation());
        let publish = || {
            let receipt = writer.commit(&cancel).unwrap();
            assert_eq!(receipt.generation(), second.generation() + 1);
            let published = (db.reserved_memory_bytes(), db.reserved_temp_bytes());
            assert!(db.reclaim(&cancel).unwrap() > 0);
            assert_eq!(
                (db.reserved_memory_bytes(), db.reserved_temp_bytes()),
                published
            );
            receipt
        };
        let finish_old = || {
            assert!(old_catalog.exists(), "reclamation removed a pinned catalog");
            let removed = unlink_old_catalog.then(|| {
                let bytes = std::fs::read(old_catalog).unwrap();
                std::fs::remove_file(old_catalog).unwrap();
                bytes
            });
            old_resume_tx.send(()).unwrap();
            let (old, verified) = old_worker.join().unwrap();
            if let Some(bytes) = removed {
                assert!(
                    matches!(verified, Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound),
                    "pin control must fail at reopened catalog input"
                );
                std::fs::write(old_catalog, bytes).unwrap();
                std::fs::File::open(old_catalog)
                    .unwrap()
                    .sync_all()
                    .unwrap();
                verify_report(
                    &db,
                    db.execute(&old, &CancellationToken::new()).unwrap(),
                    OLD_REPORT,
                )
                .unwrap();
            } else {
                verified.unwrap();
            }
            drop(old);
        };
        let finish_new = || {
            new_resume_tx.send(()).unwrap();
            new_worker.join().unwrap();
        };
        let third = match (publish_first, older_first) {
            (true, true) => {
                let receipt = publish();
                finish_old();
                assert!(db.reserved_memory_bytes() < parked.0);
                assert!(db.reclaim(&cancel).unwrap() > 0);
                finish_new();
                receipt
            }
            (true, false) => {
                let receipt = publish();
                finish_new();
                assert!(db.reserved_memory_bytes() < parked.0);
                assert!(db.reclaim(&cancel).unwrap() > 0);
                finish_old();
                receipt
            }
            (false, true) => {
                finish_old();
                assert!(db.reserved_memory_bytes() < parked.0);
                let receipt = publish();
                finish_new();
                receipt
            }
            (false, false) => {
                finish_new();
                assert!(db.reserved_memory_bytes() < parked.0);
                let receipt = publish();
                finish_old();
                receipt
            }
        };
        for receipt in [first, second, third] {
            assert_eq!(
                db.resolve_commit(receipt.transaction()).unwrap(),
                CommitResolution::Durable(receipt)
            );
        }
        third
    });
    assert_eq!(db.generation(), third.generation());
    assert_eq!(db.reserved_memory_bytes(), resident);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert!(db.reclaim(&cancel).unwrap() > 0);
    assert_eq!(db.reclaim(&cancel).unwrap(), 0);
    let latest = prepare_report(&db);
    verify_report(&db, db.execute(&latest, &cancel).unwrap(), LATEST_REPORT).unwrap();
    drop(latest);
    assert_eq!(db.reserved_memory_bytes(), resident);
    assert_eq!(db.reserved_temp_bytes(), 0);
    db.close().unwrap();
    let reopened = Database::open(&path, config).unwrap();
    for receipt in [first, second, third] {
        assert_eq!(
            reopened.resolve_commit(receipt.transaction()).unwrap(),
            CommitResolution::Durable(receipt)
        );
    }
    let latest = prepare_report(&reopened);
    verify_report(
        &reopened,
        reopened.execute(&latest, &cancel).unwrap(),
        LATEST_REPORT,
    )
    .unwrap();
    drop(latest);
    assert_eq!(reopened.reserved_temp_bytes(), 0);
    reopened.close().unwrap();
}
