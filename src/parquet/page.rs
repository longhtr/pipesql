//! Validate a data page before exposing its encoded values.
//!
//! Headers describe the row count and NULL stream; the enclosing column chunk
//! supplies the byte boundary. An optional CRC covers the entire page payload,
//! including definition levels. Compression and non-PLAIN values are refused.
//!
//! The returned Page borrows level and value slices from the caller's input buffer
//! and retains their file offsets for later diagnostics. This stage checks framing
//! and header consistency; level and scalar decoding still have to establish that
//! the payload matches those counts and the declared column type.

use super::bound;
use super::compact::{BOOL_FALSE, BOOL_TRUE, Fields, I32, Reader, STRUCT};
use crate::{CancellationToken, Error};

pub(super) struct Page<'a> {
    pub(super) rows: usize,
    pub(super) nulls: Option<usize>,
    pub(super) levels: &'a [u8],
    pub(super) values: &'a [u8],
    pub(super) levels_offset: u64,
    pub(super) values_offset: u64,
    pub(super) bytes: usize,
}

struct DataHeader {
    rows: usize,
    nulls: Option<usize>,
    level_bytes: Option<usize>,
}

impl<'a> Page<'a> {
    pub(super) fn read(
        bytes: &'a [u8],
        base: u64,
        optional: bool,
        row_limit: usize,
        page_limit: u32,
        cancel: &CancellationToken,
    ) -> Result<Self, Error> {
        let mut reader = Reader::new(bytes, base, cancel)?;
        reader.limit_page_header();
        let mut fields = Fields::new();
        let (mut kind, mut raw, mut encoded, mut checksum) = (0, 0, 0, None);
        let (mut v1, mut v2) = (None, None);
        while let Some(field) = fields.next(&mut reader)? {
            match field.id {
                1 => kind = field.integer(&mut reader, I32)?,
                2 => raw = size(field.integer(&mut reader, I32)?, &reader)?,
                3 => encoded = size(field.integer(&mut reader, I32)?, &reader)?,
                4 => checksum = Some(field.integer(&mut reader, I32)? as i32 as u32),
                5 => {
                    field.require(STRUCT)?;
                    v1 = Some(data_header(&mut reader, false)?);
                }
                8 => {
                    field.require(STRUCT)?;
                    v2 = Some(data_header(&mut reader, true)?);
                }
                6 | 7 => return Err(reader.error("unsupported Parquet page header")),
                _ => reader.skip(field.kind, 1)?,
            }
        }
        fields.require(&[1, 2, 3], &reader)?;
        let header = match (kind, v1, v2) {
            (0, Some(header), None) | (3, None, Some(header)) => header,
            _ => return Err(reader.error("missing or inconsistent Parquet data-page header")),
        };
        if raw != encoded {
            return Err(reader.error("compressed Parquet data pages are unsupported"));
        }
        bound(
            encoded as u64,
            u64::from(page_limit),
            "Parquet page payload",
        )?;
        if header.rows == 0 || header.rows > row_limit {
            return Err(reader.error("Parquet page row count exceeds the remaining column"));
        }
        let header_bytes = reader.position();
        let payload = bytes
            .get(header_bytes..header_bytes + encoded)
            .ok_or_else(|| reader.error("truncated Parquet page payload"))?;
        if let Some(expected) = checksum
            && crc32(payload, cancel)? != expected
        {
            return Err(reader.error("Parquet page checksum differs"));
        }
        let payload_offset = base + header_bytes as u64;
        let (prefix, level_bytes) = match header.level_bytes {
            Some(length) => (0, length),
            None if optional => {
                let prefix: [u8; 4] = payload
                    .get(..4)
                    .ok_or_else(|| reader.error("truncated Parquet definition-level length"))?
                    .try_into()
                    .expect("four-byte length");
                (4, u32::from_le_bytes(prefix) as usize)
            }
            None => (0, 0),
        };
        if !optional && (level_bytes != 0 || header.nulls.is_some_and(|n| n != 0)) {
            return Err(reader.error("required Parquet column has definition levels or NULLs"));
        }
        let values_start = prefix + level_bytes;
        let levels = payload
            .get(prefix..values_start)
            .ok_or_else(|| reader.error("Parquet definition levels exceed the page payload"))?;
        Ok(Self {
            rows: header.rows,
            nulls: header.nulls,
            levels,
            values: &payload[values_start..],
            levels_offset: payload_offset + prefix as u64,
            values_offset: payload_offset + values_start as u64,
            bytes: header_bytes + encoded,
        })
    }
}

fn data_header(reader: &mut Reader<'_, '_>, v2: bool) -> Result<DataHeader, Error> {
    let mut fields = Fields::new();
    let (mut rows, mut nulls, mut row_count, mut definition, mut repetition) = (0, 0, 0, 0, 0);
    while let Some(field) = fields.next(reader)? {
        match (v2, field.id) {
            (_, 1) => rows = size(field.integer(reader, I32)?, reader)?,
            (false, 2) | (true, 4) => {
                if field.integer(reader, I32)? != 0 {
                    return Err(reader.error("Parquet values require PLAIN encoding"));
                }
            }
            (false, 3 | 4) => {
                if field.integer(reader, I32)? != 3 {
                    return Err(reader.error("Parquet levels require RLE encoding"));
                }
            }
            (true, 2) => nulls = size(field.integer(reader, I32)?, reader)?,
            (true, 3) => row_count = size(field.integer(reader, I32)?, reader)?,
            (true, 5) => definition = size(field.integer(reader, I32)?, reader)?,
            (true, 6) => repetition = size(field.integer(reader, I32)?, reader)?,
            (true, 7) if matches!(field.kind, BOOL_TRUE | BOOL_FALSE) => {
                // The column codec is UNCOMPRESSED. This flag only controls
                // whether that codec applies, so either value has the same bytes.
            }
            (true, 7) => return Err(reader.error("invalid Parquet page compression flag")),
            _ => reader.skip(field.kind, 2)?,
        }
    }
    fields.require(
        if v2 {
            &[1, 2, 3, 4, 5, 6]
        } else {
            &[1, 2, 3, 4]
        },
        reader,
    )?;
    if v2 && (nulls > rows || row_count != rows || repetition != 0) {
        return Err(reader.error("inconsistent flat Parquet page counts or repetition levels"));
    }
    Ok(DataHeader {
        rows,
        nulls: v2.then_some(nulls),
        level_bytes: v2.then_some(definition),
    })
}

fn size(value: i64, reader: &Reader<'_, '_>) -> Result<usize, Error> {
    usize::try_from(value).map_err(|_| reader.error("negative Parquet page size or count"))
}

// Parquet uses IEEE CRC-32, unlike native storage's CRC32C. Keep cancellation
// between 4096-byte updates regardless of the selected checksum implementation.
pub(super) fn crc32(bytes: &[u8], cancel: &CancellationToken) -> Result<u32, Error> {
    let mut crc = pipesql_checksum::Crc32::new();
    for chunk in bytes.chunks(4096) {
        cancel.check()?;
        crc.update(chunk);
    }
    Ok(crc.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A most-significant-bit recurrence with the forward IEEE polynomial is
    // independent of the reflected tables used by production.
    fn bitwise_ieee(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for byte in bytes {
            crc ^= u32::from(byte.reverse_bits()) << 24;
            for _ in 0..8 {
                let top = crc & 0x8000_0000 != 0;
                crc <<= 1;
                if top {
                    crc ^= 0x04c1_1db7;
                }
            }
        }
        !crc.reverse_bits()
    }

    #[test]
    fn checksum_matches_independent_bits_at_offsets_and_chunk_boundaries() {
        let cancel = CancellationToken::new();
        assert_eq!(bitwise_ieee(b"123456789"), 0xcbf4_3926);
        assert_eq!(bitwise_ieee(b""), 0);
        for byte in 0..=u8::MAX {
            for bytes in [&[byte][..], &[byte, !byte][..]] {
                assert_eq!(crc32(bytes, &cancel).unwrap(), bitwise_ieee(bytes));
            }
        }
        let input: Vec<u8> = (0..131_136)
            .map(|index| ((index * 73) ^ (index >> 3)) as u8)
            .collect();
        for offset in 0..64 {
            for length in (0..=33)
                .chain([255, 256, 257])
                .chain(4095..=4104)
                .chain(8191..=8200)
            {
                let bytes = &input[offset..offset + length];
                assert_eq!(
                    crc32(bytes, &cancel).unwrap(),
                    bitwise_ieee(bytes),
                    "offset={offset}, length={length}"
                );
            }
        }
        for bytes in [&input[..65_536], &input[63..131_135]] {
            assert_eq!(crc32(bytes, &cancel).unwrap(), bitwise_ieee(bytes));
        }
    }

    #[test]
    fn checksum_cancellation_keeps_empty_and_nonempty_behavior() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(crc32(&[], &cancel).unwrap(), 0);
        let bytes = [0xa5; 8193];
        for length in [1, 4095, 4096, 4097, 8192, 8193] {
            assert!(matches!(
                crc32(&bytes[..length], &cancel),
                Err(Error::Cancelled)
            ));
        }
        let fresh = CancellationToken::new();
        assert_eq!(crc32(&bytes, &fresh).unwrap(), bitwise_ieee(&bytes));
    }

    #[test]
    fn header_limit_and_inconsistent_literal_headers_fail() {
        let cancel = CancellationToken::new();
        // One required INT64 value: PLAIN, RLE level encodings, no checksum.
        let header = [
            0x15, 0, 0x15, 16, 0x15, 16, 0x2c, 0x15, 2, 0x15, 0, 0x15, 6, 0x15, 6, 0,
        ];
        let mut valid = header.to_vec();
        valid.push(0);
        valid.extend_from_slice(&42_i64.to_le_bytes());
        let page = Page::read(&valid, 0, false, 1, 8, &cancel).unwrap();
        assert_eq!(page.values, 42_i64.to_le_bytes());
        for (index, replacement) in [(1, 4), (3, 14), (8, 0), (10, 2), (12, 8), (14, 8)] {
            let mut invalid = valid.clone();
            invalid[index] = replacement;
            assert!(matches!(
                Page::read(&invalid, 0, false, 1, 8, &cancel),
                Err(Error::Input { .. })
            ));
        }
        let mut huge = header.to_vec();
        // Unknown optional field 9: a 65,536-byte binary string. Skipping it
        // still consumes the header budget, even though no value is retained.
        huge.extend_from_slice(&[0x48, 0x80, 0x80, 4]);
        huge.resize(huge.len() + 65_536, 0);
        huge.push(0);
        huge.extend_from_slice(&42_i64.to_le_bytes());
        assert!(matches!(
            Page::read(&huge, 0, false, 1, 8, &cancel),
            Err(Error::Resource {
                owner: "Parquet page header",
                limit: 65_536,
                ..
            })
        ));
    }

    #[test]
    fn independent_pages_and_ieee_checksum() {
        let cancel = CancellationToken::new();
        assert_eq!(crc32(b"123456789", &cancel).unwrap(), 0xcbf43926);
        for fixture in [
            &include_bytes!("../../test/data/parquet/plain-v1.parquet")[..],
            &include_bytes!("../../test/data/parquet/plain-v2.parquet")[..],
        ] {
            let page = Page::read(&fixture[4..], 4, false, 3, 1024, &cancel).unwrap();
            assert_eq!(page.rows, 3);
            assert!(page.levels.is_empty());
            assert_eq!(page.levels_offset, page.values_offset);
            assert_eq!(
                page.values,
                &[
                    1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0
                ]
            );
            assert!(page.nulls.is_none_or(|n| n == 0));
            assert!(page.bytes > page.values.len());
            assert!(matches!(
                Page::read(&fixture[4..], 4, false, 3, 23, &cancel),
                Err(Error::Resource { .. })
            ));
            assert!(Page::read(&fixture[4..], 4, false, 2, 1024, &cancel).is_err());
            let mut damaged = fixture[4..4 + page.bytes].to_vec();
            *damaged.last_mut().unwrap() ^= 1;
            assert!(matches!(
                Page::read(&damaged, 4, false, 3, 1024, &cancel),
                Err(Error::Input {
                    message: "Parquet page checksum differs",
                    ..
                })
            ));
            for length in 0..page.bytes {
                assert!(Page::read(&fixture[4..4 + length], 4, false, 3, 1024, &cancel).is_err());
            }
        }
    }
}
