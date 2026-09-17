//! Encode declared-table samples without importing production codecs.
//!
//! Schema and physical column orders differ. Four rows separate NULL, empty text,
//! Unicode, embedded NUL, signed zero, NaN and infinity. Attempts 3 and 5 succeed;
//! issued attempt 6 and the intervening gaps are not successes.
//!
//! The generated maps describe exact object bytes for comparison with retained
//! samples. Database construction adds the namespace records that refer to those
//! objects. Explicit offsets and checksums keep this encoder independent of the
//! production layout code; the campaign still checks every resulting filename and byte.

use super::catalog::crc32c;
use crate::Result;
use std::{collections::BTreeMap, fs, path::Path};

type Files = BTreeMap<&'static str, Vec<u8>>;
fn u32_at(data: &mut [u8], at: usize, value: u32) {
    data[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn u64_at(data: &mut [u8], at: usize, value: u64) {
    data[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn header(magic: &[u8; 8], count: u32) -> Vec<u8> {
    let mut data = vec![0; 64];
    data[..8].copy_from_slice(magic);
    u32_at(&mut data, 8, 6);
    u32_at(&mut data, 12, count);
    data[16..32].fill(7);
    data
}
fn reference(data: &mut [u8], at: usize, attempt: u64, ordinal: u32, object: &[u8]) {
    u64_at(data, at, attempt);
    u32_at(data, at + 8, ordinal);
    u32_at(data, at + 12, object.len() as u32);
    u32_at(data, at + 16, crc32c(object));
}
fn schema() -> Vec<u8> {
    let mut data = header(b"PSQLSCHM", 2);
    u64_at(&mut data, 32, 0x0102030405060708);
    for (id, name, kind, nullable) in [(29, "note", 3, 1), (3, "amount", 2, 0)] {
        let mut column = [0; 48];
        u32_at(&mut column, 0, id);
        column[4..7].copy_from_slice(&[kind, nullable, name.len() as u8]);
        column[8..8 + name.len()].copy_from_slice(name.as_bytes());
        data.extend_from_slice(&column);
    }
    data
}
fn catalog(schema: &[u8]) -> Vec<u8> {
    let mut data = header(b"PSQLCATL", 1);
    data.resize(192, 0);
    u64_at(&mut data, 32, 4);
    u32_at(&mut data, 40, 1);
    u64_at(&mut data, 64, 0x0102030405060708);
    data[72] = 5;
    data[80..85].copy_from_slice(b"facts");
    reference(&mut data, 112, 1, 1, schema);
    data
}
fn native() -> Vec<u8> {
    let mut doubles = vec![15];
    for bits in [
        0x8000000000000000_u64,
        0x7ff0000000000001,
        0x7ff0000000000000,
        0xffefffffffffffff,
    ] {
        doubles.extend_from_slice(&bits.to_le_bytes());
    }
    let text = "雪é\0🙂".as_bytes();
    let mut strings = vec![14];
    for offset in [0, 0, 0, 3, text.len() as u32] {
        strings.extend_from_slice(&offset.to_le_bytes());
    }
    strings.extend_from_slice(text);
    let mut data = header(b"PSQLDATA", 2);
    u64_at(&mut data, 32, 0x0102030405060708);
    u64_at(&mut data, 40, 3);
    u32_at(&mut data, 48, 2);
    u32_at(&mut data, 52, 4);
    u32_at(&mut data, 56, 128);
    let mut offset = 128;
    for (id, kind, nullable, payload) in [(3, 2, 0, &doubles), (29, 3, 1, &strings)] {
        let mut entry = [0; 32];
        u32_at(&mut entry, 0, id);
        entry[4..6].copy_from_slice(&[kind, nullable]);
        u64_at(&mut entry, 8, offset);
        u32_at(&mut entry, 16, payload.len() as u32);
        u32_at(&mut entry, 20, crc32c(payload));
        data.extend_from_slice(&entry);
        offset += payload.len() as u64;
    }
    data.extend(doubles);
    data.extend(strings);
    data
}
fn table(native: &[u8]) -> Vec<u8> {
    let mut data = header(b"PSQLTBLD", 1);
    data.resize(112, 0);
    u64_at(&mut data, 32, 0x0102030405060708);
    u64_at(&mut data, 40, 4);
    u32_at(&mut data, 48, 2);
    u64_at(&mut data, 56, 4);
    u64_at(&mut data, 64, 3);
    u32_at(&mut data, 72, 2);
    u32_at(&mut data, 76, 4);
    u32_at(&mut data, 80, native.len() as u32);
    u32_at(&mut data, 84, crc32c(&native[..128]));
    data
}

pub fn vectors() -> BTreeMap<&'static str, Files> {
    let schema = schema();
    let catalog = catalog(&schema);
    let native = native();
    let index = table(&native);
    let mut data_catalog = catalog.clone();
    u64_at(&mut data_catalog, 32, 5);
    reference(&mut data_catalog, 136, 4, 2, &index);
    u64_at(&mut data_catalog, 160, 4);
    u32_at(&mut data_catalog, 168, 1);
    let schema_files = BTreeMap::from([
        ("columns.bin", schema),
        ("catalog.bin", catalog),
        ("native-unit.bin", native),
        ("table-data.bin", index.clone()),
        ("data-catalog.bin", data_catalog.clone()),
    ]);
    let mut index = index;
    u64_at(&mut index, 40, 5);
    u32_at(&mut index, 48, 3);
    let mut catalog = data_catalog;
    u64_at(&mut catalog, 32, 5);
    u32_at(&mut catalog, 40, 4);
    u64_at(&mut catalog, 112, 3);
    reference(&mut catalog, 136, 5, 3, &index);
    let mut successes = header(b"PSQLSUCC", 0);
    u64_at(&mut successes, 32, 2);
    u64_at(&mut successes, 40, 5);
    u32_at(&mut successes, 48, 5);
    u64_at(&mut successes, 56, 5);
    successes.extend_from_slice(&3_u64.to_le_bytes());
    successes.extend_from_slice(&5_u64.to_le_bytes());
    let mut roots = BTreeMap::new();
    let mut control = vec![0; 128];
    control[..8].copy_from_slice(b"PIPESQL\0");
    u32_at(&mut control, 8, 7);
    u32_at(&mut control, 12, 128);
    control[16..32].fill(7);
    let crc = crc32c(&control);
    u32_at(&mut control, 36, crc);
    roots.insert("CONTROL", control);
    for (name, size, magic, role, generation, issued) in [
        ("ROOT.A", 4096, b"PSQLROOT", 0, 2, 6),
        ("ROOT.B", 4096, b"PSQLROOT", 1, 2, 6),
        ("WAL", 512, b"PSQLWAL\0", 0, 2, 6),
        ("GENESIS", 4096, b"PSQLROOT", 0, 0, 2),
    ] {
        let mut record = vec![0; size];
        record[..8].copy_from_slice(magic);
        u32_at(&mut record, 8, 7);
        u32_at(&mut record, 12, size as u32);
        record[16..32].fill(7);
        record[32] = role;
        u64_at(&mut record, 40, generation);
        u64_at(&mut record, 112, issued);
        if generation != 0 {
            record[56..72].fill(7);
            u64_at(&mut record, 72, 5);
            reference(&mut record, 128, 5, 4, &catalog);
            reference(&mut record, 152, 5, 5, &successes);
        }
        let crc = crc32c(&record);
        u32_at(&mut record, 108, crc);
        roots.insert(name, record);
    }
    roots.extend([
        ("catalog.bin", catalog),
        ("table-data.bin", index),
        ("successes.bin", successes),
    ]);
    BTreeMap::from([("catalog-schema", schema_files), ("catalog-roots", roots)])
}

/// Copy retained bytes into a fresh writable database; the caller owns cleanup.
pub fn database(destination: &Path, data: &Path) -> Result<()> {
    fs::create_dir(destination)?;
    fs::create_dir(destination.join("units"))?;
    fs::create_dir(destination.join("private"))?;
    fs::write(destination.join("LOCK"), [])?;
    for name in ["CONTROL", "ROOT.A", "ROOT.B", "WAL"] {
        fs::write(
            destination.join(name),
            fs::read(data.join("catalog-roots").join(name))?,
        )?;
    }
    for (name, source) in [
        (
            "0000000000000003-00000001.obj",
            "catalog-schema/columns.bin",
        ),
        (
            "0000000000000003-00000002.obj",
            "catalog-schema/native-unit.bin",
        ),
        (
            "0000000000000005-00000003.obj",
            "catalog-roots/table-data.bin",
        ),
        ("0000000000000005-00000004.obj", "catalog-roots/catalog.bin"),
        (
            "0000000000000005-00000005.obj",
            "catalog-roots/successes.bin",
        ),
    ] {
        fs::write(
            destination.join("units").join(name),
            fs::read(data.join(source))?,
        )?;
    }
    Ok(())
}
