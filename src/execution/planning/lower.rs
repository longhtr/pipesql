//! Lower semantic producers to physical pipelines and storage positions.

use super::demand::{ColumnSet, demand_masks};
use super::{
    MAX_PIPELINES, OrderColumn, PhysicalPlan, Pipeline, PipelineId, Producer, pipeline_end,
};
use crate::execution::predicate::PhysicalFilter;
use crate::frontend::{
    self, ColumnId, MAX_COLUMNS, MAX_ORDER_ITEMS, MAX_QUERY_COLUMNS, MAX_STAGES, PreparedQuery,
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
            columns: [0; MAX_COLUMNS],
            column_count: 0,
            identities: [ColumnId::EMPTY; MAX_COLUMNS],
            computed: &semantic.computed,
            slots: [u8::MAX; MAX_QUERY_COLUMNS + 1],
        };
        for id in semantic.relation_columns(relation)?.iter() {
            if masks[index].contains(id) {
                pipeline.slots[id.value() as usize] =
                    base_position(&pipelines, semantic, producer, relation, id)?;
            }
        }
        for (index, definition) in semantic.computed.iter().enumerate() {
            if semantic.producer(definition.input)? == relation {
                pipeline.slots[definition.column.identity().value() as usize] =
                    (MAX_COLUMNS + index) as u8;
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
        for id in semantic.relation_columns(end)?.iter() {
            if !masks[usize::from(end.0)].contains(id)
                || (end != semantic.final_relation() && pipeline.position(id).is_some())
            {
                continue;
            }
            let index = pipeline.column_count;
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
                right,
                left_key,
                right_key,
            } => {
                let left = lookup(node.input)?;
                let right = lookup(right)?;
                Producer::Join {
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
            Stage::Alias | Stage::Derived | Stage::Select { .. } | Stage::Where(_) => {
                return Ok(None);
            }
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
    let position = match producer {
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
        Producer::Order { input, .. } | Producer::Limit { input, .. } => pipelines
            .get(input.index())
            .and_then(|input| input.position(id)),
        Producer::Join { left, right, .. } => {
            let left = pipelines
                .get(left.index())
                .ok_or(Error::Corrupt("join left pipeline absent"))?;
            let right = pipelines
                .get(right.index())
                .ok_or(Error::Corrupt("join right pipeline absent"))?;
            left.position(id).or_else(|| {
                right
                    .position(id)
                    .map(|position| left.column_count + position)
            })
        }
    };
    let Some(position) = position else {
        for (index, definition) in semantic.computed.iter().enumerate() {
            if definition.column.identity() == id
                && semantic.producer(definition.input)? == relation
            {
                return Ok((MAX_COLUMNS + index) as u8);
            }
        }
        return Err(Error::Corrupt(
            "physical producer does not provide demanded identity",
        ));
    };
    if position >= MAX_COLUMNS || !semantic.relation_columns(relation)?.contains(id) {
        return Err(Error::Corrupt("physical producer position outside its row"));
    }
    Ok(position as u8)
}
