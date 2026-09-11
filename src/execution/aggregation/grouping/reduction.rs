//! Fold globally sorted arguments into one reusable aggregate cell.
//! The sorter retains its buffers; a completed group keeps its key until consumed.
use crate::execution::aggregation::arguments::ArgumentBatch;
use crate::execution::aggregation::numeric::AggregateState;
use crate::execution::blocking::{Io, RowLayout, RowSort, append_bytes};
use crate::{Error, Value};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reduction {
    Start,
    Input,
    Group,
    Done,
    Failed,
}

impl Reduction {
    pub(super) fn step(
        &mut self,
        sorted: &mut RowSort<'_>,
        arguments: &mut ArgumentBatch<'_>,
        aggregate: &mut AggregateState<'_>,
        keys: &RowLayout,
        io: &mut Io<'_, '_>,
    ) -> Result<Self, Error> {
        let phase = std::mem::replace(self, Self::Failed);
        if phase == Self::Done {
            *self = Self::Done;
            return Ok(*self);
        }
        io.cancel.check()?;
        let input = sorted.sorted_rows();
        *self = match phase {
            Self::Start => {
                if aggregate.cells.counts.len() != 1 {
                    return Err(Error::Corrupt("reduction requires one reusable group"));
                }
                arguments.check_program(aggregate)?;
                if arguments.shape != input.arguments {
                    return Err(Error::Corrupt("reduction argument plan differs"));
                }
                input.cursor.begin(input.run);
                input.previous_key.clear();
                *input.previous_ordinal = None;
                arguments.clear();
                aggregate.cells.clear_group(0);
                Self::Input
            }
            Self::Input => {
                input.cursor.load(input.slot, keys, input.arguments, io)?;
                let next = input.cursor.record();
                let boundary = if let (Some(ordinal), Some(next)) = (*input.previous_ordinal, next)
                {
                    let order = keys.compare(input.previous_key, next.key())?;
                    if order == Ordering::Greater
                        || (order == Ordering::Equal && ordinal >= next.ordinal())
                    {
                        return Err(Error::Corrupt("reduction input is not ordered"));
                    }
                    order == Ordering::Less
                } else {
                    next.is_none()
                };
                if boundary || arguments.rows == arguments.capacity {
                    arguments.fold_group(aggregate, 0)?;
                    arguments.clear();
                    if boundary {
                        if input.previous_ordinal.is_some() {
                            Self::Group
                        } else {
                            Self::Done
                        }
                    } else {
                        Self::Input
                    }
                } else {
                    let record = next.expect("a non-boundary step has an input record");
                    if input.previous_ordinal.is_none() {
                        append_bytes(input.previous_key, record.key())?;
                    }
                    arguments.append(record)?;
                    *input.previous_ordinal = Some(record.ordinal());
                    input.cursor.consume()?;
                    Self::Input
                }
            }
            Self::Group => {
                // The caller has consumed this group's private values. Leave a
                // different group's loaded record pending until the next step.
                input.previous_key.clear();
                *input.previous_ordinal = None;
                aggregate.cells.clear_group(0);
                Self::Input
            }
            Self::Failed => return Err(Error::Corrupt("reduction is not active")),
            Self::Done => unreachable!("handled before cancellation"),
        };
        Ok(*self)
    }

    pub(super) fn value<'a>(
        &self,
        sorted: &'a RowSort<'_>,
        aggregate: &'a AggregateState<'_>,
        keys: &RowLayout,
        column: usize,
    ) -> Result<Value<'a>, Error> {
        assert_eq!(
            *self,
            Self::Group,
            "only a completed private group has values"
        );
        if column < keys.count {
            keys.value(sorted.previous_key(), column)
        } else {
            aggregate.value(0, column - keys.count)
        }
    }
}
