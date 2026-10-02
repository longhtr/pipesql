use super::*;

// Independent bit-at-a-time recurrence, with no production tables or intrinsics.
fn bitwise(mut state: u32, bytes: &[u8]) -> u32 {
    for &byte in bytes {
        state ^= u32::from(byte);
        for _ in 0..8 {
            state = if state & 1 != 0 {
                (state >> 1) ^ 0x82f6_3b78
            } else {
                state >> 1
            };
        }
    }
    state
}

#[test]
fn known_vectors_and_empty_updates() {
    assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    assert_eq!(crc32c(b""), 0);
    let mut crc = Crc32c::default();
    crc.update(b"1234");
    crc.update(b"");
    crc.update(b"56789");
    assert_eq!(crc.finish(), 0xe306_9283);
}

#[test]
fn all_paths_match_bitwise_states_offsets_tails_and_partitions() {
    let bytes: Vec<u8> = (0_u32..4112)
        .map(|n| (n.wrapping_mul(157) ^ (n >> 3) ^ (n >> 8)) as u8)
        .collect();
    #[cfg(all(target_arch = "aarch64", any(target_os = "linux", target_os = "macos")))]
    let hardware = {
        let observed = aarch64::update(0, &[]).is_some();
        // A disabled fast path must not silently turn hardware checks into
        // software-only success on a capable host. This detector is test-only.
        assert_eq!(observed, std::arch::is_aarch64_feature_detected!("crc"));
        println!("AArch64 CRC32C available: {observed}");
        observed
    };
    for offset in 0..16 {
        for length in (0..=257).chain([1023, 1024, 1025, 4096]) {
            let input = &bytes[offset..offset + length];
            for state in [0, u32::MAX, 0x1234_5678, 0x82f6_3b78] {
                let expected = bitwise(state, input);
                let mut selected = Crc32c(state);
                selected.update(input);
                assert_eq!(
                    selected.0, expected,
                    "selected: {offset}, {length}, {state}"
                );
                let mut portable = Crc32c(state);
                portable.update_portable(input);
                assert_eq!(
                    portable.0, expected,
                    "portable: {offset}, {length}, {state}"
                );
                #[cfg(all(target_arch = "aarch64", any(target_os = "linux", target_os = "macos")))]
                assert_eq!(aarch64::update(state, input), hardware.then_some(expected));
                for size in [1, 7, 8, 9, 63, 64] {
                    let mut partitioned = Crc32c(state);
                    for (index, chunk) in input.chunks(size).enumerate() {
                        // States must be interchangeable across implementations.
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
fn large_record_and_concurrent_callers_match_independent_answers() {
    let bytes: Vec<u8> = (0_u32..262_145)
        .map(|n| (n.wrapping_mul(83) ^ (n >> 7)) as u8)
        .collect();
    let expected = bitwise(u32::MAX, &bytes) ^ u32::MAX;
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| assert_eq!(crc32c(&bytes), expected));
        }
    });
    let mut portable = Crc32c::new();
    portable.update_portable(&bytes);
    assert_eq!(portable.finish(), expected);
}
