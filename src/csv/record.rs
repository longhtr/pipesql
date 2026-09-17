//! Find CSV records and fields while retaining their original byte positions.
//!
//! Quoted newlines remain inside a record. The scanner rejects malformed quoting
//! before field decoding removes doubled quotes. Validating UTF-8 in the original
//! bytes keeps error locations exact even when earlier quotes will be removed.
//!
//! Record scanning owns refill and end-of-input handling; field decoding operates
//! only on a complete record. Input, record and field bounds are checked here,
//! with cancellation and bounded retries around reads, before typed conversion.

use super::{CsvDecoder, Field, READ_BYTES, input_error};
use crate::{CancellationToken, Error};
use std::io::{ErrorKind, Read};

#[derive(Clone, Copy)]
enum State {
    Start,
    Plain,
    Quoted,
    ClosedQuote,
    CarriageReturn,
}

impl<R: Read> CsvDecoder<'_, R> {
    fn byte(&mut self, cancel: &CancellationToken) -> Result<Option<u8>, Error> {
        cancel.check()?;
        if self.input_finished {
            return Ok(None);
        }
        if self.input_position == self.input_end {
            // Read one extra byte at the total limit to distinguish exact EOF
            // from excess input. No extra byte is accepted into a record.
            let length =
                (self.limits.input_bytes - self.offset).min((READ_BYTES - 1) as u64) as usize + 1;
            let mut interruptions = 0;
            loop {
                cancel.check()?;
                match self.reader.read(&mut self.input[..length]) {
                    Ok(length) => {
                        self.input_end = length;
                        self.input_position = 0;
                        break;
                    }
                    Err(error) if error.kind() == ErrorKind::Interrupted && interruptions < 16 => {
                        interruptions += 1;
                    }
                    Err(source) => {
                        return Err(Error::Io {
                            operation: "read CSV",
                            source,
                        });
                    }
                }
            }
            cancel.check()?;
            if self.input_end == 0 {
                self.input_finished = true;
                return Ok(None);
            }
        }
        if self.offset == self.limits.input_bytes {
            return Err(input_error("CSV input byte limit", self.offset));
        }
        let byte = self.input[self.input_position];
        self.input_position += 1;
        self.offset += 1;
        Ok(Some(byte))
    }

    pub(super) fn read_record(&mut self, cancel: &CancellationToken) -> Result<bool, Error> {
        self.raw.clear();
        self.field_count = 0;
        self.record_start = self.offset;
        let mut state = State::Start;
        let mut field_start = 0;
        let mut quoted = false;
        let mut field_bytes = 0;
        loop {
            let Some(byte) = self.byte(cancel)? else {
                if matches!(state, State::Quoted | State::CarriageReturn) {
                    return Err(input_error("incomplete CSV record", self.offset));
                }
                if self.raw.is_empty() {
                    return Ok(false);
                }
                self.finish_field(field_start, self.raw.len(), quoted)?;
                return self.finish_record(self.raw.len());
            };
            let position = self.raw.len();
            if position == self.limits.record_bytes as usize {
                return Err(input_error("CSV record byte limit", self.offset - 1));
            }
            self.raw.push(byte);
            let adds_byte = match (state, byte) {
                (State::Quoted, b'"') => false,
                (State::Quoted, _) | (State::ClosedQuote, b'"') => true,
                (State::Start | State::Plain, b',' | b'\r' | b'\n' | b'"') => false,
                (State::Start | State::Plain, _) => true,
                _ => false,
            };
            if adds_byte {
                if field_bytes == self.limits.field_bytes {
                    // An escaped quote belongs to the first byte of its pair.
                    let offset = self.offset
                        - if matches!(state, State::ClosedQuote) {
                            2
                        } else {
                            1
                        };
                    return Err(input_error("CSV field byte limit", offset));
                }
                field_bytes += 1;
            }
            match (state, byte) {
                (State::CarriageReturn, b'\n') => {
                    self.finish_field(field_start, position - 1, quoted)?;
                    return self.finish_record(position - 1);
                }
                (State::CarriageReturn, _) => {
                    return Err(input_error("expected LF after CR", self.offset - 1));
                }
                (State::Quoted, b'"') => state = State::ClosedQuote,
                (State::Quoted, _) => {}
                (State::ClosedQuote, b'"') => state = State::Quoted,
                (State::Start, b'"') => {
                    state = State::Quoted;
                    quoted = true;
                }
                (_, b',') => {
                    self.finish_field(field_start, position, quoted)?;
                    field_start = position + 1;
                    quoted = false;
                    field_bytes = 0;
                    state = State::Start;
                }
                (_, b'\n') => {
                    self.finish_field(field_start, position, quoted)?;
                    return self.finish_record(position);
                }
                (_, b'\r') => state = State::CarriageReturn,
                (State::ClosedQuote, _) | (State::Plain, b'"') => {
                    return Err(input_error("invalid CSV quote", self.offset - 1));
                }
                (State::Start | State::Plain, _) => state = State::Plain,
            }
        }
    }

    fn finish_field(&mut self, start: usize, end: usize, quoted: bool) -> Result<(), Error> {
        if self.field_count == self.schema.len() {
            return Err(input_error(
                "too many CSV fields",
                self.record_start + start as u64,
            ));
        }
        self.fields[self.field_count] = Field {
            offset: self.record_start + start as u64,
            start,
            end,
            quoted,
        };
        self.field_count += 1;
        Ok(())
    }

    fn finish_record(&mut self, end: usize) -> Result<bool, Error> {
        if self.field_count != self.schema.len() {
            return Err(input_error(
                "too few CSV fields",
                self.record_start + end as u64,
            ));
        }
        self.raw.truncate(end);
        if let Err(error) = std::str::from_utf8(&self.raw) {
            return Err(input_error(
                "invalid CSV UTF-8",
                self.record_start + error.valid_up_to() as u64,
            ));
        }
        Ok(true)
    }

    pub(super) fn decode_fields(&mut self, cancel: &CancellationToken) -> Result<(), Error> {
        self.decoded.clear();
        for field in &mut self.fields[..self.field_count] {
            let start = self.decoded.len();
            let mut position = field.start + usize::from(field.quoted);
            let end = field.end - usize::from(field.quoted);
            while position < end {
                cancel.check()?;
                let byte = self.raw[position];
                self.decoded.push(byte);
                position += if field.quoted && byte == b'"' { 2 } else { 1 };
            }
            field.start = start;
            field.end = self.decoded.len();
        }
        Ok(())
    }
}
