//! Map header fields to the schema and convert decoded fields into one typed row.
//!
//! Conversion borrows the decoded record; text enters the batch only after every
//! field in the row succeeds and the row fits. NULL has a distinct cell variant,
//! so it cannot be confused with an empty string or a numeric placeholder.
//!
//! Header names must identify declared columns exactly once. Conversion applies each
//! column's type and nullability using the field's retained source offset for errors.
//! Numeric and date parsers reject unsupported syntax and out-of-range values before
//! any part of that row can be exposed to an importer.

use super::{Cell, CsvDecoder, input_error};
use crate::{DataType, DateValue, Error};
use std::io::Read;

impl<R: Read> CsvDecoder<'_, R> {
    pub(super) fn map_header(&mut self) -> Result<(), Error> {
        let mut seen = [false; super::MAX_COLUMNS];
        for (input, field) in self.fields[..self.field_count].iter().enumerate() {
            let name = &self.decoded[field.start..field.end];
            let Some(output) = self
                .schema
                .iter()
                .position(|column| column.name.as_bytes().eq_ignore_ascii_case(name))
            else {
                return Err(input_error("unknown CSV header column", field.offset));
            };
            if seen[output] {
                return Err(input_error("duplicate CSV header column", field.offset));
            }
            seen[output] = true;
            self.mapping[input] = output;
        }
        Ok(())
    }

    pub(super) fn type_row(&mut self) -> Result<(), Error> {
        self.row_text_bytes = 0;
        for (input, field) in self.fields[..self.field_count].iter().enumerate() {
            let output = self.mapping[input];
            let column = self.schema[output];
            let bytes = &self.decoded[field.start..field.end];
            let text = std::str::from_utf8(bytes).expect("validated CSV field");
            self.row[output] = if !field.quoted && bytes == b"\\N" {
                if !column.nullable {
                    return Err(input_error("NULL in nonnullable CSV column", field.offset));
                }
                Cell::Null
            } else {
                match column.data_type {
                    DataType::Int64 => {
                        let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
                        let value = (!digits.is_empty()
                            && digits.bytes().all(|b| b.is_ascii_digit()))
                        .then(|| text.parse::<i64>().ok())
                        .flatten()
                        .ok_or_else(|| input_error("invalid CSV INT64", field.offset))?;
                        Cell::Int64(value)
                    }
                    DataType::Double => Cell::Double(
                        double(text)
                            .ok_or_else(|| input_error("invalid CSV DOUBLE", field.offset))?,
                    ),
                    DataType::Date => Cell::Date(
                        DateValue::parse(bytes)
                            .ok_or_else(|| input_error("invalid CSV DATE", field.offset))?,
                    ),
                    DataType::String => {
                        self.row_text_bytes += bytes.len();
                        Cell::Text {
                            start: field.start,
                            end: field.end,
                        }
                    }
                }
            };
        }
        Ok(())
    }
}

fn double(text: &str) -> Option<f64> {
    match text {
        "NaN" => return Some(f64::NAN),
        "Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    let bytes = text.as_bytes();
    let mut position = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let mut digits = 0;
    while bytes.get(position).is_some_and(u8::is_ascii_digit) {
        position += 1;
        digits += 1;
    }
    if bytes.get(position) == Some(&b'.') {
        position += 1;
        while bytes.get(position).is_some_and(u8::is_ascii_digit) {
            position += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if matches!(bytes.get(position), Some(b'e' | b'E')) {
        position += 1;
        position += usize::from(matches!(bytes.get(position), Some(b'+' | b'-')));
        let start = position;
        while bytes.get(position).is_some_and(u8::is_ascii_digit) {
            position += 1;
        }
        if position == start {
            return None;
        }
    }
    if position != bytes.len() {
        return None;
    }
    text.parse::<f64>().ok().filter(|value| value.is_finite())
}
