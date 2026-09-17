//! Describe declared columns independently of SQL and stored records.
//!
//! A declaration supplies a name, type and NULL rule; storage assigns identities
//! when a table is created. The name grammar is shared by declarations and input
//! headers. Callers translate rejection into their own input or format error.

pub(crate) const MAX_COLUMNS: usize = 64;
pub(crate) const MAX_NAME_BYTES: usize = 32;

/// A borrowed column definition used to declare or inspect a table.
/// Storage assigns persistent identities separately; callers supply names, types
/// and whether NULL is allowed. The name must remain alive while this value is used.
#[derive(Clone, Copy)]
pub struct ColumnDeclaration<'a> {
    pub name: &'a str,
    pub data_type: crate::DataType,
    pub nullable: bool,
}

pub(crate) fn valid_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && (name[0].is_ascii_alphabetic() || name[0] == b'_')
        && name
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}
