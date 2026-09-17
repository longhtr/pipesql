//! Change stored fields and repair referring checksums to reach structural checks.
//!
//! Inputs are private copies of a successfully inspected seed. Fixed offsets here
//! describe deliberate edits to that seed; this is not a decoder for arbitrary data.
//!
//! Edits propagate new checksums through the referring objects and roots, preserving
//! the surrounding graph until the selected structural invariant is checked.
//! Callers choose the mutation and expected rejection; these helpers neither decide
//! which stored state is authoritative nor repair a user database.

use crate::{Result, oracle::catalog::crc32c};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn get(data: &[u8], at: usize, width: usize) -> u64 {
    let mut bytes = [0; 8];
    bytes[..width].copy_from_slice(&data[at..at + width]);
    u64::from_le_bytes(bytes)
}

pub fn put(data: &mut [u8], at: usize, value: u64, width: usize) {
    data[at..at + width].copy_from_slice(&value.to_le_bytes()[..width]);
}

fn object(data: &[u8], at: usize) -> String {
    format!("{:016x}-{:08x}.obj", get(data, at, 8), get(data, at + 8, 4))
}

pub fn root(path: &Path, edit: impl FnOnce(&mut [u8])) -> Result<()> {
    let mut bytes = fs::read(path)?;
    edit(&mut bytes);
    put(&mut bytes, 108, 0, 4);
    let checksum = crc32c(&bytes);
    put(&mut bytes, 108, u64::from(checksum), 4);
    fs::write(path, bytes)?;
    Ok(())
}

pub struct Mutation {
    pub path: PathBuf,
    pub schema: String,
    catalog_name: String,
    history: String,
    catalog: Vec<u8>,
    index_name: String,
    index: Vec<u8>,
}

impl Mutation {
    pub fn catalog_path(&self) -> PathBuf {
        self.path.join("units").join(&self.catalog_name)
    }

    pub fn payload(
        &mut self,
        column: u64,
        repair_checksum: bool,
        edit: impl FnOnce(&mut [u8], usize),
    ) -> Result<()> {
        self.change(if repair_checksum { "payload" } else { "unit" }, |data| {
            let columns = get(data, 12, 4) as usize;
            let descriptor = (64..64 + 32 * columns)
                .step_by(32)
                .find(|at| get(data, *at, 4) == column)
                .expect("declared column");
            let at = get(data, descriptor + 8, 8) as usize;
            edit(data, at);
        })
    }

    pub fn new(path: &Path) -> Result<Self> {
        let root = fs::read(path.join("ROOT.A"))?;
        let catalog_name = object(&root, 128);
        let history = object(&root, 152);
        let catalog = fs::read(path.join("units").join(&catalog_name))?;
        let schema = object(&catalog, 112);
        let index_name = object(&catalog, 136);
        let index = fs::read(path.join("units").join(&index_name))?;
        Ok(Self {
            path: path.to_owned(),
            schema,
            catalog_name,
            history,
            catalog,
            index_name,
            index,
        })
    }

    pub fn roots(&self, edit: impl Fn(&mut [u8])) -> Result<()> {
        for name in ["ROOT.A", "ROOT.B", "WAL"] {
            root(&self.path.join(name), &edit)?;
        }
        Ok(())
    }

    fn save_catalog(&self) -> Result<()> {
        fs::write(
            self.path.join("units").join(&self.catalog_name),
            &self.catalog,
        )?;
        self.roots(|root| put(root, 144, u64::from(crc32c(&self.catalog)), 4))
    }

    pub fn change(&mut self, kind: &str, edit: impl FnOnce(&mut [u8])) -> Result<()> {
        if kind == "catalog" {
            edit(&mut self.catalog);
            return self.save_catalog();
        }
        if kind == "schema" || kind == "history" {
            let name = if kind == "schema" {
                &self.schema
            } else {
                &self.history
            };
            let path = self.path.join("units").join(name);
            let mut data = fs::read(&path)?;
            edit(&mut data);
            fs::write(path, &data)?;
            let crc = u64::from(crc32c(&data));
            if kind == "history" {
                return self.roots(|root| put(root, 168, crc, 4));
            }
            put(&mut self.catalog, 128, crc, 4);
            return self.save_catalog();
        }
        if kind == "index" {
            edit(&mut self.index);
        } else {
            assert!(matches!(kind, "unit" | "payload" | "last-payload"));
            let at = if kind == "last-payload" { 112 } else { 64 };
            let path = self.path.join("units").join(object(&self.index, at));
            let mut data = fs::read(&path)?;
            let metadata = 64 + 32 * get(&data, 12, 4) as usize;
            edit(&mut data);
            if kind != "unit" {
                for offset in (64..metadata).step_by(32) {
                    let begin = get(&data, offset + 8, 8) as usize;
                    let length = get(&data, offset + 16, 4) as usize;
                    let crc = crc32c(&data[begin..begin + length]);
                    put(&mut data, offset + 20, u64::from(crc), 4);
                }
            }
            fs::write(path, &data)?;
            put(
                &mut self.index,
                at + 20,
                u64::from(crc32c(&data[..metadata])),
                4,
            );
        }
        fs::write(self.path.join("units").join(&self.index_name), &self.index)?;
        put(&mut self.catalog, 152, u64::from(crc32c(&self.index)), 4);
        self.save_catalog()
    }
}
