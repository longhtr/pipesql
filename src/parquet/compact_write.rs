//! Encode compact metadata into caller-admitted storage.
//!
//! Each structure's caller keeps its previous field number, so entering a nested
//! structure does not obscure the outer field order. Output cannot grow the
//! buffer: every write checks the remaining capacity before copying bytes.
//!
//! The writer returns the used extent for the page or footer owner to publish.
//! Insufficient capacity fails with the supplied resource-owner label; this encoder
//! cannot allocate a larger buffer or write a partial metadata object to the sink.

use super::bound;
use super::compact::{BINARY, LIST, STRUCT};
use crate::Error;

pub(super) struct Writer<'a> {
    bytes: &'a mut [u8],
    position: usize,
    owner: &'static str,
}

impl<'a> Writer<'a> {
    pub(super) fn new(bytes: &'a mut [u8], owner: &'static str) -> Self {
        Self {
            bytes,
            position: 0,
            owner,
        }
    }

    pub(super) fn length(&self) -> usize {
        self.position
    }

    pub(super) fn raw(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let required = self
            .position
            .checked_add(bytes.len())
            .ok_or(Error::Unsupported(
                "Parquet encoded size exceeds addressable storage",
            ))?;
        bound(required as u64, self.bytes.len() as u64, self.owner)?;
        self.bytes[self.position..required].copy_from_slice(bytes);
        self.position = required;
        Ok(())
    }

    pub(super) fn unsigned(&mut self, mut value: u64) -> Result<(), Error> {
        while value >= 128 {
            self.raw(&[(value as u8 & 127) | 128])?;
            value >>= 7;
        }
        self.raw(&[value as u8])
    }

    pub(super) fn integer(&mut self, value: i64) -> Result<(), Error> {
        self.unsigned(((value as u64) << 1) ^ ((value >> 63) as u64))
    }

    fn field(&mut self, id: i16, previous: &mut i16, kind: u8) -> Result<(), Error> {
        let delta = i32::from(id) - i32::from(*previous);
        if (1..=15).contains(&delta) {
            self.raw(&[((delta as u8) << 4) | kind])?;
        } else {
            self.raw(&[kind])?;
            self.integer(i64::from(id))?;
        }
        *previous = id;
        Ok(())
    }

    pub(super) fn integer_field(
        &mut self,
        id: i16,
        previous: &mut i16,
        kind: u8,
        value: i64,
    ) -> Result<(), Error> {
        self.field(id, previous, kind)?;
        self.integer(value)
    }

    pub(super) fn binary_field(
        &mut self,
        id: i16,
        previous: &mut i16,
        bytes: &[u8],
    ) -> Result<(), Error> {
        self.field(id, previous, BINARY)?;
        self.binary(bytes)
    }

    pub(super) fn binary(&mut self, bytes: &[u8]) -> Result<(), Error> {
        bound(bytes.len() as u64, i32::MAX as u64, "Parquet binary length")?;
        self.unsigned(bytes.len() as u64)?;
        self.raw(bytes)
    }

    pub(super) fn list(
        &mut self,
        id: i16,
        previous: &mut i16,
        kind: u8,
        count: usize,
    ) -> Result<(), Error> {
        bound(count as u64, i32::MAX as u64, "Parquet list length")?;
        self.field(id, previous, LIST)?;
        if count < 15 {
            self.raw(&[((count as u8) << 4) | kind])
        } else {
            self.raw(&[0xf0 | kind])?;
            self.unsigned(count as u64)
        }
    }

    pub(super) fn structure(&mut self, id: i16, previous: &mut i16) -> Result<(), Error> {
        self.field(id, previous, STRUCT)
    }

    pub(super) fn stop(&mut self) -> Result<(), Error> {
        self.raw(&[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parquet::compact::{I16, I32, I64};

    #[test]
    fn literal_wire_bytes_and_exact_buffer_refusal() {
        let mut bytes = [0; 64];
        let mut writer = Writer::new(&mut bytes, "test compact bytes");
        let mut previous = 0;
        writer.integer_field(1, &mut previous, I32, -42).unwrap();
        writer.binary_field(2, &mut previous, b"abc").unwrap();
        writer.list(3, &mut previous, I32, 2).unwrap();
        writer.integer(1).unwrap();
        writer.integer(-1).unwrap();
        writer.structure(4, &mut previous).unwrap();
        let mut nested = 0;
        writer.integer_field(20, &mut nested, I16, -1).unwrap();
        writer.stop().unwrap();
        writer
            .integer_field(5, &mut previous, I64, i64::MIN)
            .unwrap();
        writer.stop().unwrap();
        let length = writer.length();
        assert_eq!(
            &bytes[..length],
            &[
                0x15, 83, 0x18, 3, b'a', b'b', b'c', 0x19, 0x25, 2, 1, 0x1c, 4, 40, 1, 0, 0x16,
                255, 255, 255, 255, 255, 255, 255, 255, 255, 1, 0
            ]
        );
        let mut short = [0; 2];
        let mut writer = Writer::new(&mut short, "test compact bytes");
        writer.raw(&[1, 2]).unwrap();
        assert!(matches!(
            writer.stop(),
            Err(Error::Resource {
                required: 3,
                limit: 2,
                ..
            })
        ));
        assert_eq!(short, [1, 2]);
    }
}
