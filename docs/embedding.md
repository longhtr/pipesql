# Embed PipeSQL

The Rust library runs in your process and stores one database in a local directory.
Your application chooses when to prepare, execute, write, cancel, and release work.
The engine provides no background scheduler that will finish an abandoned operation.

Start with [declared.rs](../examples/declared.rs), a complete program that creates,
writes, reopens, and queries a table. [event_report.rs](../examples/event_report.rs)
adds a join and a larger report. The method reference is built with
`cargo dev test documentation`.

## Open a database

Use `Database::create_empty` for a new declared-table database and
`Database::open` for an existing one. Supply an absolute path and a `Config` with
memory and temporary-space limits. The database holds an exclusive process lease
until it closes. Opening may recover unfinished work before returning a handle;
opening successfully does not mean every stored payload has been read.

Declare a table with `declare_table`, then append typed column batches with
`begin_append`, `write`, and `commit`. The declaration owns its copied schema;
input names need not outlive the call. Each write borrows its batch, so input
storage can be reused when that write returns. Visibility begins at commit,
not at the first accepted batch.

Schemas allow INT64, DOUBLE, STRING, and DATE, with a separate nullable flag.
A required column rejects NULL. Keep exact numeric types when constructing input:
an INT64 column and a DOUBLE column are not interchangeable representations.
[Formats](formats.md) describes the stream adapters when input arrives as CSV or
Parquet instead of typed batches.

## Keep a snapshot

`prepare` selects a committed version and returns a `PreparedQuery` that borrows
the database. You can release the SQL source string after preparation. The plan
retains the facts needed to execute and the snapshot needed to read its input.

Suppose you prepare a monthly report, append another day's records, then execute
the prepared report. It still reads the earlier version. Prepare again when you
want the new data. Re-executing the existing prepared query starts another run
against the same version.

That stability has a lifetime cost. A prepared query protects old files and a
registry slot even when no execution is running. Drop it when the application no
longer needs the snapshot. [snapshots.rs](../examples/snapshots.rs) demonstrates
this behavior across append, reclamation, and reopen.

## Consume the whole result

`execute` returns a `QueryResult` borrowing the prepared query and cancellation
token. Call `step` until it reports `Finished` or `Failed`:

- `Rows` lends a batch. Read or copy it before the next step. STRING values borrow
  that batch; they are not independently owned strings.
- `Progress` means more work is needed, not end of input.
- `Finished` confirms a complete result.
- `Failed` means all earlier batches are a prefix of a failed result.

A cell containing SQL NULL is `Some(Value::Null)`. `None` from a cell lookup means
the row or column index is out of range. Confusing those cases can hide a caller bug.

Dropping a result abandons execution and releases its runtime buffers and scratch.
It neither completes the query nor drops the prepared snapshot. The
[query-results example](../examples/query_results.rs) shows these separate owners.

## Budget the live work

Memory and temporary-space limits apply across simultaneously live operations on
the database. Preparation, execution, and writing have different owners and can
overlap. Releasing a result does not release a still-live prepared query.

A resource error identifies the refusing owner, required amount, and limit.
Increase the relevant budget within the host's capacity, narrow the work, or
release owners that are no longer needed. Spill still requires buffers and disk
space; it cannot make every query fit an arbitrarily small budget. A final LIMIT
usually limits results, not the work needed to produce them.

A reservation records engine-owned capacity. It is not a measurement of the
allocator or the process. Requested heap bytes can differ from the allocator's
usable extents; both omit costs such as retained free pages and thread stacks.
Caller-provided readers, writers, callbacks, and copied results remain caller
costs. A zero reservation does not mean zero resident memory or an empty database.

## Finish writes explicitly

Save an append's transaction token before a failure can prevent learning its
outcome. CSV and Parquet import callbacks expose this point. The callback must
persist the token before allowing work to continue when interruption recovery
matters to the application.

Explicit abort removes private work. Dropping an unfinished writer performs no
rollback I/O and requires reopen. A reader or callback panic during import can
leave the same condition. Release borrowed owners, close, and reopen before more
writes. Closing releases the lease and accounts; it does not repair the files.

A commit can be uncertain. Do not turn that uncertainty into an automatic retry:
follow [write resolution](operations.md#resolve-a-write) first. Export has a
different ownership rule: the caller owns any incomplete output and decides how
to remove or publish it.
