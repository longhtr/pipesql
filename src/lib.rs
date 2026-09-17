//! An embedded database for analytical queries over local tables.
//!
//! PipeSQL runs inside your application. Create a database with
//! [`Database::create_empty`], add tables with [`Database::declare_table`], and
//! write rows with [`Database::begin_append`]. Reopen a stored database with
//! [`Database::open`].
//!
//! To run a query, call [`Database::prepare`] and pass the prepared query to
//! [`Database::execute`]. The returned [`QueryResult`] yields batches of rows.
//! Keep stepping until [`QueryStep::Finished`]; [`QueryStep::Failed`] means the
//! result is incomplete, even if earlier steps returned rows.
//!
//! Preparing a query keeps one committed version of the database available,
//! called a snapshot. Later writes cannot change that query's input. Result text
//! borrows its batch, so copy strings you need to keep before releasing the batch.
//!
//! Interfaces and storage formats are not stable yet. The repository's
//! `docs/start.md` walks through a complete example; the other `docs/`
//! guides specify supported syntax, limits and platform requirements.

#![forbid(unsafe_code)]

mod batch;
mod cancellation;
mod config;
mod csv;
mod database;
mod effects;
mod error;
mod execution;
mod export_io;
mod file_io;
mod import_columns;
mod jsonl;
mod parquet;
mod path;
mod query;
mod resources;
mod schema;
mod storage;
mod transaction;
mod value;

pub use batch::input::{ColumnInput, ColumnValues, InputBatch};
pub use cancellation::CancellationToken;
pub use config::Config;
pub use csv::{CsvBatch, CsvDecoder, CsvLimits, ImportLimits};
pub use database::Database;
pub use error::{CauseKind, Error, ErrorCause, SourceSpan};
pub use execution::{QueryResult, QueryStep, ResultBatch};
pub use jsonl::ExportLimits;
pub use parquet::{ParquetExportLimits, ParquetImportLimits, ParquetReadLimits};
pub use query::{LogicalPlan, PreparedQuery, ResultColumn};
pub use schema::ColumnDeclaration;
pub use storage::{Append, AppendLimits, TableSchema};
pub use transaction::{Commit, CommitResolution, DatabaseId, TransactionId};
pub use value::{DataType, DateValue, StringValue, Value};

#[cfg(test)]
#[path = "../test/support/mod.rs"]
mod test_support;

#[cfg(test)]
#[path = "../test/support/child.rs"]
mod test_child;

#[cfg(test)]
#[path = "../test/support/subprocess.rs"]
mod test_subprocess;

#[cfg(test)]
use test_support::cleanup as test_cleanup;

#[cfg(test)]
#[path = "../test/support/writer.rs"]
mod test_writer;
