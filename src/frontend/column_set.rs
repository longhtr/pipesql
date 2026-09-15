//! Track which query columns are present or needed using one bit per identity.
//!
//! Query column identities have a fixed upper bound, so a fixed array of 64-bit
//! words can represent any set without allocating. For example, identity `c65`
//! occupies bit 1 of word 1, counting both from zero. Bitwise OR combines sets;
//! bitwise AND keeps their common members.
//!
//! Scope validation uses sets of available columns; execution planning uses sets
//! of required columns. Each caller owns that meaning. Zero is reserved, and
//! inserting an identity beyond the query bound is a programmer error. Iteration
//! yields identities in increasing numeric order.

use super::{ColumnId, MAX_QUERY_COLUMNS};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ColumnSet([u64; (MAX_QUERY_COLUMNS + 1).div_ceil(64)]);

impl ColumnSet {
    pub(crate) const EMPTY: Self = Self([0; (MAX_QUERY_COLUMNS + 1).div_ceil(64)]);

    pub(crate) fn len(self) -> usize {
        self.0.iter().map(|word| word.count_ones() as usize).sum()
    }

    pub(crate) fn iter(self) -> impl Iterator<Item = ColumnId> {
        self.0.into_iter().enumerate().flat_map(|(word, mut bits)| {
            std::iter::from_fn(move || {
                if bits == 0 {
                    return None;
                }
                let offset = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                Some(ColumnId::new((word * 64 + offset) as u32))
            })
        })
    }

    pub(crate) fn insert(&mut self, id: ColumnId) {
        *self |= bit(id);
    }

    pub(crate) fn contains(self, id: ColumnId) -> bool {
        let id = id.value() as usize;
        id > 0 && id <= MAX_QUERY_COLUMNS && self.0[id / 64] & (1 << (id % 64)) != 0
    }
}

impl std::ops::BitOr for ColumnSet {
    type Output = Self;

    fn bitor(mut self, rhs: Self) -> Self {
        self |= rhs;
        self
    }
}

impl std::ops::BitOrAssign for ColumnSet {
    fn bitor_assign(&mut self, rhs: Self) {
        for (left, right) in self.0.iter_mut().zip(rhs.0) {
            *left |= right;
        }
    }
}

impl std::ops::BitAnd for ColumnSet {
    type Output = Self;

    fn bitand(mut self, rhs: Self) -> Self {
        for (left, right) in self.0.iter_mut().zip(rhs.0) {
            *left &= right;
        }
        self
    }
}

fn bit(id: ColumnId) -> ColumnSet {
    let id = id.value() as usize;
    assert!(id > 0 && id <= MAX_QUERY_COLUMNS);
    let mut set = ColumnSet::EMPTY;
    set.0[id / 64] = 1 << (id % 64);
    set
}
