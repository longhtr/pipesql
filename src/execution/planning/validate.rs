//! Check that a physical plan executes the query it was built from.
//!
//! A position can be in bounds and still point to the wrong column. Decode each
//! input, output and filter position back to its logical identity, then compare
//! it with the prepared query. This path is separate from the builder's search
//! for positions, so validation can catch a mistake in that search.
//!
//! Visit pipelines in input-first order. An input must name an earlier pipeline
//! with the expected relation, and every pipeline must contribute to the final
//! result. Also check the snapshot, allocation charge and unused array entries.
//! Reject a mismatch before the runtime reads source values.
//!
//! Demand analysis and semantic accessors are shared with the builder. Mutation
//! tests challenge the physical representation; independent SQL results must
//! check the meaning supplied by those shared helpers.

use super::demand::{ColumnSet, demand_masks};
use super::{
    MAX_PIPELINES, OrderColumn, PhysicalPlan, Pipeline, PipelineId, Producer, WindowFunction,
    pipeline_end, snapshot_matches,
};
use crate::execution::predicate::PhysicalFilter;
use crate::query::{self, ColumnId, MAX_ROW_VALUES, MAX_STAGES, PreparedQuery, RelationId, Stage};
use crate::storage::format::RootState;
use crate::{Database, Error};

pub(in crate::execution) fn validate_physical(
    plan: &PhysicalPlan<'_>,
    query: &PreparedQuery<'_>,
    database: &Database,
    root: RootState,
    crc: u32,
) -> Result<(), Error> {
    query::validate(&query.plan)?;
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
            || pipeline.column_count > MAX_ROW_VALUES
            || pipeline.end != pipeline_end(semantic, pipeline.relation)
        {
            return Err(Error::Corrupt("physical pipeline envelope"));
        }
        // Restrict lookups to pipelines already checked. A malformed edge must
        // not make validation inspect an unchecked future pipeline's arrays.
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
            (
                Producer::Analytic {
                    input,
                    partition,
                    function,
                },
                Some(Stage::Select { .. } | Stage::Extend { .. }),
            ) if semantic.analytic_projection(pipeline.relation) => {
                check_input(input, node.expect("analytic node").input)?;
                let window = semantic.demanded_analytic(
                    pipeline.relation,
                    masks[usize::from(pipeline.relation.0)],
                )?;
                let keys = match window {
                    Some(query::Computation::WindowCount(keys)) => *keys,
                    Some(query::Computation::WindowSum(sum)) => sum.partition,
                    None => query::PartitionKeys::EMPTY,
                    _ => return Err(Error::Corrupt("nonanalytic computation in window")),
                };
                let identity = |position: u8| {
                    inputs[input.index()].identities[..inputs[input.index()].column_count]
                        .get(usize::from(position))
                        .copied()
                };
                let mut expected = [None; query::MAX_PARTITION_KEYS];
                for (slot, key) in expected.iter_mut().zip(keys.columns()) {
                    *slot = Some(key.identity());
                }
                for (position, key) in partition.iter().zip(expected) {
                    if position.and_then(identity) != key || position.is_some() != key.is_some() {
                        return Err(Error::Corrupt("physical partition key"));
                    }
                }
                match (function, window) {
                    (WindowFunction::Count, None | Some(query::Computation::WindowCount(_))) => {}
                    (
                        WindowFunction::RunningSum { argument, order },
                        Some(query::Computation::WindowSum(sum)),
                    ) => {
                        if identity(argument) != Some(sum.argument.identity()) {
                            return Err(Error::Corrupt("physical running SUM argument"));
                        }
                        for (actual, expected) in order.iter().zip(sum.order) {
                            match (actual, expected) {
                                (None, None) => (),
                                (Some(actual), Some(expected))
                                    if identity(actual.column)
                                        == Some(expected.column.identity())
                                        && actual.direction == expected.direction
                                        && actual.nulls == expected.nulls => {}
                                _ => return Err(Error::Corrupt("physical running SUM order")),
                            }
                        }
                    }
                    _ => return Err(Error::Corrupt("physical analytic function")),
                }
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
            (
                Producer::SetOperation {
                    left,
                    right,
                    descriptor,
                },
                Some(Stage::SetOperation {
                    right: right_relation,
                    descriptor: expected,
                }),
            ) => {
                check_input(left, node.expect("set node").input)?;
                check_input(right, right_relation)?;
                if left == right || descriptor != expected {
                    return Err(Error::Corrupt("physical set inputs or descriptor"));
                }
                let bound = &semantic.set_operations[usize::from(expected)];
                for position in 0..usize::from(node.expect("set node").columns) {
                    let output = bound
                        .output(position)
                        .ok_or(Error::Corrupt("physical set output"))?;
                    if bound.kind() != query::SetKind::UnionAll
                        || masks[usize::from(pipeline.relation.0)].contains(output.identity())
                    {
                        let columns = bound
                            .inputs(position)
                            .ok_or(Error::Corrupt("physical set inputs"))?;
                        for (child, column) in [left, right].into_iter().zip(columns) {
                            if inputs[child.index()].position(column.identity()).is_none() {
                                return Err(Error::Corrupt("physical set demanded input absent"));
                            }
                        }
                    }
                }
                reached[left.index()] = true;
                reached[right.index()] = true;
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
                    kind,
                    left,
                    right,
                    left_key,
                    right_key,
                },
                Some(Stage::Join {
                    nulls,
                    right: right_relation,
                    left_key: left_id,
                    right_key: right_id,
                }),
            ) => {
                check_input(left, node.expect("join node").input)?;
                check_input(right, right_relation)?;
                if kind
                    != if nulls.is_some() {
                        query::JoinKind::Left
                    } else {
                        query::JoinKind::Inner
                    }
                    || left == right
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
    semantic: &query::Plan,
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
            )? != value_identity(semantic, pipeline.relation, identity as u32)?
        {
            return Err(Error::Corrupt("physical computation mapping"));
        }
    }
    for definition in pipeline.computed {
        if semantic.computation_producer(definition)? == pipeline.relation {
            for column in definition.expression.columns() {
                // The plan retains unused definitions too. Require a slot only
                // for inputs that demand analysis says this query may read.
                if masks[usize::from(definition.input.0)].contains(column.identity())
                    && pipeline.slots[column.identity().value() as usize] == u8::MAX
                {
                    return Err(Error::Corrupt("missing computed input mapping"));
                }
            }
        }
    }
    Ok(())
}

fn validate_filters(
    pipeline: &Pipeline<'_>,
    inputs: &[Pipeline<'_>],
    semantic: &query::Plan,
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
            )? != value_identity(semantic, pipeline.relation, filter.column.value())?
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
    semantic: &query::Plan,
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
            )? != value_identity(semantic, pipeline.relation, id.value())?
        {
            return Err(Error::Corrupt("physical output differs from binder"));
        }
        outputs += 1;
    }
    if pipeline.end != semantic.final_relation() {
        let available = semantic.available_columns(pipeline.end)?;
        for index in outputs..pipeline.column_count {
            let id = pipeline.identities[index];
            if !available.contains(id)
                || !demand.contains(id)
                || seen.contains(id)
                || identity_at(
                    inputs,
                    semantic,
                    pipeline.producer,
                    pipeline.relation,
                    pipeline.columns[index],
                )? != value_identity(semantic, pipeline.relation, id.value())?
            {
                return Err(Error::Corrupt("invalid retained physical input"));
            }
            seen.insert(id);
        }
        if available
            .iter()
            .any(|id| demand.contains(id) && !seen.contains(id))
        {
            return Err(Error::Corrupt("retained physical input absent"));
        }
        outputs = pipeline.column_count;
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

// Copies made within this producer can share a value slot despite having
// different identities. Follow their source identity for comparison, but stop
// at an upstream result: that value already has its own input position.
fn value_identity(
    semantic: &query::Plan,
    relation: RelationId,
    mut identity: u32,
) -> Result<ColumnId, Error> {
    for _ in 0..=semantic.computed.len() {
        let Some(definition) = semantic
            .computed
            .iter()
            .find(|definition| definition.column.identity().value() == identity)
        else {
            return semantic
                .available_columns(if semantic.analytic_projection(relation) {
                    semantic.node(relation)?.input
                } else {
                    relation
                })?
                .iter()
                .find(|id| id.value() == identity)
                .ok_or(Error::Corrupt("physical identity outside producer"));
        };
        if semantic.computation_producer(definition)? == relation
            && let query::Computation::Copy(column) = &definition.expression
        {
            identity = column.identity().value();
            continue;
        }
        return Ok(definition.column.identity());
    }
    Err(Error::Corrupt("cyclic semantic copy"))
}

// Decode what a position actually names. Keep this separate from base_position
// in the builder, which searches for a position given the desired identity.
fn identity_at(
    pipelines: &[Pipeline],
    semantic: &query::Plan,
    producer: Producer,
    relation: RelationId,
    position: u8,
) -> Result<ColumnId, Error> {
    let position = usize::from(position);
    if position >= MAX_ROW_VALUES {
        let definition = semantic
            .computed
            .get(position - MAX_ROW_VALUES)
            .ok_or(Error::Corrupt("computed slot outside definitions"))?;
        if semantic.computation_producer(definition)? != relation {
            return Err(Error::Corrupt("computed slot belongs to another producer"));
        }
        if !matches!(
            definition.expression,
            query::Computation::Numeric(_)
                | query::Computation::StringLength { .. }
                | query::Computation::DateYear(_)
                | query::Computation::Constant(_)
                | query::Computation::WindowCount(_)
                | query::Computation::WindowSum(_)
        ) {
            return Err(Error::Corrupt("typed copy mapped to numeric slot"));
        }
        return Ok(definition.column.identity());
    }
    let identity = match producer {
        Producer::Scan(source) => semantic
            .occurrence_columns(source)?
            .iter()
            .find(|column| usize::from(column.storage_slot()) == position)
            .map(|column| column.semantic().identity()),
        Producer::Aggregate { .. } | Producer::SetOperation { .. } => {
            semantic.relation_columns(relation)?.get(position)
        }
        Producer::Distinct { input, descriptor } => pipelines
            .get(input.index())
            .and_then(|input| input.identities[..input.column_count].get(position))
            .and_then(|id| {
                semantic
                    .distinct
                    .get(usize::from(descriptor))?
                    .output_for(*id)
            }),
        Producer::Analytic { input, .. }
        | Producer::Order { input, .. }
        | Producer::Limit { input, .. } => pipelines
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
                let original = right.identities[..right.column_count]
                    .get(position - left.column_count)
                    .copied();
                match semantic.node(relation)?.stage {
                    Stage::Join {
                        nulls: Some(descriptor),
                        ..
                    } => original.and_then(|id| {
                        semantic
                            .null_extensions
                            .get(usize::from(descriptor))?
                            .output_for(id)
                    }),
                    Stage::Join { nulls: None, .. } => original,
                    _ => return Err(Error::Corrupt("join output semantic stage")),
                }
            }
        }
    };
    identity.ok_or(Error::Corrupt("physical position outside producer row"))
}
