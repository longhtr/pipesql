//! Stop an import after issuance, a private unit and each root replacement.
//!
//! A child reports its token before page reads and pauses at a named real effect.
//! The parent owns its lifetime through a deadline, kill and exit wait, then
//! reopens and checks every row and the receipt. This is process-interruption
//! evidence with visible writes retained, not a power-loss experiment.

use super::*;
use crate::effects::{Effect, Faults, LoadEffect, MetadataKind};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn pause(marker: &Path) {
    std::fs::write(marker, b"ready").unwrap();
    for _ in 0..2000 {
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("Parquet interruption child exceeded its supervision deadline");
}

#[test]
fn process_interruption_resolves_empty_or_complete_import() {
    const CHILD: &str = "PIPESQL_PARQUET_INTERRUPTION_DIRECTORY";
    if let Some(directory) = std::env::var_os(CHILD) {
        let directory = std::path::PathBuf::from(directory);
        let cut = std::env::var("PIPESQL_PARQUET_INTERRUPTION_CUT")
            .unwrap()
            .parse::<u8>()
            .unwrap();
        let selected = match cut {
            0 => None,
            1 => Some(Effect::SyncMetadata(MetadataKind::CatalogObject)),
            2 => Some(Effect::Load(LoadEffect::RenameRootA)),
            3 => Some(Effect::Load(LoadEffect::RenameRootB)),
            _ => panic!("unknown interruption cut"),
        };
        let db = Database::open(
            &directory.join("db"),
            Config::new(8_000_000, 8_000_000).unwrap(),
        )
        .unwrap();
        let marker = directory.join("ready");
        let mut effects = Effects::with_faults(Faults {
            after_action: Some(Box::new(move |_, effect| {
                if Some(effect) == selected {
                    pause(&marker);
                }
            })),
            ..Faults::default()
        });
        let outcome = db.import_parquet_with_effects(
            "facts",
            Cursor::new(INPUT),
            limits(),
            &CancellationToken::new(),
            |token| {
                std::fs::write(directory.join("transaction"), token.as_bytes())?;
                if cut == 0 {
                    pause(&directory.join("ready"));
                }
                Ok(())
            },
            &mut effects,
        );
        panic!("child passed interruption cut: {outcome:?}");
    }
    for cut in 0..4 {
        let directory = Directory::new();
        database(&directory).close().unwrap();
        let marker = directory.0.join("ready");
        let mut child = crate::test_child::ChildProcess(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    concat!(
                        module_path!(),
                        "::process_interruption_resolves_empty_or_complete_import"
                    )
                    .strip_prefix("pipesql::")
                    .unwrap(),
                    "--nocapture",
                ])
                .env(CHILD, &directory.0)
                .env("PIPESQL_PARQUET_INTERRUPTION_CUT", cut.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut ready = false;
        while Instant::now() < deadline {
            if marker.exists() {
                ready = true;
                break;
            }
            if child.0.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if child.0.try_wait().unwrap().is_none() {
            child.0.kill().unwrap();
        }
        let status = child.0.wait().unwrap();
        drop(child);
        assert!(ready, "Parquet child did not reach cut {cut}");
        assert_eq!(status.signal(), Some(9));
        let token = TransactionId::from_bytes(
            std::fs::read(directory.0.join("transaction"))
                .unwrap()
                .try_into()
                .unwrap(),
        )
        .unwrap();
        let db = Database::open(
            &directory.0.join("db"),
            Config::new(8_000_000, 8_000_000).unwrap(),
        )
        .unwrap();
        assert_eq!(db.reserved_temp_bytes(), 0);
        let outcome = db.resolve_commit(token).unwrap();
        if cut < 2 {
            assert_eq!(outcome, CommitResolution::Aborted);
            check_rows(&db, 0);
            db.import_parquet(
                "facts",
                Cursor::new(INPUT),
                limits(),
                &CancellationToken::new(),
                |_| Ok(()),
            )
            .unwrap();
            check_rows(&db, 600);
            assert_eq!(db.resolve_commit(token).unwrap(), CommitResolution::Aborted);
        } else {
            assert!(
                matches!(outcome, CommitResolution::Durable(commit) if commit.generation() == 2)
            );
            check_rows(&db, 600);
        }
        db.close().unwrap();
    }
}
