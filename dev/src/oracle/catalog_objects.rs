//! Decode the catalog's referenced objects into complete typed table rows.
//!
//! The parent inspector supplies bounded object reads and reference validation.
//! This layer follows schema, table and native-column relationships, checking column
//! identity, row coverage and checksums before assembling values. Text names and
//! payloads are validated rather than trusted because a root was readable. The
//! returned graph keeps schema and values for independent comparison; decoding does
//! not grant permission to repair or delete any referenced object.

use super::*;

struct Column {
    id: u32,
    name: String,
    kind: u8,
    nullable: bool,
}
impl Column {
    fn value(&self) -> Value {
        json!({"id": self.id, "name": self.name, "type": self.kind, "nullable": self.nullable})
    }
}

fn name(bytes: &[u8]) -> Result<String> {
    require(
        !bytes.is_empty()
            && bytes.len() <= 32
            && (bytes[0].is_ascii_alphabetic() || bytes[0] == b'_')
            && bytes
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'_'),
        "invalid name",
    )?;
    Ok(std::str::from_utf8(bytes)?.to_owned())
}

impl Reader {
    fn schema(&mut self, reference: &Reference, table: u64) -> Result<Vec<Column>> {
        let data = self.object(reference, 3136, b"PSQLSCHM", None)?;
        require(u64_at(&data, 32)? == table, "schema table identity")?;
        zero(&data[40..64])?;
        let count = u32_at(&data, 12)? as usize;
        require(
            (1..=64).contains(&count) && data.len() == 64 + 48 * count,
            "schema count/extent",
        )?;
        let mut columns: Vec<Column> = Vec::new();
        for row in data[64..].as_chunks::<48>().0.iter() {
            let id = u32_at(row, 0)?;
            let kind = row[4];
            let nullable = row[5];
            let length = row[6] as usize;
            require(
                id > 0 && (1..=4).contains(&kind) && nullable <= 1 && (1..=32).contains(&length),
                "column fields",
            )?;
            zero(&row[7..8])?;
            zero(&row[8 + length..])?;
            let name = name(&row[8..8 + length])?;
            require(
                columns
                    .iter()
                    .all(|column| column.id != id && !column.name.eq_ignore_ascii_case(&name)),
                "duplicate column identity/name",
            )?;
            columns.push(Column {
                id,
                name,
                kind,
                nullable: nullable != 0,
            });
        }
        Ok(columns)
    }

    fn unit(
        &mut self,
        reference: &Reference,
        rows: usize,
        table: u64,
        columns: &[Column],
    ) -> Result<Vec<Value>> {
        require((1..=32768).contains(&rows), "unit row count")?;
        self.values = self
            .values
            .checked_add(rows * columns.len())
            .ok_or("value count overflow")?;
        require(self.values <= self.limits.values, "decoded value budget")?;
        let metadata = 64 + 32 * columns.len();
        let data = self.object(reference, 33_556_544, b"PSQLDATA", Some(metadata))?;
        require(
            data.len() >= metadata
                && u32_at(&data, 12)? as usize == columns.len()
                && u64_at(&data, 32)? == table
                && u64_at(&data, 40)? == reference.attempt
                && u32_at(&data, 48)? == reference.ordinal
                && u32_at(&data, 52)? as usize == rows
                && u32_at(&data, 56)? as usize == metadata,
            "unit identity/geometry",
        )?;
        zero(&data[60..64])?;
        let mut decoded = BTreeMap::new();
        let mut offset = metadata;
        for descriptor in data[64..metadata].as_chunks::<32>().0.iter() {
            let id = u32_at(descriptor, 0)?;
            let column = columns
                .iter()
                .find(|column| column.id == id)
                .ok_or("unit column identity")?;
            require(!decoded.contains_key(&id), "duplicate unit column identity")?;
            require(
                descriptor[4] == column.kind
                    && descriptor[5] == u8::from(column.nullable)
                    && u64_at(descriptor, 8)? == offset as u64,
                "column type/coverage",
            )?;
            zero(&descriptor[6..8])?;
            zero(&descriptor[24..])?;
            let length = u32_at(descriptor, 16)? as usize;
            let bitmap = rows.div_ceil(8);
            let width = match column.kind {
                1 | 2 => Some(8),
                4 => Some(4),
                _ => None,
            };
            let minimum = bitmap + width.map_or((rows + 1) * 4, |width| rows * width);
            require(
                length >= minimum && length <= 524_288 && width.is_none_or(|_| length == minimum),
                "payload extent",
            )?;
            let payload = region(&data, offset, length)?;
            offset += length;
            require(
                crc32c(payload) == u32_at(descriptor, 20)?,
                "payload checksum",
            )?;
            require(
                rows.is_multiple_of(8) || payload[bitmap - 1] >> (rows % 8) == 0,
                "validity padding",
            )?;
            let textbase = bitmap + (rows + 1) * 4;
            if width.is_none() {
                require(u32_at(payload, bitmap)? == 0, "initial string offset")?;
            }
            let mut previous = 0;
            let mut values = Vec::with_capacity(rows);
            for row in 0..rows {
                let present = payload[row / 8] & (1 << (row % 8)) != 0;
                require(present || column.nullable, "NULL in nonnullable column")?;
                let value = if let Some(width) = width {
                    let raw = region(payload, bitmap + row * width, width)?;
                    if !present {
                        zero(raw)?;
                        Value::Null
                    } else if column.kind == 2 {
                        json!({"double_bits": format!("{:016x}", u64::from_le_bytes(raw.try_into()?))})
                    } else {
                        let number = if width == 8 {
                            i64::from_le_bytes(raw.try_into()?)
                        } else {
                            i64::from(i32::from_le_bytes(raw.try_into()?))
                        };
                        require(
                            column.kind != 4 || (-719_162..=2_932_896).contains(&number),
                            "DATE domain",
                        )?;
                        json!(number)
                    }
                } else {
                    let end = u32_at(payload, bitmap + (row + 1) * 4)? as usize;
                    require(
                        previous <= end && end <= length - textbase && end - previous <= 65_536,
                        "string offsets/length",
                    )?;
                    require(present || previous == end, "NULL string payload")?;
                    let value = std::str::from_utf8(&payload[textbase + previous..textbase + end])?;
                    previous = end;
                    if present { json!(value) } else { Value::Null }
                };
                values.push(value);
            }
            if width.is_none() {
                require(previous == length - textbase, "trailing string bytes")?;
            }
            decoded.insert(id, values);
        }
        require(offset == data.len(), "trailing unit bytes")?;
        Ok((0..rows)
            .map(|row| {
                Value::Array(
                    columns
                        .iter()
                        .map(|column| decoded[&column.id][row].clone())
                        .collect(),
                )
            })
            .collect())
    }

    pub(super) fn catalog(&mut self, reference: &Reference) -> Result<Vec<Value>> {
        let data = self.object(reference, 8256, b"PSQLCATL", None)?;
        require(
            u64_at(&data, 32)? == reference.attempt && u32_at(&data, 40)? == reference.ordinal,
            "catalog identity",
        )?;
        zero(&data[44..64])?;
        let count = u32_at(&data, 12)? as usize;
        require(
            count <= 64 && data.len() == 64 + 128 * count,
            "catalog count/extent",
        )?;
        let mut tables: Vec<Value> = Vec::new();
        let mut previous = 0;
        for entry in data[64..].as_chunks::<128>().0.iter() {
            let id = u64_at(entry, 0)?;
            let length = entry[8] as usize;
            require(
                id > previous && (1..=32).contains(&length),
                "table identity/name length",
            )?;
            previous = id;
            zero(&entry[9..16])?;
            zero(&entry[16 + length..48])?;
            zero(&entry[108..])?;
            let name = name(&entry[16..16 + length])?;
            require(
                tables
                    .iter()
                    .all(|table| !table["name"].as_str().unwrap().eq_ignore_ascii_case(&name)),
                "duplicate table name",
            )?;
            let schema = Reference::decode(&entry[48..72])?;
            require(
                schema.attempt <= reference.attempt,
                "future schema reference",
            )?;
            let columns = self.schema(&schema, id)?;
            let rows = u64_at(entry, 96)?;
            let units = u32_at(entry, 104)? as usize;
            require(
                units <= 4096 && units as u64 <= rows && rows <= units as u64 * 32768,
                "table row/unit counts",
            )?;
            let mut output = Vec::new();
            if units == 0 {
                zero(&entry[72..96])?;
            } else {
                let index_ref = Reference::decode(&entry[72..96])?;
                require(
                    index_ref.attempt <= reference.attempt
                        && index_ref.size as usize == 64 + 48 * units,
                    "table index reference",
                )?;
                let index = self.object(&index_ref, 196_672, b"PSQLTBLD", None)?;
                require(
                    u32_at(&index, 12)? as usize == units
                        && u64_at(&index, 32)? == id
                        && u64_at(&index, 40)? == index_ref.attempt
                        && u32_at(&index, 48)? == index_ref.ordinal
                        && u64_at(&index, 56)? == rows,
                    "table index header",
                )?;
                zero(&index[52..56])?;
                let mut prior = (0, 0);
                for unit in index[64..].as_chunks::<48>().0.iter() {
                    let attempt = u64_at(unit, 0)?;
                    let ordinal = u32_at(unit, 8)?;
                    let count = u32_at(unit, 12)? as usize;
                    require(
                        prior < (attempt, ordinal) && attempt <= index_ref.attempt,
                        "unit order/creator",
                    )?;
                    require(
                        u64_at(unit, 24)? == output.len() as u64,
                        "unit row coverage",
                    )?;
                    zero(&unit[32..])?;
                    prior = (attempt, ordinal);
                    output.extend(self.unit(
                        &Reference {
                            attempt,
                            ordinal,
                            size: u32_at(unit, 16)?,
                            crc: u32_at(unit, 20)?,
                        },
                        count,
                        id,
                        &columns,
                    )?);
                }
                require(output.len() as u64 == rows, "table row total")?;
            }
            tables.push(json!({"id": id, "name": name, "columns": columns.iter().map(Column::value).collect::<Vec<_>>(), "rows": output}));
        }
        Ok(tables)
    }
}
