//! Measure UTF-8 text in bytes or Unicode scalar values without copying it.
//!
//! For example, `é` occupies two UTF-8 bytes but one scalar value. A displayed
//! character can contain several scalars, such as a letter and a combining mark;
//! this module does not group them into user-perceived characters. Binding uses
//! the same measurement for literals that execution uses for stored text.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Unit {
    Bytes,
    // Includes combining marks and joiners; does not normalize or segment graphemes.
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
