//! Backward column demand shared by lowering and physical validation.
//! This is shared analysis, not an independent oracle for demanded evaluation.

use super::MAX_PIPELINES;
use crate::Error;
use crate::frontend::{self, ColumnId, MAX_QUERY_COLUMNS, RelationId, Stage};
use crate::scalar::Op;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ColumnSet([u64; (MAX_QUERY_COLUMNS + 1).div_ceil(64)]);

impl ColumnSet {
    pub(super) const EMPTY: Self = Self([0; (MAX_QUERY_COLUMNS + 1).div_ceil(64)]);

    pub(super) fn insert(&mut self, id: ColumnId) {
        *self |= bit(id);
    }

    pub(super) fn contains(self, id: ColumnId) -> bool {
        self & bit(id) != Self::EMPTY
    }
}

impl std::ops::BitOr for ColumnSet {
    type Output = Self;

    fn bitor(mut self, rhs: Self) -> Self {
        self |= rhs;
        self
    }
}

impl std::ops::BitOrAssign for ColumnSet {
    fn bitor_assign(&mut self, rhs: Self) {
        for (left, right) in self.0.iter_mut().zip(rhs.0) {
            *left |= right;
        }
    }
}

impl std::ops::BitAnd for ColumnSet {
    type Output = Self;

    fn bitand(mut self, rhs: Self) -> Self {
        for (left, right) in self.0.iter_mut().zip(rhs.0) {
            *left &= right;
        }
        self
    }
}

fn bit(id: ColumnId) -> ColumnSet {
    let id = id.value() as usize;
    assert!(id > 0 && id <= MAX_QUERY_COLUMNS);
    let mut set = ColumnSet::EMPTY;
    set.0[id / 64] = 1 << (id % 64);
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
            Stage::Alias | Stage::Derived | Stage::Limit(_) => masks[input] |= output,
            Stage::Select { .. } => {
                for id in plan.relation_columns(node.input)?.iter() {
                    masks[input] |= output & bit(id);
                }
                for definition in &plan.computed {
                    if definition.input == node.input
                        && output.contains(definition.column.identity())
                    {
                        for op in
                            &definition.expression.ops[..usize::from(definition.expression.len)]
                        {
                            if let Op::Column(column) = op {
                                masks[input] |= bit(column.identity());
                            }
                        }
                    }
                }
            }
            Stage::Where(filter) => masks[input] |= output | bit(filter.column),
            Stage::Join {
                right,
                left_key,
                right_key,
            } => {
                masks[input] |= bit(left_key);
                masks[usize::from(right.0)] |= bit(right_key);
                for id in plan.relation_columns(node.input)?.iter() {
                    masks[input] |= output & bit(id);
                }
                for id in plan.relation_columns(right)?.iter() {
                    masks[usize::from(right.0)] |= output & bit(id);
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
                        && let Some(expression) = &aggregate.entries[entry].expression
                    {
                        for op in &expression.ops[..usize::from(expression.len)] {
                            if let Op::Column(column) = op {
                                masks[input] |= bit(column.identity());
                            }
                        }
                    }
                }
            }
            Stage::Empty => return Err(Error::Corrupt("empty bound producer")),
        }
    }
    Ok(masks)
}
