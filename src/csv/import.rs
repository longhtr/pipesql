//! Import a CSV stream as one append transaction.
//!
//! Inspect the schema and admit decoder/conversion buffers before issuing the
//! append. Report its token before reading input. Every decoded batch stays
//! private until EOF; an input or callback error aborts the whole attempt.
//! Conversion storage is freed before commit validation or abort needs workspace.
//!
//! Publication and cleanup remain owned by the existing append implementation.
//! In particular, an ambiguous commit must be resolved after reopen, and a panic
//! in caller code leaves the unfinished append for recovery.

use crate::effects::Effects;
use crate::import_columns::Columns;
use crate::{
    AppendLimits, CancellationToken, ColumnDeclaration, Commit, CsvDecoder, CsvLimits, Database,
    Error, TransactionId,
};
use std::io::Read;

/// Bounds for decoding and for the private native units created by one import.
/// A decoded batch may need several units to satisfy stored column byte limits.
#[derive(Clone, Copy)]
pub struct ImportLimits {
    pub csv: CsvLimits,
    pub append: AppendLimits,
}

impl Database {
    /// Decode and publish one CSV stream into a declared table.
    ///
    /// `issued` runs once after the transaction is issued and before reading CSV.
    /// Save its token if interruption or uncertain publication must be resolved.
    /// An I/O error from the callback aborts the attempt. Schema and buffer
    /// admission can fail before the callback runs. A token does not imply commit.
    ///
    /// A missing header, no data rows, malformed input, I/O failure or cancellation
    /// aborts all private batches. Cleanup failure retains both errors. A panic in
    /// the reader or callback requires close/reopen, as with an unfinished append.
    /// The reader is consumed and dropped before publication. Its own allocations
    /// and callback allocations remain caller costs; decoder and conversion buffers
    /// share the database budget.
    pub fn import_csv<R: Read>(
        &self,
        table: &str,
        reader: R,
        limits: ImportLimits,
        cancel: &CancellationToken,
        issued: impl FnOnce(TransactionId) -> std::io::Result<()>,
    ) -> Result<Commit, Error> {
        self.import_csv_with_effects(
            table,
            reader,
            limits,
            cancel,
            issued,
            &mut Effects::default(),
        )
    }

    pub(crate) fn import_csv_with_effects<R: Read>(
        &self,
        table: &str,
        reader: R,
        limits: ImportLimits,
        cancel: &CancellationToken,
        issued: impl FnOnce(TransactionId) -> std::io::Result<()>,
        effects: &mut Effects,
    ) -> Result<Commit, Error> {
        let mut committed = None;
        // Inspection lends validated names while its snapshot and read buffers
        // remain charged. It holds no registry lock during this callback. Declared
        // schemas are immutable, so the later writer uses the same column order.
        self.inspect_table(table, cancel, |schema| {
            let first = schema.column(0).expect("declared schema is nonempty");
            let mut columns: [ColumnDeclaration<'_>; 64] = [first; 64];
            for (index, column) in columns[..schema.column_count()].iter_mut().enumerate() {
                *column = schema.column(index).expect("schema ordinal");
            }
            let columns = &columns[..schema.column_count()];
            let required = CsvDecoder::<R>::required_memory(columns, limits.csv)?;
            let decoder_reservation = self.memory.reserve(required, "CSV import decoder")?;
            let mut decoder = CsvDecoder::new(reader, columns, limits.csv, required, cancel)?;
            let mut conversion = Columns::new(&self.memory, columns, limits.csv.batch_rows)?;
            let mut append =
                self.catalog_writer()?
                    .begin_append_named(table, limits.append, cancel, effects)?;
            let result = issued(append.transaction())
                .map_err(|source| Error::Io {
                    operation: "report CSV transaction",
                    source,
                })
                .and_then(|()| {
                    let mut populated = false;
                    while let Some(batch) = decoder.next_batch(cancel)? {
                        let mut start = 0;
                        while start < batch.row_count() {
                            let end = conversion.next_end(&batch, start, cancel)?;
                            conversion.write(&batch, start..end, &mut append, cancel, effects)?;
                            start = end;
                        }
                        populated = true;
                    }
                    if !populated {
                        return Err(Error::Input {
                            message: "CSV import requires data rows",
                            byte_offset: 0,
                        });
                    }
                    Ok(())
                });
            drop(conversion);
            drop(decoder);
            drop(decoder_reservation);
            if let Err(error) = result {
                return Err(append.abort_error(error, effects));
            }
            committed = Some(append.commit_with_effects(cancel, effects)?);
            Ok(())
        })?;
        Ok(committed.expect("successful inspection callback committed the import"))
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;
