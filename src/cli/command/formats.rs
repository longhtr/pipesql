//! Select an interchange format and validate its required resource options.
//!
//! Common command parsing captures each option once. This module checks which
//! bounds belong to each format and constructs typed limits before database I/O.
//!
//! Import and export retain distinct format variants: CSV is input, JSON Lines is
//! output, and Parquet supports both. Options belonging to another format are
//! errors, even if their values would otherwise be valid; silently ignoring them
//! would give the caller a different resource limit than requested.

use super::{ArgumentError, IMPORT_OPTIONS};
use pipesql::{
    AppendLimits, ExportLimits, ImportLimits, ParquetExportLimits, ParquetImportLimits,
    ParquetReadLimits,
};
use std::ffi::OsStr;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Format {
    Csv,
    Jsonl,
    Parquet,
}

impl Format {
    pub(super) fn parse(value: &OsStr) -> Result<Self, ArgumentError> {
        match value.to_str() {
            Some("csv") => Ok(Self::Csv),
            Some("jsonl") => Ok(Self::Jsonl),
            Some("parquet") => Ok(Self::Parquet),
            _ => Err("--format must be csv, jsonl or parquet".into()),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ImportFormat {
    Csv(ImportLimits),
    Parquet(ParquetImportLimits),
}

#[derive(Clone, Copy)]
pub(crate) enum ExportFormat {
    Jsonl(ExportLimits),
    Parquet(ParquetExportLimits),
}

pub(super) const PARQUET_OPTIONS: [&str; 6] = [
    "--metadata-limit-bytes",
    "--row-group-limit",
    "--row-group-rows",
    "--row-group-limit-bytes",
    "--page-limit-bytes",
    "--row-group-text-bytes",
];

fn positive(value: Option<u64>, name: &'static str, maximum: u64) -> Result<u64, ArgumentError> {
    let value = value.ok_or(ArgumentError::InvalidValue {
        owner: name,
        requirement: "is required for this format",
    })?;
    if value == 0 || value > maximum {
        return Err(ArgumentError::InvalidValue {
            owner: name,
            requirement: "is outside the supported positive range",
        });
    }
    Ok(value)
}

pub(super) fn import_limits(
    common: [Option<u64>; 8],
    parquet: [Option<u64>; 6],
) -> Result<ParquetImportLimits, ArgumentError> {
    if common[2..6].iter().any(Option::is_some) {
        return Err("CSV record, field and batch bounds are invalid for Parquet".into());
    }
    if parquet[5].is_some() {
        return Err("--row-group-text-bytes is accepted only for Parquet export".into());
    }
    let input_bytes = positive(common[0], IMPORT_OPTIONS[0], u64::MAX)?;
    if input_bytes < 12 {
        return Err("Parquet input limit must allow at least 12 bytes".into());
    }
    Ok(ParquetImportLimits {
        parquet: ParquetReadLimits {
            input_bytes,
            rows: positive(common[1], IMPORT_OPTIONS[1], u64::MAX)?,
            metadata_bytes: positive(parquet[0], PARQUET_OPTIONS[0], 16_777_216)? as u32,
            row_groups: positive(parquet[1], PARQUET_OPTIONS[1], 4096)? as u32,
            row_group_rows: positive(parquet[2], PARQUET_OPTIONS[2], 65_536)? as u32,
            row_group_bytes: positive(parquet[3], PARQUET_OPTIONS[3], 67_108_864)? as u32,
            page_bytes: positive(parquet[4], PARQUET_OPTIONS[4], 16_777_216)? as u32,
        },
        append: AppendLimits {
            batches: positive(common[6], IMPORT_OPTIONS[6], 4096)? as u32,
            encoded_bytes: positive(common[7], IMPORT_OPTIONS[7], u64::MAX)?,
        },
    })
}

pub(super) fn export_limits(
    common: ExportLimits,
    parquet: [Option<u64>; 6],
) -> Result<ParquetExportLimits, ArgumentError> {
    if parquet[3].is_some() || parquet[4].is_some() {
        return Err("row-group byte and page bounds are accepted only for Parquet import".into());
    }
    Ok(ParquetExportLimits {
        rows: common.rows,
        bytes: common.bytes,
        metadata_bytes: positive(parquet[0], PARQUET_OPTIONS[0], 16_777_216)? as u32,
        row_groups: positive(parquet[1], PARQUET_OPTIONS[1], 4096)? as u32,
        row_group_rows: positive(parquet[2], PARQUET_OPTIONS[2], 65_536)? as u32,
        row_group_text_bytes: positive(parquet[5], PARQUET_OPTIONS[5], 8_388_608)? as u32,
    })
}
