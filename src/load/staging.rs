//! Seven bounded column streams backed by disjoint regions of the load arena.
//! First-pass row admission limits every write before its effect.
use crate::effects::{Effect, Effects, LoadEffect, write_nonempty};
use crate::error::io_error;
use crate::load_input::ProjectedRow;
use crate::namespace::PRIVATE_NAME;
use crate::path::joined_path;
use crate::storage_format;
use crate::{CancellationToken, Error};
use pipesql_filesystem as filesystem;
use std::fs::File;
use std::path::Path;

pub(super) const BLOCK_BYTES: usize = 262_144;
const BLOCK_BYTES_U64: u64 = 262_144;
const KEY_BLOCK_BYTES: usize = 32_768;
pub(super) const STAGING_BUFFER_BYTES: usize = 5 * BLOCK_BYTES + 2 * KEY_BLOCK_BYTES;

// The fixed schema's order is shared by the projected row, staging files, and
// unit descriptors. Each column owns one disjoint arena region. A block never
// crosses that region, even when its element width differs from its neighbors.
#[derive(Clone, Copy)]
pub(super) struct ColumnLayout {
    pub(super) name: &'static str,
    pub(super) width: u64,
    buffer_offset: usize,
    pub(super) block_bytes: usize,
}

pub(super) const COLUMNS: [ColumnLayout; 7] = [
    ColumnLayout {
        name: "quantity.stage",
        width: 8,
        buffer_offset: 0,
        block_bytes: BLOCK_BYTES,
    },
    ColumnLayout {
        name: "extendedprice.stage",
        width: 8,
        buffer_offset: BLOCK_BYTES,
        block_bytes: BLOCK_BYTES,
    },
    ColumnLayout {
        name: "discount.stage",
        width: 8,
        buffer_offset: 2 * BLOCK_BYTES,
        block_bytes: BLOCK_BYTES,
    },
    ColumnLayout {
        name: "tax.stage",
        width: 8,
        buffer_offset: 3 * BLOCK_BYTES,
        block_bytes: BLOCK_BYTES,
    },
    ColumnLayout {
        name: "returnflag.stage",
        width: 1,
        buffer_offset: 4 * BLOCK_BYTES,
        block_bytes: KEY_BLOCK_BYTES,
    },
    ColumnLayout {
        name: "linestatus.stage",
        width: 1,
        buffer_offset: 4 * BLOCK_BYTES + KEY_BLOCK_BYTES,
        block_bytes: KEY_BLOCK_BYTES,
    },
    ColumnLayout {
        name: "shipdate.stage",
        width: 4,
        buffer_offset: 4 * BLOCK_BYTES + 2 * KEY_BLOCK_BYTES,
        block_bytes: BLOCK_BYTES,
    },
];

pub(super) struct Staging<'arena> {
    files: [File; 7],
    buffers: &'arena mut [u8],
    buffered: [usize; 7],
    written: [u64; 7],
    checksums: [storage_format::Crc32c; 7],
    admitted_rows: u64,
    remaining_rows: u64,
}

impl<'arena> Staging<'arena> {
    pub(super) fn new(
        root: &Path,
        buffers: &'arena mut [u8],
        admitted_rows: u64,
        effects: &mut Effects,
    ) -> Result<Self, Error> {
        assert_eq!(buffers.len(), STAGING_BUFFER_BYTES);
        let create = |name: &str, effects: &mut Effects| -> Result<File, Error> {
            effects.before(Effect::Load(LoadEffect::CreateStaging))?;
            filesystem::create_new_read_write(joined_path(&joined_path(root, PRIVATE_NAME)?, name)?)
                .map_err(|source| io_error("create staging file", source))
        };
        let files = [
            create(COLUMNS[0].name, effects)?,
            create(COLUMNS[1].name, effects)?,
            create(COLUMNS[2].name, effects)?,
            create(COLUMNS[3].name, effects)?,
            create(COLUMNS[4].name, effects)?,
            create(COLUMNS[5].name, effects)?,
            create(COLUMNS[6].name, effects)?,
        ];
        Ok(Self {
            files,
            buffers,
            buffered: [0; 7],
            written: [0; 7],
            checksums: std::array::from_fn(|_| storage_format::Crc32c::new()),
            admitted_rows,
            remaining_rows: admitted_rows,
        })
    }

    pub(super) fn append(
        &mut self,
        row: ProjectedRow,
        byte_offset: u64,
        cancellation: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        // Pass one's reservation owns these rows. Refuse a changed source before
        // touching any column; an append error terminates this construction.
        if self.remaining_rows == 0 {
            return Err(Error::Input {
                message: "input exceeds admitted row count",
                byte_offset,
            });
        }
        self.remaining_rows -= 1;
        let values: [&[u8]; 7] = [
            &row.quantity,
            &row.extended_price,
            &row.discount,
            &row.tax,
            &row.return_flag,
            &row.line_status,
            &row.ship_date,
        ];
        for (index, value) in values.into_iter().enumerate() {
            let start = COLUMNS[index].buffer_offset + self.buffered[index];
            let end = start.checked_add(value.len()).ok_or(Error::Resource {
                owner: "staging buffer",
                required: u64::MAX,
                limit: BLOCK_BYTES_U64,
            })?;
            assert!(
                end <= COLUMNS[index].buffer_offset + COLUMNS[index].block_bytes,
                "staging buffer overflow"
            );
            self.buffers[start..end].copy_from_slice(value);
            self.buffered[index] += value.len();
            if self.buffered[index] == COLUMNS[index].block_bytes {
                self.flush(index, cancellation, effects)?;
            }
        }
        Ok(())
    }

    fn flush(
        &mut self,
        index: usize,
        cancellation: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<(), Error> {
        if self.buffered[index] == 0 {
            return Ok(());
        }
        cancellation.check()?;
        let start = COLUMNS[index].buffer_offset;
        let end = start + self.buffered[index];
        let buffered = u64::try_from(self.buffered[index])
            .map_err(|_| Error::Corrupt("staging buffer length does not fit"))?;
        let limit = self
            .admitted_rows
            .checked_mul(COLUMNS[index].width)
            .ok_or(Error::Corrupt("admitted staging extent overflow"))?;
        let next = self.written[index]
            .checked_add(buffered)
            .ok_or(Error::Resource {
                owner: "staging extent",
                required: u64::MAX,
                limit,
            })?;
        if next > limit {
            return Err(Error::Resource {
                owner: "staging extent",
                required: next,
                limit,
            });
        }
        write_nonempty(
            &mut self.files[index],
            &self.buffers[start..end],
            Effect::Load(LoadEffect::WriteStaging),
            effects,
        )?;
        self.checksums[index].update(&self.buffers[start..end]);
        self.written[index] = next;
        self.buffered[index] = 0;
        Ok(())
    }

    pub(super) fn finish(
        mut self,
        rows: u64,
        cancellation: &CancellationToken,
        effects: &mut Effects,
    ) -> Result<[u32; 7], Error> {
        for (index, column) in COLUMNS.into_iter().enumerate() {
            self.flush(index, cancellation, effects)?;
            effects.before(Effect::Load(LoadEffect::InspectStaging))?;
            let metadata = filesystem::file_metadata(&self.files[index])
                .map_err(|source| io_error("inspect staging file", source))?;
            let expected = rows.checked_mul(column.width).ok_or(Error::Resource {
                owner: "staging extent",
                required: u64::MAX,
                limit: u64::MAX,
            })?;
            if self.written[index] != expected || metadata.len() != expected {
                return Err(Error::Corrupt("staging length mismatch"));
            }
        }
        Ok(self.checksums.map(storage_format::Crc32c::finish))
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;
