//! Embedded analytical queries over immutable, locally stored table generations.
//!
//! Use [`Database::create_empty`] for declared tables, then
//! [`Database::declare_table`] and [`Database::begin_append`] to add data.
//! [`Database::prepare`] pins a generation; [`Database::execute`] borrows that
//! plan and returns a streaming [`QueryResult`]. Consume every step through
//! [`QueryStep::Finished`] or handle [`QueryStep::Failed`]; rows alone do not
//! establish successful completion. Result values may borrow the current batch.
//!
//! APIs and persistent formats are pre-release. Supported syntax, platform
//! premises and operational limits are specified in the repository's `docs/`
//! contract guides; `docs/getting-started.md` provides a complete Rust walkthrough.

#![forbid(unsafe_code)]

mod batch;
mod cancellation;
mod catalog;
mod catalog_schema;
mod catalog_snapshot;
mod config;
mod database;
mod date;
mod effects;
mod error;
mod error_cause;

#[cfg(test)]
mod error_cause_tests;
mod execution;
mod file_io;
mod fixed_text;
mod frontend;
mod load;
mod load_input;
mod namespace;
mod native_unit;
mod path;
mod publication;
mod resources;
mod scalar;
mod scratch;
mod storage_format;
mod success_index;
mod table_data;
mod text_literal;
mod transaction;
mod value;

pub use cancellation::CancellationToken;
pub use catalog_snapshot::{AppendLimits, ColumnDeclaration, ColumnInput};
pub use config::Config;
pub use database::Database;
pub use error::{Error, SourceSpan};
pub use error_cause::{CauseKind, ErrorCause};
pub use execution::{QueryResult, QueryStep, ResultBatch};
pub use frontend::{DataType, PreparedQuery, ResultColumn};
pub use native_unit::InputValues as ColumnValues;
pub use storage_format::{DatabaseId, TransactionId};
pub use transaction::{Append, Commit, CommitResolution};
pub use value::{DateValue, StringValue, Value};
