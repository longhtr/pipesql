use crate::storage_format::Crc32c;

pub(super) const MAX_INPUT_BYTES: u64 = 1_073_741_824;
pub(super) const MAX_CHUNK_BYTES: usize = 65_536;
const MAX_ROWS: u64 = 6_500_000;
const MAX_ROW_BYTES: usize = 512;
const FIELD_COUNT: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InputError {
    InputTooLarge,
    ChunkTooLarge,
    TooManyRows,
    RowTooLarge,
    PartialRow,
    CarriageReturn,
    FieldCount,
    Double,
    FixedText,
    Date,
}

impl InputError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::InputTooLarge => "input exceeds 1 GiB",
            Self::ChunkTooLarge => "input read exceeds 65536 bytes",
            Self::TooManyRows => "input exceeds 6500000 rows",
            Self::RowTooLarge => "input row exceeds 512 bytes including LF",
            Self::PartialRow => "input ends without LF",
            Self::CarriageReturn => "carriage return is not accepted",
            Self::FieldCount => "row must contain exactly 16 fields and a trailing delimiter",
            Self::Double => "projected DOUBLE is not canonical",
            Self::FixedText => "projected fixed text must be one printable ASCII character",
            Self::Date => "projected DATE is not canonical",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ProjectedRow {
    pub(super) quantity: [u8; 8],
    pub(super) extended_price: [u8; 8],
    pub(super) discount: [u8; 8],
    pub(super) tax: [u8; 8],
    pub(super) return_flag: [u8; 1],
    pub(super) line_status: [u8; 1],
    pub(super) ship_date: [u8; 4],
}

pub(super) struct Scanner {
    row: [u8; MAX_ROW_BYTES - 1],
    row_bytes: usize,
    input_bytes: u64,
    consumed_bytes: u64,
    rows: u64,
    input_crc: Crc32c,
    projected_crc: Crc32c,
}

impl Scanner {
    pub(super) fn new() -> Self {
        Self {
            row: [0; MAX_ROW_BYTES - 1],
            row_bytes: 0,
            input_bytes: 0,
            consumed_bytes: 0,
            rows: 0,
            input_crc: Crc32c::new(),
            projected_crc: Crc32c::new(),
        }
    }

    pub(super) fn begin_chunk(&mut self, bytes: &[u8]) -> Result<(), InputError> {
        admit_chunk(bytes.len())?;
        self.input_bytes = admit_input_bytes(self.input_bytes, bytes.len())?;
        self.input_crc.update(bytes);
        Ok(())
    }

    pub(super) fn consume(&mut self, byte: u8) -> Result<Option<ProjectedRow>, InputError> {
        self.consumed_bytes = self
            .consumed_bytes
            .checked_add(1)
            .ok_or(InputError::InputTooLarge)?;
        assert!(
            self.consumed_bytes <= self.input_bytes,
            "scanner consumed bytes that were not admitted"
        );
        if byte == b'\r' {
            return Err(InputError::CarriageReturn);
        }
        if byte == b'\n' {
            let parsed = parse_row(&self.row[..self.row_bytes])?;
            self.rows = admit_row(self.rows)?;
            self.projected_crc.update(&parsed.quantity);
            self.projected_crc.update(&parsed.extended_price);
            self.projected_crc.update(&parsed.discount);
            self.projected_crc.update(&parsed.tax);
            self.projected_crc.update(&parsed.return_flag);
            self.projected_crc.update(&parsed.line_status);
            self.projected_crc.update(&parsed.ship_date);
            self.row_bytes = 0;
            Ok(Some(parsed))
        } else {
            if self.row_bytes == self.row.len() {
                return Err(InputError::RowTooLarge);
            }
            self.row[self.row_bytes] = byte;
            self.row_bytes += 1;
            Ok(None)
        }
    }

    #[cfg(test)]
    fn feed(&mut self, bytes: &[u8]) -> Result<(), InputError> {
        self.begin_chunk(bytes)?;
        for byte in bytes {
            self.consume(*byte)?;
        }
        Ok(())
    }

    pub(super) fn byte_offset(&self) -> u64 {
        self.consumed_bytes
    }

    pub(super) fn finish(self) -> Result<Scan, InputError> {
        assert_eq!(
            self.consumed_bytes, self.input_bytes,
            "scan has unconsumed admitted bytes"
        );
        if self.row_bytes != 0 {
            return Err(InputError::PartialRow);
        }
        Ok(Scan {
            input_bytes: self.input_bytes,
            rows: self.rows,
            input_crc32c: self.input_crc.finish(),
            projected_crc32c: self.projected_crc.finish(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Scan {
    pub(super) input_bytes: u64,
    pub(super) rows: u64,
    pub(super) input_crc32c: u32,
    pub(super) projected_crc32c: u32,
}

fn admit_chunk(bytes: usize) -> Result<(), InputError> {
    if bytes > MAX_CHUNK_BYTES {
        return Err(InputError::ChunkTooLarge);
    }
    Ok(())
}

fn admit_input_bytes(current: u64, next: usize) -> Result<u64, InputError> {
    let next = u64::try_from(next).map_err(|_| InputError::InputTooLarge)?;
    let total = current.checked_add(next).ok_or(InputError::InputTooLarge)?;
    if total > MAX_INPUT_BYTES {
        return Err(InputError::InputTooLarge);
    }
    Ok(total)
}

fn admit_row(current: u64) -> Result<u64, InputError> {
    let total = current.checked_add(1).ok_or(InputError::TooManyRows)?;
    if total > MAX_ROWS {
        return Err(InputError::TooManyRows);
    }
    Ok(total)
}

fn parse_row(row: &[u8]) -> Result<ProjectedRow, InputError> {
    if row.len() >= MAX_ROW_BYTES || row.last() != Some(&b'|') {
        return Err(if row.len() >= MAX_ROW_BYTES {
            InputError::RowTooLarge
        } else {
            InputError::FieldCount
        });
    }
    let mut starts = [0_usize; FIELD_COUNT];
    let mut ends = [0_usize; FIELD_COUNT];
    let mut field = 0_usize;
    let mut start = 0_usize;
    for (index, byte) in row.iter().enumerate() {
        if *byte == b'|' {
            if field == FIELD_COUNT {
                return Err(InputError::FieldCount);
            }
            starts[field] = start;
            ends[field] = index;
            field += 1;
            start = index.checked_add(1).ok_or(InputError::FieldCount)?;
        }
    }
    if field != FIELD_COUNT || start != row.len() {
        return Err(InputError::FieldCount);
    }
    let value = |one_based: usize| &row[starts[one_based - 1]..ends[one_based - 1]];
    let quantity = parse_double(value(5))?.to_bits().to_le_bytes();
    let extended_price = parse_double(value(6))?.to_bits().to_le_bytes();
    let discount = parse_double(value(7))?.to_bits().to_le_bytes();
    let tax = parse_double(value(8))?.to_bits().to_le_bytes();
    let return_flag = [parse_fixed_text(value(9))?];
    let line_status = [parse_fixed_text(value(10))?];
    let ship_date = parse_date(value(11))?.to_le_bytes();
    Ok(ProjectedRow {
        quantity,
        extended_price,
        discount,
        tax,
        return_flag,
        line_status,
        ship_date,
    })
}

fn parse_double(bytes: &[u8]) -> Result<f64, InputError> {
    let exceptional = match bytes {
        b"NaN" => Some(f64::NAN),
        b"inf" => Some(f64::INFINITY),
        b"-inf" => Some(f64::NEG_INFINITY),
        _ => None,
    };
    if let Some(value) = exceptional {
        return Ok(value);
    }
    if bytes.is_empty() || bytes.len() > 32 {
        return Err(InputError::Double);
    }
    let digits = bytes.strip_prefix(b"-").unwrap_or(bytes);
    if digits.is_empty() {
        return Err(InputError::Double);
    }
    let mut point = None;
    for (index, byte) in digits.iter().enumerate() {
        if *byte == b'.' {
            if point.replace(index).is_some() {
                return Err(InputError::Double);
            }
        } else if !byte.is_ascii_digit() {
            return Err(InputError::Double);
        }
    }
    if matches!(point, Some(0)) || point == Some(digits.len() - 1) {
        return Err(InputError::Double);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| InputError::Double)?;
    let value = text.parse::<f64>().map_err(|_| InputError::Double)?;
    if !value.is_finite() {
        return Err(InputError::Double);
    }
    Ok(value)
}

fn parse_fixed_text(bytes: &[u8]) -> Result<u8, InputError> {
    match bytes {
        [byte] => crate::fixed_text::StringValue::from_byte(*byte)
            .map(crate::fixed_text::StringValue::byte)
            .ok_or(InputError::FixedText),
        _ => Err(InputError::FixedText),
    }
}

fn parse_date(bytes: &[u8]) -> Result<i32, InputError> {
    crate::date::DateValue::parse(bytes)
        .map(crate::date::DateValue::days_since_unix_epoch)
        .ok_or(InputError::Date)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: &[u8] = b"1|2|3|4|17.00|21168.23|0.04|0.02|N|O|1996-03-13|1996-04-12|1996-03-22|DELIVER IN PERSON|TRUCK|sentinel|";

    fn line(row: &[u8]) -> Vec<u8> {
        let mut line = row.to_vec();
        line.push(b'\n');
        line
    }

    #[test]
    fn exact_tpch_ordinals_preserve_canonical_values() {
        let parsed = parse_row(ROW).unwrap();
        assert_eq!(f64::from_bits(u64::from_le_bytes(parsed.quantity)), 17.0);
        assert_eq!(
            f64::from_bits(u64::from_le_bytes(parsed.extended_price)),
            21_168.23
        );
        assert_eq!(f64::from_bits(u64::from_le_bytes(parsed.discount)), 0.04);
        assert_eq!(f64::from_bits(u64::from_le_bytes(parsed.tax)), 0.02);
        assert_eq!(parsed.return_flag, *b"N");
        assert_eq!(parsed.line_status, *b"O");
        assert_eq!(i32::from_le_bytes(parsed.ship_date), 9_568);
    }

    #[test]
    fn chunk_partition_does_not_change_scan() {
        let mut input = line(ROW);
        input.extend_from_slice(&line(ROW));
        let mut whole = Scanner::new();
        whole.feed(&input).unwrap();
        let expected = whole.finish().unwrap();
        for chunk in [1_usize, 2, 7, 31, 64, input.len()] {
            let mut scanner = Scanner::new();
            for bytes in input.chunks(chunk) {
                scanner.feed(bytes).unwrap();
            }
            assert_eq!(scanner.finish(), Ok(expected));
        }
        for split in 0..=input.len() {
            let mut scanner = Scanner::new();
            scanner.feed(&input[..split]).unwrap();
            scanner.feed(&input[split..]).unwrap();
            assert_eq!(scanner.finish(), Ok(expected), "split {split}");
        }
        assert_eq!(expected.rows, 2);
        assert_eq!(expected.input_bytes, input.len() as u64);
    }

    #[test]
    fn exceptional_double_and_signed_zero_are_explicit() {
        for (text, bits) in [
            (b"NaN".as_slice(), f64::NAN.to_bits()),
            (b"inf".as_slice(), f64::INFINITY.to_bits()),
            (b"-inf".as_slice(), f64::NEG_INFINITY.to_bits()),
            (b"-0.0".as_slice(), (-0.0_f64).to_bits()),
        ] {
            assert_eq!(parse_double(text).unwrap().to_bits(), bits);
        }
        for rejected in [
            b"nan".as_slice(),
            b"+inf".as_slice(),
            b"1e3".as_slice(),
            b".5".as_slice(),
            b"5.".as_slice(),
            b"+1".as_slice(),
            b"-".as_slice(),
        ] {
            assert_eq!(parse_double(rejected), Err(InputError::Double));
        }
    }

    #[test]
    fn fixed_text_covers_the_complete_admitted_domain() {
        let mut accepted = 0_usize;
        for byte in u8::MIN..=u8::MAX {
            let actual = parse_fixed_text(&[byte]);
            if (0x20..=0x7e).contains(&byte) && byte != b'|' {
                assert_eq!(actual, Ok(byte));
                accepted += 1;
            } else {
                assert_eq!(actual, Err(InputError::FixedText));
            }
        }
        assert_eq!(accepted, 94);
        assert_eq!(parse_fixed_text(b""), Err(InputError::FixedText));
        assert_eq!(parse_fixed_text(b"AA"), Err(InputError::FixedText));
    }

    #[test]
    fn gregorian_dates_cover_epoch_leaps_and_range_edges() {
        for (text, expected) in [
            (b"1970-01-01".as_slice(), 0),
            (b"1969-12-31".as_slice(), -1),
            (b"2000-02-29".as_slice(), 11_016),
            (b"0001-01-01".as_slice(), -719_162),
            (b"9999-12-31".as_slice(), 2_932_896),
        ] {
            assert_eq!(parse_date(text), Ok(expected));
        }
        for rejected in [
            b"0000-01-01".as_slice(),
            b"1900-02-29".as_slice(),
            b"2001-02-29".as_slice(),
            b"2000-13-01".as_slice(),
            b"2000-01-00".as_slice(),
            b"2000/01/01".as_slice(),
        ] {
            assert_eq!(parse_date(rejected), Err(InputError::Date));
        }
    }

    #[test]
    fn every_supported_calendar_day_matches_a_sequential_oracle() {
        let mut expected = -719_162_i32;
        let mut text = *b"0000-00-00";
        for year in 1_u32..=9_999 {
            put_decimal(&mut text[0..4], year);
            let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
            let month_days = [
                31_u32,
                28 + u32::from(leap),
                31,
                30,
                31,
                30,
                31,
                31,
                30,
                31,
                30,
                31,
            ];
            for (month_index, days) in month_days.into_iter().enumerate() {
                put_decimal(&mut text[5..7], u32::try_from(month_index + 1).unwrap());
                for day in 1..=days {
                    put_decimal(&mut text[8..10], day);
                    assert_eq!(parse_date(&text), Ok(expected));
                    expected = expected.checked_add(1).unwrap();
                }
                put_decimal(&mut text[8..10], days + 1);
                assert_eq!(parse_date(&text), Err(InputError::Date));
            }
        }
        assert_eq!(expected, 2_932_897);
    }

    fn put_decimal(target: &mut [u8], mut value: u32) {
        for byte in target.iter_mut().rev() {
            *byte = b'0' + u8::try_from(value % 10).unwrap();
            value /= 10;
        }
        assert_eq!(value, 0);
    }

    #[test]
    fn structural_neighbors_and_stream_boundaries_fail() {
        let mut missing = ROW.to_vec();
        missing.pop();
        assert_eq!(parse_row(&missing), Err(InputError::FieldCount));
        let extra = [ROW, b"extra|"].concat();
        assert_eq!(parse_row(&extra), Err(InputError::FieldCount));
        let mut fewer = ROW.to_vec();
        let delimiter = fewer.iter().position(|byte| *byte == b'|').unwrap();
        fewer.remove(delimiter);
        assert_eq!(parse_row(&fewer), Err(InputError::FieldCount));

        let mut scanner = Scanner::new();
        scanner.feed(ROW).unwrap();
        assert_eq!(scanner.finish(), Err(InputError::PartialRow));
        let mut scanner = Scanner::new();
        assert_eq!(scanner.feed(b"a\r\n"), Err(InputError::CarriageReturn));

        let mut maximum = b"1|2|3|4|1|2|3|8|9|A|1970-01-01|12|13|14|15|16|".to_vec();
        let filler = MAX_ROW_BYTES - 1 - maximum.len();
        maximum.splice(
            maximum.len() - 1..maximum.len() - 1,
            std::iter::repeat_n(b'x', filler),
        );
        assert_eq!(maximum.len(), MAX_ROW_BYTES - 1);
        assert!(parse_row(&maximum).is_ok());
        maximum.insert(maximum.len() - 1, b'x');
        assert_eq!(parse_row(&maximum), Err(InputError::RowTooLarge));
    }

    #[test]
    fn byte_chunk_and_row_next_values_refuse_without_saturation() {
        assert_eq!(admit_chunk(MAX_CHUNK_BYTES), Ok(()));
        assert_eq!(
            admit_chunk(MAX_CHUNK_BYTES + 1),
            Err(InputError::ChunkTooLarge)
        );
        assert_eq!(admit_input_bytes(MAX_INPUT_BYTES, 0), Ok(MAX_INPUT_BYTES));
        assert_eq!(
            admit_input_bytes(MAX_INPUT_BYTES, 1),
            Err(InputError::InputTooLarge)
        );
        assert_eq!(
            admit_input_bytes(u64::MAX, 1),
            Err(InputError::InputTooLarge)
        );
        assert_eq!(admit_row(MAX_ROWS - 1), Ok(MAX_ROWS));
        assert_eq!(admit_row(MAX_ROWS), Err(InputError::TooManyRows));
        assert_eq!(admit_row(u64::MAX), Err(InputError::TooManyRows));
    }

    #[test]
    fn scan_summary_cannot_certify_unconsumed_chunks() {
        let mut scanner = Scanner::new();
        scanner.begin_chunk(b"\n").unwrap();
        assert!(std::panic::catch_unwind(|| scanner.finish()).is_err());
    }

    #[test]
    fn checksums_match_the_independent_castagnoli_vector() {
        let mut crc = Crc32c::new();
        crc.update(b"1234");
        crc.update(b"56789");
        assert_eq!(crc.finish(), 0xe306_9283);
    }
}
