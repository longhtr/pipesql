//! Decode NULL positions in a flat optional column without allocating.
//!
//! A definition level is one bit: zero means NULL and one means a stored value.
//! Parquet groups these bits into repeated runs or packed groups of eight. The
//! declared page count bounds both forms; only the last packed group may contain
//! unused positions. Required columns have no definition-level stream.

use crate::{CancellationToken, Error};

pub(super) struct Levels<'a> {
    bytes: &'a [u8],
    base: u64,
    position: usize,
    remaining: usize,
    run: Run,
}

enum Run {
    Empty,
    Repeated {
        value: bool,
        remaining: usize,
    },
    Packed {
        start: usize,
        bit: usize,
        count: usize,
    },
}

impl<'a> Levels<'a> {
    pub(super) fn new(bytes: &'a [u8], base: u64, count: usize) -> Result<Self, Error> {
        if base.checked_add(bytes.len() as u64).is_none() {
            return Err(Error::Input {
                message: "Parquet definition-level byte range overflows",
                byte_offset: base,
            });
        }
        Ok(Self {
            bytes,
            base,
            position: 0,
            remaining: count,
            run: Run::Empty,
        })
    }

    pub(super) fn next(&mut self, cancel: &CancellationToken) -> Result<bool, Error> {
        cancel.check()?;
        if self.remaining == 0 {
            return Err(self.error("too many Parquet definition levels requested"));
        }
        if matches!(self.run, Run::Empty) {
            self.read_run()?;
        }
        let value = match &mut self.run {
            Run::Repeated { value, remaining } => {
                let value = *value;
                *remaining -= 1;
                if *remaining == 0 {
                    self.run = Run::Empty;
                }
                value
            }
            Run::Packed { start, bit, count } => {
                let value = self.bytes[*start + *bit / 8] & (1 << (*bit % 8)) != 0;
                *bit += 1;
                if *bit == *count {
                    self.run = Run::Empty;
                }
                value
            }
            Run::Empty => unreachable!("validated nonempty definition-level run"),
        };
        self.remaining -= 1;
        Ok(value)
    }

    pub(super) fn finish(&self) -> Result<(), Error> {
        if self.remaining != 0 || self.position != self.bytes.len() {
            return Err(self.error("Parquet definition-level count or byte length differs"));
        }
        Ok(())
    }

    fn read_run(&mut self) -> Result<(), Error> {
        let mut header = 0_u32;
        for shift in (0..35).step_by(7) {
            let byte = self.byte()?;
            if shift == 28 && byte > 15 {
                return Err(self.error("Parquet definition-level run length overflows"));
            }
            header |= u32::from(byte & 127) << shift;
            if byte & 128 == 0 {
                break;
            }
        }
        let length = (header >> 1) as usize;
        if length == 0 {
            return Err(self.error("empty Parquet definition-level run"));
        }
        self.run = if header & 1 == 0 {
            if length > self.remaining {
                return Err(self.error("Parquet repeated levels exceed the page count"));
            }
            let value = self.byte()?;
            if value > 1 {
                return Err(self.error("invalid flat Parquet definition level"));
            }
            Run::Repeated {
                value: value == 1,
                remaining: length,
            }
        } else {
            // Each byte holds eight one-bit levels. The final byte may have up
            // to seven padding bits, but an extra whole group is invalid.
            if length > self.remaining.div_ceil(8) || length > self.bytes.len() - self.position {
                return Err(self.error("Parquet packed levels exceed the page or byte count"));
            }
            let start = self.position;
            self.position += length;
            Run::Packed {
                start,
                bit: 0,
                count: (length * 8).min(self.remaining),
            }
        };
        Ok(())
    }

    fn byte(&mut self) -> Result<u8, Error> {
        let value = *self
            .bytes
            .get(self.position)
            .ok_or_else(|| self.error("truncated Parquet definition levels"))?;
        self.position += 1;
        Ok(value)
    }

    fn error(&self, message: &'static str) -> Error {
        Error::Input {
            message,
            byte_offset: self.base + self.position as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_repeated_and_packed_runs_with_final_padding() {
        let cancel = CancellationToken::new();
        // Three present values, two NULLs, eight packed bits, then three bits
        // in the final padded byte. Expected positions are literal and separate.
        let bytes = [6, 1, 4, 0, 3, 0b1010_0110, 3, 0b1111_1101];
        let expected = [
            true, true, true, false, false, false, true, true, false, false, true, false, true,
            true, false, true,
        ];
        let mut levels = Levels::new(&bytes, 100, expected.len()).unwrap();
        for value in expected {
            assert_eq!(levels.next(&cancel).unwrap(), value);
        }
        levels.finish().unwrap();
        assert!(levels.next(&cancel).is_err());
        Levels::new(&[], 0, 0).unwrap().finish().unwrap();
    }

    #[test]
    fn malformed_runs_trailing_bytes_and_cancellation_fail() {
        let cancel = CancellationToken::new();
        for bytes in [
            &[][..],
            &[0],
            &[1],
            &[2],
            &[2, 2],
            &[4, 1],
            &[5, 0, 0],
            &[3],
            &[128, 128, 128, 128, 16],
            &[255; 5],
        ] {
            let mut levels = Levels::new(bytes, 17, 1).unwrap();
            assert!(
                matches!(levels.next(&cancel), Err(Error::Input { .. })),
                "{bytes:?}"
            );
        }
        let mut trailing = Levels::new(&[2, 1, 2, 0], 17, 1).unwrap();
        assert!(trailing.next(&cancel).unwrap());
        assert!(trailing.finish().is_err());
        let mut incomplete = Levels::new(&[4, 1], 17, 2).unwrap();
        incomplete.next(&cancel).unwrap();
        assert!(incomplete.finish().is_err());
        cancel.cancel();
        assert!(matches!(incomplete.next(&cancel), Err(Error::Cancelled)));
        assert!(Levels::new(&[0], u64::MAX, 1).is_err());
    }
}
