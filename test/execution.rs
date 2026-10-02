//! Test public queries over the fixed legacy lineitem schema.
//!
//! `queries` checks composed SQL results and input-key coverage. `ownership`
//! checks result and plan lifetimes, plus errors retained after their source is
//! dropped. `cli` checks printed results and command failures. `stack` runs the
//! public API on ordinary and small native stacks.
//!
//! The modules share literal input and SQL fixtures here; each owns its expected
//! results. Run `cargo test --release --test execution` because the stack cases
//! require optimized code. Declared-table tests live in `public`.

#![cfg(any(target_os = "macos", target_os = "linux"))]

use pipesql::Config;

const Q6: &str = include_str!("data/q6.pipe.sql");
const Q1: &str = include_str!("data/upstream/q1-upstream.pipe.sql");
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
