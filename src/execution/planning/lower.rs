//! Build the physical plan from a validated prepared query.
//!
//! First work backward to find required columns, then visit relations with inputs
//! before consumers. Start a pipeline for each row-producing operation and attach
//! the filters and projections that run with it. Map each logical value to either
//! an input position or a computed expression in that pipeline.
//!
//! Intermediate batches retain only needed values, including hidden sort or join
//! keys. The final batch preserves the caller's output order and repeated columns.
//! The plan borrows expressions from the prepared query and owns its pipeline
//! allocation and reservation. Construction reads no source files; the separate
//! physical validator must accept the resulting plan before execution uses it.

use super::demand::{ColumnSet, demand_masks};
use super::{
    MAX_PIPELINES, OrderColumn, PhysicalPlan, Pipeline, PipelineId, Producer, WindowFunction,
    pipeline_end,
};
use crate::execution::predicate::PhysicalFilter;
use crate::query::{
    self, ColumnId, MAX_ORDER_ITEMS, MAX_QUERY_COLUMNS, MAX_ROW_VALUES, MAX_STAGES, PreparedQuery,
    RelationId, Stage,
};
use crate::resources::allocate;
use crate::storage::format::RootState;
use crate::{Database, Error};

pub(in crate::execution) fn lower<'db>(
    database: &'db Database,
    query: &'db PreparedQuery<'_>,
    root: RootState,
    projected_crc32c: u32,
) -> Result<PhysicalPlan<'db>, Error> {
    query::validate(&query.plan)?;
    let bytes = PhysicalPlan::required_bytes(query);
    let reservation = database.reserve_memory(bytes, "physical producer pipelines")?;
    let count = PhysicalPlan::pipeline_count(query);
    let mut pipelines: Vec<Pipeline> = allocate(count, count, "physical pipelines", bytes)?;
    let mut mapping = [None; MAX_PIPELINES];
    let mut order_columns = [OrderColumn::EMPTY; MAX_ORDER_ITEMS];
    let semantic = &query.plan;
    let masks = demand_masks(semantic)?;
    for index in 0..=semantic.nodes().len() {
        let relation = RelationId(index as u8);
        let Some(producer) = select_producer(
            semantic,
            relation,
            masks[index],
            &pipelines,
            &mapping,
            &mut order_columns,
        )?
        else {
            continue;
        };
        let end = pipeline_end(semantic, relation);
        let mut pipeline = Pipeline {
            producer,
            relation,
            end,
            filters: [PhysicalFilter::EMPTY; MAX_STAGES],
            filter_count: 0,
            columns: [0; MAX_ROW_VALUES],
            column_count: 0,
            identities: [ColumnId::EMPTY; MAX_ROW_VALUES],
            computed: &semantic.computed,
            slots: [u8::MAX; MAX_QUERY_COLUMNS + 1],
        };
        // An analytic SELECT can remove an input column from its visible
        // output while still using that column in an expression. Map against
        // the input scope so the expression can still read it.
        let input_scope = if let Producer::Analytic { .. } = producer {
            semantic.node(relation)?.input
        } else {
            relation
        };
        for id in semantic.available_columns(input_scope)?.iter() {
            if masks[usize::from(input_scope.0)].contains(id) {
                pipeline.slots[id.value() as usize] =
                    base_position(&pipelines, semantic, producer, relation, id)?;
            }
        }
        for definition in &semantic.computed {
            if semantic.computation_producer(definition)? == relation {
                let id = definition.column.identity();
                pipeline.slots[id.value() as usize] =
                    base_position(&pipelines, semantic, producer, relation, id)?;
            }
        }
        for node in semantic.nodes() {
            if let Stage::Where(filter) = &node.stage
                && semantic.producer(node.input)? == relation
            {
                pipeline.filters[pipeline.filter_count] = PhysicalFilter {
                    column: base_position(&pipelines, semantic, producer, relation, filter.column)?,
                    predicate: &filter.predicate,
                    control: filter.control,
                };
                pipeline.filter_count += 1;
            }
        }
        let visible = semantic.relation_columns(end)?;
        let available = semantic.available_columns(end)?;
        let retained = available
            .iter()
            .filter(|id| end != semantic.final_relation() && !visible.contains(*id));
        for id in visible.iter().chain(retained) {
            if !masks[usize::from(end.0)].contains(id)
                || (end != semantic.final_relation() && pipeline.position(id).is_some())
            {
                continue;
            }
            let index = pipeline.column_count;
            if index >= pipeline.columns.len() {
                return Err(Error::Corrupt("physical payload exceeds column capacity"));
            }
            pipeline.columns[index] = base_position(&pipelines, semantic, producer, relation, id)?;
            pipeline.identities[index] = id;
            pipeline.column_count += 1;
        }
        mapping[index] = Some(PipelineId(pipelines.len() as u8));
        pipelines.push(pipeline);
    }
    Ok(PhysicalPlan {
        database: semantic.database,
        generation: semantic.generation,
        root,
        projected_crc32c,
        pipelines,
        order_columns,
        reservation: Some(reservation),
    })
}

// Return None for stages that run within an existing producer. New producers
// refer to pipelines already built for their inputs; join and sort keys must
// resolve to positions in those inputs before runtime construction.
fn select_producer(
    semantic: &query::Plan,
    relation: RelationId,
    demanded_columns: ColumnSet,
    pipelines: &[Pipeline<'_>],
    mapping: &[Option<PipelineId>; MAX_PIPELINES],
    order_columns: &mut [OrderColumn; MAX_ORDER_ITEMS],
) -> Result<Option<Producer>, Error> {
    let lookup = |input: RelationId| -> Result<PipelineId, Error> {
        let producer = semantic.producer(input)?;
        mapping[usize::from(producer.0)].ok_or(Error::Corrupt("physical input producer absent"))
    };
    let producer = if relation == RelationId::SOURCE {
        Producer::Scan(0)
    } else {
        let node = semantic.node(relation)?;
        if semantic.analytic_projection(relation) {
            let input = lookup(node.input)?;
            let window = semantic.demanded_analytic(relation, demanded_columns)?;
            let keys = match window {
                Some(query::Computation::WindowCount(keys)) => *keys,
                Some(query::Computation::WindowSum(sum)) => sum.partition,
                None => query::PartitionKeys::EMPTY,
                _ => return Err(Error::Corrupt("nonanalytic computation in window")),
            };
            let position = |column: query::SemanticColumn| -> Result<u8, Error> {
                pipelines[input.index()]
                    .position(column.identity())
                    .map(|position| position as u8)
                    .ok_or(Error::Corrupt("analytic input not demanded"))
            };
            let mut partition = [None; query::MAX_PARTITION_KEYS];
            for (slot, column) in partition.iter_mut().zip(keys.columns()) {
                *slot = Some(position(column)?);
            }
            let function = if let Some(query::Computation::WindowSum(sum)) = window {
                let mut order = [None; query::MAX_WINDOW_ORDER_KEYS];
                for (slot, key) in order.iter_mut().zip(sum.order.iter().flatten()) {
                    *slot = Some(OrderColumn {
                        column: position(key.column)?,
                        direction: key.direction,
                        nulls: key.nulls,
                    });
                }
                WindowFunction::RunningSum {
                    argument: position(sum.argument)?,
                    order,
                }
            } else {
                WindowFunction::Count
            };
            return Ok(Some(Producer::Analytic {
                input,
                partition,
                function,
            }));
        }
        match node.stage {
            Stage::Source(source) => Producer::Scan(source),
            Stage::Distinct(descriptor) => Producer::Distinct {
                input: lookup(node.input)?,
                descriptor,
            },
            Stage::Limit(bounds) => Producer::Limit {
                input: lookup(node.input)?,
                bounds,
            },
            Stage::Order { start, len } => {
                let input = lookup(node.input)?;
                for (index, key) in semantic.order_items(start, len)?.iter().enumerate() {
                    order_columns[usize::from(start) + index] = OrderColumn {
                        column: pipelines[input.index()]
                            .position(key.column)
                            .ok_or(Error::Corrupt("order key not demanded"))?
                            as u8,
                        direction: key.direction,
                        nulls: key.nulls,
                    };
                }
                Producer::Order { input, start, len }
            }
            Stage::Aggregate(aggregate_index) => {
                let mut demand = 0;
                let aggregate = semantic
                    .aggregates
                    .get(usize::from(aggregate_index))
                    .ok_or(Error::Corrupt("aggregate absent"))?;
                for (entry, id) in semantic
                    .relation_columns(relation)?
                    .iter()
                    .skip(usize::from(aggregate.group_count))
                    .enumerate()
                {
                    if demanded_columns.contains(id) {
                        demand |= 1 << entry;
                    }
                }
                Producer::Aggregate {
                    aggregate: aggregate_index,
                    input: lookup(node.input)?,
                    demand,
                }
            }
            Stage::Join {
                nulls,
                right,
                left_key,
                right_key,
            } => {
                let left = lookup(node.input)?;
                let right = lookup(right)?;
                Producer::Join {
                    kind: if nulls.is_some() {
                        query::JoinKind::Left
                    } else {
                        query::JoinKind::Inner
                    },
                    left,
                    right,
                    left_key: pipelines[left.index()]
                        .position(left_key)
                        .ok_or(Error::Corrupt("join left key not demanded"))?
                        as u8,
                    right_key: pipelines[right.index()]
                        .position(right_key)
                        .ok_or(Error::Corrupt("join right key not demanded"))?
                        as u8,
                }
            }
            Stage::Alias
            | Stage::Rename
            | Stage::Set { .. }
            | Stage::Drop { .. }
            | Stage::Derived
            | Stage::Select { .. }
            | Stage::Extend { .. }
            | Stage::Where(_) => {
                return Ok(None);
            }
            Stage::SetOperation { right, descriptor } => Producer::SetOperation {
                left: lookup(node.input)?,
                right: lookup(right)?,
                descriptor,
            },
            Stage::Empty => return Err(Error::Corrupt("empty producer")),
        }
    };
    Ok(Some(producer))
}

fn base_position(
    pipelines: &[Pipeline],
    semantic: &query::Plan,
    producer: Producer,
    relation: RelationId,
    id: ColumnId,
) -> Result<u8, Error> {
    let mut id = id;
    // A copied column gets a new identity but can use the original value slot.
    // Follow copies only within this producer. A value already produced upstream
    // must be read from its input position rather than recomputed here.
    for _ in 0..=semantic.computed.len() {
        let position = match producer {
            Producer::SetOperation { .. } => semantic
                .relation_columns(relation)?
                .iter()
                .position(|column| column == id),
            Producer::Scan(source) => semantic
                .occurrence_columns(source)?
                .iter()
                .find(|column| column.semantic().identity() == id)
                .map(|column| usize::from(column.storage_slot())),
            Producer::Distinct { input, descriptor } => semantic
                .distinct
                .get(usize::from(descriptor))
                .and_then(|bound| bound.input_for(id))
                .and_then(|column| pipelines.get(input.index())?.position(column.identity())),
            Producer::Aggregate { aggregate, .. } => semantic
                .aggregates
                .get(usize::from(aggregate))
                .and_then(|aggregate| aggregate.output_position(id)),
            Producer::Analytic { input, .. }
            | Producer::Order { input, .. }
            | Producer::Limit { input, .. } => pipelines
                .get(input.index())
                .and_then(|input| input.position(id)),
            Producer::Join { left, right, .. } => {
                let left = pipelines
                    .get(left.index())
                    .ok_or(Error::Corrupt("join left pipeline absent"))?;
                let right = pipelines
                    .get(right.index())
                    .ok_or(Error::Corrupt("join right pipeline absent"))?;
                let right_id = match semantic.node(relation)?.stage {
                    Stage::Join {
                        nulls: Some(descriptor),
                        ..
                    } => semantic
                        .null_extensions
                        .get(usize::from(descriptor))
                        .and_then(|extension| extension.input_for(id))
                        .map(|column| column.identity()),
                    Stage::Join { nulls: None, .. } => Some(id),
                    _ => return Err(Error::Corrupt("join producer semantic stage")),
                };
                left.position(id).or_else(|| {
                    right_id
                        .and_then(|id| right.position(id))
                        .map(|position| left.column_count + position)
                })
            }
        };
        if let Some(position) = position {
            let scope = if matches!(producer, Producer::Analytic { .. }) {
                semantic.node(relation)?.input
            } else {
                relation
            };
            if position >= MAX_ROW_VALUES || !semantic.available_columns(scope)?.contains(id) {
                return Err(Error::Corrupt("physical producer position outside its row"));
            }
            return Ok(position as u8);
        }
        let (index, definition) = semantic
            .computed
            .iter()
            .enumerate()
            .find(|(_, definition)| {
                definition.column.identity() == id
                    && semantic.computation_producer(definition).ok() == Some(relation)
            })
            .ok_or(Error::Corrupt(
                "physical producer does not provide demanded identity",
            ))?;
        match &definition.expression {
            query::Computation::Numeric(_)
            | query::Computation::StringLength { .. }
            | query::Computation::DateYear(_)
            | query::Computation::Constant(_)
            | query::Computation::WindowCount(_)
            | query::Computation::WindowSum(_) => {
                return Ok((MAX_ROW_VALUES + index) as u8);
            }
            query::Computation::Copy(column) => id = column.identity(),
        }
    }
    Err(Error::Corrupt("cyclic physical copy mapping"))
}
