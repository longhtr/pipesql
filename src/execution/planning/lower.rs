//! Lower semantic producers to physical pipelines and storage positions.

use super::demand::{ColumnSet, demand_masks};
use super::{
    MAX_PIPELINES, OrderColumn, PhysicalPlan, Pipeline, PipelineId, Producer, pipeline_end,
};
use crate::execution::predicate::PhysicalFilter;
use crate::frontend::{
    self, ColumnId, MAX_ORDER_ITEMS, MAX_QUERY_COLUMNS, MAX_ROW_VALUES, MAX_STAGES, PreparedQuery,
    RelationId, Stage,
};
use crate::resources::allocate;
use crate::storage_format::RootState;
use crate::{Database, Error};

pub(in crate::execution) fn lower<'db>(
    database: &'db Database,
    query: &'db PreparedQuery<'_>,
    root: RootState,
    projected_crc32c: u32,
) -> Result<PhysicalPlan<'db>, Error> {
    frontend::validate(&query.plan)?;
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
        // Analytic SELECT can hide an input used by one of its expressions.
        // Keep that raw mapping in the new producer without exposing its name.
        let input_scope = if let Producer::WindowCount { .. } = producer {
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

// Transparent unary stages belong to an existing pipeline. ORDER BY records
// demanded positions from its selected input before execution can use its keys.
fn select_producer(
    semantic: &frontend::Plan,
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
            return Ok(Some(Producer::WindowCount {
                input: lookup(node.input)?,
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
                        frontend::JoinKind::Left
                    } else {
                        frontend::JoinKind::Inner
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
    semantic: &frontend::Plan,
    producer: Producer,
    relation: RelationId,
    id: ColumnId,
) -> Result<u8, Error> {
    let mut id = id;
    // Copies retain distinct semantic identities while borrowing the same
    // physical value until the producer materializes its output.
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
            Producer::WindowCount { input }
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
            let scope = if matches!(producer, Producer::WindowCount { .. }) {
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
            frontend::Computation::Numeric(_)
            | frontend::Computation::Constant(_)
            | frontend::Computation::WindowCount => {
                return Ok((MAX_ROW_VALUES + index) as u8);
            }
            frontend::Computation::Copy(column) => id = column.identity(),
        }
    }
    Err(Error::Corrupt("cyclic physical copy mapping"))
}
