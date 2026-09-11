//! Validate physical envelopes, edges, filters, mappings, and reachable outputs.
//! Producer positions are decoded independently of the lowering implementation.

use super::demand::{ColumnSet, demand_masks};
use super::{
    MAX_PIPELINES, OrderColumn, PhysicalPlan, Pipeline, PipelineId, Producer, pipeline_end,
    snapshot_matches,
};
use crate::execution::predicate::PhysicalFilter;
use crate::frontend::{self, ColumnId, MAX_COLUMNS, MAX_STAGES, PreparedQuery, RelationId, Stage};
use crate::scalar::Op;
use crate::storage_format::RootState;
use crate::{Database, Error};

pub(in crate::execution) fn validate_physical(
    plan: &PhysicalPlan<'_>,
    query: &PreparedQuery<'_>,
    database: &Database,
    root: RootState,
    crc: u32,
) -> Result<(), Error> {
    frontend::validate(&query.plan)?;
    let semantic = &query.plan;
    if plan.database != database.database_identity()
        || plan.database != semantic.database
        || !snapshot_matches(query, database)
        || plan.generation != semantic.generation
        || plan.root != root
        || plan.projected_crc32c != crc
        || plan.pipelines.is_empty()
        || plan.pipelines.len() > MAX_PIPELINES
        || plan.pipelines.capacity() != plan.pipelines.len()
        || plan.memory_bytes() != PhysicalPlan::required_bytes(query)
    {
        return Err(Error::Corrupt("physical envelope is invalid"));
    }
    let masks = demand_masks(semantic)?;
    let mut reached = [false; MAX_PIPELINES];
    let mut order_end = 0;
    for (index, pipeline) in plan.pipelines.iter().enumerate() {
        if pipeline.filter_count > MAX_STAGES
            || pipeline.column_count > MAX_COLUMNS
            || pipeline.end != pipeline_end(semantic, pipeline.relation)
        {
            return Err(Error::Corrupt("physical pipeline envelope"));
        }
        // Only earlier pipelines have validated row bounds and mappings.
        let inputs = &plan.pipelines[..index];
        validate_computations(pipeline, inputs, semantic, &masks)?;
        let check_input = |id: PipelineId, relation: RelationId| -> Result<(), Error> {
            let input = inputs
                .get(id.index())
                .ok_or(Error::Corrupt("physical input is not earlier"))?;
            if input.end != relation {
                return Err(Error::Corrupt("physical input differs from bound relation"));
            }
            Ok(())
        };
        let node = if pipeline.relation == RelationId::SOURCE {
            None
        } else {
            Some(semantic.node(pipeline.relation)?)
        };
        match (pipeline.producer, node.map(|node| node.stage)) {
            (Producer::Scan(source), stage)
                if (source == 0 && stage.is_none() && index == 0)
                    || stage == Some(Stage::Source(source)) =>
            {
                semantic.occurrence(source)?;
            }
            (
                Producer::Order { input, start, len },
                Some(Stage::Order {
                    start: expected_start,
                    len: expected_len,
                }),
            ) => {
                check_input(input, node.expect("order node").input)?;
                if start != expected_start || len != expected_len || usize::from(start) != order_end
                {
                    return Err(Error::Corrupt("physical order range"));
                }
                for (physical, bound) in plan
                    .order_columns(start, len)
                    .iter()
                    .zip(semantic.order_items(start, len)?)
                {
                    if inputs[input.index()].identities[..inputs[input.index()].column_count]
                        .get(usize::from(physical.column))
                        != Some(&bound.column)
                        || physical.direction != bound.direction
                        || physical.nulls != bound.nulls
                    {
                        return Err(Error::Corrupt("physical order key"));
                    }
                }
                order_end += usize::from(len);
                reached[input.index()] = true;
            }
            (Producer::Distinct { input, descriptor }, Some(Stage::Distinct(expected))) => {
                check_input(input, node.expect("DISTINCT node").input)?;
                let bound = semantic
                    .distinct
                    .get(usize::from(expected))
                    .ok_or(Error::Corrupt("physical DISTINCT descriptor absent"))?;
                let child = &inputs[input.index()];
                if descriptor != expected
                    || child.column_count != bound.inputs().len()
                    || bound
                        .inputs()
                        .iter()
                        .any(|column| child.position(column.identity()).is_none())
                {
                    return Err(Error::Corrupt("physical DISTINCT input coverage"));
                }
                reached[input.index()] = true;
            }
            (Producer::Limit { input, bounds }, Some(Stage::Limit(expected))) => {
                check_input(input, node.expect("limit node").input)?;
                if bounds != expected {
                    return Err(Error::Corrupt("physical limit bounds"));
                }
                reached[input.index()] = true;
            }
            (
                Producer::Aggregate {
                    aggregate,
                    input,
                    demand,
                },
                Some(Stage::Aggregate(expected)),
            ) => {
                check_input(input, node.expect("aggregate node").input)?;
                if aggregate != expected || demand != semantic.aggregate_demand(expected) {
                    return Err(Error::Corrupt("physical aggregate demand"));
                }
                reached[input.index()] = true;
            }
            (
                Producer::Join {
                    left,
                    right,
                    left_key,
                    right_key,
                },
                Some(Stage::Join {
                    right: right_relation,
                    left_key: left_id,
                    right_key: right_id,
                }),
            ) => {
                check_input(left, node.expect("join node").input)?;
                check_input(right, right_relation)?;
                if left == right
                    || inputs[left.index()].position(left_id) != Some(usize::from(left_key))
                    || inputs[right.index()].position(right_id) != Some(usize::from(right_key))
                {
                    return Err(Error::Corrupt("physical join keys"));
                }
                reached[left.index()] = true;
                reached[right.index()] = true;
            }
            _ => {
                return Err(Error::Corrupt(
                    "physical producer differs from bound operator",
                ));
            }
        }
        validate_filters(pipeline, inputs, semantic)?;
        validate_outputs(
            pipeline,
            inputs,
            semantic,
            masks[usize::from(pipeline.end.0)],
        )?;
    }
    if plan.order_columns[order_end..]
        .iter()
        .any(|key| *key != OrderColumn::EMPTY)
    {
        return Err(Error::Corrupt("physical order tail"));
    }
    let last = plan.pipelines.len() - 1;
    if plan.pipelines[last].end != semantic.final_relation()
        || reached[last]
        || reached[..last].iter().any(|used| !*used)
    {
        return Err(Error::Corrupt("unreachable physical producer"));
    }
    Ok(())
}

fn validate_computations(
    pipeline: &Pipeline<'_>,
    inputs: &[Pipeline<'_>],
    semantic: &frontend::Plan,
    masks: &[ColumnSet; MAX_PIPELINES],
) -> Result<(), Error> {
    if pipeline.computed != semantic.computed || pipeline.slots[0] != u8::MAX {
        return Err(Error::Corrupt("physical computation definitions"));
    }
    for (identity, &position) in pipeline.slots.iter().enumerate().skip(1) {
        if position != u8::MAX
            && identity_at(
                inputs,
                semantic,
                pipeline.producer,
                pipeline.relation,
                position,
            )?
            .value() as usize
                != identity
        {
            return Err(Error::Corrupt("physical computation mapping"));
        }
    }
    for definition in pipeline.computed {
        if semantic.producer(definition.input)? == pipeline.relation {
            for op in &definition.expression.ops[..usize::from(definition.expression.len)] {
                if let Op::Column(column) = op {
                    // Unused definitions may reference undemanded materialized
                    // inputs. Every demanded dependency is checked by demand_masks.
                    if masks[usize::from(definition.input.0)].contains(column.identity())
                        && pipeline.slots[column.identity().value() as usize] == u8::MAX
                    {
                        return Err(Error::Corrupt("missing computed input mapping"));
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_filters(
    pipeline: &Pipeline<'_>,
    inputs: &[Pipeline<'_>],
    semantic: &frontend::Plan,
) -> Result<(), Error> {
    let mut filter_index = 0;
    for node in semantic.nodes() {
        if let Stage::Where(filter) = &node.stage
            && semantic.producer(node.input)? == pipeline.relation
        {
            let physical = pipeline
                .filters
                .get(filter_index)
                .filter(|_| filter_index < pipeline.filter_count)
                .ok_or(Error::Corrupt("missing physical filter"))?;
            if identity_at(
                inputs,
                semantic,
                pipeline.producer,
                pipeline.relation,
                physical.column,
            )? != filter.column
                || *physical.predicate != filter.predicate
                || physical.control != filter.control
            {
                return Err(Error::Corrupt("physical filter differs from binder"));
            }
            filter_index += 1;
        }
    }
    if filter_index != pipeline.filter_count
        || pipeline.filters[filter_index..]
            .iter()
            .any(|filter| *filter != PhysicalFilter::EMPTY)
    {
        return Err(Error::Corrupt("physical filter tail"));
    }
    Ok(())
}

fn validate_outputs(
    pipeline: &Pipeline<'_>,
    inputs: &[Pipeline<'_>],
    semantic: &frontend::Plan,
    demand: ColumnSet,
) -> Result<(), Error> {
    let mut outputs = 0;
    let mut seen = ColumnSet::EMPTY;
    for id in semantic.relation_columns(pipeline.end)?.iter() {
        if !demand.contains(id) || (pipeline.end != semantic.final_relation() && seen.contains(id))
        {
            continue;
        }
        seen.insert(id);
        if outputs >= pipeline.column_count
            || pipeline.identities[outputs] != id
            || identity_at(
                inputs,
                semantic,
                pipeline.producer,
                pipeline.relation,
                pipeline.columns[outputs],
            )? != id
        {
            return Err(Error::Corrupt("physical output differs from binder"));
        }
        outputs += 1;
    }
    if outputs != pipeline.column_count
        || pipeline.columns[outputs..]
            .iter()
            .any(|column| *column != 0)
        || pipeline.identities[outputs..]
            .iter()
            .any(|id| *id != ColumnId::EMPTY)
    {
        return Err(Error::Corrupt("physical output tail"));
    }
    Ok(())
}

// Validation resolves positions back to identities instead of repeating the
// planner's identity-to-position search.
fn identity_at(
    pipelines: &[Pipeline],
    semantic: &frontend::Plan,
    producer: Producer,
    relation: RelationId,
    position: u8,
) -> Result<ColumnId, Error> {
    let position = usize::from(position);
    if position >= MAX_COLUMNS {
        let definition = semantic
            .computed
            .get(position - MAX_COLUMNS)
            .ok_or(Error::Corrupt("computed slot outside definitions"))?;
        if semantic.producer(definition.input)? != relation {
            return Err(Error::Corrupt("computed slot belongs to another producer"));
        }
        return Ok(definition.column.identity());
    }
    let identity = match producer {
        Producer::Scan(source) => semantic
            .occurrence_columns(source)?
            .iter()
            .find(|column| usize::from(column.storage_slot()) == position)
            .map(|column| column.semantic().identity()),
        Producer::Aggregate { .. } => semantic.relation_columns(relation)?.get(position),
        Producer::Distinct { input, descriptor } => pipelines
            .get(input.index())
            .and_then(|input| input.identities[..input.column_count].get(position))
            .and_then(|id| {
                semantic
                    .distinct
                    .get(usize::from(descriptor))?
                    .output_for(*id)
            }),
        Producer::Order { input, .. } | Producer::Limit { input, .. } => pipelines
            .get(input.index())
            .and_then(|input| input.identities[..input.column_count].get(position))
            .copied(),
        Producer::Join { left, right, .. } => {
            let left = pipelines
                .get(left.index())
                .ok_or(Error::Corrupt("join left pipeline absent"))?;
            let right = pipelines
                .get(right.index())
                .ok_or(Error::Corrupt("join right pipeline absent"))?;
            if position < left.column_count {
                left.identities.get(position).copied()
            } else {
                right.identities[..right.column_count]
                    .get(position - left.column_count)
                    .copied()
            }
        }
    };
    identity.ok_or(Error::Corrupt("physical position outside producer row"))
}
