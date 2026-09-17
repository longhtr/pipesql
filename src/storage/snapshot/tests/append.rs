//! Test how append turns borrowed batches into one published table update.
//!
//! Successful writes must copy their input before returning: callers overwrite
//! the same buffers for the next batch. Readers prepared before commit keep the
//! old rows; readers opened afterward see all committed batches. Literal values
//! check column mapping, NULLs and floating-point bits through production readers.
//!
//! A failed write leaves the append responsible for earlier unpublished files.
//! These tests check explicit abort, cleanup failures and recovery after reopen,
//! together with memory and temporary-space refusal. Effect faults simulate
//! reported I/O failures; they do not simulate a process or machine losing power.

use super::{Fixture, append_columns, declarations, snapshot_cells, token};
use crate::ColumnValues;
use crate::effects::{DirectoryKind, Effect, Effects, Faults};
use crate::storage::catalog;
use crate::storage::recovery::{UNITS_NAME, WAL_NAME};
use crate::storage::schema::{self, TableId};
use crate::storage::unit::{self, InputColumn};
use crate::{CancellationToken, CommitResolution, Database, Error};
use std::fs;
use std::sync::{Arc, Mutex};

fn positional_append_columns() -> [crate::ColumnInput<'static>; 2] {
    use crate::ColumnInput;
    // Positional input follows the declaration: note (ID 29), then amount
    // (ID 3). Sorting these fields by ID would fail the column type check.
    [
        ColumnInput {
            values: ColumnValues::String(&["雪", "", "abc", "ignored"]),
            validity: &[7],
        },
        ColumnInput {
            values: ColumnValues::Double(&[1., -0., f64::NAN, 42.]),
            validity: &[15],
        },
    ]
}

fn streaming_database() -> (Fixture, Database, TableId) {
    let fixture = Fixture::directory();
    let db = Database::create_catalog_with_effects(
        &fixture.root().join("db"),
        crate::Config::new(4_000_000, 2_000_000).unwrap(),
        &mut Effects::default(),
    )
    .unwrap();
    let table = TableId::new(17).unwrap();
    db.catalog_writer()
        .unwrap()
        .create_table(
            "facts",
            table,
            &declarations(),
            &CancellationToken::new(),
            &mut Effects::default(),
        )
        .unwrap();
    (fixture, db, table)
}

#[test]
fn catalog_engine_append_preserves_values_snapshots_and_receipts() {
    let fixture = Fixture::new();
    let config = crate::Config::new(2_000_000, 1_000_000).unwrap();
    let database = Database::open(fixture.root(), config).unwrap();
    let cancel = CancellationToken::new();
    let table = TableId::new(17).unwrap();
    database
        .catalog_writer()
        .unwrap()
        .create_table(
            "facts",
            table,
            &declarations(),
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let first = database
        .catalog_writer()
        .unwrap()
        .append(table, &append_columns(), &cancel, &mut Effects::default())
        .unwrap();
    let old = database.catalog_snapshot().unwrap();
    let mut aborted = database.catalog_writer().unwrap();
    aborted.issue(&cancel, &mut Effects::default()).unwrap();
    aborted.abort_unbuilt().unwrap();
    let second = database
        .catalog_writer()
        .unwrap()
        .append(table, &append_columns(), &cancel, &mut Effects::default())
        .unwrap();
    let expected = vec![
        (Some(1f64.to_bits()), Some("雪".to_owned())),
        (Some((-0f64).to_bits()), Some(String::new())),
        (Some(f64::NAN.to_bits()), Some("abc".to_owned())),
        (Some(42f64.to_bits()), None),
    ];
    assert_eq!(snapshot_cells(&old, &fixture.objects()), expected);
    let new = database.catalog_snapshot().unwrap();
    assert_eq!(
        snapshot_cells(&new, &fixture.objects()),
        [expected.clone(), expected.clone()].concat()
    );
    assert_eq!(database.reserved_temp_bytes(), 0);
    assert_eq!(
        database.reserved_memory_bytes(),
        crate::storage::snapshot::REGISTRY_BYTES + database.path_memory_bytes()
    );
    assert_eq!(
        database.resolve_commit(token(5)).unwrap(),
        CommitResolution::Aborted
    );
    drop(new);
    drop(old);
    database.close().unwrap();
    let database = Database::open(fixture.root(), config).unwrap();
    for commit in [first, second] {
        assert_eq!(
            database.resolve_commit(commit.transaction()).unwrap(),
            CommitResolution::Durable(commit)
        );
    }
    assert_eq!(
        snapshot_cells(&database.catalog_snapshot().unwrap(), &fixture.objects()),
        [expected.clone(), expected].concat()
    );
}

#[test]
fn catalog_engine_append_refuses_space_without_issuing() {
    let fixture = Fixture::new();
    let database =
        Database::open(fixture.root(), crate::Config::new(2_000_000, 4520).unwrap()).unwrap();
    let cancel = CancellationToken::new();
    let table = TableId::new(17).unwrap();
    database
        .catalog_writer()
        .unwrap()
        .create_table(
            "facts",
            table,
            &declarations(),
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let before = fs::read(fixture.root().join(WAL_NAME)).unwrap();
    assert!(matches!(
        database.catalog_writer().unwrap().append(
            table,
            &append_columns(),
            &cancel,
            &mut Effects::default()
        ),
        Err(Error::Resource { .. })
    ));
    assert_eq!(fs::read(fixture.root().join(WAL_NAME)).unwrap(), before);
    assert_eq!(fs::read_dir(fixture.objects()).unwrap().count(), 3);
    assert_eq!(database.reserved_temp_bytes(), 0);
    database.catalog_writer().unwrap().abort_unbuilt().unwrap();
}

#[test]
fn catalog_engine_append_cleans_construction_failures() {
    let config = crate::Config::new(2_000_000, 1_000_000).unwrap();
    let cancel = CancellationToken::new();
    let table = TableId::new(17).unwrap();
    let setup = || {
        let fixture = Fixture::new();
        let database = Database::open(fixture.root(), config).unwrap();
        database
            .catalog_writer()
            .unwrap()
            .create_table(
                "facts",
                table,
                &declarations(),
                &cancel,
                &mut Effects::default(),
            )
            .unwrap();
        database
            .catalog_writer()
            .unwrap()
            .append(table, &append_columns(), &cancel, &mut Effects::default())
            .unwrap();
        (fixture, database)
    };
    let (baseline, database) = setup();
    let trace = Arc::new(Mutex::new(Vec::new()));
    let observed = trace.clone();
    database
        .catalog_writer()
        .unwrap()
        .append(
            table,
            &append_columns(),
            &cancel,
            &mut Effects::with_faults(Faults {
                action: Some(Box::new(move |index, effect| {
                    observed.lock().unwrap().push((index, effect));
                })),
                ..Faults::default()
            }),
        )
        .unwrap();
    let trace = trace.lock().unwrap();
    let start = trace
        .iter()
        .position(|(_, effect)| {
            *effect == Effect::CreateMetadata(crate::effects::MetadataKind::CatalogObject)
        })
        .unwrap();
    let end = start
        + trace[start..]
            .iter()
            .position(|(_, effect)| *effect == Effect::SyncDirectory(DirectoryKind::Units))
            .unwrap();
    assert!(end - start < 128);
    // Fail each construction effect before publication. The seven objects from
    // the declaration and first append must survive; the new attempt must abort.
    for &(cut, _) in &trace[start..=end] {
        let (fixture, database) = setup();
        let old = database.catalog_snapshot().unwrap();
        let values = snapshot_cells(&old, &fixture.objects());
        assert!(
            database
                .catalog_writer()
                .unwrap()
                .append(
                    table,
                    &append_columns(),
                    &cancel,
                    &mut Effects::with_faults(Faults {
                        fail_at: Some(cut),
                        ..Faults::default()
                    })
                )
                .is_err(),
            "cut {cut}"
        );
        assert_eq!(database.reserved_temp_bytes(), 0, "cut {cut}");
        assert_eq!(
            fs::read_dir(fixture.objects()).unwrap().count(),
            7,
            "cut {cut}"
        );
        assert_eq!(snapshot_cells(&old, &fixture.objects()), values);
        assert_eq!(
            database.resolve_commit(token(5)).unwrap(),
            CommitResolution::Aborted
        );
        let retry = database
            .catalog_writer()
            .unwrap()
            .append(table, &append_columns(), &cancel, &mut Effects::default())
            .unwrap();
        assert_eq!(retry.transaction(), token(6));
    }
    println!("catalog_append_construction_effects={}", end - start + 1);
    drop(database);
    drop(baseline);
}

#[test]
fn catalog_streaming_append_reuses_inputs_and_publishes_once() {
    use crate::AppendLimits;
    use crate::{QueryStep, Value};
    let parent = Fixture::directory();
    let path = parent.root().join("streaming-append");
    let config = crate::Config::new(4_000_000, 2_000_000).unwrap();
    let db = Database::create_catalog_with_effects(&path, config, &mut Effects::default()).unwrap();
    let cancel = CancellationToken::new();
    let table = TableId::new(17).unwrap();
    db.catalog_writer()
        .unwrap()
        .create_table(
            "facts",
            table,
            &declarations(),
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let old = crate::query::prepare_catalog(&db, "FROM facts |> SELECT note, amount").unwrap();
    let mut append = db
        .catalog_writer()
        .unwrap()
        .begin_append(
            table,
            AppendLimits {
                batches: 2,
                encoded_bytes: 100_000,
            },
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let token = append.transaction();
    assert!(matches!(
        db.resolve_catalog(token, &mut Effects::default()),
        Err(Error::Contention(_))
    ));
    assert!(db.reserved_temp_bytes() > 100_000);
    let mut text = String::from("first");
    let mut numbers = [1.0];
    for next in ["second", "overwritten"] {
        let text_values = [text.as_str()];
        let columns = [
            InputColumn {
                id: schema::ColumnId::new(29).unwrap(),
                values: ColumnValues::String(&text_values),
                validity: &[1],
            },
            InputColumn {
                id: schema::ColumnId::new(3).unwrap(),
                values: ColumnValues::Double(&numbers),
                validity: &[1],
            },
        ];
        append
            .write_identified(&columns, &cancel, &mut Effects::default())
            .unwrap();
        text.clear();
        text.push_str(next);
        numbers[0] += 1.0;
        assert_eq!(db.generation(), 1);
        // Each write has returned, but commit has not published either batch.
        // The prepared query must still execute against its empty table snapshot.
        let mut reader = db.execute(&old, &cancel).unwrap();
        assert!(matches!(reader.step(), QueryStep::Finished));
    }
    let commit = append
        .commit_with_effects(&cancel, &mut Effects::default())
        .unwrap();
    assert_eq!(commit.transaction(), token);
    assert_eq!(commit.generation(), 2);
    assert_eq!(db.reserved_temp_bytes(), 0);
    let mut old_reader = db.execute(&old, &cancel).unwrap();
    assert!(matches!(old_reader.step(), QueryStep::Finished));
    drop(old_reader);
    drop(old);
    db.close().unwrap();
    let db = Database::open(&path, config).unwrap();
    assert_eq!(
        db.resolve_catalog(token, &mut Effects::default()).unwrap(),
        crate::CommitResolution::Durable(commit)
    );
    let query = crate::query::prepare_catalog(&db, "FROM facts |> SELECT note, amount").unwrap();
    let mut reader = db.execute(&query, &cancel).unwrap();
    let mut rows = Vec::new();
    let mut finished = false;
    for _ in 0..64 {
        match reader.step() {
            QueryStep::Progress => {}
            QueryStep::Rows(batch) => {
                for row in 0..batch.len() {
                    let Some(Value::String(text)) = batch.value(row, 0) else {
                        panic!("STRING");
                    };
                    let Some(Value::Double(value)) = batch.value(row, 1) else {
                        panic!("DOUBLE");
                    };
                    rows.push((text.as_str().to_owned(), value));
                }
            }
            QueryStep::Finished => {
                finished = true;
                break;
            }
            QueryStep::Failed(error) => panic!("query failed: {error:?}"),
        }
    }
    assert!(finished);
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        rows,
        [("first".to_owned(), 1.0), ("second".to_owned(), 2.0)]
    );
    drop(reader);
    drop(query);
    assert_eq!(
        db.reserved_memory_bytes(),
        crate::storage::snapshot::REGISTRY_BYTES + db.path_memory_bytes()
    );
}

#[test]
fn catalog_streaming_append_limits_abort_and_recovery() {
    use crate::AppendLimits;
    let (_fixture, db, table) = streaming_database();
    let cancel = CancellationToken::new();
    let objects = db.path().join(UNITS_NAME);
    let initial = fs::read(db.path().join(WAL_NAME)).unwrap();
    for limits in [
        AppendLimits {
            batches: 0,
            encoded_bytes: 1,
        },
        AppendLimits {
            batches: catalog::MAX_UNITS + 1,
            encoded_bytes: 1,
        },
        AppendLimits {
            batches: 1,
            encoded_bytes: 0,
        },
    ] {
        assert!(matches!(
            db.catalog_writer().unwrap().begin_append(
                table,
                limits,
                &cancel,
                &mut Effects::default()
            ),
            Err(Error::Resource { .. })
        ));
        assert_eq!(fs::read(db.path().join(WAL_NAME)).unwrap(), initial);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(
            db.reserved_memory_bytes(),
            crate::storage::snapshot::REGISTRY_BYTES + db.path_memory_bytes()
        );
    }
    // The small allowance rejects the first batch by size. The larger one
    // accepts it, then rejects the second batch by count. Both failures stop
    // further writes without preventing abort.
    for bytes in [1, 100_000] {
        let mut append = db
            .catalog_writer()
            .unwrap()
            .begin_append(
                table,
                AppendLimits {
                    batches: 1,
                    encoded_bytes: bytes,
                },
                &cancel,
                &mut Effects::default(),
            )
            .unwrap();
        let token = append.transaction();
        if bytes > 1 {
            append
                .write_identified(&append_columns(), &cancel, &mut Effects::default())
                .unwrap();
        }
        let mut effects = Effects::default();
        assert!(matches!(
            append.write_identified(&append_columns(), &cancel, &mut effects),
            Err(Error::Resource { .. })
        ));
        assert_eq!(effects.count(), 0, "limits refuse before file effects");
        assert!(matches!(
            append.write_identified(&append_columns(), &cancel, &mut effects),
            Err(Error::Unsupported(_))
        ));
        append.abort_with_effects(&mut Effects::default()).unwrap();
        assert_eq!(
            db.resolve_catalog(token, &mut Effects::default()).unwrap(),
            crate::CommitResolution::Aborted
        );
        assert_eq!(fs::read_dir(&objects).unwrap().count(), 3);
        assert_eq!(db.generation(), 1);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(
            db.reserved_memory_bytes(),
            crate::storage::snapshot::REGISTRY_BYTES + db.path_memory_bytes()
        );
    }
    // Consume all remaining memory after admission. Encoding the first batch
    // must then fail before creating a file, and explicit abort must still work.
    let mut append = db
        .catalog_writer()
        .unwrap()
        .begin_append(
            table,
            AppendLimits {
                batches: 2,
                encoded_bytes: 100_000,
            },
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let pressure = db
        .reserve_memory(
            db.config().memory_limit_bytes() - db.reserved_memory_bytes(),
            "deny streaming encoder",
        )
        .unwrap();
    let mut effects = Effects::default();
    assert!(matches!(
        append.write_identified(&append_columns(), &cancel, &mut effects),
        Err(Error::Resource { .. })
    ));
    assert_eq!(effects.count(), 0);
    drop(pressure);
    append.abort_with_effects(&mut Effects::default()).unwrap();
    // Dropping the append leaves its two files for recovery and blocks another
    // writer. Reopen must remove them and resolve the issued transaction as
    // aborted, since no commit added it to the success history.
    let mut append = db
        .catalog_writer()
        .unwrap()
        .begin_append(
            table,
            AppendLimits {
                batches: 2,
                encoded_bytes: 100_000,
            },
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    let token = append.transaction();
    for _ in 0..2 {
        append
            .write_identified(&append_columns(), &cancel, &mut Effects::default())
            .unwrap();
    }
    drop(append);
    assert!(db.catalog_writer().is_err());
    assert!(db.reserved_temp_bytes() > 0);
    let path = db.path().to_owned();
    let config = db.config();
    db.close().unwrap();
    let db = Database::open(&path, config).unwrap();
    assert_eq!(
        db.resolve_catalog(token, &mut Effects::default()).unwrap(),
        crate::CommitResolution::Aborted
    );
    assert_eq!(fs::read_dir(objects).unwrap().count(), 3);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert_eq!(db.generation(), 1);
}

#[test]
fn catalog_streaming_append_second_batch_faults_clean_entire_prefix() {
    use crate::AppendLimits;
    let (_fixture, db, table) = streaming_database();
    let cancel = CancellationToken::new();
    let limits = AppendLimits {
        batches: 2,
        encoded_bytes: 100_000,
    };
    let mut baseline = db
        .catalog_writer()
        .unwrap()
        .begin_append(table, limits, &cancel, &mut Effects::default())
        .unwrap();
    baseline
        .write_identified(&append_columns(), &cancel, &mut Effects::default())
        .unwrap();
    let mut effects = Effects::default();
    baseline
        .write_identified(&append_columns(), &cancel, &mut effects)
        .unwrap();
    let count = effects.count();
    assert!(count > 0 && count < 32);
    baseline
        .abort_with_effects(&mut Effects::default())
        .unwrap();
    // A failure in batch two must not strand the already written first batch.
    // Abort must remove both and release their reservations at every effect.
    for cut in 0..count {
        let mut append = db
            .catalog_writer()
            .unwrap()
            .begin_append(table, limits, &cancel, &mut Effects::default())
            .unwrap();
        let token = append.transaction();
        append
            .write_identified(&append_columns(), &cancel, &mut Effects::default())
            .unwrap();
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(cut),
            ..Faults::default()
        });
        assert!(
            append
                .write_identified(&append_columns(), &cancel, &mut effects)
                .is_err(),
            "second batch cut {cut}"
        );
        assert_eq!(effects.count(), cut + 1);
        append.abort_with_effects(&mut Effects::default()).unwrap();
        assert_eq!(
            db.resolve_catalog(token, &mut Effects::default()).unwrap(),
            crate::CommitResolution::Aborted
        );
        assert_eq!(fs::read_dir(db.path().join(UNITS_NAME)).unwrap().count(), 3);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(
            db.reserved_memory_bytes(),
            crate::storage::snapshot::REGISTRY_BYTES + db.path_memory_bytes()
        );
        assert_eq!(db.generation(), 1);
    }
    let mut append = db
        .catalog_writer()
        .unwrap()
        .begin_append(table, limits, &cancel, &mut Effects::default())
        .unwrap();
    append
        .write_identified(&append_columns(), &cancel, &mut Effects::default())
        .unwrap();
    let stopped = CancellationToken::new();
    stopped.cancel();
    assert!(matches!(
        append.write_identified(&append_columns(), &stopped, &mut Effects::default()),
        Err(Error::Cancelled)
    ));
    append.abort_with_effects(&mut Effects::default()).unwrap();
}

#[test]
fn append_rounding_is_admitted_before_allocation_or_issuance() {
    let (_fixture, db, table) = streaming_database();
    let cancel = CancellationToken::new();
    let wal = fs::read(db.path().join(WAL_NAME)).unwrap();
    let resident = db.reserved_memory_bytes();
    // Keep the path and allocation-rounding allowances literal, independent of
    // the admission calculation. Only the handle size comes from its Rust type.
    // At exactly this allowance, the next reservation (catalog scratch) must fail.
    let owner = std::mem::size_of::<crate::storage::append::Append<'_>>() as u64 + 8_192 + 49_152;
    for (available, refused_owner) in [
        (owner - 1, "streaming append owner"),
        (owner, "append catalog admission"),
    ] {
        let pressure = db
            .reserve_memory(
                db.config().memory_limit_bytes() - resident - available,
                "append admission boundary",
            )
            .unwrap();
        let mut effects = Effects::default();
        let result = db.catalog_writer().unwrap().begin_append(
            table,
            crate::AppendLimits {
                batches: 1,
                encoded_bytes: 100_000,
            },
            &cancel,
            &mut effects,
        );
        assert!(matches!(result, Err(Error::Resource { owner, .. }) if owner == refused_owner));
        drop(result);
        assert_eq!(effects.count(), 0, "refusal precedes native effects");
        assert_eq!(fs::read(db.path().join(WAL_NAME)).unwrap(), wal);
        assert_eq!(db.reserved_temp_bytes(), 0);
        drop(pressure);
        assert_eq!(db.reserved_memory_bytes(), resident);
        db.catalog_writer().unwrap().abort_unbuilt().unwrap();
    }
}

#[test]
fn append_workspace_reuses_disjoint_phases_at_exact_growth_admission() {
    for one_byte_short in [false, true] {
        let (_fixture, db, table) = streaming_database();
        let cancel = CancellationToken::new();
        let mut append = db
            .catalog_writer()
            .unwrap()
            .begin_append(
                table,
                crate::AppendLimits {
                    batches: 2,
                    encoded_bytes: 100_000,
                },
                &cancel,
                &mut Effects::default(),
            )
            .unwrap();
        let admitted = db.reserved_memory_bytes();
        append
            .write_identified(&append_columns(), &cancel, &mut Effects::default())
            .unwrap();
        let initial_charge = if cfg!(target_os = "macos") {
            131_072
        } else {
            65_536
        };
        let grown_charge = if cfg!(target_os = "macos") {
            163_840
        } else {
            65_673
        };
        assert_eq!(db.reserved_memory_bytes() - admitted, initial_charge);
        let text = "x".repeat(65_536);
        let strings = [text.as_str()];
        let columns = [
            InputColumn {
                id: schema::ColumnId::new(29).unwrap(),
                values: ColumnValues::String(&strings),
                validity: &[1],
            },
            InputColumn {
                id: schema::ColumnId::new(3).unwrap(),
                values: ColumnValues::Double(&[1.0]),
                validity: &[1],
            },
        ];
        // This batch needs 128 metadata bytes and 65,545 STRING bytes: 65,673
        // total. Darwin admission grows by 32 KiB; GNU by 137 bytes. Leave exactly
        // that charge difference (or one byte less). Success requires freeing the old buffer
        // before reserving its replacement.
        let pressure = db
            .reserve_memory(
                db.config().memory_limit_bytes()
                    - db.reserved_memory_bytes()
                    - (grown_charge - initial_charge - u64::from(one_byte_short)),
                "exact append workspace growth",
            )
            .unwrap();
        let mut effects = Effects::default();
        let result = append.write_identified(&columns, &cancel, &mut effects);
        if one_byte_short {
            assert!(matches!(result, Err(Error::Resource { .. })));
            assert_eq!(effects.count(), 0);
            assert_eq!(db.reserved_memory_bytes() - pressure.bytes(), admitted);
        } else {
            result.unwrap();
            assert_eq!(
                db.reserved_memory_bytes() - pressure.bytes(),
                admitted + grown_charge
            );
        }
        drop(pressure);
        append.abort_with_effects(&mut Effects::default()).unwrap();
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(
            db.reserved_memory_bytes(),
            crate::storage::snapshot::REGISTRY_BYTES + db.path_memory_bytes()
        );
        assert_eq!(fs::read_dir(db.path().join(UNITS_NAME)).unwrap().count(), 3);
    }
}

#[test]
fn catalog_streaming_append_growth_refusal_preserves_cleanup_ownership() {
    use crate::AppendLimits;
    let (_fixture, db, table) = streaming_database();
    let cancel = CancellationToken::new();
    let mut append = db
        .catalog_writer()
        .unwrap()
        .begin_append(
            table,
            AppendLimits {
                batches: 2,
                encoded_bytes: 100_000,
            },
            &cancel,
            &mut Effects::default(),
        )
        .unwrap();
    append
        .write_identified(&append_columns(), &cancel, &mut Effects::default())
        .unwrap();
    let text = "x".repeat(unit::MAX_TEXT_BYTES);
    let strings = [text.as_str()];
    let columns = [
        InputColumn {
            id: schema::ColumnId::new(29).unwrap(),
            values: ColumnValues::String(&strings),
            validity: &[1],
        },
        InputColumn {
            id: schema::ColumnId::new(3).unwrap(),
            values: ColumnValues::Double(&[1.0]),
            validity: &[1],
        },
    ];
    let pressure = db
        .reserve_memory(
            db.config().memory_limit_bytes() - db.reserved_memory_bytes(),
            "deny encoder growth",
        )
        .unwrap();
    let mut effects = Effects::default();
    assert!(matches!(
        append.write_identified(&columns, &cancel, &mut effects),
        Err(Error::Resource { .. })
    ));
    assert_eq!(effects.count(), 0);
    assert!(db.reserved_temp_bytes() > 0);
    drop(pressure);
    append.abort_with_effects(&mut Effects::default()).unwrap();
    assert_eq!(fs::read_dir(db.path().join(UNITS_NAME)).unwrap().count(), 3);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert_eq!(
        db.reserved_memory_bytes(),
        crate::storage::snapshot::REGISTRY_BYTES + db.path_memory_bytes()
    );
}

#[test]
fn catalog_streaming_append_transfers_commit_and_releases_owners() {
    check_streaming_append(false);
}

#[test]
fn catalog_streaming_append_has_bounded_reported_stack() {
    check_streaming_append(true);
}

// Run the same named, positional append on ordinary and checked small stacks.
// This covers this two-batch flow, not every possible append input or platform.
fn check_streaming_append(small_stack: bool) {
    use crate::AppendLimits;
    let (_fixture, db, _) = streaming_database();
    let thread = if small_stack {
        std::thread::Builder::new().stack_size(pipesql_filesystem::TEST_SMALL_STACK_REQUEST_BYTES)
    } else {
        std::thread::Builder::new()
    };
    let commit = std::thread::scope(|scope| {
        thread
            .spawn_scoped(scope, || {
                if small_stack {
                    pipesql_filesystem::test_assert_small_stack();
                }
                let cancel = CancellationToken::new();
                let mut append = db
                    .catalog_writer()
                    .unwrap()
                    .begin_append_named(
                        "facts",
                        AppendLimits {
                            batches: 2,
                            encoded_bytes: 100_000,
                        },
                        &cancel,
                        &mut Effects::default(),
                    )
                    .unwrap();
                for _ in 0..2 {
                    append
                        .write_columns(
                            &positional_append_columns(),
                            &cancel,
                            &mut Effects::default(),
                        )
                        .unwrap();
                }
                append
                    .commit_with_effects(&cancel, &mut Effects::default())
                    .unwrap()
            })
            .unwrap()
            .join()
            .unwrap()
    });
    assert_eq!(commit.generation(), 2);
    assert_eq!(db.reserved_temp_bytes(), 0);
    assert_eq!(
        db.reserved_memory_bytes(),
        crate::storage::snapshot::REGISTRY_BYTES + db.path_memory_bytes()
    );
}

#[test]
fn catalog_streaming_append_cleanup_failure_requires_recovery() {
    use crate::AppendLimits;
    for cut in 0..3 {
        let (_fixture, db, table) = streaming_database();
        let cancel = CancellationToken::new();
        let mut append = db
            .catalog_writer()
            .unwrap()
            .begin_append(
                table,
                AppendLimits {
                    batches: 2,
                    encoded_bytes: 100_000,
                },
                &cancel,
                &mut Effects::default(),
            )
            .unwrap();
        let token = append.transaction();
        for _ in 0..2 {
            append
                .write_identified(&append_columns(), &cancel, &mut Effects::default())
                .unwrap();
        }
        assert!(matches!(
            append.abort_with_effects(&mut Effects::with_faults(Faults {
                fail_at: Some(cut),
                ..Faults::default()
            })),
            Err(Error::RecoveryRequired { .. })
        ));
        assert!(
            db.reserved_temp_bytes() > 0,
            "cleanup cut {cut} retains debt"
        );
        assert!(db.catalog_writer().is_err());
        let path = db.path().to_owned();
        let config = db.config();
        db.close().unwrap();
        let db = Database::open(&path, config).unwrap();
        assert_eq!(
            db.resolve_catalog(token, &mut Effects::default()).unwrap(),
            crate::CommitResolution::Aborted
        );
        assert_eq!(fs::read_dir(path.join(UNITS_NAME)).unwrap().count(), 3);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(db.generation(), 1);
    }
}

#[test]
fn catalog_streaming_append_commit_failure_cleans_all_five_objects() {
    use crate::AppendLimits;
    let cancel = CancellationToken::new();
    let limits = AppendLimits {
        batches: 2,
        encoded_bytes: 100_000,
    };
    let (_baseline, db, table) = streaming_database();
    let mut append = db
        .catalog_writer()
        .unwrap()
        .begin_append(table, limits, &cancel, &mut Effects::default())
        .unwrap();
    for _ in 0..2 {
        append
            .write_identified(&append_columns(), &cancel, &mut Effects::default())
            .unwrap();
    }
    let trace = Arc::new(Mutex::new(Vec::new()));
    let captured = trace.clone();
    append
        .commit_with_effects(
            &cancel,
            &mut Effects::with_faults(Faults {
                action: Some(Box::new(move |index, effect| {
                    captured.lock().unwrap().push((index, effect))
                })),
                ..Faults::default()
            }),
        )
        .unwrap();
    let trace = trace.lock().unwrap();
    let last_create = trace
        .iter()
        .rfind(|(_, effect)| {
            *effect == Effect::CreateMetadata(crate::effects::MetadataKind::CatalogObject)
        })
        .unwrap()
        .0;
    let last_sync = trace
        .iter()
        .rfind(|(_, effect)| {
            *effect == Effect::SyncMetadata(crate::effects::MetadataKind::CatalogObject)
        })
        .unwrap()
        .0;
    let directory_sync = trace
        .iter()
        .find(|(_, effect)| *effect == Effect::SyncDirectory(DirectoryKind::Units))
        .unwrap()
        .0;
    // These late construction failures occur before publication. Cleanup must
    // remove both data units and the three new metadata objects, leaving only
    // the declaration. Publication failures have separate tests.
    for cut in [last_create, last_sync, directory_sync] {
        let (_fixture, db, table) = streaming_database();
        let mut append = db
            .catalog_writer()
            .unwrap()
            .begin_append(table, limits, &cancel, &mut Effects::default())
            .unwrap();
        let token = append.transaction();
        for _ in 0..2 {
            append
                .write_identified(&append_columns(), &cancel, &mut Effects::default())
                .unwrap();
        }
        assert!(
            append
                .commit_with_effects(
                    &cancel,
                    &mut Effects::with_faults(Faults {
                        fail_at: Some(cut),
                        ..Faults::default()
                    })
                )
                .is_err(),
            "commit construction cut {cut}"
        );
        assert_eq!(
            db.resolve_catalog(token, &mut Effects::default()).unwrap(),
            crate::CommitResolution::Aborted
        );
        assert_eq!(fs::read_dir(db.path().join(UNITS_NAME)).unwrap().count(), 3);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(
            db.reserved_memory_bytes(),
            crate::storage::snapshot::REGISTRY_BYTES + db.path_memory_bytes()
        );
        assert_eq!(db.generation(), 1);
    }
}

#[test]
fn catalog_named_append_maps_stored_identities_and_reopens() {
    use crate::AppendLimits;
    use crate::storage::snapshot::REGISTRY_BYTES;
    let (_fixture, db, _) = streaming_database();
    let path = db.path().to_owned();
    let config = db.config();
    let cancel = CancellationToken::new();
    let limits = AppendLimits {
        batches: 2,
        encoded_bytes: 100_000,
    };
    let initial = fs::read(path.join(WAL_NAME)).unwrap();
    for name in [
        "missing",
        "",
        "bad name",
        "abcdefghijklmnopqrstuvwxyzabcdefg",
    ] {
        assert!(
            db.catalog_writer()
                .unwrap()
                .begin_append_named(name, limits, &cancel, &mut Effects::default())
                .is_err()
        );
        assert_eq!(fs::read(path.join(WAL_NAME)).unwrap(), initial);
        assert_eq!(
            db.reserved_memory_bytes(),
            REGISTRY_BYTES + db.path_memory_bytes()
        );
        assert_eq!(db.reserved_temp_bytes(), 0);
    }
    let mut append = db
        .catalog_writer()
        .unwrap()
        .begin_append_named("FaCtS", limits, &cancel, &mut Effects::default())
        .unwrap();
    let transaction = append.transaction();
    assert_eq!(transaction.sequence(), 2);
    for _ in 0..2 {
        append
            .write_columns(
                &positional_append_columns(),
                &cancel,
                &mut Effects::default(),
            )
            .unwrap();
    }
    let commit = append
        .commit_with_effects(&cancel, &mut Effects::default())
        .unwrap();
    assert_eq!(commit.transaction(), transaction);
    db.close().unwrap();
    let db = Database::open(&path, config).unwrap();
    assert_eq!(
        db.resolve_commit(transaction).unwrap(),
        CommitResolution::Durable(commit)
    );
    let expected = vec![
        (Some(1f64.to_bits()), Some("雪".to_owned())),
        (Some((-0f64).to_bits()), Some(String::new())),
        (Some(f64::NAN.to_bits()), Some("abc".to_owned())),
        (Some(42f64.to_bits()), None),
    ];
    assert_eq!(
        snapshot_cells(&db.catalog_snapshot().unwrap(), &path.join(UNITS_NAME)),
        [expected.clone(), expected].concat()
    );
}

#[test]
fn catalog_named_append_invalid_second_batch_requires_abort() {
    use crate::AppendLimits;
    use crate::ColumnInput;
    use crate::storage::snapshot::REGISTRY_BYTES;
    let valid = positional_append_columns();
    // Each malformed second batch follows a valid first batch. Rejecting it
    // must stop later writes while keeping the first batch available for abort.
    let wrong_type = [valid[1], valid[0]];
    let wrong_rows = [
        valid[0],
        ColumnInput {
            values: ColumnValues::Double(&[1.]),
            validity: &[1],
        },
    ];
    let null_nonnullable = [
        valid[0],
        ColumnInput {
            validity: &[7],
            ..valid[1]
        },
    ];
    let invalid_bitmap = [
        ColumnInput {
            validity: &[255],
            ..valid[0]
        },
        valid[1],
    ];
    for bad in [
        &[][..],
        &valid[..1],
        &wrong_type,
        &wrong_rows,
        &null_nonnullable,
        &invalid_bitmap,
    ] {
        let (_fixture, db, _) = streaming_database();
        let cancel = CancellationToken::new();
        let mut append = db
            .catalog_writer()
            .unwrap()
            .begin_append_named(
                "facts",
                AppendLimits {
                    batches: 2,
                    encoded_bytes: 100_000,
                },
                &cancel,
                &mut Effects::default(),
            )
            .unwrap();
        let transaction = append.transaction();
        append
            .write_columns(&valid, &cancel, &mut Effects::default())
            .unwrap();
        let mut effects = Effects::default();
        assert!(append.write_columns(bad, &cancel, &mut effects).is_err());
        assert_eq!(effects.count(), 0);
        assert!(append.write_columns(&valid, &cancel, &mut effects).is_err());
        assert_eq!(effects.count(), 0);
        append.abort_with_effects(&mut Effects::default()).unwrap();
        assert_eq!(
            db.resolve_commit(transaction).unwrap(),
            CommitResolution::Aborted
        );
        assert_eq!(db.generation(), 1);
        assert_eq!(db.reserved_temp_bytes(), 0);
        assert_eq!(
            db.reserved_memory_bytes(),
            REGISTRY_BYTES + db.path_memory_bytes()
        );
        assert_eq!(fs::read_dir(db.path().join(UNITS_NAME)).unwrap().count(), 3);
    }
}
