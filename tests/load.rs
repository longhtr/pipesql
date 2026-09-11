#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use pipesql::Config;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("pipesql-public-load-{}-{id}", std::process::id()));
        fs::create_dir(&path).expect("create public load directory");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            // Preserve the test failure during unwinding; report cleanup failures
            // when the scenario itself completed successfully.
            if error.kind() != std::io::ErrorKind::NotFound && !std::thread::panicking() {
                panic!("remove test directory {}: {error}", self.0.display());
            }
        }
    }
}

fn config() -> Config {
    Config::new(2_000_000, 1_000_000).expect("load config")
}

const ROW: &[u8] = b"1|2|3|4|17.00|21168.23|0.04|8|R|F|1996-03-13|12|13|14|15|16|\n";

#[path = "load/cli.rs"]
mod cli;
#[path = "load/lifecycle.rs"]
mod lifecycle;
#[path = "load/stack.rs"]
mod stack;
