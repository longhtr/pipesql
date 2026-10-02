//! Describe where each operator gets its input values and puts its output values.
//!
//! The prepared query identifies columns by meaning; execution reads them by
//! position. A projection can change a column's position without changing its
//! identity. The physical plan connects the two so an operator reads the intended
//! value even after columns have been reordered or unused columns omitted.
//!
//! A `Pipeline` combines one row producer with its filters and projections.
//! A scan and a following SELECT can run together this way. An aggregate starts
//! a new pipeline because it produces new rows from its input. Pipelines refer
//! to earlier pipelines so the runtime knows where to request input.
//! Each source occurrence owns a separate scan, even when it reads the same
//! table: sharing a mutable cursor would let one consumer steal another's rows.
//! Planning selects the implemented strategies; it has no cost optimizer.
//!
//! `lower` constructs these mappings; `validate` decodes them separately and
//! compares them with the prepared query before table values are read. Both use
//! the demand analysis in `demand` to find needed columns. Their agreement cannot
//! establish that this shared analysis is correct; independent SQL tests must
//! check which values the query actually needs.

mod demand;
mod lower;
mod validate;

pub(super) use lower::lower;
pub(super) use validate::validate_physical;

use super::predicate::PhysicalFilter;
use crate::query::{
    self, ColumnId, Computed, Direction, MAX_ORDER_ITEMS, MAX_QUERY_COLUMNS, MAX_ROW_VALUES,
    MAX_STAGES, NullPlacement, PreparedQuery, RelationId, SemanticColumn, Stage,
};
use crate::resources::Reservation;
use crate::storage::format::RootState;
use crate::{Database, DatabaseId, Error};
use std::mem::size_of;

// Declared queries may retain an older committed snapshot. Legacy queries have
// no such retained snapshot, so their generation must still be the current one.
pub(super) fn snapshot_matches(query: &PreparedQuery<'_>, database: &Database) -> bool {
    if query.plan.table().is_some() != query.snapshot.is_some() {
        return false;
    }
    if let Some(snapshot) = &query.snapshot {
        return snapshot.belongs_to(database) && snapshot.generation() == query.plan.generation;
    }
    query.plan.generation == database.generation()
}

pub(super) const MAX_PIPELINES: usize = MAX_STAGES + 1;
const ALLOCATION_ALLOWANCE: u64 = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PipelineId(u8);

impl PipelineId {
    pub(super) fn index(self) -> usize {
        usize::from(self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OrderColumn {
    pub(super) column: u8,
    pub(super) direction: Direction,
    pub(super) nulls: NullPlacement,
}

impl OrderColumn {
    const EMPTY: Self = Self {
        column: 0,
        direction: Direction::Ascending,
        nulls: NullPlacement::First,
    };
}

/// Positions refer to the analytic producer's validated input pipeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WindowFunction {
    Count,
    RunningSum {
        argument: u8,
        order: [Option<OrderColumn>; query::MAX_WINDOW_ORDER_KEYS],
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Producer {
    Scan(u8),
    Analytic {
        input: PipelineId,
        partition: [Option<u8>; query::MAX_PARTITION_KEYS],
        function: WindowFunction,
    },
    SetOperation {
        left: PipelineId,
        right: PipelineId,
        descriptor: u8,
    },
    Distinct {
        input: PipelineId,
        descriptor: u8,
    },
    Limit {
        input: PipelineId,
        bounds: query::LimitBounds,
    },
    Order {
        input: PipelineId,
        start: u8,
        len: u8,
    },
    Aggregate {
        aggregate: u8,
        input: PipelineId,
        demand: u16,
    },
    Join {
        kind: query::JoinKind,
        left: PipelineId,
        right: PipelineId,
        left_key: u8,
        right_key: u8,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Pipeline<'query> {
    pub(super) producer: Producer,
    // The producer's logical relation and the last relation executed with it.
    relation: RelationId,
    end: RelationId,
    pub(super) filters: [PhysicalFilter<'query>; MAX_STAGES],
    pub(super) filter_count: usize,
    // For each output position, the slot supplying its value. `identities`
    // records the matching logical column, including repeated final outputs.
    pub(super) columns: [u8; MAX_ROW_VALUES],
    pub(super) column_count: usize,
    identities: [ColumnId; MAX_ROW_VALUES],
    pub(super) computed: &'query [Computed],
    // Map logical ColumnId to a value slot; u8::MAX means there is no mapping.
    // Slots below MAX_ROW_VALUES read values supplied by the producer. Higher slots
    // index `computed` after subtracting MAX_ROW_VALUES. An expression completed
    // by an earlier producer maps to an input slot, so it is not evaluated again.
    pub(super) slots: [u8; MAX_QUERY_COLUMNS + 1],
}

impl Pipeline<'_> {
    pub(super) fn output_columns<'a>(
        &'a self,
        plan: &'a query::Plan,
    ) -> impl Iterator<Item = SemanticColumn> + Clone + 'a {
        self.identities[..self.column_count].iter().map(|id| {
            let (kind, nullable) = plan
                .column_type(*id)
                .expect("validated physical output type");
            SemanticColumn::new(id.value(), kind, nullable)
        })
    }

    pub(super) fn position(&self, identity: ColumnId) -> Option<usize> {
        self.identities[..self.column_count]
            .iter()
            .position(|id| *id == identity)
    }

    pub(super) fn source(&self) -> Result<u8, Error> {
        match self.producer {
            Producer::Scan(source) => Ok(source),
            _ => Err(Error::Corrupt("pipeline is not a scan")),
        }
    }
}

pub(super) struct PhysicalPlan<'db> {
    pub(super) database: DatabaseId,
    pub(super) generation: u64,
    pub(super) root: RootState,
    pub(super) projected_crc32c: u32,
    pipelines: Vec<Pipeline<'db>>,
    order_columns: [OrderColumn; MAX_ORDER_ITEMS],
    // Rust drops fields in order. Free the pipeline vector before returning
    // its reserved bytes to the database budget.
    reservation: Option<Reservation<'db>>,
}

impl<'db> PhysicalPlan<'db> {
    pub(super) const fn maximum_bytes() -> u64 {
        (MAX_PIPELINES * size_of::<Pipeline>()) as u64 + ALLOCATION_ALLOWANCE
    }

    pub(super) fn required_bytes(query: &PreparedQuery<'_>) -> u64 {
        (Self::pipeline_count(query) * size_of::<Pipeline>()) as u64 + ALLOCATION_ALLOWANCE
    }

    fn pipeline_count(query: &PreparedQuery<'_>) -> usize {
        1 + query
            .plan
            .nodes()
            .iter()
            .enumerate()
            .filter(|(index, node)| {
                query.plan.analytic_projection(RelationId(*index as u8 + 1))
                    || matches!(
                        node.stage,
                        Stage::Source(_)
                            | Stage::Aggregate(_)
                            | Stage::Distinct(_)
                            | Stage::Join { .. }
                            | Stage::SetOperation { .. }
                            | Stage::Order { .. }
                            | Stage::Limit(_)
                    )
            })
            .count()
    }

    pub(super) fn row_scratch_bytes(&self) -> u64 {
        if self.pipelines.iter().any(|pipeline| {
            !matches!(pipeline.producer, Producer::Scan(_)) && pipeline.has_computed_work()
        }) {
            super::computed::ROW_SCRATCH_BYTES
        } else {
            0
        }
    }

    pub(super) fn memory_bytes(&self) -> u64 {
        self.reservation.as_ref().map_or(0, |owner| owner.bytes())
    }

    pub(super) fn clear(&mut self) {
        drop(std::mem::take(&mut self.pipelines));
        self.reservation = None;
    }

    pub(super) fn order_columns(&self, start: u8, len: u8) -> &[OrderColumn] {
        &self.order_columns[usize::from(start)..usize::from(start) + usize::from(len)]
    }

    pub(super) fn scan(&self) -> &Pipeline<'db> {
        &self.pipelines()[0]
    }

    pub(super) fn pipelines(&self) -> &[Pipeline<'db>] {
        &self.pipelines
    }

    #[cfg(test)]
    pub(super) fn output(&self) -> &Pipeline<'db> {
        self.pipelines.last().expect("admitted physical output")
    }

    pub(super) fn aggregate_pipeline(&self) -> Option<&Pipeline<'db>> {
        self.pipelines()
            .iter()
            .find(|pipeline| matches!(pipeline.producer, Producer::Aggregate { .. }))
    }

    #[cfg(test)]
    pub(super) fn aggregate_columns(&self) -> Option<usize> {
        matches!(self.output().producer, Producer::Aggregate { .. })
            .then_some(self.output().column_count)
    }

    #[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
    pub(super) fn scan_mut(&mut self) -> &mut Pipeline<'db> {
        &mut self.pipelines[0]
    }

    #[cfg(test)]
    pub(super) fn output_mut(&mut self) -> &mut Pipeline<'db> {
        self.pipelines.last_mut().expect("physical output")
    }
}

fn pipeline_end(plan: &query::Plan, mut relation: RelationId) -> RelationId {
    // The validated query gives each producer one consumer. Follow stages that
    // can use the same row producer; stop before an operator such as aggregation
    // or joining needs to produce a different set of rows.
    // LIMIT also ends a pipeline: moving a later predicate before the limit
    // would select different rows. DISTINCT must compare the complete input
    // before applying any downstream predicate or projection.
    for (index, node) in plan.nodes().iter().enumerate() {
        if node.input == relation
            && !plan.analytic_projection(RelationId(index as u8 + 1))
            && matches!(
                node.stage,
                Stage::Alias
                    | Stage::Rename
                    | Stage::Set { .. }
                    | Stage::Drop { .. }
                    | Stage::Derived
                    | Stage::Select { .. }
                    | Stage::Extend { .. }
                    | Stage::Where(_)
            )
        {
            relation = RelationId(index as u8 + 1);
        }
    }
    relation
}

#[cfg(test)]
mod tests;
