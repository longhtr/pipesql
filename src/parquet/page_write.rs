//! Encode one column of a retained row group as a checksummed PLAIN v1 page.
//!
//! The caller admits storage for the largest column before collecting rows.
//! Definition levels describe NULLs separately from values. Header space stays
//! separate from the payload, avoiding a copy when its final checksum is known.
//!
//! EncodedPage returns separate header and payload ranges for the output owner to
//! write. Required columns reject NULL, and capacity or cancellation errors return
//! before that write. Chunk offsets and final file completion belong to the export
//! owner, not this in-memory page encoder.

use super::compact::I32;
use super::compact_write::Writer;
use super::page::crc32;
use crate::batch::input::Cell;
use crate::{CancellationToken, ColumnDeclaration, Error};
use std::ops::Range;

pub(super) const HEADER_BYTES: usize = 64;

pub(super) struct EncodedPage {
    pub(super) header: Range<usize>,
    pub(super) payload: Range<usize>,
}

pub(super) fn encode(
    buffer: &mut [u8],
    cells: &[Cell],
    text: &[u8],
    column: &ColumnDeclaration<'_>,
    ordinal: usize,
    columns: usize,
    cancel: &CancellationToken,
) -> Result<EncodedPage, Error> {
    let rows = cells.len() / columns;
    assert!(rows > 0 && rows <= 65_536 && cells.len().is_multiple_of(columns));
    let (header, payload) = buffer.split_at_mut(HEADER_BYTES);
    let mut writer = Writer::new(payload, "Parquet page payload");
    if column.nullable {
        let groups = rows.div_ceil(8);
        let mut run = [0; 5];
        let mut run_writer = Writer::new(&mut run, "Parquet level run header");
        run_writer.unsigned((groups as u64) * 2 + 1)?;
        let header_length = run_writer.length();
        writer.raw(&((header_length + groups) as u32).to_le_bytes())?;
        writer.raw(&run[..header_length])?;
        for group in 0..groups {
            cancel.check()?;
            let mut bits = 0;
            for bit in 0..8 {
                let row = group * 8 + bit;
                if row < rows && !matches!(cells[row * columns + ordinal], Cell::Null) {
                    bits |= 1 << bit;
                }
            }
            writer.raw(&[bits])?;
        }
    }
    for row in 0..rows {
        cancel.check()?;
        match cells[row * columns + ordinal] {
            Cell::Null if column.nullable => {}
            Cell::Null => return Err(Error::Corrupt("NULL in required Parquet output column")),
            Cell::Int64(value) => writer.raw(&value.to_le_bytes())?,
            Cell::Double(value) => writer.raw(&value.to_bits().to_le_bytes())?,
            Cell::Date(value) => writer.raw(&value.days_since_unix_epoch().to_le_bytes())?,
            Cell::Text { start, end } => {
                writer.raw(&((end - start) as u32).to_le_bytes())?;
                writer.raw(&text[start..end])?;
            }
        }
    }
    let payload_length = writer.length();
    let checksum = crc32(&payload[..payload_length], cancel)?;
    let mut writer = Writer::new(header, "Parquet page header");
    let mut fields = 0;
    writer.integer_field(1, &mut fields, I32, 0)?;
    writer.integer_field(2, &mut fields, I32, payload_length as i64)?;
    writer.integer_field(3, &mut fields, I32, payload_length as i64)?;
    writer.integer_field(4, &mut fields, I32, checksum as i32 as i64)?;
    writer.structure(5, &mut fields)?;
    let mut data = 0;
    writer.integer_field(1, &mut data, I32, rows as i64)?;
    writer.integer_field(2, &mut data, I32, 0)?;
    writer.integer_field(3, &mut data, I32, 3)?;
    writer.integer_field(4, &mut data, I32, 3)?;
    writer.stop()?;
    writer.stop()?;
    Ok(EncodedPage {
        header: 0..writer.length(),
        payload: HEADER_BYTES..HEADER_BYTES + payload_length,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataType;
    use crate::parquet::page::Page;

    #[test]
    fn literal_null_stream_values_and_checksum() {
        let column = ColumnDeclaration {
            name: "amount",
            data_type: DataType::Int64,
            nullable: true,
        };
        let cells = [Cell::Int64(i64::MIN), Cell::Null, Cell::Int64(i64::MAX)];
        let mut buffer = [0; 128];
        let cancel = CancellationToken::new();
        let encoded = encode(&mut buffer, &cells, &[], &column, 0, 1, &cancel).unwrap();
        assert_eq!(
            &buffer[encoded.payload.clone()],
            &[
                2, 0, 0, 0, 3, 5, 0, 0, 0, 0, 0, 0, 0, 128, 255, 255, 255, 255, 255, 255, 255, 127
            ]
        );
        let mut contiguous = buffer[encoded.header].to_vec();
        contiguous.extend_from_slice(&buffer[encoded.payload]);
        let page = Page::read(&contiguous, 4, true, 3, 100, &cancel).unwrap();
        assert_eq!(page.rows, 3);
        assert_eq!(page.levels, [3, 5]);
        assert_eq!(page.values.len(), 16);
        let mut short = [0; HEADER_BYTES + 21];
        assert!(matches!(
            encode(&mut short, &cells, &[], &column, 0, 1, &cancel),
            Err(Error::Resource { .. })
        ));
    }
}
