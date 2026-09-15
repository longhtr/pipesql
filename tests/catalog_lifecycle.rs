//! Exercise declared databases through the public Rust API and SQL frontend.
//!
//! Capability modules own literal answers, independent row models and rejection
//! cases. `fixtures` shares only input construction, typed result capture and
//! completion/release checks. Private phase injection and native failure campaigns
//! remain separate. Run with `cargo test --release --test catalog_lifecycle`;
//! `tests/README.md` maps capabilities and the verification guide selects scope.

#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "catalog_lifecycle/aggregates.rs"]
mod aggregates;

#[path = "catalog_lifecycle/append.rs"]
mod append;

#[path = "catalog_lifecycle/boolean.rs"]
mod boolean;

#[path = "catalog_lifecycle/byte_length.rs"]
mod byte_length;

#[path = "catalog_lifecycle/char_length.rs"]
mod char_length;

#[path = "catalog_lifecycle/date_year.rs"]
mod date_year;

#[path = "catalog_lifecycle/cast.rs"]
mod cast;

#[path = "catalog_lifecycle/computed.rs"]
mod computed;

#[path = "catalog_lifecycle/constant_projection.rs"]
mod constant_projection;

#[path = "catalog_lifecycle/distinct.rs"]
mod distinct;

#[path = "catalog_lifecycle/except.rs"]
mod except;
#[path = "catalog_lifecycle/intersect.rs"]
mod intersect;

#[path = "catalog_lifecycle/grouping.rs"]
mod grouping;

#[path = "catalog_lifecycle/join_corpus.rs"]
mod join_corpus;

#[path = "catalog_lifecycle/joins.rs"]
mod joins;

#[path = "catalog_lifecycle/limit.rs"]
mod limit;

#[path = "catalog_lifecycle/logical_plan.rs"]
mod logical_plan;

#[path = "catalog_lifecycle/membership.rs"]
mod membership;

#[path = "catalog_lifecycle/multiset.rs"]
mod multiset;

#[path = "catalog_lifecycle/null_predicate.rs"]
mod null_predicate;

#[path = "catalog_lifecycle/null_safe.rs"]
mod null_safe;

#[path = "catalog_lifecycle/nullif.rs"]
mod nullif;

#[path = "catalog_lifecycle/order.rs"]
mod order;

#[path = "catalog_lifecycle/snapshots.rs"]
mod snapshots;

#[path = "catalog_lifecycle/schema.rs"]
mod schema;

#[path = "catalog_lifecycle/spooling.rs"]
mod spooling;

#[path = "catalog_lifecycle/text_filter.rs"]
mod text_filter;

#[path = "catalog_lifecycle/union.rs"]
mod union;

#[path = "catalog_lifecycle/window_count.rs"]
mod window_count;

#[path = "catalog_lifecycle/wide.rs"]
mod wide;

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues,
    CommitResolution, Config, DataType, Database, DateValue, Error, QueryResult, QueryStep, Value,
};

mod support;
use support::Directory;

#[path = "catalog_lifecycle/fixtures.rs"]
mod fixtures;
use fixtures::*;
