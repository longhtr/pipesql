//! Public test directory ownership. SQL, schemas, and expectations stay in each suite.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Directory(pub(crate) PathBuf);

impl Directory {
    pub(crate) fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("pipesql-public-{}-{id}", std::process::id()));
        std::fs::create_dir(&path).expect("create public test directory");
        Self(path)
    }
}

mod cleanup;

impl Drop for Directory {
    fn drop(&mut self) {
        cleanup::directory(&self.0);
    }
}
