# Source map

This map locates responsibilities; it is not a verification claim. Read
[Architecture](architecture.md) for boundaries and [Testing](testing.md) for
checks. Paths below identify current owners, including places where several
concerns still share one source file.

## Public API and shared state

| Owner | Responsibility |
| --- | --- |
| [lib.rs](../src/lib.rs) | Public exports and the library entry-point documentation. |
| [database.rs](../src/database.rs) | Handle construction, canonical path ownership, exclusive lease lifetime, resident charges, reopen, and close. |
| [transaction.rs](../src/transaction.rs) | Public append ownership, transaction decoding, and read-only commit resolution. |
| [config.rs](../src/config.rs), [cancellation.rs](../src/cancellation.rs) | Validated resource limits and the cooperative request/check operation. Each caller still decides where cancellation is legal. |
| [resources.rs](../src/resources.rs) | Atomic memory/temp admission, checked allocation ceilings, reservation transfer, and release. Counter mutation stays private to these authorities; temporary charges require explicit settlement. |
| [error.rs](../src/error.rs), [error_cause.rs](../src/error_cause.rs) | Public failure outcomes, source spans, and bounded nonrecursive diagnostic causes. |

## Frontend

| Owner | Responsibility |
| --- | --- |
| [frontend.rs](../src/frontend.rs) | Semantic identities, relation state, immutable plans, prepared-query ownership, and read-only column facts. |
| [lexer.rs](../src/frontend/lexer.rs), [parser.rs](../src/frontend/parser.rs) | Bounded tokens, identifier policy, parsed syntax, and source spans. Boolean syntax lowering lives under the parser. |
| [binding.rs](../src/frontend/binding.rs), [admission.rs](../src/frontend/binding/admission.rs) | Catalog source facts, mutable name scope, per-stage binding, and exact prepared-plan admission. Start with `bind_plan`; follow `Binder::bind_stage` to each operation. |
| [validation.rs](../src/frontend/validation.rs), [distinct.rs](../src/frontend/distinct.rs) | Independent semantic-plan checks and DISTINCT identity mapping. Neither validator depends on the binder. |

## Query execution

| Owner | Responsibility |
| --- | --- |
| [execution.rs](../src/execution.rs) | Public result lifecycle and the shared producer/consumer step protocol. |
| [admission.rs](../src/execution/admission.rs) | Query identity checks, physical planning, and reservation of runtime/result/source owners before source effects. |
| [scan.rs](../src/execution/scan.rs) | Shared scan cursor, conditional demand, and batch publication. |
| [scan/legacy.rs](../src/execution/scan/legacy.rs) | Legacy unit admission, demanded buffer geometry, and stored-column decoding. |
| [predicate.rs](../src/execution/predicate.rs) | Physical leaf comparisons and optional scan branching scratch. |
| [planning.rs](../src/execution/planning.rs) | Physical graph representation, pipeline capacities, snapshot identity, and fused stage boundaries. |
| [planning/lower.rs](../src/execution/planning/lower.rs) | Producer selection and identity-to-position mapping. |
| [planning/demand.rs](../src/execution/planning/demand.rs) | Shared backward column-demand analysis. |
| [planning/validate.rs](../src/execution/planning/validate.rs) | Graph, computation, filter, and output validation using a separate position-to-identity mapping. |
| [aggregation.rs](../src/execution/aggregation.rs) | Controller selection and bounded dense-key grouping: input, checking, emission, and replay. |
| [numeric.rs](../src/execution/aggregation/numeric.rs), [arguments.rs](../src/execution/aggregation/arguments.rs) | Shared aggregate layout, admitted typed cells, numeric evaluation/finalization, and argument capture/replay. Numeric state receives only memory authority. |
| [runtime.rs](../src/execution/runtime.rs) | Admission and scheduling of producer owners, input/output lifetimes, and replay. |
| [computed.rs](../src/execution/computed.rs) and [scalar.rs](../src/scalar.rs) | Demanded row/batch evaluation and shared checked numeric kernels. Scalar kernels have no parser, catalog, or I/O authority. |
| [declared.rs](../src/execution/scan/declared.rs) | Pinned declared-table scans, demanded payload buffers, and source replay. |
| [blocking.rs](../src/execution/blocking.rs) | Sorted-input ownership, run construction, checked cursors, pair merging, and merge-pass/sort transitions. `RowSort::sorted_rows` lends the completed cursor and comparison buffers to consumers while retaining their charges. |
| [blocking/record.rs](../src/execution/blocking/record.rs) | Typed row layouts, key equality/order/hash policy, and checked sort-frame encoding/decoding. |
| [blocking/io.rs](../src/execution/blocking/io.rs) | Bounded read/write caches and one borrowed scratch-file effect authority per call. Cache state stays private. |
| [grouping.rs](../src/execution/aggregation/grouping.rs), [hash.rs](../src/execution/aggregation/grouping/hash.rs), [reduction.rs](../src/execution/aggregation/grouping/reduction.rs) | General grouping control and checked result spooling, optional hash ownership, and sorted reduction into one reusable aggregate cell. |
| [join.rs](../src/execution/blocking/join.rs), [order.rs](../src/execution/blocking/order.rs), [limit.rs](../src/execution/limit.rs) | Equality matching, ORDER BY/DISTINCT consumption, and prefix counters. |
| [value.rs](../src/value.rs) | Scalar cells and borrowed UTF-8 values shared by kernels, batches, and the public API. |
| [batch.rs](../src/batch.rs) | Typed reusable batches, validity, text capacity, and complete-row publication. |
| [date.rs](../src/date.rs), [fixed_text.rs](../src/fixed_text.rs), [text_literal.rs](../src/text_literal.rs) | Validated dates, legacy key domain, and quoted literal decoding. |

The important query relationships are:

```text
source text -> frontend -> immutable prepared plan + snapshot pin
                              |
                              v
                         physical planning
                              |
                              v
                     runtime producer graph
                    /          |           \
              declared scan aggregates   sorted inputs
                    \          |           /
                     typed batches -> result
```

A file boundary does not by itself separate authority. Some modules import their
parent's private types and helpers; follow the concrete owner when tracing a
lifetime, reservation, or effect. Do not merge independent validators with their
producers merely to reduce duplicated-looking checks.

## Command-line interface

| Owner | Responsibility |
| --- | --- |
| [cli/mod.rs](../src/cli/mod.rs), [command.rs](../src/cli/command.rs) | Process entry, database lifetime, and command grammar with owned arguments. |
| [query.rs](../src/cli/query.rs), [output.rs](../src/cli/output.rs) | Query-file admission, streaming execution, and result encoding. |
| [arguments.rs](../src/cli/arguments.rs), [sink.rs](../src/cli/sink.rs), [diagnostic.rs](../src/cli/diagnostic.rs) | Bounded native argument capture, owned output descriptors, and allocation-free diagnostics. |


## Persistence and native effects

| Owner | Responsibility |
| --- | --- |
| [load.rs](../src/load.rs) | Legacy load admission, temporary reservation, and commit/cleanup outcome handling. |
| [load/input.rs](../src/load/input.rs), [load/staging.rs](../src/load/staging.rs), [load/unit.rs](../src/load/unit.rs) | Two-pass source identity, admitted column streams, private-unit construction/readback, and publication handoff. |
| [publication.rs](../src/publication.rs) | The shared root/fence publisher. `publish_snapshot` exposes replacement order and failure classification; `write_fence` and `write_root_next` own verified file preparation. |
| [namespace.rs](../src/namespace.rs) | Persistent names, read-only graph inspection, exclusive recovery, metadata identity checks, and root/fence reconciliation. |
| [path.rs](../src/path.rs) | Bounded lexical path admission and fallible pathname construction. |
| [load_input.rs](../src/load_input.rs) | Bounded lineitem parsing and two-pass fingerprints. |
| [catalog_snapshot.rs](../src/catalog_snapshot.rs) | Snapshot and resolution pins, serialized writer/maintenance authority, and registry publication. |
| [declare.rs](../src/catalog_snapshot/declare.rs) | Table declaration validation, identity assignment, and replacement catalog construction. |
| [append.rs](../src/catalog_snapshot/append.rs), [admission.rs](../src/catalog_snapshot/append/admission.rs) | Typed batch writes and publication; separate table selection, buffer/reservation admission, and issuance. |
| [construction.rs](../src/catalog_snapshot/construction.rs) | Shared private-object creation/synchronization, builder rollback, and exclusive orphan recovery. |
| [reclaim.rs](../src/catalog_snapshot/reclaim.rs) and [inventory.rs](../src/catalog_snapshot/reclaim/inventory.rs) | Protected graph traversal, external name/reference inventory, and obsolete-object deletion. |
| [catalog.rs](../src/catalog.rs), [catalog_schema.rs](../src/catalog_schema.rs) | Catalog and schema codecs and validated identities. |
| [table_data.rs](../src/table_data.rs), [native_unit.rs](../src/native_unit.rs), [success_index.rs](../src/success_index.rs) | Data indexes, typed unit payloads, and complete successful-attempt history. |
| [storage_format.rs](../src/storage_format.rs) | Shared root/fence format, legacy codec, checksums, and publication identities. |
| [scratch.rs](../src/scratch.rs) | Two unlinked files, bootstrap/reopen debt, and shared temporary extents. It has no publication authority. |
| [effects.rs](../src/effects.rs) | Named filesystem attempts, checked effect counts, positional transfers with short-I/O injection, and test fault schedules. Callers retain ordering and recovery authority. |
| [file_io.rs](../src/file_io.rs) | Concrete-file exact I/O, checked extents, and positive progress or terminal failure. |
| [filesystem](../filesystem/src/lib.rs) | Private unsafe OS boundary: paths, metadata, directory cursors, native synchronization, and fallible stationary mutexes. |

Namespace format 7 uses format-6 child codecs. Legacy storage uses format 4.
Their distinct discriminators are documented in [Storage](storage.md); none is a
stable compatibility promise.

To trace reopen, start at `Database::open_with_effects` in `database.rs`: admit
the path, acquire the lease, validate and recover the namespace, then recover
catalog construction before allocating the live registry. In `namespace.rs`,
`check_namespace` first calls `read_namespace_authority`, then
`validate_namespace_contents`. Only successful validation can reach
`recover_namespace`, which repairs roots before removing admitted debris and
reconciling the fence. Read-only inspection instead calls
`require_settled_namespace`; it cannot enter the mutation phase.

## Build and dependency ownership

The root Cargo package supplies the library and CLI; `filesystem/` is its
private workspace dependency. The pinned compiler and vendored libc inputs
support offline builds. See [Testing](testing.md#prerequisites) for setup and
[the tool inventory](../tools/README.md) for independent encoders and observers.

Licensed SQL included by Rust tests lives under
[`tests/fixtures/upstream`](../tests/fixtures/upstream/README.md). Independent
reference encoders and comparators live under
[`tools/oracles`](../tools/oracles/README.md). Current builds and checks do not
import historical notes.
