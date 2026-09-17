//! Read and write the bounded flat Parquet profile defined in docs/formats.md#parquet.
//!
//! Metadata establishes finite column and row-group ranges before page decoding.
//! Transactional import and complete export use the existing append and cursor
//! owners; the format codec does not acquire publication authority.
//!
//! A first-party codec keeps allocation fallible and charged before decoding.
//! The inspected Arrow Rust 59.2.0 reader/writer and parquet2 revision
//! b0e654526ce06bcb02b3e0e1f5aa3e1a10580350 allocate some buffers and metadata
//! infallibly, so a thin adapter would not preserve typed allocation refusal.
//! This choice also makes us responsible for the format: add an encoding only
//! for a concrete workload, with independent interoperability checks.
//! See the compared [Arrow reader](https://github.com/apache/arrow-rs/blob/59.2.0/parquet/src/file/serialized_reader.rs),
//! [writer](https://github.com/apache/arrow-rs/blob/59.2.0/parquet/src/file/writer.rs),
//! and parquet2 [metadata](https://github.com/jorgecarleitao/parquet2/blob/b0e654526ce06bcb02b3e0e1f5aa3e1a10580350/src/read/metadata.rs)
//! and [error conversion](https://github.com/jorgecarleitao/parquet2/blob/b0e654526ce06bcb02b3e0e1f5aa3e1a10580350/src/error.rs).

mod compact;
mod compact_write;
mod decoder;
mod export;
mod footer_write;
mod import;
mod levels;
mod metadata;
mod page;
mod page_write;
mod rows;

use crate::Error;

pub use export::ParquetExportLimits;
pub use import::ParquetImportLimits;

/// Bounds for the flat, uncompressed Parquet input profile in `docs/formats.md#parquet`.
/// Every bound applies before the corresponding input is retained or decoded.
#[derive(Clone, Copy, Debug)]
pub struct ParquetReadLimits {
    /// Total file bytes, including metadata and magic bytes.
    pub input_bytes: u64,
    /// Footer bytes; positive and at most 16 MiB.
    pub metadata_bytes: u32,
    /// Row-group count; positive and at most 4,096.
    pub row_groups: u32,
    /// Rows per group; positive and at most 65,536.
    pub row_group_rows: u32,
    /// Encoded bytes per group; positive and at most 64 MiB.
    pub row_group_bytes: u32,
    /// Encoded page payload bytes; positive and at most 16 MiB.
    pub page_bytes: u32,
    /// Total rows across all groups.
    pub rows: u64,
}

impl ParquetReadLimits {
    fn validate(self) -> Result<(), Error> {
        if self.input_bytes < 12
            || !(1..=16_777_216).contains(&self.metadata_bytes)
            || !(1..=4_096).contains(&self.row_groups)
            || !(1..=65_536).contains(&self.row_group_rows)
            || !(1..=67_108_864).contains(&self.row_group_bytes)
            || !(1..=16_777_216).contains(&self.page_bytes)
        {
            return Err(Error::InvalidConfig("invalid Parquet input limits"));
        }
        Ok(())
    }
}

fn bound(required: u64, limit: u64, owner: &'static str) -> Result<(), Error> {
    if required > limit {
        return Err(Error::Resource {
            owner,
            required,
            limit,
        });
    }
    Ok(())
}
