//! Borrow decoded rows and caller-owned columns without copying their values.
//!
//! Decoders retain numeric cells and validated text bytes. InputBatch lends rows
//! in declaration order until the decoder advances; text cells point into its
//! storage. Copy a value explicitly if it must survive the next decoder step.
//!
//! ColumnInput lends typed slices and validity bits during an append write.
//! Storage attaches persistent column identities after checking declaration order.
//! Neither view owns buffers or decides whether the complete input succeeded.

use crate::{DataType, DateValue, StringValue, Value};

#[derive(Clone, Copy)]
pub(crate) enum Cell {
    Null,
    Int64(i64),
    Double(f64),
    Date(DateValue),
    Text { start: usize, end: usize },
}

/// A decoded input batch in declaration order, borrowed from its decoder.
/// A valid batch is a prefix, not confirmation that the entire input is valid.
pub struct InputBatch<'a> {
    pub(crate) cells: &'a [Cell],
    pub(crate) text: &'a [u8],
    pub(crate) columns: usize,
}

impl InputBatch<'_> {
    pub fn row_count(&self) -> usize {
        self.cells.len() / self.columns
    }

    pub fn column_count(&self) -> usize {
        self.columns
    }

    /// Return a typed cell, or `None` for an out-of-range row or column.
    pub fn value(&self, row: usize, column: usize) -> Option<Value<'_>> {
        if row >= self.row_count() || column >= self.columns {
            return None;
        }
        Some(match self.cells[row * self.columns + column] {
            Cell::Null => Value::Null,
            Cell::Int64(value) => Value::Int64(value),
            Cell::Double(value) => Value::Double(value),
            Cell::Date(value) => Value::Date(value),
            Cell::Text { start, end } => Value::String(StringValue::new(
                std::str::from_utf8(&self.text[start..end]).expect("validated input text"),
            )),
        })
    }
}

/// Typed values borrowed for one [`crate::Append::write`] call.
///
/// Public callers use the name [`crate::ColumnValues`]. Every column in a batch
/// must contain the same number of values, including slots marked NULL by
/// [`crate::ColumnInput::validity`]. The call borrows these slices; the caller can
/// reuse their buffers after `write` returns.
#[derive(Clone, Copy)]
pub enum ColumnValues<'a> {
    Int64(&'a [i64]),
    Double(&'a [f64]),
    String(&'a [&'a str]),
    Date(&'a [DateValue]),
}

impl ColumnValues<'_> {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Int64(v) => v.len(),
            Self::Double(v) => v.len(),
            Self::String(v) => v.len(),
            Self::Date(v) => v.len(),
        }
    }

    pub(crate) fn kind(&self) -> DataType {
        match self {
            Self::Int64(_) => DataType::Int64,
            Self::Double(_) => DataType::Double,
            Self::String(_) => DataType::String,
            Self::Date(_) => DataType::Date,
        }
    }
}

/// One batch column, borrowed during `Append::write` in declaration order.
#[derive(Clone, Copy)]
pub struct ColumnInput<'a> {
    /// Values of the declared type, with the same row count as every other column.
    pub values: ColumnValues<'a>,
    /// One bit per row, least significant bit first: 1 is present, 0 is NULL.
    /// Supply exactly `ceil(rows / 8)` bytes with unused trailing bits clear.
    /// Nonnullable columns require every row to be present.
    pub validity: &'a [u8],
}
