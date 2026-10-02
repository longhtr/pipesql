//! Build controlled format-4 inputs without calling the production writer.
//!
//! Retained independent records supply the fixed identity and schema. This encoder
//! changes rows, block descriptions and checksums; it does not simulate publication.
//! Attempt 2 is committed and attempt 1 remains an aborted gap.
//!
//! Rows retain raw DOUBLE bits and fixed key/date fields so campaigns can construct
//! exceptional numbers or invalid stored values intentionally. Descriptor capacity
//! is checked before writing. Tests compare an encoded retained row byte-for-byte
//! and check empty input, keeping this helper accountable to external expected bytes.

use super::catalog::crc32c;
use crate::Result;
use std::{fs, path::Path};

pub struct Row {
    pub numbers: [u64; 4],
    pub keys: [u8; 2],
    pub day: i32,
}
fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn unit(seed: &[u8], rows: &[Row]) -> Result<Vec<u8>> {
    if seed.len() < 28672 || &seed[..8] != b"PSQLUNIT" || rows.len() > 6_500_000 {
        return Err("snapshot input exceeds the fixed unit profile".into());
    }
    let mut unit = vec![0; 28672];
    unit[..4096].copy_from_slice(&seed[..4096]);
    let mut descriptor = 0;
    for column in 0..7 {
        let start = unit.len();
        let width = if column < 4 {
            8
        } else if column < 6 {
            1
        } else {
            4
        };
        let rows_per_block = if column == 6 { 65536 } else { 32768 };
        for row in rows {
            if column < 4 {
                unit.extend_from_slice(&row.numbers[column].to_le_bytes());
            } else if column < 6 {
                unit.push(row.keys[column - 4]);
            } else {
                unit.extend_from_slice(&row.day.to_le_bytes());
            }
        }
        let size = unit.len() - start;
        let blocks = rows.len().div_ceil(rows_per_block);
        if descriptor + blocks > 1344 {
            return Err("snapshot descriptor capacity exceeded".into());
        }
        let base = 128 + column * 40;
        put32(&mut unit, base + 8, width as u32);
        put32(&mut unit, base + 12, rows_per_block as u32);
        put32(&mut unit, base + 16, descriptor as u32);
        put32(&mut unit, base + 20, blocks as u32);
        put64(&mut unit, base + 24, start as u64);
        put64(&mut unit, base + 32, size as u64);
        for first in (start..start + size).step_by(rows_per_block * width) {
            let length = (start + size - first).min(rows_per_block * width);
            let crc = crc32c(&unit[first..first + length]);
            let at = 4096 + descriptor * 16;
            put64(&mut unit, at, first as u64);
            put32(&mut unit, at + 8, length as u32);
            put32(&mut unit, at + 12, crc);
            descriptor += 1;
        }
    }
    let length = unit.len();
    put64(&mut unit, 48, rows.len() as u64);
    put64(&mut unit, 56, length as u64);
    put32(&mut unit, 72, descriptor as u32);
    let payload_crc = crc32c(&unit[28672..]);
    put32(&mut unit, 112, payload_crc);
    // Metadata excludes its three CRC fields; the header CRC then includes the
    // stored descriptor and metadata CRCs. Padding is outside both regions.
    unit[100..112].fill(0);
    let metadata_crc = crc32c(&unit[..4096 + 1344 * 16]);
    let descriptors_crc = crc32c(&unit[4096..4096 + 1344 * 16]);
    put32(&mut unit, 104, descriptors_crc);
    put32(&mut unit, 108, metadata_crc);
    let header_crc = crc32c(&unit[..4096]);
    put32(&mut unit, 100, header_crc);
    Ok(unit)
}

pub fn write(path: &Path, retained: &Path, rows: &[Row]) -> Result<()> {
    let unit = unit(&fs::read(retained.join("UNIT"))?, rows)?;
    fs::create_dir(path)?;
    fs::create_dir(path.join("units"))?;
    fs::create_dir(path.join("private"))?;
    for name in ["ROOT.A", "ROOT.B", "WAL"] {
        let mut record = fs::read(retained.join(name))?;
        put64(&mut record, 88, rows.len() as u64);
        put64(&mut record, 96, unit.len() as u64);
        record[104..108].copy_from_slice(&unit[108..112]);
        put32(&mut record, 108, 0);
        let crc = crc32c(&record);
        put32(&mut record, 108, crc);
        fs::write(path.join(name), record)?;
    }
    fs::write(path.join("CONTROL"), fs::read(retained.join("CONTROL"))?)?;
    fs::write(path.join("LOCK"), [])?;
    fs::write(path.join("units/0000000000000001.unit"), unit)?;
    Ok(())
}

/// Construct the six retained format-4 records without using them as templates.
pub fn vectors() -> Result<std::collections::BTreeMap<&'static str, Vec<u8>>> {
    let mut seed = vec![0; 28672];
    seed[..8].copy_from_slice(b"PSQLUNIT");
    put32(&mut seed, 8, 4);
    put32(&mut seed, 12, 4096);
    let identity: Vec<u8> = (0..16).collect();
    seed[16..32].copy_from_slice(&identity);
    put64(&mut seed, 32, 1);
    put64(&mut seed, 40, 1);
    put64(&mut seed, 64, 4096);
    put32(&mut seed, 76, 1344);
    put64(&mut seed, 80, 1344 * 16);
    put64(&mut seed, 88, 28672);
    put32(&mut seed, 96, 7);
    for (index, kind) in [1, 1, 1, 1, 3, 3, 2].into_iter().enumerate() {
        put32(&mut seed, 128 + index * 40, index as u32 + 1);
        put32(&mut seed, 132 + index * 40, kind);
    }
    let row = Row {
        numbers: [
            1.0_f64.to_bits(),
            2.0_f64.to_bits(),
            0.5_f64.to_bits(),
            0.25_f64.to_bits(),
        ],
        keys: *b"AF",
        day: 0,
    };
    let unit = unit(&seed, &[row])?;
    let empty = self::unit(&seed, &[])?;
    let mut control = vec![0; 128];
    control[..8].copy_from_slice(b"PIPESQL\0");
    put32(&mut control, 8, 4);
    put32(&mut control, 12, 128);
    control[16..32].copy_from_slice(&identity);
    let crc = crc32c(&control);
    put32(&mut control, 36, crc);
    let mut result =
        std::collections::BTreeMap::from([("CONTROL", control), ("EMPTY.UNIT", empty)]);
    for (name, magic, size, replica) in [
        ("ROOT.A", b"PSQLROOT", 4096, 0),
        ("ROOT.B", b"PSQLROOT", 4096, 1),
        ("WAL", b"PSQLWAL\0", 512, 0),
    ] {
        let mut record = vec![0; size];
        record[..8].copy_from_slice(magic);
        put32(&mut record, 8, 4);
        put32(&mut record, 12, size as u32);
        record[16..32].copy_from_slice(&identity);
        record[32] = replica;
        put64(&mut record, 40, 1);
        record[56..72].copy_from_slice(&identity);
        put64(&mut record, 72, 2);
        put64(&mut record, 80, 1);
        put64(&mut record, 88, 1);
        put64(&mut record, 96, unit.len() as u64);
        record[104..108].copy_from_slice(&unit[108..112]);
        put64(&mut record, 112, 2);
        let crc = crc32c(&record);
        put32(&mut record, 108, crc);
        result.insert(name, record);
    }
    result.insert("UNIT", unit);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_units_match_retained_independent_vectors() {
        let retained = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf()
            .join("test/data/current-single-table-format");
        let seed = fs::read(retained.join("UNIT")).unwrap();
        let row = Row {
            numbers: [
                1.0_f64.to_bits(),
                2.0_f64.to_bits(),
                0.5_f64.to_bits(),
                0.25_f64.to_bits(),
            ],
            keys: *b"AF",
            day: 0,
        };
        assert_eq!(unit(&seed, &[row]).unwrap(), seed);
        assert_eq!(
            unit(&seed, &[]).unwrap(),
            fs::read(retained.join("EMPTY.UNIT")).unwrap()
        );
    }
}
