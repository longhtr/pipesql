//! Backward column demand shared by lowering and physical validation.
//! This is shared analysis, not an independent oracle for demanded evaluation.

use super::MAX_PIPELINES;
use crate::Error;
use crate::frontend::{self, ColumnId, RelationId, Stage};

pub(super) use crate::frontend::ColumnSet;

fn bit(id: ColumnId) -> ColumnSet {
    let mut set = ColumnSet::EMPTY;
    set.insert(id);
    set
}

pub(super) fn demand_masks(plan: &frontend::Plan) -> Result<[ColumnSet; MAX_PIPELINES], Error> {
    let mut masks = [ColumnSet::EMPTY; MAX_PIPELINES];
    let last = usize::from(plan.final_relation().0);
    for id in plan.outputs() {
        masks[last] |= bit(id);
    }
    for (index, node) in plan.nodes().iter().enumerate().rev() {
        let output = masks[index + 1];
        let input = usize::from(node.input.0);
        match node.stage {
            Stage::SetOperation { right, descriptor } => {
                let set = plan
                    .set_operations
                    .get(usize::from(descriptor))
                    .ok_or(Error::Corrupt("set demand descriptor"))?;
                for position in 0..usize::from(node.columns) {
                    let column = set
                        .output(position)
                        .ok_or(Error::Corrupt("set demand output"))?;
                    if set.kind() == frontend::SetKind::ExceptDistinct
                        || output.contains(column.identity())
                    {
                        let [left, right_column] = set
                            .inputs(position)
                            .ok_or(Error::Corrupt("set demand input"))?;
                        masks[input] |= bit(left.identity());
                        masks[usize::from(right.0)] |= bit(right_column.identity());
                    }
                }
            }
            Stage::Distinct(descriptor) => {
                for column in plan.distinct[usize::from(descriptor)].inputs() {
                    masks[input] |= bit(column.identity());
                }
            }
            Stage::Order { start, len } => {
                masks[input] |= output;
                for key in plan.order_items(start, len)? {
                    masks[input] |= bit(key.column);
                }
            }
            Stage::Source(_) => (),
            Stage::Alias
            | Stage::Rename
            | Stage::Drop { .. }
            | Stage::Derived
            | Stage::Limit(_) => masks[input] |= output,
            Stage::Select { .. } | Stage::Extend { .. } | Stage::Set { .. } => {
                for id in plan.available_columns(node.input)?.iter() {
                    masks[input] |= output & bit(id);
                }
                for definition in &plan.computed {
                    if definition.input == node.input
                        && output.contains(definition.column.identity())
                    {
                        for column in definition.expression.columns() {
                            masks[input] |= bit(column.identity());
                        }
                    }
                }
            }
            Stage::Where(filter) => masks[input] |= output | bit(filter.column),
            Stage::Join {
                nulls,
                right,
                left_key,
                right_key,
            } => {
                masks[input] |= bit(left_key);
                masks[usize::from(right.0)] |= bit(right_key);
                for id in plan.available_columns(node.input)?.iter() {
                    masks[input] |= output & bit(id);
                }
                for id in plan.available_columns(right)?.iter() {
                    let joined = if let Some(descriptor) = nulls {
                        plan.null_extensions
                            .get(usize::from(descriptor))
                            .and_then(|extension| extension.output_for(id))
                            .ok_or(Error::Corrupt("join demand null extension"))?
                    } else {
                        id
                    };
                    if output.contains(joined) {
                        masks[usize::from(right.0)] |= bit(id);
                    }
                }
            }
            Stage::Aggregate(aggregate_index) => {
                let aggregate = plan
                    .aggregates
                    .get(usize::from(aggregate_index))
                    .ok_or(Error::Corrupt("aggregate absent"))?;
                for key in aggregate.group_columns() {
                    masks[input] |= bit(key.identity());
                }
                for (entry, id) in plan
                    .relation_columns(RelationId(index as u8 + 1))?
                    .iter()
                    .skip(usize::from(aggregate.group_count))
                    .enumerate()
                {
                    if output.contains(id)
                        && let Some(argument) = &aggregate.entries[entry].argument
                    {
                        for column in argument.columns() {
                            masks[input] |= bit(column.identity());
                        }
                    }
                }
            }
            Stage::Empty => return Err(Error::Corrupt("empty bound producer")),
        }
    }
    Ok(masks)
}
