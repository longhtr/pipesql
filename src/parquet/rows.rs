//! Decode a retained row group into typed cells in declaration order.
//!
//! Column pages are independent byte streams. Their NULL positions decide when
//! a PLAIN value is consumed; every stream must end at its declared row and byte
//! count. Text cells borrow offsets into the retained group, so even a large
//! string needs no separate allocation.

use super::{ParquetReadLimits, bound, levels::Levels, metadata::Metadata, page::Page};
use crate::batch::input::Cell;
use crate::{CancellationToken, ColumnDeclaration, DataType, DateValue, Error};

pub(super) fn decode(
    bytes: &[u8],
    metadata: &Metadata,
    index: usize,
    schema: &[ColumnDeclaration<'_>],
    cells: &mut [Cell],
    limits: ParquetReadLimits,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let group = &metadata.groups[index];
    assert_eq!(bytes.len() as u64, group.end - group.start);
    assert_eq!(cells.len(), group.rows * schema.len());
    let chunks = &metadata.chunks[index * schema.len()..(index + 1) * schema.len()];
    for (column, chunk) in metadata.columns.iter().zip(chunks) {
        let start = (chunk.start - group.start) as usize;
        let encoded = &bytes[start..start + chunk.bytes as usize];
        let (mut position, mut row) = (0, 0);
        while position < encoded.len() {
            cancel.check()?;
            let page = Page::read(
                &encoded[position..],
                chunk.start + position as u64,
                column.nullable,
                group.rows - row,
                limits.page_bytes,
                cancel,
            )?;
            let mut levels = Levels::new(page.levels, page.levels_offset, page.rows)?;
            let mut values = Values {
                bytes: page.values,
                position: 0,
                base: page.values_offset,
                group_base: group.start,
            };
            let mut nulls = 0;
            for offset in 0..page.rows {
                cancel.check()?;
                let present = !column.nullable || levels.next(cancel)?;
                let cell = if present {
                    values.next(column.data_type)?
                } else {
                    if !schema[column.output].nullable {
                        return Err(values.error("NULL in a required Parquet target column"));
                    }
                    nulls += 1;
                    Cell::Null
                };
                cells[(row + offset) * schema.len() + column.output] = cell;
            }
            if column.nullable {
                levels.finish()?;
            }
            if values.position != values.bytes.len() || page.nulls.is_some_and(|n| n != nulls) {
                return Err(values.error("Parquet page value bytes or NULL count differs"));
            }
            row += page.rows;
            position += page.bytes;
        }
        if row != group.rows {
            return Err(Error::Input {
                message: "Parquet column row count differs",
                byte_offset: chunk.start + chunk.bytes,
            });
        }
    }
    Ok(())
}

struct Values<'a> {
    bytes: &'a [u8],
    position: usize,
    base: u64,
    group_base: u64,
}

impl Values<'_> {
    fn next(&mut self, data_type: DataType) -> Result<Cell, Error> {
        Ok(match data_type {
            DataType::Int64 => Cell::Int64(i64::from_le_bytes(
                self.take(8)?.try_into().expect("eight bytes"),
            )),
            DataType::Double => Cell::Double(f64::from_bits(u64::from_le_bytes(
                self.take(8)?.try_into().expect("eight bytes"),
            ))),
            DataType::Date => {
                let days = i32::from_le_bytes(self.take(4)?.try_into().expect("four bytes"));
                Cell::Date(
                    DateValue::from_days_since_unix_epoch(days).ok_or_else(|| {
                        self.error("Parquet DATE is outside years 0001 through 9999")
                    })?,
                )
            }
            DataType::String => {
                let length = i32::from_le_bytes(self.take(4)?.try_into().expect("four bytes"));
                let length = usize::try_from(length)
                    .map_err(|_| self.error("negative Parquet STRING length"))?;
                bound(length as u64, 65_536, "Parquet STRING bytes")?;
                let start = (self.base - self.group_base) as usize + self.position;
                let text = self.take(length)?;
                if std::str::from_utf8(text).is_err() {
                    return Err(self.error("Parquet STRING is not UTF-8"));
                }
                Cell::Text {
                    start,
                    end: start + length,
                }
            }
        })
    }

    fn take(&mut self, length: usize) -> Result<&[u8], Error> {
        if length > self.bytes.len() - self.position {
            return Err(self.error("truncated Parquet PLAIN value"));
        }
        let start = self.position;
        self.position += length;
        Ok(&self.bytes[start..self.position])
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
    use crate::{InputBatch, Value};

    #[test]
    fn plain_values_reject_truncation_invalid_dates_and_text() {
        for (kind, bytes) in [
            (DataType::Int64, &b"1234567"[..]),
            (DataType::Double, &b"1234567"[..]),
            (DataType::Date, &b"123"[..]),
            (DataType::Date, &(-719_163_i32).to_le_bytes()),
            (DataType::Date, &2_932_897_i32.to_le_bytes()),
            (DataType::String, &[255, 255, 255, 255]),
            (DataType::String, &[1, 0, 0, 0]),
            (DataType::String, &[1, 0, 0, 0, 255]),
        ] {
            let mut values = Values {
                bytes,
                position: 0,
                base: 100,
                group_base: 20,
            };
            assert!(matches!(values.next(kind), Err(Error::Input { .. })));
        }
        let mut values = Values {
            bytes: &[1, 0, 1, 0],
            position: 0,
            base: 100,
            group_base: 20,
        };
        assert!(matches!(
            values.next(DataType::String),
            Err(Error::Resource {
                required: 65_537,
                limit: 65_536,
                ..
            })
        ));
    }

    #[test]
    fn independent_complete_values_and_reordered_target() {
        let cancel = CancellationToken::new();
        let schema = [
            ColumnDeclaration {
                name: "note",
                data_type: DataType::String,
                nullable: true,
            },
            ColumnDeclaration {
                name: "day",
                data_type: DataType::Date,
                nullable: true,
            },
            ColumnDeclaration {
                name: "number",
                data_type: DataType::Double,
                nullable: true,
            },
            ColumnDeclaration {
                name: "amount",
                data_type: DataType::Int64,
                nullable: true,
            },
            ColumnDeclaration {
                name: "ID",
                data_type: DataType::Int64,
                nullable: false,
            },
        ];
        let limits = ParquetReadLimits {
            input_bytes: 100_000,
            metadata_bytes: 16_384,
            row_groups: 3,
            row_group_rows: 3,
            row_group_bytes: 70_000,
            page_bytes: 70_000,
            rows: 8,
        };
        let amounts = [
            Some(i64::MIN),
            Some(i64::MAX),
            None,
            Some(-1),
            Some(0),
            Some(1),
            Some(42),
            Some(-42),
        ];
        let numbers = [
            Some(0),
            Some(0x8000000000000000),
            Some(0x7ff0000000000000),
            Some(0xfff0000000000000),
            Some(0x7ff8000000001234),
            Some(0xfff0000000000001),
            Some(1),
            None,
        ];
        let days = [
            Some(-719_162),
            Some(2_932_896),
            None,
            Some(-1),
            Some(0),
            Some(1),
            Some(11_016),
            Some(18_262),
        ];
        let long = "x".repeat(65_536);
        let text = [
            Some(""),
            None,
            Some("é🙂"),
            Some("a,\n\"\\\0"),
            Some("\\N"),
            Some(long.as_str()),
            Some("tail"),
            Some("end"),
        ];
        for fixture in [
            &include_bytes!("../../test/data/parquet/plain-v1.parquet")[..],
            &include_bytes!("../../test/data/parquet/plain-v2.parquet")[..],
        ] {
            let length = u32::from_le_bytes(
                fixture[fixture.len() - 8..fixture.len() - 4]
                    .try_into()
                    .unwrap(),
            ) as usize;
            let footer = fixture.len() - 8 - length;
            let metadata = Metadata::read(
                &fixture[footer..fixture.len() - 8],
                footer as u64,
                &schema,
                limits,
                2_000_000,
                &cancel,
            )
            .unwrap();
            let mut total = 0;
            for (index, group) in metadata.groups.iter().enumerate() {
                let bytes = &fixture[group.start as usize..group.end as usize];
                let mut cells = vec![Cell::Null; group.rows * 5];
                decode(
                    bytes, &metadata, index, &schema, &mut cells, limits, &cancel,
                )
                .unwrap();
                let batch = InputBatch {
                    cells: &cells,
                    text: bytes,
                    columns: 5,
                };
                for row in 0..group.rows {
                    let i = total + row;
                    assert_eq!(batch.value(row, 4), Some(Value::Int64(i as i64 + 1)));
                    assert_eq!(
                        batch.value(row, 3),
                        Some(amounts[i].map_or(Value::Null, Value::Int64))
                    );
                    match (batch.value(row, 2).unwrap(), numbers[i]) {
                        (Value::Double(value), Some(bits)) => assert_eq!(value.to_bits(), bits),
                        (Value::Null, None) => {}
                        _ => panic!("unexpected DOUBLE cell"),
                    }
                    assert_eq!(
                        batch.value(row, 1),
                        Some(days[i].map_or(Value::Null, |n| Value::Date(
                            DateValue::from_days_since_unix_epoch(n).unwrap()
                        )))
                    );
                    assert_eq!(
                        batch.value(row, 0),
                        Some(
                            text[i]
                                .map_or(Value::Null, |s| Value::String(crate::StringValue::new(s)))
                        )
                    );
                }
                total += group.rows;
                let mut required = schema;
                required[0].nullable = false;
                if index == 0 {
                    assert!(matches!(
                        decode(
                            bytes, &metadata, index, &required, &mut cells, limits, &cancel
                        ),
                        Err(Error::Input {
                            message: "NULL in a required Parquet target column",
                            ..
                        })
                    ));
                }
            }
            assert_eq!(total, 8);
        }
    }
}
