//! Read the filename and object fields returned by Darwin's getattrlist call.
//!
//! Path traversal requests one fixed set of attributes. The response contains
//! those fields followed by the filename; an attribute reference locates the
//! name within the record. Decode that layout and lend the filename bytes from
//! the supplied buffer. Filenames need not be UTF-8.
//!
//! Reject a name that extends outside the record, lacks a single final NUL, or
//! contains a slash, or is `.` or `..`. Returning None prevents traversal from
//! using a malformed response as the next path component.

const FIXED_BYTES: usize = 28;
const NAME_BYTES: usize = 1024;
pub(crate) const RECORD_BYTES: usize = FIXED_BYTES + NAME_BYTES;

#[derive(Debug, PartialEq)]
pub(crate) struct NameRecord<'a> {
    pub(crate) device: i32,
    pub(crate) object_type: u32,
    // ATTR_CMN_OBJID helps native name traversal, but is not a file identity.
    // Two hard links can report different values for the same lstat device/inode
    // pair. Never use this field to identify a database file or its lease.
    pub(crate) legacy_object: u32,
    pub(crate) name: &'a [u8],
}

pub(crate) fn decode(bytes: &[u8]) -> Option<NameRecord<'_>> {
    if bytes.len() < FIXED_BYTES || bytes.len() > RECORD_BYTES {
        return None;
    }
    let word = |offset: usize| u32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let length = usize::try_from(word(0)).ok()?;
    if length < FIXED_BYTES || length > bytes.len() {
        return None;
    }
    // The signed offset starts at the attribute reference (byte 4). Adding it
    // to the record start instead would decode the wrong bytes as the name.
    let relative = i32::from_ne_bytes(bytes[4..8].try_into().unwrap());
    let start = 4_usize.checked_add_signed(isize::try_from(relative).ok()?)?;
    let name_length = usize::try_from(word(8)).ok()?;
    if start < FIXED_BYTES || !(2..=NAME_BYTES).contains(&name_length) {
        return None;
    }
    let end = start.checked_add(name_length)?;
    if end > length {
        return None;
    }
    let terminated = &bytes[start..end];
    if terminated[name_length - 1] != 0 || terminated[..name_length - 1].contains(&0) {
        return None;
    }
    let name = &terminated[..name_length - 1];
    if name.contains(&b'/') || name == b"." || name == b".." {
        return None;
    }
    Some(NameRecord {
        device: i32::from_ne_bytes(bytes[12..16].try_into().unwrap()),
        object_type: word(16),
        legacy_object: word(20),
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> [u8; RECORD_BYTES] {
        let mut bytes = [0; RECORD_BYTES];
        bytes[..4].copy_from_slice(&32_u32.to_ne_bytes());
        bytes[4..8].copy_from_slice(&24_i32.to_ne_bytes());
        bytes[8..12].copy_from_slice(&4_u32.to_ne_bytes());
        bytes[28..32].copy_from_slice(b"abc\0");
        bytes
    }

    #[test]
    fn offset_length_and_termination_are_independent() {
        let good = fixture();
        assert_eq!(decode(&good).unwrap().name, b"abc");
        for length in 0..32 {
            assert!(decode(&good[..length]).is_none());
        }
        for (offset, values) in [
            (0, &[0, 27, 31, 1053, u32::MAX][..]),
            (4, &[0, 23, 25, i32::MAX as u32, u32::MAX][..]),
            (8, &[0, 1, 5, 1025, u32::MAX][..]),
        ] {
            for value in values {
                let mut bytes = good;
                bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
                assert!(decode(&bytes).is_none(), "offset={offset} value={value}");
            }
        }
        let mut slash = good;
        slash[29] = b'/';
        assert!(decode(&slash).is_none());
        for special in [b".\0".as_slice(), b"..\0".as_slice()] {
            let mut bytes = good;
            bytes[8..12].copy_from_slice(&(special.len() as u32).to_ne_bytes());
            bytes[28..28 + special.len()].copy_from_slice(special);
            assert!(decode(&bytes).is_none());
        }
        for index in 28..32 {
            let mut bytes = good;
            bytes[index] = if index == 31 { b'x' } else { 0 };
            assert!(decode(&bytes).is_none());
        }
    }

    #[test]
    fn maximum_name_and_non_utf8_are_borrowed_without_allocation() {
        let mut bytes = [0xff; RECORD_BYTES];
        bytes[..4].copy_from_slice(&(RECORD_BYTES as u32).to_ne_bytes());
        bytes[4..8].copy_from_slice(&24_i32.to_ne_bytes());
        bytes[8..12].copy_from_slice(&(NAME_BYTES as u32).to_ne_bytes());
        bytes[RECORD_BYTES - 1] = 0;
        let record = decode(&bytes).unwrap();
        assert_eq!(record.name.len(), NAME_BYTES - 1);
        assert_eq!(record.name.as_ptr(), bytes[FIXED_BYTES..].as_ptr());
        bytes[4..8].copy_from_slice(&25_i32.to_ne_bytes());
        assert!(decode(&bytes).is_none());
    }
}
