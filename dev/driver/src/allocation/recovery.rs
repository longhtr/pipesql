//! Check recovery when allocations fail while opening a database with a missing root.
//!
//! Setup creates an empty or populated database, removes ROOT.B, and leaves a
//! scratch file for recovery to clean up. Other cases damage ROOT.A too or deny
//! the rename that would publish a repaired ROOT.B. The parent allocation driver
//! can refuse allocations during open and close, including error construction.
//! Returned errors must explain allocation failure, corrupt roots, or failed repair
//! as appropriate, and the call must release its heap allocations and descriptors.
//!
//! A written ROOT.B.next is only a repair candidate. One case deliberately damages
//! the remaining root after a failed repair to check that open cannot promote the
//! candidate. After restoring the original bytes and permissions, two reopens must
//! retain the rows and commit receipt, remove debris, and permit a new declaration.
//! The allocation campaign runs these cases; its Darwin ACL case is excluded
//! on other platforms.

use super::super::workload::{allocation_cause, arm, finish, format_error};
use pipesql::{
    AppendLimits, CancellationToken, CauseKind, ColumnDeclaration, ColumnInput, ColumnValues,
    CommitResolution, Config, DataType, Database, Error, QueryStep, Value,
};
use std::fs;
use std::path::Path;
use std::sync::atomic::Ordering;

fn descriptors() -> usize {
    fs::read_dir("/dev/fd").unwrap().count()
}

// Used only by the Darwin permission case. Allow writing the candidate file but
// deny removing its directory entry as part of rename, so repair fails late.
fn repair_permission(path: &Path, deny: bool) {
    let mut command = std::process::Command::new("/bin/chmod");
    if deny {
        command.args(["+a", "everyone deny delete_child"]);
    } else {
        command.arg("-N");
    }
    assert!(command.arg(path).status().unwrap().success());
}

pub(crate) fn run(
    root: &Path,
    operation: &str,
    after: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(root)?;
    let path = root.join("database");
    let empty = operation == "catalog-recover-empty";
    let corrupt = operation == "catalog-recover-corrupt";
    let permission = operation == "catalog-recover-permission";
    assert!(empty || corrupt || permission || operation == "catalog-recover-data");
    let config = Config::new(4_000_000, 2_000_000)?;
    let cancel = CancellationToken::new();
    let db = Database::create_empty(&path, config)?;
    let commit = if empty {
        None
    } else {
        db.declare_table(
            "facts",
            &[ColumnDeclaration {
                name: "value",
                data_type: DataType::Int64,
                nullable: false,
            }],
            &cancel,
        )?;
        let mut append = db.begin_append(
            "facts",
            AppendLimits {
                batches: 1,
                encoded_bytes: 100_000,
            },
            &cancel,
        )?;
        append.write(
            &[ColumnInput {
                values: ColumnValues::Int64(&[7, 9]),
                validity: &[3],
            }],
            &cancel,
        )?;
        Some(append.commit(&cancel)?)
    };
    let generation = db.generation();
    db.close()?;
    let a = path.join("ROOT.A");
    let b = path.join("ROOT.B");
    let next = path.join("ROOT.B.next");
    let debris = path.join("private/SCRATCH.A");
    let original = fs::read(&a)?;
    let fence = fs::read(path.join("WAL"))?;
    let control = fs::read(path.join("CONTROL"))?;
    let objects: Vec<_> = fs::read_dir(path.join("units"))?
        .map(|entry| {
            let name = entry.unwrap().path();
            let bytes = fs::read(&name).unwrap();
            (name, bytes)
        })
        .collect();
    fs::remove_file(&b)?;
    fs::write(&debris, [])?;
    let mut damaged = original.clone();
    // Damage the checksum-covered contents without changing version or identity.
    damaged[200] ^= 1;
    if corrupt {
        fs::write(&a, &damaged)?;
        fs::write(&next, b"unpublished root")?;
    }
    if permission {
        repair_permission(&path, true);
    }
    let before_descriptors = descriptors();
    let baseline = arm(after, 128);
    let result = (|| -> Result<(), Error> {
        let db = Database::open(&path, config)?;
        assert_eq!(db.generation(), generation);
        assert_eq!(db.reserved_temp_bytes(), 0);
        db.close()
    })();
    let formatted = format_error(result.as_ref().err());
    finish(operation, baseline);
    if permission {
        repair_permission(&path, false);
    }
    assert!(formatted);
    assert_eq!(
        descriptors(),
        before_descriptors,
        "recovery leaked descriptors"
    );
    let refused = super::super::REFUSED.load(Ordering::Relaxed);
    if refused != 0 {
        assert!(
            source_is_allocation(&result),
            "unexpected allocation refusal: {result:?}"
        );
    } else if permission {
        assert!(
            matches!(&result, Err(Error::RecoveryRequired { generation: observed, source })
            if *observed == generation && matches!(source.kind(),
                CauseKind::Io { operation: "rename repaired root", source }
                if source.kind() == std::io::ErrorKind::PermissionDenied)),
            "{result:?}"
        );
    } else if corrupt {
        assert!(matches!(&result, Err(Error::Corrupt(_))), "{result:?}");
    } else {
        result.as_ref().unwrap();
    }
    if refused != 0 {
        println!("returned catalog recovery allocation refusal");
    } else if permission || corrupt {
        println!("returned expected {operation}");
    } else {
        println!("returned healthy {operation}");
    }
    assert_eq!(
        fs::read(&a)?,
        if corrupt { &damaged } else { &original }.as_slice()
    );
    assert_eq!(fs::read(path.join("WAL"))?, fence);
    assert_eq!(fs::read(path.join("CONTROL"))?, control);
    assert_eq!(fs::read_dir(path.join("units"))?.count(), objects.len());
    for (name, bytes) in &objects {
        assert_eq!(&fs::read(name)?, bytes);
    }
    if corrupt {
        // With neither root usable, recovery must leave both the unpublished
        // candidate and scratch debris alone; it has no authority to clean them.
        assert!(!b.exists());
        assert_eq!(fs::read(&next)?, b"unpublished root");
        assert_eq!(fs::read(&debris)?, b"");
        fs::write(&a, &original)?;
    }
    if permission && refused == 0 {
        // The denied rename left a complete candidate. Make the remaining root
        // unreadable: the candidate must not become a substitute committed root.
        assert!(!b.exists());
        let pending = fs::read(&next)?;
        assert_eq!(pending.len(), 4096);
        fs::write(&a, &damaged)?;
        assert!(matches!(
            Database::open(&path, config),
            Err(Error::Corrupt(_))
        ));
        assert_eq!(fs::read(&next)?, pending);
        assert_eq!(fs::read(&debris)?, b"");
        assert_eq!(fs::read(path.join("WAL"))?, fence);
        fs::write(&a, &original)?;
        println!("catalog repair replacement was not promoted to authority");
    }
    for reopening in 0..2 {
        // The second open checks that the first repair left a stable database,
        // rather than merely producing a handle that works until it closes.
        let db = Database::open(&path, config)?;
        assert_eq!(db.generation(), generation);
        if let Some(commit) = commit {
            assert_eq!(
                db.resolve_commit(commit.transaction())?,
                CommitResolution::Durable(commit)
            );
            let query = db.prepare("FROM facts |> ORDER BY value")?;
            let mut result = db.execute(&query, &cancel)?;
            let mut seen = 0;
            for step in 0..1024 {
                match result.step() {
                    QueryStep::Progress => (),
                    QueryStep::Rows(batch) => {
                        for row in 0..batch.len() {
                            assert!(seen < 2);
                            assert_eq!(batch.value(row, 0), Some(Value::Int64([7, 9][seen])));
                            seen += 1;
                        }
                    }
                    QueryStep::Finished => {
                        assert_eq!(seen, 2);
                        break;
                    }
                    QueryStep::Failed(error) => panic!("healed query: {error:?}"),
                }
                assert!(step < 1023, "healed query exceeded step bound");
            }
        }
        assert_eq!(db.reserved_temp_bytes(), 0);
        if reopening == 1 {
            let written = db.declare_table(
                "healed",
                &[ColumnDeclaration {
                    name: "value",
                    data_type: DataType::Int64,
                    nullable: false,
                }],
                &cancel,
            )?;
            assert_eq!(
                db.resolve_commit(written.transaction())?,
                CommitResolution::Durable(written)
            );
        }
        db.close()?;
        assert!(!next.exists() && !debris.exists());
    }
    println!("catalog recovery healed generation={generation}");
    Ok(())
}

fn source_is_allocation(result: &Result<(), Error>) -> bool {
    match result {
        Err(Error::Resource { .. }) => true,
        Err(Error::Io { source, .. }) => source.kind() == std::io::ErrorKind::OutOfMemory,
        Err(Error::RecoveryRequired { source, .. }) => allocation_cause(source.kind()),
        _ => false,
    }
}
