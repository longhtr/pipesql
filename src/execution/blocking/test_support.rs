//! Shared sorter fixtures; platform-specific tests own their own target guards.
use super::{KeyColumn, RowLayout};
use crate::frontend::{DataType, Direction, MAX_COLUMNS, NullPlacement};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(in crate::execution) struct Directory(pub(in crate::execution) PathBuf);

impl Directory {
    pub(in crate::execution) fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "pipesql-group-runs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, AtomicOrdering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

pub(in crate::execution) fn schema(specs: &[(DataType, bool)]) -> RowLayout {
    let mut columns = [KeyColumn {
        input: 0,
        kind: DataType::Int64,
        nullable: false,
        direction: Direction::Ascending,
        nulls: NullPlacement::First,
    }; MAX_COLUMNS];
    let mut max_bytes = 0;
    for (index, (kind, nullable)) in specs.iter().copied().enumerate() {
        columns[index] = KeyColumn {
            input: index,
            kind,
            nullable,
            direction: Direction::Ascending,
            nulls: NullPlacement::First,
        };
        max_bytes += match kind {
            DataType::Int64 | DataType::Double => 9,
            DataType::Date => 5,
            DataType::String => 5 + crate::batch::MAX_TEXT_BYTES,
        };
    }
    // Narrow codec test boundary: no relational plan is admitted by this helper.
    RowLayout {
        columns,
        count: specs.len(),
        key_count: specs.len(),
        max_bytes,
        key_max_bytes: max_bytes,
        layout: 0x44a7_3201,
    }
}
