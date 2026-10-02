//! Read Parquet's compact metadata without allocating an object tree.
//!
//! Callers select the fields they need and validate their meaning. Unknown fields
//! are skipped with a fixed container stack, so nested input cannot grow the Rust
//! stack. Every length and integer is checked before advancing the byte cursor.
//!
//! The reader carries a file offset and cancellation token so malformed fields can
//! be located and long metadata scans interrupted. Accepting a wire representation
//! does not establish that its schema, encoding or referenced page is supported;
//! those decisions belong to metadata and page validation.

use crate::{CancellationToken, Error};

pub(super) const BOOL_TRUE: u8 = 1;
pub(super) const BOOL_FALSE: u8 = 2;
pub(super) const I16: u8 = 4;
pub(super) const I32: u8 = 5;
pub(super) const I64: u8 = 6;
pub(super) const BINARY: u8 = 8;
pub(super) const LIST: u8 = 9;
pub(super) const STRUCT: u8 = 12;

#[derive(Clone, Copy)]
pub(super) struct Field {
    pub(super) id: i16,
    pub(super) kind: u8,
    offset: u64,
}

impl Field {
    pub(super) fn require(self, kind: u8) -> Result<(), Error> {
        if self.kind != kind {
            return Err(Error::Input {
                message: "Parquet metadata field has the wrong wire type",
                byte_offset: self.offset,
            });
        }
        Ok(())
    }

    pub(super) fn integer(self, reader: &mut Reader<'_, '_>, kind: u8) -> Result<i64, Error> {
        self.require(kind)?;
        reader.integer(kind)
    }

    pub(super) fn list(self, reader: &mut Reader<'_, '_>, item: u8) -> Result<u32, Error> {
        self.require(LIST)?;
        let (kind, length) = reader.list()?;
        if kind != item {
            return Err(reader.error("Parquet metadata list has the wrong element type"));
        }
        Ok(length)
    }
}

// Each Parquet structure has its own field numbering. Retain only presence bits
// for known field ids, not a heap-allocated map of arbitrary input fields.
pub(super) struct Fields {
    previous: i16,
    seen: u64,
}

impl Fields {
    pub(super) fn new() -> Self {
        Self {
            previous: 0,
            seen: 0,
        }
    }

    pub(super) fn next(&mut self, reader: &mut Reader<'_, '_>) -> Result<Option<Field>, Error> {
        let field = reader.field(&mut self.previous)?;
        if let Some(field) = field
            && (1..=64).contains(&field.id)
        {
            let bit = 1 << (field.id - 1);
            if self.seen & bit != 0 {
                return Err(reader.error("duplicate Parquet metadata field"));
            }
            self.seen |= bit;
        }
        Ok(field)
    }

    pub(super) fn require(&self, ids: &[u8], reader: &Reader<'_, '_>) -> Result<(), Error> {
        if ids.iter().any(|id| self.seen & (1 << (id - 1)) == 0) {
            return Err(reader.error("missing required Parquet metadata field"));
        }
        Ok(())
    }
}

pub(super) struct Reader<'bytes, 'cancel> {
    bytes: &'bytes [u8],
    position: usize,
    base: u64,
    cancel: &'cancel CancellationToken,
    header_limit: Option<usize>,
}

impl<'bytes, 'cancel> Reader<'bytes, 'cancel> {
    pub(super) fn new(
        bytes: &'bytes [u8],
        base: u64,
        cancel: &'cancel CancellationToken,
    ) -> Result<Self, Error> {
        cancel.check()?;
        if base.checked_add(bytes.len() as u64).is_none() {
            return Err(Error::Input {
                message: "Parquet metadata byte range overflows",
                byte_offset: base,
            });
        }
        Ok(Self {
            bytes,
            position: 0,
            base,
            cancel,
            header_limit: None,
        })
    }

    pub(super) fn position(&self) -> usize {
        self.position
    }

    pub(super) fn limit_page_header(&mut self) {
        self.header_limit = Some(65_536);
    }

    pub(super) fn offset(&self) -> u64 {
        self.base + self.position as u64
    }

    pub(super) fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    pub(super) fn error(&self, message: &'static str) -> Error {
        Error::Input {
            message,
            byte_offset: self.offset(),
        }
    }

    pub(super) fn take(&mut self, length: usize) -> Result<&'bytes [u8], Error> {
        self.cancel.check()?;
        if length > self.remaining() {
            return Err(self.error("truncated Parquet metadata"));
        }
        if let Some(limit) = self.header_limit {
            super::bound(
                (self.position + length) as u64,
                limit as u64,
                "Parquet page header",
            )?;
        }
        let start = self.position;
        self.position += length;
        Ok(&self.bytes[start..self.position])
    }

    fn byte(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }

    fn unsigned(&mut self, bits: u32) -> Result<u64, Error> {
        let mut value = 0;
        for shift in (0..bits).step_by(7) {
            let byte = self.byte()?;
            let payload = u64::from(byte & 127);
            let remaining = bits - shift;
            if remaining < 7 && payload >= 1 << remaining {
                return Err(self.error("Parquet metadata integer overflows its wire type"));
            }
            value |= payload << shift;
            if byte & 128 == 0 {
                return Ok(value);
            }
        }
        Err(self.error("unterminated Parquet metadata integer"))
    }

    pub(super) fn integer(&mut self, kind: u8) -> Result<i64, Error> {
        let bits = match kind {
            I16 => 16,
            I32 => 32,
            I64 => 64,
            _ => return Err(self.error("expected a Parquet metadata integer")),
        };
        let value = self.unsigned(bits)?;
        Ok(((value >> 1) as i64) ^ -((value & 1) as i64))
    }

    fn length(&mut self) -> Result<u32, Error> {
        let length = self.unsigned(32)?;
        if length > i32::MAX as u64 {
            return Err(self.error("Parquet metadata length exceeds signed 32-bit range"));
        }
        Ok(length as u32)
    }

    pub(super) fn binary(&mut self) -> Result<&'bytes [u8], Error> {
        let length = self.length()?;
        self.take(length as usize)
    }

    pub(super) fn field(&mut self, previous: &mut i16) -> Result<Option<Field>, Error> {
        let offset = self.offset();
        let header = self.byte()?;
        if header == 0 {
            return Ok(None);
        }
        let kind = header & 15;
        self.valid_kind(kind)?;
        let delta = i16::from(header >> 4);
        let id = if delta == 0 {
            self.integer(I16)? as i16
        } else {
            previous
                .checked_add(delta)
                .ok_or_else(|| self.error("Parquet field id overflows"))?
        };
        *previous = id;
        Ok(Some(Field { id, kind, offset }))
    }

    pub(super) fn list(&mut self) -> Result<(u8, u32), Error> {
        let header = self.byte()?;
        let kind = header & 15;
        self.valid_kind(kind)?;
        let short = u32::from(header >> 4);
        let length = if short == 15 { self.length()? } else { short };
        // Even an empty struct or binary value needs one wire byte. This rejects
        // impossible item counts before a loop can trust them as work bounds.
        if length as usize > self.remaining() {
            return Err(self.error("Parquet metadata list exceeds remaining bytes"));
        }
        Ok((kind, length))
    }

    fn valid_kind(&self, kind: u8) -> Result<(), Error> {
        if !(BOOL_TRUE..=13).contains(&kind) {
            return Err(self.error("unknown Parquet metadata wire type"));
        }
        Ok(())
    }

    pub(super) fn skip(&mut self, kind: u8, parents: usize) -> Result<(), Error> {
        let limit = 16_usize
            .checked_sub(parents)
            .ok_or_else(|| self.error("Parquet metadata nesting exceeds 16 containers"))?;
        let Some(first) = self.begin_skip(kind, true)? else {
            return Ok(());
        };
        let mut stack = [Container::Struct { previous: 0 }; 16];
        if limit == 0 {
            return Err(self.error("Parquet metadata nesting exceeds 16 containers"));
        }
        stack[0] = first;
        let mut depth = 1;
        while depth != 0 {
            self.cancel.check()?;
            let next = match &mut stack[depth - 1] {
                Container::Struct { previous } => {
                    self.field(previous)?.map(|field| (field.kind, true))
                }
                Container::Sequence { kind, remaining } => {
                    if *remaining == 0 {
                        None
                    } else {
                        *remaining -= 1;
                        Some((*kind, false))
                    }
                }
                Container::Map {
                    key,
                    value,
                    remaining,
                    key_next,
                } => {
                    if *remaining == 0 {
                        None
                    } else {
                        let kind = if *key_next { *key } else { *value };
                        if !*key_next {
                            *remaining -= 1;
                        }
                        *key_next = !*key_next;
                        Some((kind, false))
                    }
                }
            };
            let Some((kind, inline_bool)) = next else {
                depth -= 1;
                continue;
            };
            if let Some(container) = self.begin_skip(kind, inline_bool)? {
                if depth == limit {
                    return Err(self.error("Parquet metadata nesting exceeds 16 containers"));
                }
                stack[depth] = container;
                depth += 1;
            }
        }
        Ok(())
    }

    fn begin_skip(&mut self, kind: u8, inline_bool: bool) -> Result<Option<Container>, Error> {
        match kind {
            BOOL_TRUE | BOOL_FALSE => {
                if !inline_bool && !matches!(self.byte()?, BOOL_TRUE | BOOL_FALSE) {
                    return Err(self.error("invalid Parquet metadata boolean"));
                }
            }
            3 => {
                self.take(1)?;
            }
            I16 | I32 | I64 => {
                self.integer(kind)?;
            }
            7 => {
                self.take(8)?;
            }
            BINARY => {
                self.binary()?;
            }
            LIST | 10 => {
                let (kind, remaining) = self.list()?;
                return Ok(Some(Container::Sequence { kind, remaining }));
            }
            11 => {
                let remaining = self.length()?;
                if remaining == 0 {
                    return Ok(None);
                }
                let kinds = self.byte()?;
                let (key, value) = (kinds >> 4, kinds & 15);
                self.valid_kind(key)?;
                self.valid_kind(value)?;
                if remaining as usize > self.remaining() / 2 {
                    return Err(self.error("Parquet metadata map exceeds remaining bytes"));
                }
                return Ok(Some(Container::Map {
                    key,
                    value,
                    remaining,
                    key_next: true,
                }));
            }
            STRUCT => return Ok(Some(Container::Struct { previous: 0 })),
            13 => {
                self.take(16)?;
            }
            _ => return Err(self.error("unknown Parquet metadata wire type")),
        }
        Ok(None)
    }
}

#[derive(Clone, Copy)]
enum Container {
    Struct {
        previous: i16,
    },
    Sequence {
        kind: u8,
        remaining: u32,
    },
    Map {
        key: u8,
        value: u8,
        remaining: u32,
        key_next: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_fields_lists_binary_and_signed_extremes() {
        let cancel = CancellationToken::new();
        let bytes = [0x15, 0x0e, 0x28, 3, b'a', 0, b'b', 0x19, 0x25, 1, 2, 0];
        let mut reader = Reader::new(&bytes, 100, &cancel).unwrap();
        let mut previous = 0;
        let field = reader.field(&mut previous).unwrap().unwrap();
        assert_eq!(field.id, 1);
        field.require(I32).unwrap();
        assert_eq!(reader.integer(I32).unwrap(), 7);
        let field = reader.field(&mut previous).unwrap().unwrap();
        assert_eq!(field.id, 3);
        field.require(BINARY).unwrap();
        assert_eq!(reader.binary().unwrap(), b"a\0b");
        let field = reader.field(&mut previous).unwrap().unwrap();
        assert_eq!(field.id, 4);
        field.require(LIST).unwrap();
        assert_eq!(reader.list().unwrap(), (I32, 2));
        assert_eq!(reader.integer(I32).unwrap(), -1);
        assert_eq!(reader.integer(I32).unwrap(), 1);
        assert!(reader.field(&mut previous).unwrap().is_none());
        assert_eq!(reader.position(), bytes.len());
        assert_eq!(reader.offset(), 112);
        let mut reader = Reader::new(
            &[255, 255, 255, 255, 255, 255, 255, 255, 255, 1],
            0,
            &cancel,
        )
        .unwrap();
        assert_eq!(reader.integer(I64).unwrap(), i64::MIN);
        let mut reader = Reader::new(
            &[254, 255, 255, 255, 255, 255, 255, 255, 255, 1],
            0,
            &cancel,
        )
        .unwrap();
        assert_eq!(reader.integer(I64).unwrap(), i64::MAX);
    }

    #[test]
    fn unknown_containers_skip_without_losing_the_next_field() {
        let cancel = CancellationToken::new();
        // Field 1: a map with binary keys and lists of booleans as values.
        // Field 2: a struct containing a true field. Field 3: integer 42.
        let bytes = [
            0x1b, 1, 0x89, 1, b'k', 0x21, 1, 2, 0x1c, 0x11, 0, 0x15, 84, 0,
        ];
        let mut reader = Reader::new(&bytes, 0, &cancel).unwrap();
        let mut previous = 0;
        for id in [1, 2] {
            let field = reader.field(&mut previous).unwrap().unwrap();
            assert_eq!(field.id, id);
            reader.skip(field.kind, 1).unwrap();
        }
        let field = reader.field(&mut previous).unwrap().unwrap();
        assert_eq!(field.id, 3);
        assert_eq!(reader.integer(field.kind).unwrap(), 42);
        assert!(reader.field(&mut previous).unwrap().is_none());
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn truncation_overflow_wrong_types_and_deep_metadata_fail() {
        let cancel = CancellationToken::new();
        for bytes in [
            &[128][..],
            &[255, 255, 255, 255, 16],
            &[128, 128, 128, 128, 128],
        ] {
            let mut reader = Reader::new(bytes, 20, &cancel).unwrap();
            assert!(
                matches!(reader.integer(I32), Err(Error::Input { byte_offset, .. }) if byte_offset >= 20)
            );
        }
        let mut reader = Reader::new(&[0x18, 5, b'a'], 0, &cancel).unwrap();
        let field = reader.field(&mut 0).unwrap().unwrap();
        assert!(field.require(I64).is_err());
        assert!(reader.binary().is_err());
        let mut reader = Reader::new(&[0x1c; 17], 0, &cancel).unwrap();
        assert!(matches!(
            reader.skip(STRUCT, 0),
            Err(Error::Input {
                message: "Parquet metadata nesting exceeds 16 containers",
                ..
            })
        ));
        assert!(Reader::new(&[0], u64::MAX, &cancel).is_err());
        cancel.cancel();
        assert!(matches!(
            Reader::new(&[], 0, &cancel),
            Err(Error::Cancelled)
        ));
    }
}
