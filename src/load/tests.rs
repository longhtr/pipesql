use crate::Config;
use crate::namespace::{PRIVATE_NAME, ROOT_A_NAME, ROOT_B_NAME, UNITS_NAME, WAL_NAME};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);
// One-row load includes metadata, reserved-padding, and payload readback.
const LOAD_EFFECT_COUNT: u64 = 179;

pub(super) struct TempDir(pub(super) PathBuf);

impl TempDir {
    pub(super) fn new() -> Self {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("pipesql-load-{}-{id}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn config(memory: u64, temporary: u64) -> Config {
    Config::new(memory, temporary).unwrap()
}

pub(super) fn row(quantity: &str, price: &str, discount: &str, date: &str) -> String {
    format!("1|2|3|4|{quantity}|{price}|{discount}|8|R|F|{date}|12|13|14|15|16|\n")
}

fn independent_crc32c(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0x82f6_3b78 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    crc ^ u32::MAX
}

fn independent_root_generation(path: &Path, replica: u8) -> u64 {
    let mut bytes = fs::read(path).unwrap();
    assert_eq!(bytes.len(), 4_096);
    assert_eq!(&bytes[..8], b"PSQLROOT");
    assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 4);
    assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 4_096);
    assert_eq!(bytes[32], replica);
    let expected = u32::from_le_bytes(bytes[108..112].try_into().unwrap());
    bytes[108..112].fill(0);
    assert_eq!(independent_crc32c(&bytes), expected);
    u64::from_le_bytes(bytes[40..48].try_into().unwrap())
}

fn independent_namespace_matches(root: &Path, generation: u64) {
    assert_eq!(
        independent_root_generation(&root.join(ROOT_A_NAME), 0),
        generation
    );
    assert_eq!(
        independent_root_generation(&root.join(ROOT_B_NAME), 1),
        generation
    );
    let mut fence = fs::read(root.join(WAL_NAME)).unwrap();
    assert_eq!(fence.len(), 512);
    assert_eq!(&fence[..8], b"PSQLWAL\0");
    assert_eq!(u32::from_le_bytes(fence[8..12].try_into().unwrap()), 4);
    let checksum = u32::from_le_bytes(fence[108..112].try_into().unwrap());
    fence[108..112].fill(0);
    assert_eq!(independent_crc32c(&fence), checksum);
    for name in [ROOT_A_NAME, ROOT_B_NAME] {
        let mut root_bytes = fs::read(root.join(name)).unwrap();
        root_bytes[32] = 0;
        root_bytes[108..112].fill(0);
        assert_eq!(&fence[16..128], &root_bytes[16..128]);
    }
    assert_eq!(&fence[128..], &[0; 384]);
    let unit_count = fs::read_dir(root.join(UNITS_NAME)).unwrap().count();
    let private_count = fs::read_dir(root.join(PRIVATE_NAME)).unwrap().count();
    assert_eq!(private_count, 0);
    match generation {
        0 => {
            assert_eq!(unit_count, 0);
        }
        1 => {
            assert_eq!(unit_count, 1);
        }
        _ => panic!("independent checker received unsupported generation"),
    }
}

mod failures;
mod loading;
mod publication;
