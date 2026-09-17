//! Test the public legacy loader, from input rows to durable commit receipts.
//!
//! `lifecycle` follows a Rust API load through close, reopen and resolution.
//! `cli` repeats that flow across processes and checks independently encoded
//! transaction history. `stack` runs loading and querying on ordinary and small
//! stacks, with a parent process checking deadlines and completion.
//!
//! Run `cargo test --release --test load`; the stack cases require optimized code.
//! These tests use the fixed lineitem schema. Declared-table append tests live in
//! `public`.

#![cfg(any(target_os = "macos", target_os = "linux"))]

use pipesql::Config;

#[path = "support/child.rs"]
mod child;
mod support;
use support::Directory as TempDir;

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
