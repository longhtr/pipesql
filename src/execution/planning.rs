//! Physical producer pipelines over immutable bound relations.
//!
//! Lowering assigns producer positions; validation independently maps positions
//! back to semantic identities. Both consume the shared demand analysis.

mod demand;
mod lower;
mod validate;

pub(super) use lower::lower;
pub(super) use validate::validate_physical;

use super::predicate::PhysicalFilter;
use crate::frontend::{
    self, ColumnId, Computed, Direction, MAX_COLUMNS, MAX_ORDER_ITEMS, MAX_QUERY_COLUMNS,
    MAX_STAGES, NullPlacement, PreparedQuery, RelationId, SemanticColumn, Stage,
};
use crate::resources::Reservation;
use crate::storage_format::RootState;
use crate::{Database, DatabaseId, Error};
use std::mem::size_of;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Producer {
    Scan(u8),
    Distinct {
        input: PipelineId,
        descriptor: u8,
    },
    Limit {
        input: PipelineId,
        bounds: frontend::LimitBounds,
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
        left: PipelineId,
        right: PipelineId,
        left_key: u8,
        right_key: u8,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Pipeline<'query> {
    pub(super) producer: Producer,
    relation: RelationId,
    end: RelationId,
    pub(super) filters: [PhysicalFilter<'query>; MAX_STAGES],
    pub(super) filter_count: usize,
    pub(super) columns: [u8; MAX_COLUMNS],
    pub(super) column_count: usize,
    identities: [ColumnId; MAX_COLUMNS],
    pub(super) computed: &'query [Computed],
    // Indexed by semantic ColumnId; u8::MAX means the identity is not demanded.
    // Raw producer positions are below MAX_COLUMNS; higher slots name shared
    // definitions. Materialized definitions map to raw input positions instead.
    pub(super) slots: [u8; MAX_QUERY_COLUMNS + 1],
}

impl Pipeline<'_> {
    pub(super) fn output_columns<'a>(
        &'a self,
        plan: &'a frontend::Plan,
    ) -> impl Iterator<Item = SemanticColumn> + Clone + 'a {
        self.identities[..self.column_count].iter().map(|id| {
            let (kind, nullable) = plan
                .column_type(*id)
                .expect("validated physical output type");
            SemanticColumn::new(id.value(), kind, nullable)
        })
    }

    fn position(&self, identity: ColumnId) -> Option<usize> {
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
    // Drop the vector before releasing its physical allocation charge.
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
            .filter(|node| {
                matches!(
                    node.stage,
                    Stage::Source(_)
                        | Stage::Aggregate(_)
                        | Stage::Distinct(_)
                        | Stage::Join { .. }
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

fn pipeline_end(plan: &frontend::Plan, mut relation: RelationId) -> RelationId {
    // Bound validation has established one consumer per producer. Only transparent
    // unary stages fuse; crossing an aggregate or join changes the row producer.
    for (index, node) in plan.nodes().iter().enumerate() {
        if node.input == relation
            && matches!(
                node.stage,
                Stage::Alias | Stage::Derived | Stage::Select { .. } | Stage::Where(_)
            )
        {
            relation = RelationId(index as u8 + 1);
        }
    }
    relation
}

#[cfg(test)]
mod tests;
