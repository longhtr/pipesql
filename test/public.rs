//! Exercise declared-table behavior through the public API and complete SQL results.
//!
//! Cases own expected answers; support code builds inputs and collects borrowed
//! rows into owned values. Run one behavior with `cargo test --release --test
//! public window_sum::`, or omit the filter to run this target.
//!
//! Each child module groups a public behavior, from schema and append through joins,
//! windows and typed transfer. Common row collection requires query completion so
//! an expected prefix cannot hide a later error. Legacy fixed-schema query and load
//! cases have their own integration targets in execution.rs and load.rs.

#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "public/aggregates.rs"]
mod aggregates;

#[path = "public/import.rs"]
mod import;

#[path = "public/append.rs"]
mod append;

#[path = "public/boolean.rs"]
mod boolean;

#[path = "public/byte_length.rs"]
mod byte_length;

#[path = "public/char_length.rs"]
mod char_length;

#[path = "public/date_year.rs"]
mod date_year;

#[path = "public/case.rs"]
mod case;

#[path = "public/cast.rs"]
mod cast;

#[path = "public/computed.rs"]
mod computed;

#[path = "public/constant_projection.rs"]
mod constant_projection;

#[path = "public/distinct.rs"]
mod distinct;

#[path = "public/except.rs"]
mod except;
#[path = "public/intersect.rs"]
mod intersect;

#[path = "public/grouping.rs"]
mod grouping;

#[path = "public/join_corpus.rs"]
mod join_corpus;

#[path = "public/joins.rs"]
mod joins;

#[path = "public/limit.rs"]
mod limit;

#[path = "public/logical_plan.rs"]
mod logical_plan;

#[path = "public/membership.rs"]
mod membership;

#[path = "public/multiset.rs"]
mod multiset;

#[path = "public/null_predicate.rs"]
mod null_predicate;

#[path = "public/null_safe.rs"]
mod null_safe;

#[path = "public/nullif.rs"]
mod nullif;

#[path = "public/order.rs"]
mod order;

#[path = "public/snapshots.rs"]
mod snapshots;

#[path = "public/schema.rs"]
mod schema;

#[path = "public/spooling.rs"]
mod spooling;

#[path = "public/text_filter.rs"]
mod text_filter;

#[path = "public/union.rs"]
mod union;

#[path = "public/window_count.rs"]
mod window_count;
#[path = "public/window_sum.rs"]
mod window_sum;

#[path = "public/wide.rs"]
mod wide;

use pipesql::{
    AppendLimits, CancellationToken, ColumnDeclaration, ColumnInput, ColumnValues,
    CommitResolution, Config, DataType, Database, DateValue, Error, QueryResult, QueryStep, Value,
};

#[path = "support/mod.rs"]
mod resources;
use resources::Directory;

#[path = "public/support.rs"]
mod support;
use support::*;
