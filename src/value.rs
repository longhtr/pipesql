//! Values read from query results or decoded input batches.
//!
//! `Value` represents one cell: a number, a date, text or SQL NULL. NULL means
//! there is no value; it is different from zero and from an empty string.
//!
//! Numbers and dates are copied into the cell. Text is borrowed from the result
//! batch, so reading it does not allocate another string. Copy text into storage
//! of your own if you need to keep it after releasing the batch.

/// The type of a non-NULL value. Whether a column permits NULL is recorded separately.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataType {
    Double,
    Int64,
    String,
    Date,
}

/// Text borrowed from a result batch, already checked to be valid UTF-8.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StringValue<'value> {
    text: &'value str,
}

impl<'value> StringValue<'value> {
    pub(crate) fn new(text: &'value str) -> Self {
        Self { text }
    }

    /// Read the text without copying it. The slice cannot outlive its source batch.
    pub fn as_str(self) -> &'value str {
        self.text
    }
}

#[cfg(test)]
impl StringValue<'static> {
    pub(crate) fn from_byte(byte: u8) -> Option<Self> {
        crate::value::fixed_text::StringValue::from_byte(byte).map(|key| Self::new(key.as_str()))
    }
}

pub(crate) mod date;
pub(crate) mod fixed_text;
pub use date::DateValue;

/// One typed cell. Copying a STRING cell copies its reference, not its text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value<'value> {
    Null,
    Int64(i64),
    Double(f64),
    String(StringValue<'value>),
    Date(DateValue),
}
