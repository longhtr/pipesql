//! Give a test one temporary directory and remove it when the owner drops.
//!
//! Create the directory owner before opening databases or files beneath it, so
//! those handles drop first. The name combines the process ID and a counter;
//! creation fails if that name already exists instead of reusing its contents.
//! The sibling `cleanup` module defines how removal errors affect a test.

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

pub(crate) mod cleanup;

impl Drop for Directory {
    fn drop(&mut self) {
        cleanup::directory(&self.0);
    }
}
