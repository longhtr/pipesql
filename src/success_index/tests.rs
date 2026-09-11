use super::*;
use crate::effects::Faults;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn database() -> DatabaseId {
    DatabaseId::new([7; 16]).unwrap()
}

fn object(sequence: u64, ordinal: u32) -> ObjectId {
    ObjectId::new(sequence, ordinal).unwrap()
}

fn token(sequence: u64) -> TransactionId {
    TransactionId::for_attempt(database(), sequence).unwrap()
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pipesql-success-index-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, id: ObjectId) -> PathBuf {
        self.0.join(std::str::from_utf8(&id.name()).unwrap())
    }

    fn create(&self, id: ObjectId) -> File {
        pipesql_filesystem::create_new_read_write(self.path(id)).unwrap()
    }
    // Trusted stored-history fixture, not a second production history writer.
    fn history(&self, count: u64, last: u64) -> WalRecord {
        let mut bytes = vec![0; 64 + count as usize * 8];
        bytes[..8].copy_from_slice(MAGIC);
        put_u32(&mut bytes, 8, 6);
        bytes[16..32].copy_from_slice(database().as_bytes());
        put_u64(&mut bytes, 32, count);
        put_u64(&mut bytes, 40, last);
        put_u32(&mut bytes, 48, 5);
        put_u64(&mut bytes, 56, last);
        for index in 0..count {
            put_u64(
                &mut bytes,
                64 + index as usize * 8,
                last - 2 * (count - 1 - index),
            );
        }
        self.install(count, last, bytes)
    }

    fn install(&self, count: u64, last: u64, bytes: Vec<u8>) -> WalRecord {
        let reference =
            ObjectRef::new(object(last, 5), bytes.len() as u32, crc32c(&bytes)).unwrap();
        std::fs::write(self.path(reference.object()), bytes).unwrap();
        let catalog = ObjectRef::new(object(last, 4), 64, 0).unwrap();
        let commit =
            CatalogCommit::new(database(), count, token(last), catalog, reference).unwrap();
        WalRecord {
            database: database(),
            issued: last,
            state: RootState::Catalog(Some(commit)),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn copied_history_crosses_blocks_and_preserves_final_sequence() {
    let fixture = Fixture::new();
    let old = fixture.history(8193, u64::MAX - 2);
    let prior = WalRecord {
        issued: u64::MAX,
        ..old
    };
    let mut buffer = vec![0; SCRATCH_BYTES];
    let cancel = CancellationToken::new();
    let file = fixture.create(object(u64::MAX, 5));
    let reference = append(
        &file,
        &fixture.0,
        prior,
        object(u64::MAX, 5),
        &mut buffer,
        &cancel,
        &mut Effects::default(),
    )
    .unwrap();
    let previous = committed(old).unwrap().unwrap();
    let next = CatalogCommit::new(
        database(),
        8194,
        token(u64::MAX),
        previous.catalog(),
        reference,
    )
    .unwrap();
    let snapshot = WalRecord {
        state: RootState::Catalog(Some(next)),
        ..prior
    };
    let old_bytes = std::fs::read(fixture.path(previous.successes().object())).unwrap();
    let new_bytes = std::fs::read(fixture.path(reference.object())).unwrap();
    assert_eq!(&new_bytes[64..old_bytes.len()], &old_bytes[64..]);
    for (sequence, generation) in [
        (u64::MAX - 2 - 2 * 8192, 1),
        (u64::MAX - 2, 8193),
        (u64::MAX, 8194),
    ] {
        assert_eq!(
            find(
                &fixture.0,
                snapshot,
                token(sequence),
                &mut buffer,
                &cancel,
                &mut Effects::default()
            )
            .unwrap(),
            Some(generation)
        );
    }
    assert_eq!(
        find(
            &fixture.0,
            snapshot,
            token(u64::MAX - 1),
            &mut buffer,
            &cancel,
            &mut Effects::default()
        )
        .unwrap(),
        None
    );
    let mut corrupt = old_bytes;
    corrupt[64] ^= 1;
    std::fs::write(fixture.path(previous.successes().object()), corrupt).unwrap();
    assert!(
        find(
            &fixture.0,
            old,
            token(u64::MAX - 2),
            &mut buffer,
            &cancel,
            &mut Effects::default()
        )
        .is_err()
    );
    let retry = fixture.create(object(u64::MAX, 6));
    assert!(
        append(
            &retry,
            &fixture.0,
            prior,
            object(u64::MAX, 6),
            &mut buffer,
            &cancel,
            &mut Effects::default()
        )
        .is_err()
    );
    assert!(retry.metadata().unwrap().len() < u64::from(reference.bytes()));
}

#[test]
fn malformed_history_and_full_capacity_never_publish_a_reference() {
    let fixture = Fixture::new();
    let old = fixture.history(2, 5);
    let original = std::fs::read(fixture.path(object(5, 5))).unwrap();
    let mut buffer = vec![0; SCRATCH_BYTES];
    let cancel = CancellationToken::new();
    for (at, value) in [
        (12, 1),
        (16, 8),
        (32, 1),
        (40, 4),
        (48, 4),
        (52, 1),
        (56, 4),
        (64, 0),
        (64, 5),
        (72, 6),
    ] {
        let mut bad = original.clone();
        assert_ne!(bad[at], value);
        bad[at] = value;
        let snapshot = fixture.install(2, 5, bad);
        assert!(
            find(
                &fixture.0,
                snapshot,
                token(3),
                &mut buffer,
                &cancel,
                &mut Effects::default()
            )
            .is_err(),
            "field {at}"
        );
    }
    let full = CatalogCommit::new(
        database(),
        MAX_SUCCESSES,
        token(MAX_SUCCESSES),
        ObjectRef::new(object(MAX_SUCCESSES, 4), 64, 0).unwrap(),
        ObjectRef::new(object(MAX_SUCCESSES, 5), (64 + 8 * MAX_SUCCESSES) as u32, 0).unwrap(),
    )
    .unwrap();
    let prior = WalRecord {
        database: database(),
        issued: MAX_SUCCESSES + 1,
        state: RootState::Catalog(Some(full)),
    };
    let file = fixture.create(object(MAX_SUCCESSES + 1, 5));
    let mut effects = Effects::default();
    assert!(matches!(
        append(
            &file,
            &fixture.0,
            prior,
            object(MAX_SUCCESSES + 1, 5),
            &mut [],
            &cancel,
            &mut effects
        ),
        Err(Error::Resource {
            owner: "retained success count",
            ..
        })
    ));
    assert_eq!(effects.count(), 0);
    assert_eq!(file.metadata().unwrap().len(), 0);
    assert!(
        CatalogCommit::new(
            database(),
            MAX_SUCCESSES + 1,
            token(MAX_SUCCESSES + 1),
            full.catalog(),
            full.successes()
        )
        .is_err()
    );
    let _ = old;
}

#[test]
fn append_and_lookup_io_failures_do_not_escape_as_membership() {
    let control = Fixture::new();
    let old = control.history(2, 5);
    let prior = WalRecord { issued: 7, ..old };
    let mut buffer = vec![0; SCRATCH_BYTES];
    let cancel = CancellationToken::new();
    let mut baseline = Effects::default();
    append(
        &control.create(object(7, 5)),
        &control.0,
        prior,
        object(7, 5),
        &mut buffer,
        &cancel,
        &mut baseline,
    )
    .unwrap();
    assert!(baseline.count() < 32);
    for cut in 0..baseline.count() {
        let fixture = Fixture::new();
        let old = fixture.history(2, 5);
        let prior = WalRecord { issued: 7, ..old };
        let file = fixture.create(object(7, 5));
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(cut),
            ..Faults::default()
        });
        assert!(matches!(
            append(
                &file,
                &fixture.0,
                prior,
                object(7, 5),
                &mut buffer,
                &cancel,
                &mut effects
            ),
            Err(Error::Io { .. })
        ));
        assert_eq!(effects.count(), cut + 1);
    }
    let mut baseline = Effects::default();
    assert_eq!(
        find(
            &control.0,
            old,
            token(3),
            &mut buffer,
            &cancel,
            &mut baseline
        )
        .unwrap(),
        Some(1)
    );
    for cut in 0..baseline.count() {
        let mut effects = Effects::with_faults(Faults {
            fail_at: Some(cut),
            ..Faults::default()
        });
        assert!(matches!(
            find(
                &control.0,
                old,
                token(3),
                &mut buffer,
                &cancel,
                &mut effects
            ),
            Err(Error::Io { .. })
        ));
    }
    cancel.cancel();
    let mut effects = Effects::default();
    assert!(matches!(
        find(
            &control.0,
            old,
            token(3),
            &mut buffer,
            &cancel,
            &mut effects
        ),
        Err(Error::Cancelled)
    ));
    assert_eq!(effects.count(), 0);
}
