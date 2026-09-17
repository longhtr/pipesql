//! Store the legacy lineitem loader's one-character grouping keys.
//!
//! Keys can be printable ASCII, including space, but cannot be `|`, the input
//! delimiter. There are 94 accepted bytes. Each maps to an array position so
//! grouping can keep one accumulator per possible key without allocating strings.
//!
//! This representation serves the fixed-schema loader. Declared tables use the
//! general UTF-8 strings in `value`.

pub(crate) const KEY_DOMAIN: usize = 94;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StringValue {
    byte: u8,
}

impl StringValue {
    pub(crate) fn from_byte(byte: u8) -> Option<Self> {
        ((b' '..=b'~').contains(&byte) && byte != b'|').then_some(Self { byte })
    }

    pub(crate) fn byte(self) -> u8 {
        self.byte
    }

    // Leave no empty array slot for the forbidden delimiter. Bytes after `|`
    // move down one position; `from_index` adds that position back.
    pub(crate) fn index(self) -> usize {
        usize::from(self.byte - b' ') - usize::from(self.byte > b'|')
    }

    pub(crate) fn from_index(index: usize) -> Option<Self> {
        if index >= KEY_DOMAIN {
            return None;
        }
        let mut byte = u8::try_from(index).ok()?.checked_add(b' ')?;
        if byte >= b'|' {
            byte = byte.checked_add(1)?;
        }
        Self::from_byte(byte)
    }

    // The constructor accepts only ASCII, so one byte is one complete character.
    // Borrowing this static table avoids allocating a separate string.
    pub fn as_str(&self) -> &'static str {
        const ASCII: &str = " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";
        let start = usize::from(self.byte - b' ');
        &ASCII[start..start + 1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_and_dense_index_matches_the_input_contract() {
        let mut accepted = Vec::new();
        for byte in u8::MIN..=u8::MAX {
            let expected = (0x20..=0x7e).contains(&byte) && byte != 0x7c;
            let actual = StringValue::from_byte(byte);
            assert_eq!(actual.is_some(), expected, "byte {byte}");
            if let Some(value) = actual {
                assert_eq!(value.byte(), byte);
                assert_eq!(value.as_str().as_bytes(), &[byte]);
                assert_eq!(value.index(), accepted.len());
                assert_eq!(StringValue::from_index(accepted.len()), Some(value));
                accepted.push(byte);
            }
        }
        assert_eq!(accepted.len(), KEY_DOMAIN);
        for invalid in [KEY_DOMAIN, KEY_DOMAIN + 1, usize::MAX] {
            assert_eq!(StringValue::from_index(invalid), None);
        }
    }
}
