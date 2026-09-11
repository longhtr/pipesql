use super::{STAGING_BUFFER_BYTES, Staging};
use crate::effects::Effects;
use crate::load::tests::{TempDir, row};
use crate::load_input::Scanner;
use crate::namespace::PRIVATE_NAME;
use crate::{CancellationToken, Error};
use std::fs::{self};

#[test]
fn staging_admission_refuses_next_row_without_mutation() {
    let source = row("1", "2", "0.5", "1970-01-01");
    let mut scanner = Scanner::new();
    scanner.begin_chunk(source.as_bytes()).unwrap();
    let value = source
        .bytes()
        .find_map(|byte| scanner.consume(byte).unwrap())
        .unwrap();
    let token = CancellationToken::new();
    // Exercise the private writer with an explicit trusted admission. Input
    // files are not changed; this isolates row/extent ownership from parsing.
    for admitted in [0_u64, 1, 32_767, 32_768, 32_769, 65_537] {
        let temp = TempDir::new();
        fs::create_dir(temp.0.join(PRIVATE_NAME)).unwrap();
        let mut buffers = vec![0; STAGING_BUFFER_BYTES];
        let mut effects = Effects::default();
        let mut staging = Staging::new(&temp.0, &mut buffers, admitted, &mut effects).unwrap();
        for _ in 0..admitted {
            staging.append(value, 0, &token, &mut effects).unwrap();
        }
        let before = effects.count();
        let saved = staging.buffers.to_vec();
        let buffered = staging.buffered;
        let written = staging.written;
        for _ in 0..2 {
            assert!(matches!(
                staging.append(value, 123, &token, &mut effects),
                Err(Error::Input {
                    message: "input exceeds admitted row count",
                    byte_offset: 123
                })
            ));
        }
        assert_eq!(effects.count(), before);
        assert_eq!(staging.buffers, saved);
        assert_eq!(staging.buffered, buffered);
        assert_eq!(staging.written, written);
        for (index, width) in [8_u64, 8, 8, 8, 1, 1, 4].into_iter().enumerate() {
            let physical = staging.files[index].metadata().unwrap().len();
            assert_eq!(physical, written[index]);
            assert_eq!(physical + buffered[index] as u64, admitted * width);
        }
    }
}

#[test]
fn staging_extent_refusal_precedes_write_effect() {
    let temp = TempDir::new();
    fs::create_dir(temp.0.join(PRIVATE_NAME)).unwrap();
    let mut buffers = vec![0; STAGING_BUFFER_BYTES];
    let mut effects = Effects::default();
    let mut staging = Staging::new(&temp.0, &mut buffers, 0, &mut effects).unwrap();
    // Narrow test-only producer fault: an unadmitted byte reaches flush.
    // The consumer must refuse before native I/O or counter publication.
    for index in 0..7 {
        staging.buffered[index] = 1;
        let before = effects.count();
        assert!(matches!(
            staging.flush(index, &CancellationToken::new(), &mut effects),
            Err(Error::Resource {
                owner: "staging extent",
                required: 1,
                limit: 0
            })
        ));
        assert_eq!(effects.count(), before);
        assert_eq!(staging.files[index].metadata().unwrap().len(), 0);
        assert_eq!(staging.written[index], 0);
        assert_eq!(staging.buffered[index], 1);
    }
}
