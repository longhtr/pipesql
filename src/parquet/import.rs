//! Publish all validated Parquet rows through the existing append owner.
//!
//! Footer validation and buffer admission precede issuance. After the callback
//! reports the token, pages may create private native units. Only complete input
//! validation permits commit; every earlier failure follows append's abort path.
//!
//! The token callback lets the caller retain transaction identity before row writes
//! begin. Callback, decoder and cancellation errors must clean up private work;
//! failed cleanup retains the original cause and requires recovery. A publication
//! error may still leave an uncertain commit, resolved with that token after reopen.

use super::{ParquetReadLimits, decoder::Decoder};
use crate::effects::Effects;
use crate::import_columns::Columns;
use crate::{
    AppendLimits, CancellationToken, ColumnDeclaration, Commit, Database, Error, TransactionId,
};
use std::io::{Read, Seek};

/// Bounds for Parquet decoding and the private native units of one import.
#[derive(Clone, Copy)]
pub struct ParquetImportLimits {
    pub parquet: ParquetReadLimits,
    pub append: AppendLimits,
}

impl Database {
    /// Import one seekable Parquet file as a single append transaction.
    ///
    /// The supported flat, uncompressed PLAIN profile is defined in `docs/formats.md#parquet`.
    /// Footer validation and memory admission occur before issuance. `issued` then
    /// runs once, before data-page reads; retain its token for outcome resolution.
    /// A token alone does not imply commit. The callback's I/O failure aborts the
    /// attempt, as do malformed pages, bounds, cancellation and input I/O errors.
    ///
    /// The source must remain unchanged throughout the call. Completion checks its
    /// length again, but the library cannot detect arbitrary same-length changes.
    /// The CLI additionally checks file identity and timestamps. Reader and callback
    /// allocations remain caller costs; decoder/conversion storage shares the
    /// database budget and is freed before commit or rollback.
    ///
    /// Empty input is refused before issuance. An ordinary failed import publishes
    /// no rows. Cleanup failure and uncertain commit retain append's distinct error
    /// outcomes. A reader or callback panic after issuance requires close/reopen.
    pub fn import_parquet<R: Read + Seek>(
        &self,
        table: &str,
        reader: R,
        limits: ParquetImportLimits,
        cancel: &CancellationToken,
        issued: impl FnOnce(TransactionId) -> std::io::Result<()>,
    ) -> Result<Commit, Error> {
        self.import_parquet_with_effects(
            table,
            reader,
            limits,
            cancel,
            issued,
            &mut Effects::default(),
        )
    }

    pub(crate) fn import_parquet_with_effects<R: Read + Seek>(
        &self,
        table: &str,
        reader: R,
        limits: ParquetImportLimits,
        cancel: &CancellationToken,
        issued: impl FnOnce(TransactionId) -> std::io::Result<()>,
        effects: &mut Effects,
    ) -> Result<Commit, Error> {
        let mut committed = None;
        self.inspect_table(table, cancel, |schema| {
            let first = schema.column(0).expect("declared schema is nonempty");
            let mut columns: [ColumnDeclaration<'_>; 64] = [first; 64];
            for (index, column) in columns[..schema.column_count()].iter_mut().enumerate() {
                *column = schema.column(index).expect("schema ordinal");
            }
            let columns = &columns[..schema.column_count()];
            let required = Decoder::<R>::required_memory(columns, limits.parquet)?;
            let reservation = self.memory.reserve(required, "Parquet import decoder")?;
            let mut decoder = Decoder::new(reader, columns, limits.parquet, required, cancel)?;
            if decoder.row_count() == 0 {
                return Err(Error::Input {
                    message: "Parquet import requires data rows",
                    byte_offset: 0,
                });
            }
            let batch_rows = limits.parquet.row_group_rows.min(256) as u16;
            let mut conversion = Columns::new(&self.memory, columns, batch_rows)?;
            let mut append =
                self.catalog_writer()?
                    .begin_append_named(table, limits.append, cancel, effects)?;
            let result = issued(append.transaction())
                .map_err(|source| Error::Io {
                    operation: "report Parquet transaction",
                    source,
                })
                .and_then(|()| {
                    while let Some(batch) = decoder.next_batch(cancel)? {
                        let mut start = 0;
                        while start < batch.row_count() {
                            let end = conversion.next_end(&batch, start, cancel)?;
                            conversion.write(&batch, start..end, &mut append, cancel, effects)?;
                            start = end;
                        }
                    }
                    Ok(())
                });
            drop(conversion);
            drop(decoder);
            drop(reservation);
            if let Err(error) = result {
                return Err(append.abort_error(error, effects));
            }
            committed = Some(append.commit_with_effects(cancel, effects)?);
            Ok(())
        })?;
        Ok(committed.expect("successful inspection callback committed the import"))
    }
}

#[cfg(test)]
mod tests;
