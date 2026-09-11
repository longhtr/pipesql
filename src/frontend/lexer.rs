//! Bounded lexical analysis and the pinned identifier policy. No catalog or allocation authority.
use super::{
    Comparison, Error, MAX_NAME_BYTES, MAX_SOURCE_BYTES, MAX_TOKENS, SourceSpan, parse, span,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kind {
    Empty,
    Identifier,
    Reserved,
    Number,
    Quoted,
    From,
    Select,
    Where,
    As,
    Pipe,
    Comma,
    Dot,
    Join,
    Order,
    Limit,
    Distinct,
    Semicolon,
    Minus,
    Plus,
    Compare(Comparison),
    Aggregate,
    LeftParen,
    RightParen,
    Star,
}

#[derive(Clone, Copy)]
pub(super) struct Token {
    pub(super) kind: Kind,
    pub(super) span: SourceSpan,
}
pub(super) const ZERO_SPAN: SourceSpan = SourceSpan { start: 0, end: 0 };

pub(super) struct Tokens {
    pub(super) values: [Token; MAX_TOKENS],
    pub(super) len: usize,
}

impl Tokens {
    fn push(&mut self, kind: Kind, start: usize, end: usize) -> Result<(), Error> {
        if self.len == MAX_TOKENS {
            return Err(parse("token limit exceeded", start, end));
        }
        self.values[self.len] = Token {
            kind,
            span: span(start, end),
        };
        self.len += 1;
        Ok(())
    }
}

pub(super) fn lex(source: &str) -> Result<Tokens, Error> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(parse(
            "source exceeds 4096 bytes",
            MAX_SOURCE_BYTES,
            MAX_SOURCE_BYTES,
        ));
    }
    let bytes = source.as_bytes();
    let mut tokens = Tokens {
        values: [Token {
            kind: Kind::Empty,
            span: ZERO_SPAN,
        }; MAX_TOKENS],
        len: 0,
    };
    let mut cursor = 0;
    while cursor < bytes.len() {
        let start = cursor;
        let byte = bytes[cursor];
        if byte.is_ascii_whitespace() {
            cursor += 1;
            continue;
        }
        if byte == b'#' {
            while cursor < bytes.len() && bytes[cursor] != b'\n' {
                cursor += 1;
            }
            continue;
        }
        if byte.is_ascii_alphabetic() || byte == b'_' {
            cursor += 1;
            while cursor < bytes.len()
                && (bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == b'_')
            {
                cursor += 1;
            }
            if cursor - start > MAX_NAME_BYTES {
                return Err(parse("identifier exceeds 32 bytes", start, cursor));
            }
            let word = &source[start..cursor];
            let kind = if word.eq_ignore_ascii_case("FROM") {
                Kind::From
            } else if word.eq_ignore_ascii_case("SELECT") {
                Kind::Select
            } else if word.eq_ignore_ascii_case("WHERE") {
                Kind::Where
            } else if word.eq_ignore_ascii_case("AS") {
                Kind::As
            } else if word.eq_ignore_ascii_case("JOIN") {
                Kind::Join
            } else if word.eq_ignore_ascii_case("DISTINCT") {
                Kind::Distinct
            } else if word.eq_ignore_ascii_case("LIMIT") {
                Kind::Limit
            } else if word.eq_ignore_ascii_case("ORDER") {
                Kind::Order
            } else if word.eq_ignore_ascii_case("AGGREGATE") {
                Kind::Aggregate
            } else if reserved_identifier(word) {
                Kind::Reserved
            } else {
                Kind::Identifier
            };
            tokens.push(kind, start, cursor)?;
            continue;
        }
        if byte.is_ascii_digit() {
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                cursor += 1;
            }
            if bytes.get(cursor) == Some(&b'.') {
                cursor += 1;
                let first = cursor;
                while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                    cursor += 1;
                }
                if first == cursor {
                    return Err(parse("fraction requires digits", start, cursor));
                }
            }
            if matches!(bytes.get(cursor), Some(b'e' | b'E')) {
                cursor += 1;
                if matches!(bytes.get(cursor), Some(b'+' | b'-')) {
                    cursor += 1;
                }
                let first = cursor;
                while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                    cursor += 1;
                }
                if first == cursor {
                    return Err(parse("exponent requires digits", start, cursor));
                }
            }
            if cursor - start > MAX_NAME_BYTES {
                return Err(parse("literal exceeds 32 bytes", start, cursor));
            }
            tokens.push(Kind::Number, start, cursor)?;
            continue;
        }
        if matches!(byte, b'\'' | b'"') {
            let (_, length) = crate::text_literal::TextLiteral::parse(&source[start..])
                .map_err(|message| parse(message, start, start + 1))?;
            cursor += length;
            tokens.push(Kind::Quoted, start, cursor)?;
            continue;
        }
        let pair = bytes.get(cursor + 1).copied();
        let (kind, length) = match (byte, pair) {
            (b'|', Some(b'>')) => (Kind::Pipe, 2),
            (b'<', Some(b'=')) => (Kind::Compare(Comparison::LessEqual), 2),
            (b'>', Some(b'=')) => (Kind::Compare(Comparison::GreaterEqual), 2),
            (b'!', Some(b'=')) => (Kind::Compare(Comparison::NotEqual), 2),
            (b'<', _) => (Kind::Compare(Comparison::Less), 1),
            (b'>', _) => (Kind::Compare(Comparison::Greater), 1),
            (b'=', _) => (Kind::Compare(Comparison::Equal), 1),
            (b'(', _) => (Kind::LeftParen, 1),
            (b')', _) => (Kind::RightParen, 1),
            (b'*', _) => (Kind::Star, 1),
            (b',', _) => (Kind::Comma, 1),
            (b'.', _) => (Kind::Dot, 1),
            (b';', _) => (Kind::Semicolon, 1),
            (b'-', _) => (Kind::Minus, 1),
            (b'+', _) => (Kind::Plus, 1),
            _ => return Err(parse("unsupported source byte", start, start + 1)),
        };
        cursor += length;
        tokens.push(kind, start, cursor)?;
    }
    Ok(tokens)
}

pub(super) const RESERVED_IDENTIFIERS: &[&str] = &[
    "ALIGN",
    "ALL",
    "AND",
    "ANY",
    "ARRAY",
    "AS",
    "ASC",
    "ASSERT_ROWS_MODIFIED",
    "AT",
    "BETWEEN",
    "BY",
    "CASE",
    "CAST",
    "COLLATE",
    "CONTAINS",
    "CREATE",
    "CROSS",
    "CUBE",
    "CURRENT",
    "DEFAULT",
    "DEFINE",
    "DESC",
    "DISTINCT",
    "ELSE",
    "END",
    "ENUM",
    "ESCAPE",
    "EXCEPT",
    "EXCLUDE",
    "EXISTS",
    "EXTRACT",
    "FALSE",
    "FETCH",
    "FOLLOWING",
    "FOR",
    "FROM",
    "FULL",
    "GRAPH_TABLE",
    "GROUP",
    "GROUPING",
    "GROUPS",
    "HASH",
    "HAVING",
    "IF",
    "IGNORE",
    "IN",
    "INNER",
    "INTERSECT",
    "INTERVAL",
    "INTO",
    "IS",
    "JOIN",
    "LATERAL",
    "LEFT",
    "LIKE",
    "LIMIT",
    "LOOKUP",
    "MATCH_RECOGNIZE",
    "MERGE",
    "NATURAL",
    "NEW",
    "NO",
    "NOT",
    "NULL",
    "NULLS",
    "OF",
    "ON",
    "OR",
    "ORDER",
    "OUTER",
    "OVER",
    "PARTITION",
    "PRECEDING",
    "PROTO",
    "QUALIFY",
    "RANGE",
    "RECURSIVE",
    "RESPECT",
    "RIGHT",
    "ROLLUP",
    "ROWS",
    "SELECT",
    "SET",
    "SOME",
    "STRUCT",
    "TABLESAMPLE",
    "THEN",
    "TO",
    "TREAT",
    "TRUE",
    "UNBOUNDED",
    "UNION",
    "UNNEST",
    "USING",
    "WHEN",
    "WHERE",
    "WINDOW",
    "WITH",
    "WITHIN",
];

pub(super) fn reserved_identifier(word: &str) -> bool {
    RESERVED_IDENTIFIERS
        .iter()
        .any(|reserved| word.eq_ignore_ascii_case(reserved))
}
