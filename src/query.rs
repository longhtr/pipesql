//! Prepare a query and retain the committed data it will read.
//!
//! Preparation parses SQL, resolves sources, binds names and validates the plan.
//! Catalog reads use a snapshot pin; transient read buffers are released before
//! binding. Any failure releases the unfinished owners. A successful query keeps
//! its plan, memory reservation and pin until its last execution lets go.
//!
//! Binding consumes source facts without opening files. Execution separately maps
//! validated column identities to batches and owns the work of producing results.

mod binding;
mod column_set;
mod distinct;
mod explain;
mod join;
mod lexer;
mod parser;
mod plan;
pub(crate) mod scalar;
mod set_operation;
pub(crate) mod sources;
pub(crate) mod string_length;
pub(crate) mod text_literal;
mod validation;

use crate::resources::Reservation;
use crate::value::{DataType, DateValue};
use crate::{Database, Error, SourceSpan};
pub(crate) use column_set::ColumnSet;
use distinct::DistinctPlan;
pub use explain::LogicalPlan;
pub(crate) use join::NullExtension;
pub use plan::ResultColumn;
use plan::*;
pub(crate) use plan::{
    AggregateArgument, AggregateEntry, AggregateKind, AggregatePlan, ColumnId, Comparison,
    Computation, Computed, Constant, Direction, Filter, FilterControl, FilterLiteral, JoinKind,
    LimitBounds, MAX_AGGREGATE_COLUMNS, MAX_COLUMNS, MAX_COMPUTED, MAX_ORDER_ITEMS,
    MAX_PARTITION_KEYS, MAX_QUERY_COLUMNS, MAX_ROW_VALUES, MAX_STAGES, MAX_WINDOW_ORDER_KEYS, Node,
    NullPlacement, OrderKey, OwnedPlan, PartitionKeys, Plan, Predicate, RelationColumns,
    RelationId, RunningSum, SemanticColumn, SourceColumn, SourceOccurrence, Stage, WindowOrderKey,
};
use scalar::{Expression, Op};
pub(crate) use set_operation::{SetKind, SetPlan};
use std::mem::size_of;
pub(crate) use validation::validate;

pub(crate) fn prepare<'db>(
    database: &'db Database,
    source: &str,
) -> Result<PreparedQuery<'db>, Error> {
    let parsed = parser::parse_query(source)?;
    let sources = sources::legacy(source, &parsed)?;
    binding::bind_plan(
        &database.memory,
        database.database_identity(),
        source,
        &parsed,
        sources,
        database.catalog_generation(),
    )
}

pub(crate) fn prepare_catalog<'db>(
    database: &'db Database,
    source: &str,
) -> Result<PreparedQuery<'db>, Error> {
    let parsed = parser::parse_query(source)?;
    let snapshot = database.catalog_snapshot()?;
    let generation = snapshot.generation();
    let sources = sources::catalog(database, &snapshot, source, &parsed)?;
    let mut query = binding::bind_plan(
        &database.memory,
        database.database_identity(),
        source,
        &parsed,
        sources,
        generation,
    )?;
    query.snapshot = Some(snapshot);
    Ok(query)
}

/// A validated query that borrows its database and retains its input snapshot.
///
/// Declared-table queries keep the generation selected during preparation, even
/// after later appends. A running result borrows this query, so Rust requires the
/// result to be dropped first. Dropping the query frees its plan and releases its
/// snapshot pin and memory reservation.
pub struct PreparedQuery<'database> {
    pub(crate) plan: OwnedPlan,
    pub(crate) snapshot: Option<crate::storage::snapshot::Snapshot<'database>>,
    reservation: Reservation<'database>,
}

impl PreparedQuery<'_> {
    /// Borrow a diagnostic view of the validated logical plan.
    ///
    /// Creating and formatting the view performs no engine allocation, I/O or
    /// locking and acquires no additional snapshot pin. Formatting writes to the
    /// caller's sink, which may allocate or fail. The view cannot outlive this
    /// prepared query. Its text describes semantic relationships, not physical
    /// scheduling, costs or a stable serialization format.
    pub fn logical_plan(&self) -> LogicalPlan<'_> {
        LogicalPlan::new(&self.plan)
    }

    /// Number of columns in the query's output schema, including repeated outputs.
    pub fn result_column_count(&self) -> usize {
        usize::from(self.plan.output_count)
    }

    /// Inspect a zero-based output position, or return `None` outside the schema.
    pub fn result_column(&self, index: usize) -> Option<ResultColumn<'_>> {
        let output = self.plan.outputs[..usize::from(self.plan.output_count)].get(index)?;
        let (data_type, nullable) = self.plan.column_type(output.id)?;
        Some(ResultColumn {
            name: (output.name != Name::EMPTY).then(|| output.name.as_str()),
            data_type,
            nullable,
        })
    }

    /// The retained plan's logical memory charge, excluding running-query owners.
    pub fn accounted_memory_bytes(&self) -> u64 {
        self.reservation.bytes()
    }

    /// A conservative retained-plan ceiling in bytes, not an exact query minimum.
    /// Preparation also needs transient catalog and name-scope storage. This value
    /// excludes those temporary peaks and the database's resident charge.
    pub fn memory_requirement_bytes() -> u64 {
        (size_of::<Self>()
            + size_of::<Plan>()
            + MAX_AGGREGATE_COLUMNS * (size_of::<AggregateEntry>() + size_of::<AggregatePlan>())
            + Computed::allocation_capacity(MAX_COMPUTED).expect("bounded computed capacity")
                * size_of::<Computed>()
            + MAX_STAGES * size_of::<DistinctPlan>()
            + MAX_STAGES * size_of::<SetPlan>()
            + MAX_STAGES * size_of::<NullExtension>()
            + PREPARED_ALLOCATION_ALLOWANCE
            + (MAX_AGGREGATE_COLUMNS + 5) * PREPARED_ALLOCATION_ALLOWANCE) as u64
    }
}

fn span(start: usize, end: usize) -> SourceSpan {
    SourceSpan {
        start: u16::try_from(start).expect("source admission bounds span"),
        end: u16::try_from(end).expect("source admission bounds span"),
    }
}

fn parse(message: &'static str, start: usize, end: usize) -> Error {
    Error::Parse {
        message,
        span: span(start, end),
    }
}

fn bind_error(message: &'static str, span: SourceSpan) -> Error {
    Error::Bind { message, span }
}

fn text(source: &str, span: SourceSpan) -> &str {
    &source[span.start()..span.end()]
}
