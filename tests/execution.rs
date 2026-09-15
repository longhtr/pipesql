//! Exercise queries over the legacy lineitem format through public interfaces.
//!
//! The child modules separate typed results and ownership, input/key coverage,
//! stock CLI behavior and observed stack limits. Literal rows and retained SQL
//! provide their premises; general declared-table behavior lives in
//! `catalog_lifecycle`. Run this suite with `cargo test --release --test execution`
//! because its stack checks depend on optimized code.

#![cfg(any(target_os = "macos", target_os = "linux"))]

use pipesql::Config;

const Q6: &str = include_str!("fixtures/q6.pipe.sql");
const Q1: &str = include_str!("fixtures/upstream/q1-upstream.pipe.sql");
const ROW: &[u8] = b"1|2|3|4|1|100|0.08|8|R|F|1994-01-01|12|13|14|15|16|\n";
mod support;
use support::Directory as TempDir;

fn config() -> Config {
    Config::new(2_000_000, 1_000_000).expect("config")
}

#[path = "execution/cli.rs"]
mod cli;
#[path = "execution/ownership.rs"]
mod ownership;
#[path = "execution/queries.rs"]
mod queries;
#[path = "execution/stack.rs"]
mod stack;
