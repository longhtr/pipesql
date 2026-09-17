//! Own stored records, live snapshots and changes to the database directory.
//!
//! Catalog readers validate immutable objects. Snapshot admission protects their
//! lifetime while builders construct private replacements. Publication changes
//! authoritative roots; recovery resolves unfinished changes under the lease.
//! Scratch shares directory admission and cleanup rules but stores no durable rows.
//!
//! Database owns the lease, registry and resource accounts. These modules borrow
//! that owner; they do not create a second database service or copy its authority.

pub(crate) mod append;
pub(crate) mod catalog;
pub(crate) mod construction;
mod declare;
pub(crate) mod format;
pub(crate) mod history;
pub(crate) mod inspect;
pub(crate) mod legacy;
pub(crate) mod publication;
pub(crate) mod reclaim;
pub(crate) mod recovery;
pub(crate) mod schema;
pub(crate) mod scratch;
pub(crate) mod snapshot;
pub(crate) mod table;
pub(crate) mod unit;

pub use append::{Append, AppendLimits};
pub use inspect::TableSchema;
