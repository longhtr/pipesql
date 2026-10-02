//! Check that failed scratch creation leaves legacy table data readable and recoverable.
//!
//! Run the constructor on empty and populated databases, first recording its
//! effects and then failing each one. Check that reservations are released and
//! that uncertain filename removal prevents another constructor from proceeding
//! until reopen. Recovery must reject unknown or invalid files before deleting them.
//!
//! The process-death test runs this test executable as a child and exits before
//! selected effects. The parent requires the interruption exit code before opening
//! the database and checking the expected row count. This tests process death,
//! not loss of power. The coexistence test uses callbacks during construction;
//! it checks overlapping operations without claiming arbitrary thread scheduling.

use super::*;
use crate::effects::Faults;
use crate::{Config, QueryStep, Value};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "pipesql-legacy-scratch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn database(&self) -> PathBuf {
        self.0.join("db")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        crate::test_cleanup::directory(&self.0);
    }
}
fn config() -> Config {
    Config::new(4_000_000, 4_000_000).unwrap()
}
fn database(directory: &Directory, populated: bool) -> Database {
    let mut db = Database::create(&directory.database(), config()).unwrap();
    if populated {
        let input = directory.0.join("lineitem.tbl");
        std::fs::write(
            &input,
            b"1|2|3|4|1|100|0.08|8|R|F|1994-01-01|12|13|14|15|16|\n".repeat(2),
        )
        .unwrap();
        db.load_lineitem(&input, &CancellationToken::new()).unwrap();
    }
    db
}
fn count(db: &Database, expected: i64) {
    let query = db
        .prepare("FROM lineitem |> AGGREGATE COUNT(*) AS n")
        .unwrap();
    let cancel = CancellationToken::new();
    let mut result = db.execute(&query, &cancel).unwrap();
    let mut rows = 0;
    for _ in 0..1000 {
        match result.step() {
            QueryStep::Progress => (),
            QueryStep::Rows(batch) => {
                assert_eq!(batch.len(), 1);
                assert_eq!(batch.value(0, 0), Some(Value::Int64(expected)));
                rows += 1;
            }
            QueryStep::Finished => {
                assert_eq!(rows, 1);
                return;
            }
            QueryStep::Failed(error) => panic!("legacy scratch-free read: {error}"),
        }
    }
    panic!("legacy read did not finish");
}
fn constructor_events(db: &Database) -> Vec<Effect> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    let mut effects = Effects::with_faults(Faults {
        action: Some(Box::new(move |_, e| captured.borrow_mut().push(e))),
        ..Faults::default()
    });
    drop(Scratch::new(db, &CancellationToken::new(), &mut effects).unwrap());
    drop(effects);
    Rc::try_unwrap(events).unwrap().into_inner()
}

#[test]
fn legacy_scratch_failure_recovery_preserves_source_and_strict_writer_admission() {
    for populated in [false, true] {
        let control = Directory::new();
        let db = database(&control, populated);
        let events = constructor_events(&db);
        let first_create = events
            .iter()
            .position(|e| *e == Effect::Load(LoadEffect::CreateStaging))
            .unwrap();
        let barrier = events
            .iter()
            .position(|e| *e == Effect::SyncDirectory(DirectoryKind::Private))
            .unwrap();
        for fault in 0..events.len() {
            let directory = Directory::new();
            let db = database(&directory, populated);
            let baseline = db.reserved_memory_bytes();
            let mut effects = Effects::with_faults(Faults {
                fail_at: Some(fault as u64),
                ..Faults::default()
            });
            assert!(
                Scratch::new(&db, &CancellationToken::new(), &mut effects).is_err(),
                "effect {fault}"
            );
            assert_eq!(db.reserved_memory_bytes(), baseline);
            assert_eq!(db.reserved_temp_bytes(), 0);
            count(&db, if populated { 2 } else { 0 });
            let private = directory.database().join(PRIVATE_NAME);
            if std::fs::read_dir(&private).unwrap().next().is_some() {
                let mut inspect = Effects::default();
                assert!(matches!(
                    crate::storage::recovery::inspect_namespace(
                        db.path(),
                        &db.memory,
                        &mut inspect
                    ),
                    Err(Error::RecoveryRequired { .. })
                ));
            }
            let mut retry = Effects::default();
            let result = Scratch::new(&db, &CancellationToken::new(), &mut retry);
            // From the first creation attempt through the directory sync, the
            // constructor cannot promise that its filenames are durably absent.
            if (first_create..=barrier).contains(&fault) {
                assert!(matches!(result, Err(Error::RecoveryRequired { .. })));
                assert_eq!(retry.count(), 0);
                drop(result);
            } else {
                drop(result.unwrap());
            }
            db.close().unwrap();
            let db = Database::open(&directory.database(), config()).unwrap();
            assert_eq!(std::fs::read_dir(&private).unwrap().count(), 0);
            count(&db, if populated { 2 } else { 0 });
            drop(Scratch::new(&db, &CancellationToken::new(), &mut Effects::default()).unwrap());
        }
    }
}

#[test]
fn legacy_scratch_reopen_rejects_unowned_debris_before_cleanup() {
    for populated in [false, true] {
        for invalid in [
            "nonempty",
            "alias",
            "directory",
            "unknown",
            "corrupt-source",
        ] {
            let directory = Directory::new();
            let db = database(&directory, populated);
            db.close().unwrap();
            let private = directory.database().join(PRIVATE_NAME);
            let path = private.join(if invalid == "unknown" {
                "unknown"
            } else {
                crate::storage::recovery::SCRATCH_NAMES[0]
            });
            if invalid == "directory" {
                std::fs::create_dir(&path).unwrap();
            } else {
                std::fs::write(
                    &path,
                    if invalid == "nonempty" {
                        b"x".as_slice()
                    } else {
                        b"".as_slice()
                    },
                )
                .unwrap();
            }
            if invalid == "alias" {
                std::fs::hard_link(
                    &path,
                    private.join(crate::storage::recovery::SCRATCH_NAMES[1]),
                )
                .unwrap();
            }
            if invalid == "corrupt-source" {
                let control = directory
                    .database()
                    .join(crate::storage::recovery::CONTROL_NAME);
                let mut bytes = std::fs::read(&control).unwrap();
                bytes[0] ^= 1;
                std::fs::write(control, bytes).unwrap();
            }
            assert!(
                matches!(
                    Database::open(&directory.database(), config()),
                    Err(Error::Corrupt(_))
                ),
                "{invalid}"
            );
            assert!(path.exists(), "invalid debris must remain for diagnosis");
        }
    }
}

#[test]
fn legacy_scratch_process_death_heals_only_disposable_names() {
    const CHILD_PATH: &str = "PIPESQL_LEGACY_SCRATCH_CHILD_PATH";
    const CHILD_CUT: &str = "PIPESQL_LEGACY_SCRATCH_CHILD_CUT";
    if let Some(path) = std::env::var_os(CHILD_PATH) {
        let cut: u64 = std::env::var(CHILD_CUT).unwrap().parse().unwrap();
        let db = Database::open(&PathBuf::from(path), config()).unwrap();
        // Exit without running Rust destructors. The parent checks that recovery
        // can remove the constructor's unfinished names without changing table data.
        let mut effects = Effects::with_faults(Faults {
            action: Some(Box::new(move |index, _| {
                if index == cut {
                    std::process::exit(73);
                }
            })),
            ..Faults::default()
        });
        drop(Scratch::new(&db, &CancellationToken::new(), &mut effects).unwrap());
        panic!("scratch process cut was not reached");
    }
    for populated in [false, true] {
        let control = Directory::new();
        let db = database(&control, populated);
        let events = constructor_events(&db);
        let cuts: Vec<_> = events
            .iter()
            .enumerate()
            .filter_map(|(index, e)| {
                matches!(
                    e,
                    Effect::Load(
                        LoadEffect::CreateStaging
                            | LoadEffect::RemoveStaging
                            | LoadEffect::InspectStaging
                    ) | Effect::SyncDirectory(DirectoryKind::Private)
                )
                .then_some(index)
            })
            .collect();
        assert_eq!(
            cuts.len(),
            7,
            "two creates, two unlinks, barrier and two descriptor inspections"
        );
        for cut in cuts {
            let directory = Directory::new();
            let db = database(&directory, populated);
            db.close().unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg(
                    concat!(
                        module_path!(),
                        "::legacy_scratch_process_death_heals_only_disposable_names"
                    )
                    .strip_prefix("pipesql::")
                    .unwrap(),
                )
                .arg("--test-threads=1")
                .env(CHILD_PATH, directory.database())
                .env(CHILD_CUT, cut.to_string())
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(73), "cut {cut}");
            let db = Database::open(&directory.database(), config()).unwrap();
            count(&db, if populated { 2 } else { 0 });
            assert_eq!(
                std::fs::read_dir(directory.database().join(PRIVATE_NAME))
                    .unwrap()
                    .count(),
                0
            );
            drop(Scratch::new(&db, &CancellationToken::new(), &mut Effects::default()).unwrap());
        }
    }
}

#[test]
fn legacy_query_admission_coexists_with_live_scratch_names() {
    for populated in [false, true] {
        let directory = Directory::new();
        let db = Rc::new(database(&directory, populated));
        let shared = db.clone();
        let observed = Rc::new(std::cell::Cell::new(0));
        let captured = observed.clone();
        let mut effects = Effects::with_faults(Faults {
            after_action: Some(Box::new(move |_, effect| {
                if effect == Effect::Load(LoadEffect::CreateStaging) {
                    count(&shared, if populated { 2 } else { 0 });
                    assert!(matches!(
                        Scratch::new(&shared, &CancellationToken::new(), &mut Effects::default()),
                        Err(Error::Contention("scratch bootstrap"))
                    ));
                    captured.set(captured.get() + 1);
                }
            })),
            ..Faults::default()
        });
        drop(Scratch::new(&db, &CancellationToken::new(), &mut effects).unwrap());
        assert_eq!(observed.get(), 2);
        assert_eq!(
            std::fs::read_dir(directory.database().join(PRIVATE_NAME))
                .unwrap()
                .count(),
            0
        );
    }
}
