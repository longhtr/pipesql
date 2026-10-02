//! Challenge the independent catalog reader with malformed records and options.
//!
//! A published CRC32C vector checks the checksum calculation independently. Small
//! root and reference records distinguish damaged bytes from conflicting authority,
//! and test complete identities and extents rather than a checksum alone.
//! Inspection options must reject missing, duplicate or excessive bounds. These
//! focused cases check refusal and classification without relying on engine-produced
//! valid databases to expose the reader's own assumptions.

use super::*;

#[test]
fn inspection_options_reject_unbounded_or_ambiguous_requests() {
    for arguments in [
        vec!["--max-values"],
        vec!["--unknown", "1"],
        vec!["--max-values", "0"],
        vec!["--max-values", "10000001"],
        vec!["--max-objects", "1048577"],
        vec!["--max-read-bytes", "1073741825"],
        vec!["--max-values", "1", "--max-values", "2"],
    ] {
        let arguments = arguments.into_iter().map(str::to_owned).collect::<Vec<_>>();
        assert!(Limits::options(&arguments).is_err(), "{arguments:?}");
    }
    let arguments = [
        "--max-values",
        "10000000",
        "--max-objects",
        "1048576",
        "--max-read-bytes",
        "1073741824",
    ]
    .map(str::to_owned);
    let limits = Limits::options(&arguments).unwrap();
    assert_eq!(
        (limits.objects, limits.read_bytes, limits.values),
        (1_048_576, 1_073_741_824, 10_000_000)
    );
}

fn checksum(bytes: &mut [u8]) {
    bytes[108..112].fill(0);
    let crc = crc32c(bytes);
    bytes[108..112].copy_from_slice(&crc.to_le_bytes());
}

fn genesis(wal: bool) -> Vec<u8> {
    let size: u32 = if wal { 512 } else { 4096 };
    let mut bytes = vec![0; size as usize];
    bytes[..8].copy_from_slice(if wal { b"PSQLWAL\0" } else { b"PSQLROOT" });
    bytes[8..12].copy_from_slice(&7_u32.to_le_bytes());
    bytes[12..16].copy_from_slice(&size.to_le_bytes());
    bytes[16..32].fill(0x17);
    bytes[32] = 0; // ROOT.A and WAL both carry role zero.
    checksum(&mut bytes);
    bytes
}

#[test]
fn checksum_matches_published_crc32c_vector() {
    assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    assert_eq!(crc32c(b""), 0);
}

#[test]
fn snapshot_damage_and_conflicting_authority_are_distinct() {
    for wal in [false, true] {
        let bytes = genesis(wal);
        let role = 0;
        let decode = |data: &[u8]| Snapshot::decode(data, &[0x17; 16], role, wal);
        let snapshot = decode(&bytes).unwrap().unwrap();
        assert_eq!(
            (snapshot.issued, snapshot.generation, snapshot.last),
            (0, 0, 0)
        );
        assert!(snapshot.catalog.is_none() && snapshot.successes.is_none());

        for end in 0..bytes.len() {
            assert!(decode(&bytes[..end]).unwrap().is_none(), "extent {end}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(decode(&longer).unwrap().is_none());

        let mut damaged = bytes.clone();
        damaged[200] = 1;
        assert!(decode(&damaged).unwrap().is_none());
        // Recompute checksums to reach structural validation rather than merely
        // testing the checksum guard repeatedly.
        for (offset, value, reason) in [
            (8, 8, "unsupported authoritative version"),
            (16, 0x18, "foreign snapshot identity"),
            (32, 1, "snapshot extent/role"),
            (33, 1, "nonzero reserved bytes"),
            (128, 1, "nonzero reserved bytes"),
        ] {
            let mut invalid = bytes.clone();
            invalid[offset] = value;
            checksum(&mut invalid);
            let error = decode(&invalid).unwrap_err().to_string();
            assert!(error.contains(reason), "offset {offset}: {error}");
        }
    }
}

#[test]
fn references_require_complete_identity_and_extent() {
    let mut bytes = [0; 24];
    bytes[..8].copy_from_slice(&1_u64.to_le_bytes());
    bytes[8..12].copy_from_slice(&1_u32.to_le_bytes());
    bytes[12..16].copy_from_slice(&64_u32.to_le_bytes());
    assert_eq!(
        Reference::decode(&bytes).unwrap().name(),
        "0000000000000001-00000001.obj"
    );
    for end in 0..bytes.len() {
        assert!(Reference::decode(&bytes[..end]).is_err());
    }
    for offset in [0, 8, 12] {
        let mut invalid = bytes;
        invalid[offset] = 0;
        assert!(Reference::decode(&invalid).is_err());
    }
    bytes[20] = 1;
    assert!(Reference::decode(&bytes).is_err());
}
