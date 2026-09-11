//! Bounded owned text constants. Query source is never retained by execution.
use std::str;

pub(crate) const MAX_LITERAL_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TextLiteral {
    bytes: [u8; MAX_LITERAL_BYTES],
    len: u8,
}

impl TextLiteral {
    pub(crate) fn valid(self) -> bool {
        let len = usize::from(self.len);
        len <= self.bytes.len()
            && str::from_utf8(&self.bytes[..len]).is_ok()
            && self.bytes[len..].iter().all(|byte| *byte == 0)
    }

    pub(crate) fn as_str(&self) -> &str {
        str::from_utf8(&self.bytes[..usize::from(self.len)]).expect("validated text literal")
    }

    /// Parse one quoted token and return its byte extent in the source suffix.
    /// Both source payload and decoded UTF-8 are bounded; escapes cannot hide work.
    pub(crate) fn parse(source: &str) -> Result<(Self, usize), &'static str> {
        let bytes = source.as_bytes();
        let quote = *bytes.first().ok_or("quoted literal required")?;
        if !matches!(quote, b'\'' | b'"') {
            return Err("quoted literal required");
        }
        if bytes.get(1) == Some(&quote) && bytes.get(2) == Some(&quote) {
            return Err("triple-quoted literals are unsupported");
        }
        let mut result = Self {
            bytes: [0; MAX_LITERAL_BYTES],
            len: 0,
        };
        let mut cursor = 1;
        while cursor < bytes.len() {
            if bytes[cursor] == quote {
                return Ok((result, cursor + 1));
            }
            let value = if bytes[cursor] == b'\\' {
                cursor += 1;
                let escape = *bytes.get(cursor).ok_or("unterminated escape")?;
                cursor += 1;
                match escape {
                    b'a' => '\u{7}',
                    b'b' => '\u{8}',
                    b'f' => '\u{c}',
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    b'v' => '\u{b}',
                    b'\\' => '\\',
                    b'?' => '?',
                    b'"' => '"',
                    b'\'' => '\'',
                    b'`' => '`',
                    b'0'..=b'3' => {
                        let tail = digits(bytes, &mut cursor, 2, 8)?;
                        char::from_u32(u32::from(escape - b'0') * 64 + tail)
                            .expect("octal byte range")
                    }
                    b'x' | b'X' | b'u' | b'U' => {
                        let count = match escape {
                            b'u' => 4,
                            b'U' => 8,
                            _ => 2,
                        };
                        char::from_u32(digits(bytes, &mut cursor, count, 16)?)
                            .ok_or("escape is not a Unicode scalar value")?
                    }
                    _ => return Err("invalid string escape"),
                }
            } else {
                let value = source[cursor..]
                    .chars()
                    .next()
                    .expect("source character boundary");
                if matches!(value, '\r' | '\n') {
                    return Err("newline in quoted literal");
                }
                cursor += value.len_utf8();
                value
            };
            if cursor - 1 > MAX_LITERAL_BYTES {
                return Err("literal payload exceeds 32 bytes");
            }
            let mut encoded = [0; 4];
            let encoded = value.encode_utf8(&mut encoded).as_bytes();
            let start = usize::from(result.len);
            let end = start + encoded.len(); // At most 32 retained bytes plus one scalar.
            if end > result.bytes.len() {
                return Err("decoded literal exceeds 32 bytes");
            }
            result.bytes[start..end].copy_from_slice(encoded);
            result.len = u8::try_from(end).expect("bounded literal length");
        }
        Err("unterminated quoted literal")
    }
}

fn digits(bytes: &[u8], cursor: &mut usize, count: usize, radix: u32) -> Result<u32, &'static str> {
    let mut value = 0u32;
    for _ in 0..count {
        let digit = char::from(*bytes.get(*cursor).ok_or("incomplete escape")?)
            .to_digit(radix)
            .ok_or("invalid escape digit")?;
        value = value
            .checked_mul(radix)
            .and_then(|v| v.checked_add(digit))
            .ok_or("escape value overflow")?;
        *cursor += 1;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_escape_vectors_and_payload_bounds() {
        // GoogleSQL 0e7d707: public/strings.cc and strings_test.cc.
        for (source, expected) in [
            ("''", ""),
            ("\"it's\"", "it's"),
            (r"'it\'s'", "it's"),
            ("'é'", "é"),
            (r"'e\u0301'", "e\u{301}"),
            (r"'\a\b\f\n\r\t\v'", "\u{7}\u{8}\u{c}\n\r\t\u{b}"),
            (r#"'\\\?\"\'\`'"#, "\\?\"'`"),
            (r"'\000\x00'", "\0\0"),
            (r"'\377\xFF\Xff'", "ÿÿÿ"),
            (r"'\x41B'", "AB"),
            (r"'\uD7FF\uE000'", "\u{d7ff}\u{e000}"),
            (r"'\U00010000\U0010FFFF'", "\u{10000}\u{10ffff}"),
        ] {
            let (literal, consumed) = TextLiteral::parse(source).unwrap();
            assert_eq!(consumed, source.len(), "{source}");
            assert!(literal.valid());
            assert_eq!(literal.as_str(), expected, "{source}");
        }
        for source in [
            r"'\400'",
            r"'\777'",
            r"'\38x'",
            r"'\0'",
            r"'\x1'",
            r"'\xgg'",
            r"'\uD800'",
            r"'\uDFFF'",
            r"'\U00110000'",
            r"'\UFFFFFFFF'",
            r"'\z'",
            "'unterminated",
            "'line\nnext'",
            "'line\rnext'",
            "'''triple'''",
            "r'raw'",
        ] {
            assert!(TextLiteral::parse(source).is_err(), "{source}");
        }
        for text in ["a".repeat(32), "é".repeat(16), "\u{10ffff}".repeat(8)] {
            let (literal, _) = TextLiteral::parse(&format!("'{text}'")).unwrap();
            assert_eq!(literal.as_str(), text);
            assert!(TextLiteral::parse(&format!("'{text}a'")).is_err());
        }
        assert!(TextLiteral::parse(&format!("'{}'", r"\u0061".repeat(6))).is_err());
        let mut invalid = TextLiteral {
            bytes: [0; MAX_LITERAL_BYTES],
            len: 33,
        };
        assert!(!invalid.valid());
        invalid.len = 1;
        invalid.bytes[0] = 255;
        assert!(!invalid.valid());
        invalid.bytes[0] = b'a';
        invalid.bytes[2] = 1;
        assert!(!invalid.valid());
    }
}
