//! Describe completed column chunks in a flat Parquet footer.
//!
//! Offsets and lengths come from successful page writes. Schema names are borrowed
//! from the prepared query. The footer is built in admitted storage before any of
//! its bytes are sent; only a completed query may call this encoder.
//!
//! Group and chunk records supply row counts and byte ranges for each schema column.
//! The compact writer enforces the provided buffer's capacity; cancellation or an
//! encoding failure returns before the output owner writes the footer and trailer.

use super::compact::{BINARY, I32, I64, STRUCT};
use super::compact_write::Writer;
use crate::{CancellationToken, ColumnDeclaration, DataType, Error};

pub(super) struct Group {
    pub(super) rows: u32,
    pub(super) bytes: u64,
}

pub(super) struct Chunk {
    pub(super) start: u64,
    pub(super) bytes: u64,
}

pub(super) fn encode(
    buffer: &mut [u8],
    schema: &[ColumnDeclaration<'_>],
    groups: &[Group],
    chunks: &[Chunk],
    rows: u64,
    cancel: &CancellationToken,
) -> Result<usize, Error> {
    assert_eq!(chunks.len(), schema.len() * groups.len());
    let mut writer = Writer::new(buffer, "Parquet footer bytes");
    let mut fields = 0;
    writer.integer_field(1, &mut fields, I32, 1)?;
    writer.list(2, &mut fields, STRUCT, schema.len() + 1)?;
    let mut root = 0;
    writer.binary_field(4, &mut root, b"schema")?;
    writer.integer_field(5, &mut root, I32, schema.len() as i64)?;
    writer.stop()?;
    for column in schema {
        cancel.check()?;
        schema_element(&mut writer, column)?;
    }
    writer.integer_field(3, &mut fields, I64, rows as i64)?;
    writer.list(4, &mut fields, STRUCT, groups.len())?;
    for (index, group) in groups.iter().enumerate() {
        cancel.check()?;
        let mut fields = 0;
        writer.list(1, &mut fields, STRUCT, schema.len())?;
        for (column, chunk) in schema
            .iter()
            .zip(&chunks[index * schema.len()..(index + 1) * schema.len()])
        {
            column_chunk(&mut writer, column, group.rows, chunk)?;
        }
        writer.integer_field(2, &mut fields, I64, group.bytes as i64)?;
        writer.integer_field(3, &mut fields, I64, i64::from(group.rows))?;
        writer.stop()?;
    }
    writer.stop()?;
    Ok(writer.length())
}

fn physical(data_type: DataType) -> i64 {
    match data_type {
        DataType::Int64 => 2,
        DataType::Double => 5,
        DataType::Date => 1,
        DataType::String => 6,
    }
}

fn schema_element(writer: &mut Writer<'_>, column: &ColumnDeclaration<'_>) -> Result<(), Error> {
    let mut fields = 0;
    writer.integer_field(1, &mut fields, I32, physical(column.data_type))?;
    writer.integer_field(3, &mut fields, I32, i64::from(column.nullable))?;
    writer.binary_field(4, &mut fields, column.name.as_bytes())?;
    // Converted types are sufficient for STRING and DATE interoperability. The
    // reader also accepts newer logical annotations when external writers use them.
    match column.data_type {
        DataType::String => writer.integer_field(6, &mut fields, I32, 0)?,
        DataType::Date => writer.integer_field(6, &mut fields, I32, 6)?,
        DataType::Int64 | DataType::Double => {}
    }
    writer.stop()
}

fn column_chunk(
    writer: &mut Writer<'_>,
    column: &ColumnDeclaration<'_>,
    rows: u32,
    chunk: &Chunk,
) -> Result<(), Error> {
    let mut fields = 0;
    // Column metadata is inline in this footer; this deprecated offset is zero,
    // matching current external writers. data_page_offset is the actual location.
    writer.integer_field(2, &mut fields, I64, 0)?;
    writer.structure(3, &mut fields)?;
    let mut metadata = 0;
    writer.integer_field(1, &mut metadata, I32, physical(column.data_type))?;
    writer.list(2, &mut metadata, I32, 2)?;
    writer.integer(0)?; // PLAIN.
    writer.integer(3)?; // RLE.
    writer.list(3, &mut metadata, BINARY, 1)?;
    writer.binary(column.name.as_bytes())?;
    writer.integer_field(4, &mut metadata, I32, 0)?; // UNCOMPRESSED.
    writer.integer_field(5, &mut metadata, I64, i64::from(rows))?;
    writer.integer_field(6, &mut metadata, I64, chunk.bytes as i64)?;
    writer.integer_field(7, &mut metadata, I64, chunk.bytes as i64)?;
    writer.integer_field(9, &mut metadata, I64, chunk.start as i64)?;
    writer.stop()?;
    writer.stop()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CancellationToken;
    use crate::parquet::{ParquetReadLimits, metadata::Metadata};

    #[test]
    fn empty_footer_has_literal_schema_and_zero_rows() {
        let schema = [ColumnDeclaration {
            name: "id",
            data_type: DataType::Int64,
            nullable: false,
        }];
        let mut buffer = [0; 128];
        let length = encode(&mut buffer, &schema, &[], &[], 0, &CancellationToken::new()).unwrap();
        assert_eq!(
            &buffer[..length],
            &[
                0x15, 2, 0x19, 0x2c, 0x48, 6, b's', b'c', b'h', b'e', b'm', b'a', 0x15, 2, 0, 0x15,
                4, 0x25, 0, 0x18, 2, b'i', b'd', 0, 0x16, 0, 0x19, 0x0c, 0
            ]
        );
        let limits = ParquetReadLimits {
            input_bytes: 1024,
            metadata_bytes: 128,
            row_groups: 1,
            row_group_rows: 1,
            row_group_bytes: 1024,
            page_bytes: 1024,
            rows: 0,
        };
        let decoded = Metadata::read(
            &buffer[..length],
            4,
            &schema,
            limits,
            100_000,
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(decoded.rows, 0);
        assert!(decoded.groups.is_empty());
        assert!(matches!(
            encode(
                &mut buffer[..length - 1],
                &schema,
                &[],
                &[],
                0,
                &CancellationToken::new()
            ),
            Err(Error::Resource { .. })
        ));
    }
}
