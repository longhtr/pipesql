//! Borrowed scalar values shared by batches, kernels, and public results.

/// Validated UTF-8 borrowed from a result batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StringValue<'value> {
    text: &'value str,
}

impl<'value> StringValue<'value> {
    pub(crate) fn new(text: &'value str) -> Self {
        Self { text }
    }

    /// Borrow the text for the lifetime of the source batch.
    pub fn as_str(self) -> &'value str {
        self.text
    }
}

#[cfg(test)]
impl StringValue<'static> {
    pub(crate) fn from_byte(byte: u8) -> Option<Self> {
        crate::fixed_text::StringValue::from_byte(byte).map(|key| Self::new(key.as_str()))
    }
}

pub use crate::date::DateValue;

/// A scalar cell. Text borrows batch storage; other payloads are copied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value<'value> {
    Null,
    Int64(i64),
    Double(f64),
    String(StringValue<'value>),
    Date(DateValue),
}
