//! Check legacy load publication, token resolution and caller stack boundaries.
//!
//! `lifecycle` checks the public Rust API; `cli` also resolves independently
//! encoded persisted history. `stack` crosses empty, single-row and block-boundary
//! inputs in supervised child processes. Run with `cargo test --release --test load`
//! to use the optimized code required by its observed stack checks.

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
