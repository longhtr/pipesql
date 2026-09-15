# Inside PipeSQL

PipeSQL turns a pipe query into typed batches of rows and stores committed data
as immutable files. This directory owns database behavior. The CLI calls the
same public library that an embedded application uses.

Start with the [event report](../docs/event-report.md), then open
[`examples/event_report.rs`](../examples/event_report.rs) at `run`. It creates
tables, appends events, queries them, appends more and reopens the database.
Its old prepared query keeps seeing the old data. Following that example explains
why the engine needs both query plans and snapshot ownership.

## Follow one operation

Read [lib.rs](lib.rs) for the public API and [database.rs](database.rs) for the
handle behind it. `Database` owns the database path, exclusive process lease and
resource accounts. Query and writer lifetimes borrow that handle.

| Operation | Start here | Follow the work |
| --- | --- | --- |
| Create | `Database::create_empty` in [database.rs](database.rs) | Establish an empty catalog and its persistent namespace before returning a usable handle. |
| Declare and append | `Database::declare_table` and `Database::begin_append` | [declare.rs](catalog_snapshot/declare.rs) builds table metadata; [append.rs](catalog_snapshot/append.rs) accepts typed batches. Both use [construction.rs](catalog_snapshot/construction.rs) to prepare private files and [publication.rs](publication.rs) to commit. |
| Prepare | `Database::prepare` | [parser.rs](frontend/parser.rs) recognizes syntax. [binding.rs](frontend/binding.rs) resolves names and types against a pinned catalog. [validation.rs](frontend/validation.rs) independently checks the resulting plan. |
| Execute | [execution/admission.rs](execution/admission.rs) | Reserve resources, build and validate a physical plan, then enter [runtime.rs](execution/runtime.rs). [execution.rs](execution.rs) defines how callers receive batches, completion or failure. |
| Reopen | `Database::open` | [namespace.rs](namespace.rs) validates committed files and determines recovery actions before repairing anything. Catalog construction recovery then removes abandoned private work. |

The [CLI dispatcher](cli/mod.rs) shows how these operations fit into a process:
parse arguments, open the database, call the library, report the outcome and close.
[table_schema.rs](table_schema.rs) provides inspection without exposing stored
bytes or mutable catalog state.

## How a query becomes rows

A logical plan describes the requested transformations. Binding assigns columns
identities so later renaming cannot accidentally change which value an expression
uses. A prepared query also retains a **snapshot**: one committed catalog and the
data it references.

[Physical planning](execution/planning.rs) chooses the producers that will carry
out those transformations. [Declared scans](execution/scan/declared.rs) read the
needed columns. [Computed expressions](execution/computed.rs) evaluate demanded
values through the checked kernels in [scalar.rs](scalar.rs). Grouping, joins and
ordering consume these rows and produce new ones.

Sorting can **spill**: write intermediate rows to temporary files when they do
not fit in admitted memory, then merge sorted runs. [blocking.rs](execution/blocking.rs)
owns this shared mechanism; its consumers retain their own SQL meaning.

A batch is partial progress. Callers must reach `QueryStep::Finished` before
accepting a complete answer; a later step can still fail. The event report checks
both the rows and this terminal outcome.

## How a write becomes durable

An append builds new files without changing a reader's existing snapshot.
Publication makes the replacement snapshot authoritative using the ordered
synchronization protocol in [publication.rs](publication.rs).
[catalog_snapshot.rs](catalog_snapshot.rs) coordinates the writer and reader pins;
a pin keeps a snapshot's files alive until its reader releases them.

A failed commit can have an uncertain outcome if publication may already have
happened. [transaction.rs](transaction.rs) resolves the transaction token against
persistent evidence. Retrying blindly could duplicate data. Read
[Transactions](../docs/transactions.md) for the exact success, abort and recovery
rules, including platform synchronization assumptions.

## Boundaries worth preserving

- [resources.rs](resources.rs) grants memory and temporary-space reservations
  before work consumes them. Each owner must release its storage and charge on
  success, failure and cancellation.
- Plan validators check their producers independently. Sharing their reasoning
  would let the same mistake create and approve an invalid plan.
- Scalar kernels calculate values; they cannot read files or publish data.
- [effects.rs](effects.rs) exposes named filesystem operations for fault injection.
  The calling algorithm still decides their order and interprets failures.
- The separate [filesystem crate](../filesystem/src/lib.rs) owns unsafe native
  operations. The engine forbids unsafe Rust.

## Read further

Use the [source map](../docs/source-map.md) to locate an individual responsibility,
[Architecture](../docs/architecture.md) for design boundaries and
[Resources](../docs/resources.md) for exact admission and release contracts.
The [test map](../tests/README.md) distinguishes public behavior from local
invariants and independent oracles. The [tool map](../tools/README.md) identifies
campaign entry points; [Testing](../docs/testing.md) gives the fast development
loop and the conditions that require a full platform gate.
