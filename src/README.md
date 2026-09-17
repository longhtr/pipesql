# Database implementation

This directory contains the embedded database and its command-line interface.
[lib.rs](lib.rs) exposes the public API. [database.rs](database.rs) owns the open
handle, its directory lease and shared resource accounts. [main.rs](main.rs)
starts the commands in [cli/](cli/).

## Structure

| Owner | Responsibility |
| --- | --- |
| [query.rs](query.rs), [query/](query/) | Prepare SQL, resolve source facts, bind names and types, and retain the validated plan and its snapshot. |
| [execution.rs](execution.rs), [execution/](execution/) | Map logical values to batches and execute scans, joins, aggregation, windows and spill. |
| [storage.rs](storage.rs), [storage/](storage/) | Read stored records, retain snapshots, construct writes, publish changes and recover unfinished work. |
| [csv.rs](csv.rs), [csv/](csv/), [parquet.rs](parquet.rs), [parquet/](parquet/) | Decode bounded input, import a table transactionally and encode Parquet output. |
| [jsonl.rs](jsonl.rs) | Export typed query results as JSON Lines, including the completion record. |
| [value.rs](value.rs), [schema.rs](schema.rs), [batch.rs](batch.rs) | Define values, declarations and batches without depending on SQL binding. |
| [transaction.rs](transaction.rs), [error.rs](error.rs) | Describe write identities, outcomes and failures. |
| [resources.rs](resources.rs), [file_io.rs](file_io.rs) | Account for live resources and provide bounded file access. |

A module's `name.rs` is its entry point; `name/` holds its children. Private tests
are inline or beside their owner. Tests across public interfaces live in
[test/](../test/README.md).

## Follow an operation

`Database::prepare` enters [query.rs](query.rs). Parsing happens before snapshot
admission. [sources.rs](query/sources.rs) reads that snapshot into bounded source
facts, then binding resolves SQL without further file access. The prepared query
retains its plan, reservation and pin. Execution borrows that owner until the
caller finishes or drops the result.

`Database::begin_append` enters [append admission](storage/append/admission.rs).
The returned [Append](storage/append.rs) owns private construction through commit
or abort. [snapshot.rs](storage/snapshot.rs) controls writer admission and live
versions; [publication.rs](storage/publication.rs) changes the roots. On reopen,
[recovery.rs](storage/recovery.rs) validates the stored state before repairs.
The fixed-schema `lineitem` loader has its own path in [legacy.rs](storage/legacy.rs).

Read [architecture](../docs/architecture.md) for the relationships between these
operations. Native filesystem operations live in the separate
[filesystem crate](../filesystem/); development checks live in
[dev/](../dev/README.md).
