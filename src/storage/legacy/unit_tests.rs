//! Check that private-unit verification reads the reserved padding.
//!
//! The checked-in empty and one-row fixtures supply valid bytes. Changing either
//! edge of their padding must fail readback, even when the unit has no data
//! blocks. Otherwise construction could publish an empty unit that later readers
//! reject. The healthy case prevents a verifier that always fails from passing.

use super::{BLOCK_BYTES, verify_unit};
use crate::effects::Effects;
use crate::storage::format::{self};
use crate::storage::legacy::tests::TempDir;
use crate::{CancellationToken, Error};
use std::fs::{self, File};

#[test]
fn private_readback_requires_zero_padding_even_without_payload() {
    for rows in [0, 1] {
        let fixture = if rows == 0 {
            include_bytes!("../../../test/data/current-single-table-format/EMPTY.UNIT").as_slice()
        } else {
            include_bytes!("../../../test/data/current-single-table-format/UNIT").as_slice()
        };
        let header: &[u8; format::HEADER_BYTES] =
            fixture[..format::HEADER_BYTES].try_into().unwrap();
        let padding_start = format::HEADER_BYTES + format::DESCRIPTOR_BYTES;
        let descriptors: &[u8; format::DESCRIPTOR_BYTES] = fixture
            [format::HEADER_BYTES..padding_start]
            .try_into()
            .unwrap();
        let (_, checksum) = format::decode_unit_metadata(header, descriptors).unwrap();
        for changed in [None, Some(0), Some(format::UNIT_PADDING_BYTES - 1)] {
            let temp = TempDir::new();
            let path = temp.0.join("private-unit");
            let mut bytes = fixture.to_vec();
            if let Some(offset) = changed {
                bytes[padding_start + offset] = 1;
            }
            fs::write(&path, bytes).unwrap();
            let file = File::open(&path).unwrap();
            let mut buffer = vec![0; BLOCK_BYTES];
            let outcome = verify_unit(
                &file,
                (header, descriptors),
                checksum,
                u64::try_from(fixture.len()).unwrap(),
                &mut buffer,
                &CancellationToken::new(),
                &mut Effects::default(),
            );
            if changed.is_some() {
                assert!(matches!(
                    outcome,
                    Err(Error::Corrupt("private unit padding is nonzero"))
                ));
            } else {
                outcome.unwrap();
            }
        }
    }
}
