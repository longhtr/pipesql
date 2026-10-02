//! Incremental IEEE CRC-32 with a portable table fallback.
//!
//! Parquet uses this polynomial; native records use the separate CRC32C type.
//! Neither implementation allocates. Callers retain their own cancellation and
//! format-validation boundaries around updates.

/// IEEE CRC-32 with initial and final inversion (the CRC used by Parquet).
/// Separate updates produce the checksum of their concatenation.
#[derive(Clone, Copy)]
pub struct Crc32(u32);

impl Crc32 {
    /// Start a checksum of an empty input.
    #[inline]
    pub fn new() -> Self {
        Self(u32::MAX)
    }

    /// Append bytes without allocating. Empty slices leave the state unchanged.
    #[inline]
    pub fn update(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        #[cfg(all(target_arch = "aarch64", any(target_os = "linux", target_os = "macos")))]
        if let Some(state) = super::aarch64::update_ieee(self.0, bytes) {
            self.0 = state;
            return;
        }
        self.update_portable(bytes);
    }

    fn update_portable(&mut self, bytes: &[u8]) {
        let (groups, tail) = bytes.as_chunks::<8>();
        for bytes in groups {
            let first = (u32::from_le_bytes(bytes[..4].try_into().expect("four bytes")) ^ self.0)
                .to_le_bytes();
            self.0 = TABLES[7][usize::from(first[0])]
                ^ TABLES[6][usize::from(first[1])]
                ^ TABLES[5][usize::from(first[2])]
                ^ TABLES[4][usize::from(first[3])]
                ^ TABLES[3][usize::from(bytes[4])]
                ^ TABLES[2][usize::from(bytes[5])]
                ^ TABLES[1][usize::from(bytes[6])]
                ^ TABLES[0][usize::from(bytes[7])];
        }
        for byte in tail {
            self.0 = (self.0 >> 8) ^ TABLES[0][usize::from(self.0 as u8 ^ byte)];
        }
    }

    /// Return the checksum of all appended bytes.
    #[inline]
    pub fn finish(self) -> u32 {
        !self.0
    }
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

// The reflected IEEE polynomial, with each table accounting for one byte's
// position in an eight-byte group. These are not the native CRC32C tables.
static TABLES: [[u32; 256]; 8] = {
    let mut tables = [[0; 256]; 8];
    let mut byte = 0;
    while byte < 256 {
        let mut crc = byte as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
            bit += 1;
        }
        tables[0][byte] = crc;
        byte += 1;
    }
    let mut position = 1;
    while position < tables.len() {
        let mut byte = 0;
        while byte < 256 {
            let previous = tables[position - 1][byte];
            tables[position][byte] = (previous >> 8) ^ tables[0][(previous & 255) as usize];
            byte += 1;
        }
        position += 1;
    }
    tables
};

#[cfg(test)]
mod tests {
    use super::*;

    // Forward IEEE polynomial and reversed input bits: no production tables or
    // intrinsics. Return the reflected state, before final inversion.
    fn bitwise(state: u32, bytes: &[u8]) -> u32 {
        let mut crc = state.reverse_bits();
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
        crc.reverse_bits()
    }

    #[test]
    fn ieee_vectors_keep_the_polynomial_and_empty_updates() {
        assert_eq!(Crc32::new().finish(), 0);
        let mut crc = Crc32::default();
        crc.update(b"1234");
        crc.update(b"");
        crc.update(b"56789");
        assert_eq!(crc.finish(), 0xcbf4_3926);
        assert_ne!(crc.finish(), crate::crc32c(b"123456789"));
    }

    #[test]
    fn ieee_native_portable_and_partitioned_states_match_independent_bits() {
        let bytes: Vec<_> = (0_u32..4112)
            .map(|n| (n.wrapping_mul(157) ^ (n >> 3) ^ (n >> 8)) as u8)
            .collect();
        #[cfg(all(target_arch = "aarch64", any(target_os = "linux", target_os = "macos")))]
        let hardware = {
            let observed = crate::aarch64::update_ieee(0, &[]).is_some();
            assert_eq!(observed, std::arch::is_aarch64_feature_detected!("crc"));
            println!("AArch64 IEEE CRC-32 available: {observed}");
            observed
        };
        for offset in 0..16 {
            for length in (0..=257).chain([1023, 1024, 1025, 4095, 4096, 4097]) {
                let input = &bytes[offset..offset + length];
                for state in [0, u32::MAX, 0x1234_5678, 0xedb8_8320] {
                    let expected = bitwise(state, input);
                    let mut selected = Crc32(state);
                    selected.update(input);
                    assert_eq!(
                        selected.0, expected,
                        "selected: {offset}, {length}, {state}"
                    );
                    let mut portable = Crc32(state);
                    portable.update_portable(input);
                    assert_eq!(
                        portable.0, expected,
                        "portable: {offset}, {length}, {state}"
                    );
                    #[cfg(all(
                        target_arch = "aarch64",
                        any(target_os = "linux", target_os = "macos")
                    ))]
                    assert_eq!(
                        crate::aarch64::update_ieee(state, input),
                        hardware.then_some(expected)
                    );
                    for size in [1, 7, 8, 9, 63, 64, 4096] {
                        let mut partitioned = Crc32(state);
                        for (index, chunk) in input.chunks(size).enumerate() {
                            if index % 2 == 0 {
                                partitioned.update(chunk);
                            } else {
                                partitioned.update_portable(chunk);
                            }
                            partitioned.update(&[]);
                        }
                        assert_eq!(
                            partitioned.0, expected,
                            "partition: {offset}, {length}, {state}, {size}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn ieee_large_input_and_concurrent_callers_match_independent_bits() {
        let bytes: Vec<_> = (0_u32..262_145)
            .map(|n| (n.wrapping_mul(83) ^ (n >> 7)) as u8)
            .collect();
        let expected = !bitwise(u32::MAX, &bytes);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    let mut crc = Crc32::new();
                    crc.update(&bytes);
                    assert_eq!(crc.finish(), expected);
                });
            }
        });
        let mut portable = Crc32::new();
        portable.update_portable(&bytes);
        assert_eq!(portable.finish(), expected);
    }
}
