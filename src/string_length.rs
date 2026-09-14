//! Pure measurements of validated UTF-8 shared by literal folding and execution.
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
