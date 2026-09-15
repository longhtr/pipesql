# Tests and fixtures

[Build and test](../docs/testing.md) owns commands and the verification ladder.
[Verification](../docs/verification.md) owns required evidence and its limits.
This page directs changes to the suite that owns each contract.

## Public integration suites

| Entry point | Responsibility |
| --- | --- |
| [lifecycle.rs](lifecycle.rs) | Database lifecycle, pathname aliases, process leases and public preparation. |
| [load.rs](load.rs) | Legacy load, publication, reopen and receipts. |
| [execution.rs](execution.rs) | Legacy query semantics, result ownership and CLI execution. |
| [catalog_lifecycle.rs](catalog_lifecycle.rs) | Declared-table contracts, organized by capability below. |

These suites run on macOS and Linux. Ordinary and bounded-thread variants share
functional expectations but protect different premises. Keep both; the
[platform qualification](../docs/testing.md#platform-status) defines the native
stack measurements and limits.

[Directory ownership](support/mod.rs) and [cleanup](support/cleanup.rs) are shared.
Library tests can use the same owner through `crate::test_support`.
Cleanup reports failures after successful tests and preserves the original panic
during unwinding. The [child guard](support/child.rs) kills and reaps an unfinished
direct child before directory cleanup. Process tests still own their control-point
checks; successful exit alone is insufficient.

## Legacy public contracts

| Modules | Responsibility |
| --- | --- |
| [execution/queries.rs](execution/queries.rs) | Independently enumerated key pairs, grouping and composed expressions. |
| [execution/ownership.rs](execution/ownership.rs) | Borrowed results, reservation release and owned diagnostic spans. |
| [execution/stack.rs](execution/stack.rs), [load/stack.rs](load/stack.rs) | Ordinary execution and separately measured small-stack execution. |
| [load/lifecycle.rs](load/lifecycle.rs) | Commit, reopen and token shape versus actual issuance. |
| [execution/cli.rs](execution/cli.rs), [load/cli.rs](load/cli.rs) | Real CLI sources, sinks and publication outcomes. |

## Declared-table contracts

[fixtures.rs](catalog_lifecycle/fixtures.rs) owns shared input tables and typed
result collection. `query` checks ordered rows and release; `collect_unordered`
returns sorted rows for multiset comparison under a smaller progress bound.
Expected answers and failure interpretation stay in each test.

| Modules under `catalog_lifecycle/` | Responsibility |
| --- | --- |
| `append.rs`, `snapshots.rs` | Publication, pinned generations, overlapping readers, reclamation and receipts. |
| `aggregates.rs`, `grouping.rs`, `spooling.rs` | Aggregate semantics, admission, memory/disk execution and cancellation. |
| `joins.rs`, `join_corpus.rs` | Join composition, NULL extension and independent row oracles. |
| `computed.rs`, `constant_projection.rs`, `cast.rs`, `date_year.rs` | Scalar values, types, stored bits, ownership and demanded errors. |
| `boolean.rs`, `membership.rs`, `text_filter.rs`, `null_predicate.rs`, `null_safe.rs`, `nullif.rs` | Predicate truth tables, conditional demand and NULL semantics. |
| `byte_length.rs`, `char_length.rs` | Independent UTF-8 byte and scalar counts, literal ownership and composed materialization. |
| `logical_plan.rs`, `window_count.rs` | Public plan identities and full-partition cardinality. |
| `order.rs`, `distinct.rs`, `limit.rs` | Ordering, complete-row equality and prefix boundaries. |
| `except.rs`, `intersect.rs`, `multiset.rs`, `union.rs` | Positional set operations, multiplicities, typed equality and original representatives. |
| `wide.rs` | Source-pool and output-width limits, late columns and repeated outputs. |

## Internal invariants

Internal tests remain beside the implementation or in its `tests.rs` child.
They can inspect private phases and inject effects that public tests cannot.

| Owner | Responsibility |
| --- | --- |
| [Database tests](../src/database/tests.rs), [creation](../src/database/tests/creation.rs) | Leases, exact genesis validation, repair, corruption and cleanup. |
| [Catalog snapshot tests](../src/catalog_snapshot/tests.rs) | Shared persisted setup; child suites own declaration, append, publication, namespace, snapshot and query contracts. |
| [Reclamation tests](../src/catalog_snapshot/reclaim/tests.rs) | Scratch construction, protected graph traversal and cleanup/interruption schedules. |
| [Legacy scratch tests](../src/scratch/tests.rs) | Namespace debt, writer inspection and process cuts at scratch effects. |
| [Physical planning](../src/execution/planning/tests.rs) | Independent rejection of malformed producer references and demand mappings. |
| [Semantic validation](../src/frontend/validation/tests.rs), [binding](../src/frontend/binding/tests.rs) | Malformed plan rejection, name/identity rules, prepared admission and release. |
| [Declared scan](../src/execution/scan/declared.rs), [legacy scan](../src/execution/scan/legacy/tests.rs) | Actual payload ownership, admission before I/O, read faults and cancellation. |
| [Aggregation](../src/execution/aggregation/tests.rs) | Numerical vectors, argument capture, hash grouping and disk reduction. |
| [Grouping tests](../src/execution/aggregation/grouping/tests.rs) | Admission, replay, output and failure phases, sharing fixture mechanics. |
| [Row codec](../src/execution/blocking/record.rs), [sorting](../src/execution/blocking/sorting_tests.rs) | Key/value encoding, run limits, corruption, merge passes and terminal failure. |
| [Sorted sets](../src/execution/blocking/sorted_set/tests.rs) | Actual capacities, exact/short admission, replay and cancellation for DISTINCT and ALL forms. |
| [Load tests](../src/load/tests.rs) | Loading, publication and failure schedules with independent namespace expectations. |
| [CLI parser](../src/cli/command/tests.rs) | Argument grammar and ownership; source, sink and diagnostic checks remain beside those owners. |

Native and allocator [callers](../tools/README.md#native-and-allocation-callers)
exercise stock artifacts through separate observation boundaries. Public or
internal test success does not replace that evidence.

## Persistent and semantic fixtures

`fixtures/` contains licensed SQL, independent expected results and encoded
current, retired and rejected formats. Run `python3 tools/check-fixtures.py` to
compare the retained bytes with independent encoders. It neither updates fixtures
nor executes the engine.

A fixture change must retain its provenance, license, generation command and
intended admission or rejection behavior. Do not derive expected bytes from the
production codec or change expectations merely to make a failing test pass.
Historical format numbers discriminate compatibility; they are not release names.

## Adding or moving tests

Place a case under its protected contract. Share setup at the narrowest useful
scope, keeping independent results and failure interpretation local. Check test
discovery after a move; formatting and compilation do not prove execution.
An ignored test, empty selection or successful build is not runtime evidence.

## Composed report example

Start with the [event-report lesson](../docs/event-report.md) and follow the
[implementation map](../docs/source-map.md). Its executable checks in
[event_report.rs](../examples/event_report.rs) protect literal joined/yearly
answers, stored bits, retained snapshots and release. An altered expected group
must fail before a healthy rerun. [Input buffers](../examples/support/event_data.rs)
are shared; the expected groups remain in the example.

[scaled_report.rs](../examples/scaled_report.rs) uses an independent nested-loop
model across input profiles and budgets. It checks spill, refusal, cancellation
and reuse, and rejects an altered count. The internal
[grouping replay tests](../src/execution/aggregation/grouping/tests/replay.rs)
separately force hash-to-disk fallback and observe producer replay.
