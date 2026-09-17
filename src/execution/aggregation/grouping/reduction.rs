//! Turn sorted rows into one completed aggregate group at a time.
//!
//! Rows with equal keys arrive together, ordered by their original row numbers.
//! The reducer copies their captured arguments into a bounded buffer, then folds
//! that buffer into one reusable accumulator. Filling the buffer does not end a
//! group; a different key or the end of input does.
//!
//! `step` returns Group while the completed values and key are available through
//! `value`. The caller must consume them before stepping again, which clears the
//! accumulator for the next group. The next group's first record stays pending
//! in the sort cursor, so finding a boundary does not lose a row. Empty input
//! produces no group.
//!
//! Reversed keys or repeated row numbers within a group are corrupt input. Read
//! failures and cancellation also leave the reducer failed. It borrows all
//! buffers and file access from the sorter and caller, which retain cleanup duty.

use crate::execution::aggregation::accumulator::AggregateState;
use crate::execution::aggregation::arguments::ArgumentBatch;
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
                let full = match next {
                    Some(record) => !arguments.can_append(record)?,
                    None => false,
                };
                if boundary || full {
                    // Do not consume next yet: it belongs to a new group or needs
                    // the space freed by folding the captured rows.
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
                // The Group return gave the caller a chance to read the result.
                // Clearing now cannot overwrite a result it is still borrowing.
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
