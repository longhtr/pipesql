//! Validate the footer and retain only the schema and ranges needed to read rows.
//!
//! Names remain offsets into the admitted footer bytes. Row-group and column
//! vectors have fixed admitted capacities; metadata cannot grow them. All chunks
//! must be in one file, in metadata order, before the footer and without overlap.
//!
//! Schema validation accepts only the supported flat physical and logical types.
//! External column files, dictionary pages and unsupported encodings are refused
//! before row decoding. The decoder uses the retained ranges for bounded reads;
//! valid metadata alone does not establish that page payloads contain valid values.

use super::compact::{BINARY, BOOL_FALSE, BOOL_TRUE, Fields, I32, I64, Reader, STRUCT};
use super::{ParquetReadLimits, bound};
use crate::resources::allocate;
use crate::{CancellationToken, ColumnDeclaration, DataType, Error};

#[derive(Clone, Copy, Default)]
struct Name {
    start: usize,
    length: usize,
}

impl Name {
    fn read(reader: &mut Reader<'_, '_>) -> Result<Self, Error> {
        let length = reader.binary()?.len();
        Ok(Self {
            start: reader.position() - length,
            length,
        })
    }

    fn bytes(self, footer: &[u8]) -> &[u8] {
        &footer[self.start..self.start + self.length]
    }
}

#[derive(Clone, Copy)]
pub(super) struct Column {
    name: Name,
    physical: i32,
    pub(super) data_type: DataType,
    pub(super) nullable: bool,
    pub(super) output: usize,
}

impl Column {
    const EMPTY: Self = Self {
        name: Name {
            start: 0,
            length: 0,
        },
        physical: 0,
        data_type: DataType::Int64,
        nullable: false,
        output: 0,
    };
}

pub(super) struct Group {
    pub(super) rows: usize,
    pub(super) start: u64,
    pub(super) end: u64,
    uncompressed: u64,
}

pub(super) struct Chunk {
    pub(super) start: u64,
    pub(super) bytes: u64,
    pub(super) values: u64,
    uncompressed: u64,
    physical: i32,
    path: Name,
}

pub(super) struct Metadata {
    pub(super) columns: [Column; 64],
    pub(super) groups: Vec<Group>,
    pub(super) chunks: Vec<Chunk>,
    pub(super) rows: u64,
}

impl Metadata {
    pub(super) fn read(
        footer: &[u8],
        base: u64,
        schema: &[ColumnDeclaration<'_>],
        limits: ParquetReadLimits,
        memory: u64,
        cancel: &CancellationToken,
    ) -> Result<Self, Error> {
        limits.validate()?;
        if schema.is_empty() || schema.len() > 64 {
            return Err(Error::InvalidConfig(
                "Parquet requires 1 through 64 target columns",
            ));
        }
        bound(
            footer.len() as u64,
            u64::from(limits.metadata_bytes),
            "Parquet footer bytes",
        )?;
        let mut reader = Reader::new(footer, base, cancel)?;
        let mut result = Self {
            columns: [Column::EMPTY; 64],
            groups: allocate(
                limits.row_groups as usize,
                limits.row_groups as usize,
                "Parquet row-group metadata",
                memory,
            )?,
            chunks: allocate(
                limits.row_groups as usize * schema.len(),
                limits.row_groups as usize * schema.len(),
                "Parquet column metadata",
                memory,
            )?,
            rows: 0,
        };
        let mut fields = Fields::new();
        while let Some(field) = fields.next(&mut reader)? {
            match field.id {
                1 => {
                    if !matches!(field.integer(&mut reader, I32)?, 1 | 2) {
                        return Err(reader.error("unsupported Parquet metadata version"));
                    }
                }
                2 => {
                    let length = field.list(&mut reader, STRUCT)?;
                    if length as usize != schema.len() + 1 {
                        return Err(
                            reader.error("Parquet schema must match the flat target column count")
                        );
                    }
                    read_schema(&mut reader, footer, schema, &mut result.columns)?;
                }
                3 => result.rows = nonnegative(field.integer(&mut reader, I64)?, &reader)?,
                4 => {
                    let length = field.list(&mut reader, STRUCT)?;
                    bound(
                        u64::from(length),
                        u64::from(limits.row_groups),
                        "Parquet row groups",
                    )?;
                    for _ in 0..length {
                        result.groups.push(read_group(
                            &mut reader,
                            schema.len(),
                            &mut result.chunks,
                            limits,
                        )?);
                    }
                }
                8 | 9 => return Err(reader.error("encrypted Parquet metadata is unsupported")),
                _ => reader.skip(field.kind, 1)?,
            }
        }
        fields.require(&[1, 2, 3, 4], &reader)?;
        if reader.remaining() != 0 {
            return Err(reader.error("trailing bytes after Parquet footer metadata"));
        }
        bound(result.rows, limits.rows, "Parquet rows")?;
        let mut total_rows = 0_u64;
        let mut previous_end = 4;
        for (group_index, group) in result.groups.iter_mut().enumerate() {
            cancel.check()?;
            let chunks =
                &result.chunks[group_index * schema.len()..(group_index + 1) * schema.len()];
            let mut uncompressed = 0_u64;
            for (ordinal, chunk) in chunks.iter().enumerate() {
                let column = result.columns[ordinal];
                if chunk.physical != column.physical
                    || chunk.path.bytes(footer) != column.name.bytes(footer)
                {
                    return Err(reader.error("Parquet column metadata differs from the schema"));
                }
                if chunk.values != group.rows as u64 || chunk.bytes == 0 {
                    return Err(
                        reader.error("Parquet column count or extent differs from its row group")
                    );
                }
                let end = chunk
                    .start
                    .checked_add(chunk.bytes)
                    .ok_or_else(|| reader.error("Parquet column byte range overflows"))?;
                if chunk.start < previous_end || end > base {
                    return Err(reader
                        .error("Parquet chunks overlap, are out of order or enter the footer"));
                }
                previous_end = end;
                uncompressed = uncompressed
                    .checked_add(chunk.uncompressed)
                    .ok_or_else(|| reader.error("Parquet row-group size overflows"))?;
            }
            group.start = chunks[0].start;
            group.end = previous_end;
            bound(
                group.end - group.start,
                u64::from(limits.row_group_bytes),
                "Parquet row-group bytes",
            )?;
            if uncompressed != group.uncompressed {
                return Err(reader.error("Parquet row-group byte total differs from its columns"));
            }
            total_rows = total_rows
                .checked_add(group.rows as u64)
                .ok_or_else(|| reader.error("Parquet row count overflows"))?;
        }
        if total_rows != result.rows {
            return Err(reader.error("Parquet file row count differs from its row groups"));
        }
        Ok(result)
    }
}

fn nonnegative(value: i64, reader: &Reader<'_, '_>) -> Result<u64, Error> {
    u64::try_from(value).map_err(|_| reader.error("negative Parquet count or offset"))
}

#[derive(Clone, Copy, PartialEq)]
enum Annotation {
    None,
    String,
    Date,
    Int64,
    Unsupported,
}

struct SchemaElement {
    name: Name,
    physical: Option<i32>,
    repetition: Option<i32>,
    children: Option<i64>,
    converted: Option<Annotation>,
    logical: Option<Annotation>,
}

fn read_schema(
    reader: &mut Reader<'_, '_>,
    footer: &[u8],
    target: &[ColumnDeclaration<'_>],
    columns: &mut [Column; 64],
) -> Result<(), Error> {
    let root = schema_element(reader)?;
    std::str::from_utf8(root.name.bytes(footer))
        .map_err(|_| reader.error("invalid UTF-8 in Parquet root name"))?;
    if root.physical.is_some()
        || root.children != Some(target.len() as i64)
        || root.converted.is_some()
        || root.logical.is_some()
        || root.repetition.is_some_and(|value| value != 0)
    {
        return Err(reader.error("Parquet root must be a flat schema group"));
    }
    let mut seen = [false; 64];
    for column in &mut columns[..target.len()] {
        let element = schema_element(reader)?;
        let name = element.name.bytes(footer);
        if !crate::schema::valid_name(name) {
            return Err(reader.error("Parquet column name is not a declared-table identifier"));
        }
        let output = target
            .iter()
            .position(|candidate| candidate.name.as_bytes().eq_ignore_ascii_case(name))
            .ok_or_else(|| reader.error("Parquet column is absent from the target schema"))?;
        if std::mem::replace(&mut seen[output], true) {
            return Err(reader.error("duplicate Parquet column name"));
        }
        if element.children.is_some_and(|value| value != 0) {
            return Err(reader.error("nested Parquet columns are unsupported"));
        }
        let nullable = match element.repetition {
            Some(0) => false,
            Some(1) => true,
            _ => return Err(reader.error("Parquet columns must be required or optional")),
        };
        if let (Some(converted), Some(logical)) = (element.converted, element.logical)
            && converted != logical
        {
            return Err(reader.error("conflicting Parquet type annotations"));
        }
        let annotation = element
            .logical
            .or(element.converted)
            .unwrap_or(Annotation::None);
        let physical = element
            .physical
            .ok_or_else(|| reader.error("missing Parquet physical type"))?;
        let data_type = match (physical, annotation) {
            (2, Annotation::None | Annotation::Int64) => DataType::Int64,
            (5, Annotation::None) => DataType::Double,
            (1, Annotation::Date) => DataType::Date,
            (6, Annotation::String) => DataType::String,
            _ => return Err(reader.error("unsupported Parquet physical or logical type")),
        };
        if data_type != target[output].data_type {
            return Err(reader.error("Parquet type differs from the declared column"));
        }
        *column = Column {
            name: element.name,
            physical,
            data_type,
            nullable,
            output,
        };
    }
    Ok(())
}

fn schema_element(reader: &mut Reader<'_, '_>) -> Result<SchemaElement, Error> {
    let mut value = SchemaElement {
        name: Name::default(),
        physical: None,
        repetition: None,
        children: None,
        converted: None,
        logical: None,
    };
    let mut fields = Fields::new();
    while let Some(field) = fields.next(reader)? {
        match field.id {
            1 => value.physical = Some(field.integer(reader, I32)? as i32),
            3 => value.repetition = Some(field.integer(reader, I32)? as i32),
            4 => {
                field.require(BINARY)?;
                value.name = Name::read(reader)?;
            }
            5 => value.children = Some(field.integer(reader, I32)?),
            6 => {
                value.converted = Some(match field.integer(reader, I32)? {
                    0 => Annotation::String,
                    6 => Annotation::Date,
                    18 => Annotation::Int64,
                    _ => Annotation::Unsupported,
                })
            }
            7 | 8 => return Err(reader.error("Parquet decimal annotations are unsupported")),
            10 => {
                field.require(STRUCT)?;
                value.logical = Some(logical_type(reader)?);
            }
            _ => reader.skip(field.kind, 3)?,
        }
    }
    fields.require(&[4], reader)?;
    Ok(value)
}

fn logical_type(reader: &mut Reader<'_, '_>) -> Result<Annotation, Error> {
    let mut fields = Fields::new();
    let field = fields
        .next(reader)?
        .ok_or_else(|| reader.error("empty Parquet logical type"))?;
    field.require(STRUCT)?;
    let annotation = match field.id {
        1 | 6 => {
            reader.skip(STRUCT, 4)?;
            if field.id == 1 {
                Annotation::String
            } else {
                Annotation::Date
            }
        }
        10 => {
            let mut fields = Fields::new();
            let mut width = 0;
            let mut signed = false;
            while let Some(field) = fields.next(reader)? {
                match field.id {
                    1 => {
                        field.require(3)?;
                        width = reader.take(1)?[0];
                    }
                    2 => {
                        signed = match field.kind {
                            BOOL_TRUE => true,
                            BOOL_FALSE => false,
                            _ => {
                                return Err(
                                    reader.error("Parquet integer signedness is not boolean")
                                );
                            }
                        }
                    }
                    _ => reader.skip(field.kind, 5)?,
                }
            }
            fields.require(&[1, 2], reader)?;
            if width == 64 && signed {
                Annotation::Int64
            } else {
                Annotation::Unsupported
            }
        }
        _ => {
            reader.skip(STRUCT, 4)?;
            Annotation::Unsupported
        }
    };
    if fields.next(reader)?.is_some() {
        return Err(reader.error("Parquet logical type has multiple alternatives"));
    }
    Ok(annotation)
}

fn read_group(
    reader: &mut Reader<'_, '_>,
    columns: usize,
    chunks: &mut Vec<Chunk>,
    limits: ParquetReadLimits,
) -> Result<Group, Error> {
    let mut value = Group {
        rows: 0,
        start: 0,
        end: 0,
        uncompressed: 0,
    };
    let mut fields = Fields::new();
    while let Some(field) = fields.next(reader)? {
        match field.id {
            1 => {
                if field.list(reader, STRUCT)? as usize != columns {
                    return Err(reader.error("Parquet row-group column count differs from schema"));
                }
                for _ in 0..columns {
                    chunks.push(read_chunk(reader)?);
                }
            }
            2 => value.uncompressed = nonnegative(field.integer(reader, I64)?, reader)?,
            3 => {
                let rows = nonnegative(field.integer(reader, I64)?, reader)?;
                bound(
                    rows,
                    u64::from(limits.row_group_rows),
                    "Parquet rows per group",
                )?;
                if rows == 0 {
                    return Err(reader.error("empty Parquet row groups are unsupported"));
                }
                value.rows = rows as usize;
            }
            _ => reader.skip(field.kind, 3)?,
        }
    }
    fields.require(&[1, 2, 3], reader)?;
    Ok(value)
}

fn read_chunk(reader: &mut Reader<'_, '_>) -> Result<Chunk, Error> {
    let mut value = None;
    let mut fields = Fields::new();
    while let Some(field) = fields.next(reader)? {
        match field.id {
            1 => return Err(reader.error("external Parquet column files are unsupported")),
            2 => {
                nonnegative(field.integer(reader, I64)?, reader)?;
            }
            3 => {
                field.require(STRUCT)?;
                value = Some(column_metadata(reader)?);
            }
            8 | 9 => return Err(reader.error("encrypted Parquet columns are unsupported")),
            _ => reader.skip(field.kind, 5)?,
        }
    }
    fields.require(&[2, 3], reader)?;
    Ok(value.expect("required column metadata"))
}

fn column_metadata(reader: &mut Reader<'_, '_>) -> Result<Chunk, Error> {
    let mut value = Chunk {
        start: 0,
        bytes: 0,
        values: 0,
        uncompressed: 0,
        physical: 0,
        path: Name::default(),
    };
    let mut fields = Fields::new();
    let mut plain = false;
    while let Some(field) = fields.next(reader)? {
        match field.id {
            1 => value.physical = field.integer(reader, I32)? as i32,
            2 => {
                let count = field.list(reader, I32)?;
                for _ in 0..count {
                    match reader.integer(I32)? {
                        0 => plain = true,
                        3 => {}
                        _ => {
                            return Err(reader.error("unsupported Parquet value or level encoding"));
                        }
                    }
                }
            }
            3 => {
                if field.list(reader, BINARY)? != 1 {
                    return Err(reader.error("Parquet column paths must be flat"));
                }
                value.path = Name::read(reader)?;
            }
            4 => {
                if field.integer(reader, I32)? != 0 {
                    return Err(reader.error("unsupported Parquet compression codec"));
                }
            }
            5 => value.values = nonnegative(field.integer(reader, I64)?, reader)?,
            6 => value.uncompressed = nonnegative(field.integer(reader, I64)?, reader)?,
            7 => value.bytes = nonnegative(field.integer(reader, I64)?, reader)?,
            9 => value.start = nonnegative(field.integer(reader, I64)?, reader)?,
            11 => return Err(reader.error("Parquet dictionary pages are unsupported")),
            _ => reader.skip(field.kind, 6)?,
        }
    }
    fields.require(&[1, 2, 3, 4, 5, 6, 7, 9], reader)?;
    if !plain || value.bytes != value.uncompressed {
        return Err(reader.error("Parquet column must contain uncompressed PLAIN pages"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: &[u8] = include_bytes!("../../test/data/parquet/plain-v1.parquet");
    const V2: &[u8] = include_bytes!("../../test/data/parquet/plain-v2.parquet");

    fn schema() -> [ColumnDeclaration<'static>; 5] {
        [
            ColumnDeclaration {
                name: "id",
                data_type: DataType::Int64,
                nullable: false,
            },
            ColumnDeclaration {
                name: "amount",
                data_type: DataType::Int64,
                nullable: true,
            },
            ColumnDeclaration {
                name: "number",
                data_type: DataType::Double,
                nullable: true,
            },
            ColumnDeclaration {
                name: "day",
                data_type: DataType::Date,
                nullable: true,
            },
            ColumnDeclaration {
                name: "note",
                data_type: DataType::String,
                nullable: true,
            },
        ]
    }

    fn limits() -> ParquetReadLimits {
        ParquetReadLimits {
            input_bytes: 100_000,
            metadata_bytes: 16_384,
            row_groups: 3,
            row_group_rows: 3,
            row_group_bytes: 70_000,
            page_bytes: 70_000,
            rows: 8,
        }
    }

    fn read(
        bytes: &[u8],
        schema: &[ColumnDeclaration<'_>],
        limits: ParquetReadLimits,
    ) -> Result<Metadata, Error> {
        let length = u32::from_le_bytes(bytes[bytes.len() - 8..bytes.len() - 4].try_into().unwrap())
            as usize;
        let start = bytes.len() - 8 - length;
        Metadata::read(
            &bytes[start..bytes.len() - 8],
            start as u64,
            schema,
            limits,
            2_000_000,
            &CancellationToken::new(),
        )
    }

    #[test]
    fn independent_footer_ranges_types_and_name_mapping() {
        // Offsets and lengths are from PyArrow's independent metadata reader.
        for (bytes, ranges) in [
            (V1, [(4, 413), (413, 66_373), (66_373, 66_759)]),
            (V2, [(4, 422), (422, 66_391), (66_391, 66_787)]),
        ] {
            let schema = schema();
            let metadata = read(bytes, &schema, limits()).unwrap();
            assert_eq!(metadata.rows, 8);
            assert_eq!(metadata.groups.len(), 3);
            assert_eq!(metadata.chunks.len(), 15);
            for (index, group) in metadata.groups.iter().enumerate() {
                assert_eq!(group.rows, [3, 3, 2][index]);
                assert_eq!((group.start, group.end), ranges[index]);
            }
            for (index, column) in metadata.columns[..5].iter().enumerate() {
                assert_eq!(column.data_type, schema[index].data_type);
                assert_eq!(column.nullable, schema[index].nullable);
                assert_eq!(column.output, index);
            }
            let reversed = [schema[4], schema[3], schema[2], schema[1], schema[0]];
            let metadata = read(bytes, &reversed, limits()).unwrap();
            for (index, column) in metadata.columns[..5].iter().enumerate() {
                assert_eq!(column.output, 4 - index);
            }
        }
    }

    #[test]
    fn every_footer_truncation_and_duplicate_required_field_fail() {
        let length =
            u32::from_le_bytes(V1[V1.len() - 8..V1.len() - 4].try_into().unwrap()) as usize;
        let start = V1.len() - 8 - length;
        let footer = &V1[start..V1.len() - 8];
        let cancel = CancellationToken::new();
        for end in 0..footer.len() {
            assert!(
                matches!(
                    Metadata::read(
                        &footer[..end],
                        start as u64,
                        &schema(),
                        limits(),
                        2_000_000,
                        &cancel
                    ),
                    Err(Error::Input { .. })
                ),
                "accepted prefix {end}"
            );
        }
        let mut duplicate = footer[..footer.len() - 1].to_vec();
        // Long field header: I32, explicit id 1, value 2, then struct stop.
        duplicate.extend_from_slice(&[0x05, 0x02, 0x04, 0]);
        assert!(matches!(
            Metadata::read(
                &duplicate,
                start as u64,
                &schema(),
                limits(),
                2_000_000,
                &cancel
            ),
            Err(Error::Input {
                message: "duplicate Parquet metadata field",
                ..
            })
        ));
    }

    #[test]
    fn unsupported_external_profiles_and_tighter_bounds_fail() {
        for bytes in [
            include_bytes!("../../test/data/parquet/unsupported-snappy.parquet").as_slice(),
            include_bytes!("../../test/data/parquet/unsupported-dictionary.parquet").as_slice(),
            include_bytes!("../../test/data/parquet/unsupported-nested.parquet").as_slice(),
        ] {
            assert!(matches!(
                read(bytes, &schema(), limits()),
                Err(Error::Input { .. })
            ));
        }
        for limits in [
            ParquetReadLimits {
                rows: 7,
                ..limits()
            },
            ParquetReadLimits {
                row_groups: 2,
                ..limits()
            },
            ParquetReadLimits {
                row_group_rows: 2,
                ..limits()
            },
            ParquetReadLimits {
                row_group_bytes: 65_959,
                ..limits()
            },
            ParquetReadLimits {
                metadata_bytes: 1,
                ..limits()
            },
        ] {
            assert!(matches!(
                read(V1, &schema(), limits),
                Err(Error::Resource { .. })
            ));
        }
        let mut wrong = schema();
        wrong[2].data_type = DataType::Int64;
        assert!(matches!(
            read(V1, &wrong, limits()),
            Err(Error::Input { .. })
        ));
    }
}
