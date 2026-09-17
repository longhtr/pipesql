//! Count the bytes or Unicode scalar values in a string.
//!
//! These counts differ: `é` is two UTF-8 bytes but one Unicode scalar value.
//! A letter followed by a combining accent contains two scalar values even if
//! it looks like one character. This module counts them separately.
//!
//! Query preparation uses this calculation for string literals; execution uses
//! it for stored strings. Both paths therefore interpret length the same way.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Unit {
    Bytes,
    // Count combining marks and joiners separately; do not combine them into
    // displayed characters or normalize different spellings of the same text.
    UnicodeScalars,
}

impl Unit {
    pub(crate) fn measure(self, text: &str) -> usize {
        match self {
            Self::Bytes => text.len(),
            // Each Rust char represents one Unicode scalar value.
            Self::UnicodeScalars => text.chars().count(),
        }
    }
}
