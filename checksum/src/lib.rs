//! CRC32C for native records and IEEE CRC-32 for Parquet pages.
//!
//! AArch64 uses CRC instructions when the compilation target or runtime CPU
//! supports them. Other targets and failed capability probes use portable tables.
//! Neither path allocates or reads files. The engine remains free of unsafe Rust;
//! this crate confines native capability checks and instruction dispatch to one
//! private module. Format interpretation and checksum rejection belong to callers.

#![deny(unsafe_code)]

mod ieee;
pub use ieee::Crc32;

#[cfg(all(target_arch = "aarch64", any(target_os = "linux", target_os = "macos")))]
#[allow(unsafe_code)]
mod aarch64;

/// Compute CRC32C using the Castagnoli polynomial and initial/final inversion.
#[inline]
pub fn crc32c(bytes: &[u8]) -> u32 {
    let mut crc = Crc32c::new();
    crc.update(bytes);
    crc.finish()
}

/// Incremental CRC32C. Updating with separate slices gives the same checksum
/// as their concatenation, without allocating that concatenation.
#[derive(Clone, Copy)]
pub struct Crc32c(u32);

impl Crc32c {
    /// Start a checksum of an empty input.
    #[inline]
    pub fn new() -> Self {
        Self(u32::MAX)
    }

    /// Append bytes without allocation. Empty slices leave the state unchanged.
    #[inline]
    pub fn update(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        #[cfg(all(target_arch = "aarch64", any(target_os = "linux", target_os = "macos")))]
        if let Some(state) = aarch64::update(self.0, bytes) {
            self.0 = state;
            return;
        }
        self.update_portable(bytes);
    }

    fn update_portable(&mut self, bytes: &[u8]) {
        let mut offset = 0_usize;
        // Process eight bytes at once. Each table accounts for a byte's position
        // in the group; the first four also absorb the previous CRC state.
        while bytes.len() - offset >= 8 {
            let first = u32::from_le_bytes(
                bytes[offset..offset + 4]
                    .try_into()
                    .expect("CRC chunk has four bytes"),
            ) ^ self.0;
            let first_bytes = first.to_le_bytes();
            self.0 = CRC32C_TABLES[7][usize::from(first_bytes[0])]
                ^ CRC32C_TABLES[6][usize::from(first_bytes[1])]
                ^ CRC32C_TABLES[5][usize::from(first_bytes[2])]
                ^ CRC32C_TABLES[4][usize::from(first_bytes[3])]
                ^ CRC32C_TABLES[3][usize::from(bytes[offset + 4])]
                ^ CRC32C_TABLES[2][usize::from(bytes[offset + 5])]
                ^ CRC32C_TABLES[1][usize::from(bytes[offset + 6])]
                ^ CRC32C_TABLES[0][usize::from(bytes[offset + 7])];
            offset += 8;
        }
        for byte in &bytes[offset..] {
            let index = (self.0 as u8) ^ byte;
            self.0 = CRC32C_TABLES[0][usize::from(index)] ^ (self.0 >> 8);
        }
    }

    /// Return the checksum of all appended bytes.
    #[inline]
    pub fn finish(self) -> u32 {
        self.0 ^ u32::MAX
    }
}

impl Default for Crc32c {
    fn default() -> Self {
        Self::new()
    }
}

const CRC32C_TABLES: [[u32; 256]; 8] = make_crc32c_tables();

// Table 0 advances one byte using the reflected Castagnoli polynomial. Each
// later table advances the preceding table through one additional zero byte.
const fn make_crc32c_tables() -> [[u32; 256]; 8] {
    let mut tables = [[0_u32; 256]; 8];
    let mut index = 0_usize;
    while index < tables[0].len() {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 == 1 {
                (value >> 1) ^ 0x82f6_3b78
            } else {
                value >> 1
            };
            bit += 1;
        }
        tables[0][index] = value;
        index += 1;
    }
    let mut table = 1_usize;
    while table < tables.len() {
        index = 0;
        while index < tables[table].len() {
            let previous = tables[table - 1][index];
            tables[table][index] = (previous >> 8) ^ tables[0][(previous & 0xff) as usize];
            index += 1;
        }
        table += 1;
    }
    tables
}

#[cfg(test)]
mod tests;
