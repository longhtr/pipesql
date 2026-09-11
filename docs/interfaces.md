# Public interfaces

The API, CLI, and persistent formats are unstable. The library supports declared
tables, streaming append, snapshot queries, committed-object reclamation, and
the legacy fixed-schema loader. Start with [Create and query a declared
table](getting-started.md) for a runnable example.

PipeSQL ships as an embedded Rust library and thin local CLI. The library is the
semantic authority; the CLI parses process arguments, invokes public library
operations, and renders typed results and errors. There is no server or network
API.

## Query lifecycle

### Configuration and database handles

`Config` sets nonzero limits for engine-owned memory and temporary data. Create
a declared-table database with `Database::create_empty`, or use `Database::open`
to validate and recover an existing database. `Database::create` creates the
legacy fixed-schema format described below.

The handle exposes its identity, generation, configured limits, and current
reservations. An idle database retains a charged pathname allocation; catalog
databases also retain a charged registry. [Resident
ownership](resources.md#database-resident-ownership) defines those charges and
their construction requirements.

### Prepared queries and values

`Database::prepare(&str)` returns a database-borrowing `PreparedQuery` with
bounded indexed result-schema inspection. `ResultColumn.name` is `Option<&str>`:
a named column returns Some, and an anonymous expression returns None. The CLI
leaves the name field empty in that column's schema descriptor. Anonymous
columns remain accessible by position; no generated name becomes SQL-visible.
`DataType` covers DOUBLE, INT64, STRING and DATE. `Value<'batch>` adds semantic
NULL. Numeric and DATE payloads are copied; `StringValue<'batch>` borrows UTF-8
text from result storage and exposes `as_str()`. The value's borrow prevents
another mutable query step while text remains live. `StringValue` and
`DateValue` have private validated representations.

### Stepping a query

`Database::execute(&self, &PreparedQuery, &CancellationToken)` returns
`QueryResult<'query, 'cancel>`. The result borrows both database and prepared
plan for its query lifetime, and separately borrows the cancellation token. It
has no iterator or exact-remaining-row-count contract. Call `step()` and handle
all four outcomes:

- `Rows(ResultBatch)` lends at most 256 rows until the next permitted mutation.
  `value(row, column)` returns `Option<Value<'batch>>`: NULL is `Some(Value::Null)` and an
  invalid index is `None`. Fixed-width value copies need no allocation.
- `Progress` advanced bounded work without emitting rows. The caller may yield,
  cancel or step again; it must not treat this as end-of-input.
- `Finished` establishes successful completion and remains terminal.
- `Failed(&Error)` is terminal. `into_error()` consumes the result and returns its
  owned failure, or None when it had not failed.

Payload reads and arithmetic may fail after execute succeeds. Earlier scan
batches are only a prefix on failure. Aggregate demanded arithmetic is checked
before any aggregate rows are emitted; cancellation may still interrupt
emission. Dropping the result releases its owners at any step. Prepared plans
and cancellation tokens must remain alive while borrowed. A borrowed batch
prevents stepping or dropping its query.

### Failures and recovery

Errors distinguish invalid configuration/path, parse or bind failure with spans,
input failure with byte offsets, resource exhaustion or contention, existing or
missing objects, lock refusal, corruption, unsupported operations or formats,
arithmetic failure, cancellation, I/O, and unsettled persistence. Do not
collapse `CleanupRequired`, `CommitAmbiguous`, and `RecoveryRequired` into
definite abort. Their meaning and recovery rules belong to
[Transactions](transactions.md).

The database owns its exclusive advisory lease until consuming close or drop.
`reserved_temp_bytes()` includes the original construction reservation while a
load outcome is ambiguous or cleanup remains incomplete. Closing does not
resolve that outcome or remove files. A subsequent `open` performs exclusive
recovery before returning a usable handle. Its zero temporary reservation does
not imply zero persistent filesystem usage;
[Resources](resources.md#temporary-and-persistent-bytes) explains that
distinction.

## Transaction identities and diagnostics

`TransactionId::from_bytes([u8; 24])` reconstructs a token and rejects a zero
identity or sequence with `InvalidTransactionId`. Decoding validates shape, not
issuance. Retain the token returned by `Append::transaction`,
`Commit::transaction`, or an ambiguous commit error.
[Transactions](transactions.md#attempt-identity-and-resolution) owns issuance
and resolution semantics. An issuance failure requires reopen without exposing a
data-commit token.

Contextual errors contain inline `ErrorCause` values. `ErrorCause::kind()`
exposes `CauseKind`, including the operation and original `io::Error` for I/O
failures. `recovery_generation()` exposes optional recovery context. Cleanup
failures retain both causes without allocating recursive boxes or formatting
them into strings. `Error::source` exposes this chain after the query or
transaction has been dropped.

Native I/O rendering uses the error kind and exact OS code. It does not allocate
a localized OS message. Caller-requested string formatting and custom I/O error
payloads have their own allocation contracts. A cause explains a failure; only
commit resolution after recovery determines whether an uncertain transaction
committed.

Shared queries and resolution inspect authoritative state without repairing it.
Namespace inspection that finds repair work and commit resolution on an
uncertain handle return `RecoveryRequired`. Execution refuses an already
unavailable handle with `Unsupported("reopen is required before execution")`
before namespace inspection. Close and reopen to perform recovery, including its
synchronization barriers even if files look clean. Failed query scratch creation
also returns `RecoveryRequired`, but that debt blocks only further scratch
construction. Scratch-free reads and publication remain available. Reclamation
has the stronger refusal rule specified under [reclaiming
objects](#reclaim-obsolete-catalog-objects).

## Declared-table databases

`Database::create_empty(path, config)` creates a database with no tables.
`Database::create` retains the fixed `lineitem` lifecycle used by
`load_lineitem`; it does not admit table declarations or named appends.
`Database::open` identifies either supported format through the shared validated
recovery path.

`declare_table(name, columns, cancel)` accepts column names, types and
NULLability as `ColumnDeclaration` values and returns a durable `Commit`. The
engine assigns the table ID from its admitted next transaction attempt and
initial column IDs from declaration ordinals 1..N. These become stored
identities; readers never recompute them from current positions. No additional
persistent counter is introduced. Rejected declarations do not issue an attempt;
issued attempts, including aborts, are never reused. Table names are unique
ignoring ASCII case. The existing schema validator owns column-name and
duplicate checks.

A database admits at most 64 tables, each with 1..64 columns. Names contain
1..32 ASCII letters, digits or underscores and start with a letter or
underscore. Conversion uses one fixed 64-entry stack array and no additional
heap allocation; construction retains the existing memory and temporary-space
reservations. Cancellation and invalid input release the admitted writer before
returning. Column IDs are not part of the public input interface.

## Streaming ingestion

`Database::begin_append(name, limits, cancel)` resolves the table name, ignoring
ASCII case, in the admitted writer's catalog before issuing a transaction.
Invalid or unknown names release the writer without consuming an attempt.
`AppendLimits` sets positive ceilings for batch count and encoded data bytes;
index, catalog, history and root reservations are additional. The resulting
table may contain at most 4,096 native units. Admission reserves construction
space and publishes issuance before returning an owner. `transaction()` exposes
its token before any input batch is written. Only one writer may be active.

`Append::write(columns, cancel)` accepts `ColumnInput` values containing
borrowed `ColumnValues` and validity bitmaps in declaration order. Bit zero
describes the first row; unused high bits must be zero. All columns have the
same positive row count, at most 32,768. Encoded column data is bounded to
524,288 bytes and each text cell to 65,536 bytes.
`DateValue::from_days_since_unix_epoch` validates DATE input against years 0001
through 9999 and returns `None` outside that range.

The writer maps positions through its retained schema and uses the existing
native validator and encoder. Wrong column counts, types, row counts or validity
make the append abort-only. The caller can refill or release input buffers after
`write` returns. A successful write creates private data; it does not publish a
generation. `commit(cancel)` consumes a nonempty successful stream and publishes
all batches once. Query results remain pinned to the generation chosen by
preparation.

Any failed write makes the stream abort-only. `abort` consumes it, removes all
owned private files and completes the cleanup barrier before releasing temporary
space. A failed cleanup returns `RecoveryRequired`; the database retains its
debt and refuses new work until reopen. Commit construction failures attempt the
same cleanup and preserve both errors if cleanup fails. Dropping an unsettled
owner does no I/O and requires reopen; it is not rollback. A pinned catalog
reader can execute while private ingestion reservations are live. Reopen
requirements after unsettled failure still apply.

These methods are available in ordinary library builds. The CLI retains its
fixed-table create/load commands; it can open and query a database made by the
library. Global and grouped COUNT(*), SUM and AVG over declared numeric columns
use the same public query lifecycle; the language manifest defines their current
limits. `Database::reclaim` preserves pinned readers and receipts. Current
scoped evidence and outstanding release work are recorded in `notes/plan.md`.

## Reclaim obsolete catalog objects

`Database::reclaim(&CancellationToken) -> Result<u64, Error>` removes obsolete
objects from declared-table databases. Success reports the number of filenames
removed after the units directory is synchronized. Queries retain their original
values and committed receipts remain resolvable. Cleanup does not issue a
transaction, advance the generation or require a free publication slot.

Cleanup shares exclusive mutation admission with table declaration and append.
Readers and receipt lookups may continue. A concurrent writer or cleanup
receives contention. Memory and temporary storage use the database's existing
limits; insufficient resources return a typed refusal.

Errors may follow partial removal of obsolete objects. Cancellation after an
unlink still completes the directory barrier before returning. A failed barrier
or interrupted scratch creation returns `RecoveryRequired` and refuses new work;
close and reopen for recovery. No live data is rolled back by cleanup. This API
is not a format upgrade or a complete database vacuum, and legacy format-4
databases return `Unsupported`.

## Legacy lineitem lifecycle

The public path supports:

1. create a database at an explicit local path with memory/temp limits;
2. bulk-load the [16-field lineitem input](storage.md#construction-and-publication) in one
   transaction, retaining its seven supported columns;
3. return `Commit { transaction,generation }` only after durable success; return
   ordinary pre-publication errors only after definite rollback,
   `CleanupRequired` when rollback is incomplete, or
   `CommitAmbiguous { transaction,source }` after root publication may have begun;
4. close and reopen;
5. prepare the composition manifest through ordinary pipe source text;
6. execute scans, filters, projections, LIMIT, and repeated aggregates, including Q1/Q6;
7. consume bounded typed batches until Finished, or handle terminal failure; and
8. resolve an ambiguous commit identity after reopen, through the library or CLI.

## CLI operations

Use the binary built by `cargo build --release --offline --locked`. Commands
below run from the repository root and address it as `target/release/pipesql`.
The CLI can query declared tables, but declaration and typed append require the
library. CLI `create` and `load` operate on the legacy `lineitem` schema.

### CLI source and path admission

The CLI exposes `create`, `open`, `load`, `query`, and `resolve`. A query source
must be a regular, non-symlink UTF-8 file of at most 4,096 bytes and remain
unchanged while read. The CLI compares the opened descriptor with the initial
pathname's type, identity, extent, and modification/change times. After reading,
it rechecks the descriptor, pathname, and consumed length before preparing SQL.
These checks detect changes; they do not create an atomic snapshot of an
externally mutable file.

Database and legacy-load input paths must be absolute, nonempty, contain no NUL
or lexical `.`/`..` components, and have a final component. Their encoded length
and each constructed namespace path are bounded to 4,096 bytes. Canonicalization
follows lexical admission; the caller still checks physical identity and leases.
An overlength constructed path returns I/O `InvalidInput`; fallible allocation
refusal returns I/O `OutOfMemory`. Native path work can also return `Resource`.
[Native path bounds](resources.md#native-paths-stack-and-io) explain the
admission units and platform limits.

CLI capture and parsing transfer bounded path owners without allocating error
strings. Owned sinks avoid lazy standard-I/O buffers. [CLI resource
ownership](resources.md#cli-startup-owners) covers startup limits, sink
lifetime, and what remains outside those bounds.

### Resolve a CLI load outcome

Keep the transaction token printed by `load`, including the token in an
ambiguous commit diagnostic. Set the shell variable `transaction` to that token,
then run:

```sh
target/release/pipesql resolve --database /absolute/database --transaction "$transaction" \
  --memory-limit-bytes 2000000 --temp-limit-bytes 1000000
```

`--transaction` is required only for `resolve`. It accepts exactly 48 ASCII
hexadecimal digits, in either case, encoding the existing 24-byte transaction
token. Zero database identities and zero attempt sequences are invalid. No `0x`
prefix, separators or surrounding whitespace are accepted. Parsing validates
shape through `TransactionId::from_bytes`; it does not establish issuance.

The command opens the database under the ordinary exclusive lease and completes
recovery before calling read-only `resolve_commit`. It may repair persistent
state during open. A complete successful response includes `status=resolved`,
the lowercase `transaction` token, and either `resolution=aborted` or
`resolution=durable`. Only a durable result includes its `generation`.

Both settled outcomes exit 0. Argument parsing failures exit 2. Unknown or foreign
tokens, unavailable leases, failed recovery, corruption and output failures exit
1. An unknown token is not an abort. Require exit 0 before treating a response as
complete; a failed output write can leave partial text. Retry resolution after
repairing its cause. Resolving an aborted token after a later load still reports
that earlier attempt as aborted. All commands close their database after operation
or output failure; a primary failure takes precedence over a close failure.

### Query CLI completion

The query command prints `status=querying` and result schema before stepping.
Rows may follow, including DATE values rendered as ISO calendar dates and STRING
values rendered as hexadecimal bytes. DOUBLE cells include a decimal rendering
and their 16-digit raw-bit encoding, such as `double:1001:408f480000000000`. It
prints `row_count` and `status=queried` only after Finished. Require exit 0 and
complete output before treating a query as successful. A step error or closed
sink can leave schema or a row prefix; neither is a complete result. The CLI
drops the running query on output failure.

## Ownership

Database, transaction, prepared query, running query, result batch, snapshot,
and cancellation handles have documented ownership and thread capability.
Closing consumes the Rust handle; the same handle cannot be closed twice. Result
values and buffers state whether owned or borrowed and their exact lifetime.
Host code may not mutate engine-owned output.

Expected failures return typed categories for invalid input, unsupported
behavior, resource refusal, cancellation, contention, I/O, corruption and
possibly committed outcomes. Parse, bind and arithmetic diagnostics carry source
spans. `Error::ArithmeticOverflow { operation, span }` and the matching
`CauseKind` variant own a static operation name and a required half-open UTF-8
byte range. The span remains valid as an offset after the query and its source
text are dropped; the engine does not retain that text. Numeric constant
failures use the whole expression. Runtime failures use the enclosing aggregate
call, excluding its alias. Shared argument failures identify a demanded call;
final SUM overflow identifies SUM even when AVG shares its input state. For
example, the CLI reports `arithmetic overflow during multiplication at bytes
46..75`. Cause capture moves these facts without allocation. A shared formatter
renders the top-level error and cause without allocating a diagnostic String.
The Rust variant shape and displayed wording remain pre-release interfaces, not
a stable ABI.

No panic/unwind, Rust layout, allocator value, implicit thread-local error, or
ambient mutable global crosses a public FFI boundary. Expected external failure
is never an assertion; internal programmer invariants may assert in the Rust
implementation.

## Blocking and reentry

Every call documents whether it may allocate, block on I/O, invoke a callback,
or wait for another owner. No callback occurs under an engine lock. Cancellation
latency is bounded by documented work and syscall quanta, not an unconditional
wall-clock promise.

## Compatibility

Before release, the Rust API, CLI, and native bytes are unstable unless
explicitly specified otherwise. Once stable, changes require retained fixtures,
upgrade policy, and semantic compatibility tests. Development checkers and
internal plan representations have no compatibility promise.

## Required evidence

Compile and invoke exported stock interfaces for lifecycle, concurrent allowed
and forbidden calls, cancellation, partial stream failure, resource refusal and
commit resolution. Verify Rust borrow constraints for consuming close and live
results; the current API cannot call close twice on the same consumed handle.
Test stale handles and repeated close where a future interface permits them.
Callbacks and a public FFI are unimplemented; admitting either requires reentry,
foreign lifetime, panic containment and toolchain evidence at its own boundary.
