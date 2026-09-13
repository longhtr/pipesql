# Tests and fixtures

[Testing](../docs/testing.md) owns commands and coverage guidance.
[Verification](../docs/verification.md) owns what the evidence permits us to claim.

## Public integration suites

| Entry point | Concern |
| --- | --- |
| [lifecycle.rs](lifecycle.rs) | Database lifecycle, aliases, process leases, and public preparation. |
| [load.rs](load.rs) | Legacy load suite entry point and shared input/directory fixtures. |
| [execution.rs](execution.rs) | Legacy execution suite entry point and shared input/directory fixtures. |
| [catalog_lifecycle.rs](catalog_lifecycle.rs) | Declared-table suite entry point and shared public fixtures. Contract tests live in its child modules below. |

The catalog, legacy load, and legacy execution suites run on macOS and Linux.
Five public catalog, two legacy, and six internal scenarios each have an ordinary
thread test and a bounded-thread variant with the same functional expectations.
Both execute on GNU arm64. [Platform status](../docs/testing.md#platform-status)
explains the requested size, target-specific reported ceilings, native controls,
and limits of the evidence.

The four public suites share [directory ownership](support/mod.rs).
Public and internal fixtures use the same test-only [cleanup function](support/cleanup.rs),
which reports cleanup failures after successful scenarios and preserves the
original panic during unwinding. The lifecycle fixture regression
checks successful cleanup, an already removed directory, a real cleanup failure,
and a subprocess unwind so a second panic cannot abort the parent harness.

The process-lease test runs its own child branch under an exact selection and
requires a readiness marker. It checks both normal release and early child
teardown; no separate child-only test reports an empty success in an ordinary run.

## Legacy public contracts

| Modules | Concern |
| --- | --- |
| [execution/queries.rs](execution/queries.rs) | Independently enumerated key pairs, grouping, and LIMIT across batches and empty input. |
| [execution/ownership.rs](execution/ownership.rs) | Borrowed results, memory release, and diagnostic spans after source/query teardown. |
| [execution/stack.rs](execution/stack.rs), [load/stack.rs](load/stack.rs) | Loaded open, load, preparation, and execution on ordinary threads and separately qualified small stacks. |
| [load/lifecycle.rs](load/lifecycle.rs) | Load publication, reopen, retained receipts, and token shape versus issuance. |
| [execution/cli.rs](execution/cli.rs), [load/cli.rs](load/cli.rs) | CLI query sources and sinks, load publication, and independent receipt history. |

The load subprocess selects the ordinary or bounded test by its full name and
reports completion after the scenario. Successful process exit without that
marker is a failure. Each pair shares one scenario and its expected results.
Thread setup selects the stack size and checks its native extent only for the
bounded variant. Keep fixture setup and teardown outside measured workers when
those operations are outside the original stack contract.

## Declared-table contracts

| Modules under `catalog_lifecycle/` | Concern |
| --- | --- |
| `append.rs`, `snapshots.rs` | Append failure/drop, date admission, pinned generations, two threaded readers across publication/reclamation and early drop, reopen, and receipts. |
| `aggregates.rs`, `grouping.rs`, `spooling.rs` | Global arithmetic and typed extrema, grouped keys, demanded errors, mixed typed results, and memory/disk output. |
| `joins.rs`, `join_corpus.rs` | Inner/left join composition, snapshots, duplicate pairs, unmatched NULL extension, post-join filters, nested nullable producers and demanded errors, and independent row oracles (648 composition cases plus typed values through spill). |
| `computed.rs`, `constant_projection.rs`, `boolean.rs`, `membership.rs`, `text_filter.rs`, `null_predicate.rs` | SELECT/EXTEND expression demand and scalar/predicate semantics; COALESCE covers nullable defaults and skipped/demanded dependency errors; DIV/MOD cover exact signed quotients/remainders, integer extremes, NULLs and integer-only binding; FLOOR/CEIL/CEILING cover DOUBLE promotion, conversion boundaries, fractional rounding, demanded errors and buckets; shared unary tests cover stored bits and reopen; SIGN covers signed classification, zero/NaN bits, stored-value composition and reopen; ABS covers type preservation, minimum-integer overflow, nesting and deviations; division and SAFE_DIVIDE cover coercion, precedence, NULL/zero/overflow, argument errors, typed NULL constants, Boolean demand, grouped ratios, cancellation and cleanup; typed constants cover source lifetime, malformed unused values, full widths and composition; membership has an independent nullable-set model; infix NOT IN and NOT BETWEEN cover typed results, NULL/NaN negation, precedence, rejected forms and demanded errors. |
| `window_count.rs` | Full-partition cardinality, zero-temp counter selection, original scope, demanded errors, producer composition and typed snapshot retention. |
| `order.rs`, `distinct.rs`, `limit.rs` | Materialization, complete-row equality, ordering, and prefix boundaries. |
| `except.rs` | Complete positional difference, left NULLability, repeated physical slots, nested composition, typed equality, and prepared snapshots. |
| `intersect.rs` | Complete-row intersection against an independent set oracle, typed NULL/DOUBLE equality and original bits, pinned snapshots, positional names, input NULLability, and shared EXCEPT/INTERSECT demanded-error controls. |
| `null_safe.rs` | Two-valued column/literal truth tables, typed NULLs, numeric boundaries and stored bits, Boolean demand, composition, literal rejection and prepared snapshots across reopen. Shared null-predicate tests retain demanded errors and spans. |
| `nullif.rs` | Numeric sentinel normalization, mixed-type NULL coercion, stored NaN/signed-zero bits and prepared snapshots, argument error order, outer demand, grouping, joins and set composition. |
| `multiset.rs` | Independent complete-row count oracle, unequal multiplicities, typed NULL/DOUBLE classes and left bits, pinned inputs, metadata, nested arguments, joins and aggregation for EXCEPT ALL and INTERSECT ALL. |
| `union.rs` | Positional ALL/DISTINCT composition, complete-row equality and original typed representatives, snapshot retention, demanded errors, spill/refusal, and cancellation prefixes. |
| `wide.rs` | Positional set-operation source-pool and 64-column output limits, including the bounded-thread variant. Full-width schemas, late columns, repeated outputs, and scan admission. |

Shared helpers construct public fixtures and collect typed results. The test bodies
own their expectations and case-specific failure checks. Preserve independent
oracles when consolidating setup; do not replace them with engine calculations.

The ordered grouping corpus in `grouping.rs` complements the
[runnable memory comparison](../docs/getting-started.md#observe-grouping-with-less-memory).
It checks few/many groups and uniform/skewed inputs at two budgets, including
complete ordered results, observed disk use, and reservation release. Wide text
keys and cancellation across controller phases live in the internal grouping
tests; the composed ownership campaign measures a held grouped reader alongside
another query under the same database authority.

Nullable COUNT tests cover all four scalar types, demanded scalar errors,
repeated and joined inputs, and the distinction from COUNT(*). The wider
grouping fixture observes spill at 1,600,000 bytes and memory execution at
4,000,000 bytes. It checks cancellation after spill begins, admission refusal,
temporary-space refusal, retries, and release. These budgets apply to that
fixture, not to arbitrary queries.

The [physical-planning mutations](../src/execution/planning/tests.rs) run on
macOS and Linux. They check producer edges, hidden order demand, DISTINCT/LIMIT
mapping, and refusal of malformed references to unvalidated later rows.

Internal invariant and effect-cut tests remain beside the relevant implementation
or in its `tests.rs` child. The internal
[database lifecycle tests](../src/database/tests.rs) cover creation, held leases,
namespace corruption, repair, and cleanup through the injectable effect boundary.
The [catalog fixture owner](../src/catalog_snapshot/tests.rs) provides shared
on-disk setup and typed value readers. Its contract tests have explicit imports
and retain their own expectations:

| Module | Contract |
| --- | --- |
| [publication.rs](../src/catalog_snapshot/tests/publication.rs) | Publication transitions, honest receipts, and effect cuts. |
| [namespace.rs](../src/catalog_snapshot/tests/namespace.rs) | Bootstrap, selected-graph admission, and corruption before repair. |
| [snapshots.rs](../src/catalog_snapshot/tests/snapshots.rs) | Pins, concurrent publication, writer abandonment, and receipt lookup. |
| [declarations.rs](../src/catalog_snapshot/tests/declarations.rs) | Table identities, admission, construction, and rollback/reopen. |
| [append.rs](../src/catalog_snapshot/tests/append.rs) | Batch ownership, exact/short allocation admission and workspace growth, abort, commit, and cleanup. |
| [queries.rs](../src/catalog_snapshot/tests/queries.rs) | Pinned scans, demanded values, cancellation, and result ownership. |

Reclamation's [fixture owner](../src/catalog_snapshot/reclaim/tests.rs) supports
[scratch construction and recovery](../src/catalog_snapshot/reclaim/tests/scratch.rs),
[protected graph traversal](../src/catalog_snapshot/reclaim/tests/traversal.rs), and
[cleanup, cancellation, and interruption](../src/catalog_snapshot/reclaim/tests/cleanup.rs).
Its [external inventory tests](../src/catalog_snapshot/reclaim/inventory/tests.rs)
use the same narrow fixture helpers. Directory cleanup reports unexpected failures
after a successful test and preserves the original panic during unwinding.
The interruption test derives its subprocess selector from its current module and
requires the child to reach the selected exit boundary.

The [declared scan tests](../src/execution/scan/declared.rs) observe allocated
payload capacities for every type, exact/one-byte-short admission before I/O,
native read faults, truncation, and workspace release.
The internal [legacy scan tests](../src/execution/scan/legacy/tests.rs) cover scan admission,
source effects, result ownership, and cancellation. These and the shared
execution suites run on macOS and Linux with the documented target-specific
stack ceilings. Aggregate mapping and
controller checks live under [aggregation](../src/execution/aggregation/tests.rs),
with independent numerical vectors, captured arguments, hash grouping, and disk
reduction beside their respective owners. General grouping scenarios separate
[admission](../src/execution/aggregation/grouping/tests/admission.rs),
[replay](../src/execution/aggregation/grouping/tests/replay.rs),
[failure](../src/execution/aggregation/grouping/tests/failure.rs), and
[output](../src/execution/aggregation/grouping/tests/output.rs), sharing only fixture
and controller mechanics. [Row-codec tests](../src/execution/blocking/record.rs)
check key equivalence, ordering, hashing, and raw value preservation.
[Sorting tests](../src/execution/blocking/sorting_tests.rs) check wide payloads,
row/byte caps, merge passes, cancellation, corruption, and terminal failure
without importing aggregate evaluation.
[Sorted-set owner tests](../src/execution/blocking/sorted_set/tests.rs) reconcile actual
allocation capacities, exact/short admission, both sorted readers, replay and
cancellation phases for both DISTINCT and ALL forms of EXCEPT and INTERSECT. They share only the two-column database fixture with join
checks; complete-row expectations remain local. Padding checks distinguish allocated
capacity from encoded-frame, run-byte, and run-row limits, including a valid
checksummed frame beyond the reader's admitted encoded limit.
Legacy load tests separate [normal loading and admission](../src/load/tests/loading.rs),
[publication and recovery](../src/load/tests/publication.rs), and
[failure schedules](../src/load/tests/failures.rs). Their shared namespace reader
keeps its independent checksum and layout expectations. Staging and private-unit
readback checks live beside those owners. Hash-state capacity checks
compare the admission charge with actual Vec capacities and keep logical lanes
separate from physical padding.

CLI [parser tests](../src/cli/command/tests.rs) exercise grammar and argument
ownership. Query-source, diagnostic, native-capture, and sink tests stay beside
their CLI owners. Run all of them with `cargo test --release --offline --locked --bin pipesql`.

Native and allocator callers under
[`tools/fixtures`](../tools/README.md#native-and-allocation-callers) are separate
stock-artifact checks. A passing integration test does not replace either boundary.

## Persistent and semantic fixtures

`fixtures/` contains licensed SQL, independent expected results, and encoded
current/retired/rejected formats. The maintained fixture gate identifies their
encoders. Use `python3 tools/check-fixtures.py` from the repository root to compare
bytes; that command does not update fixtures or execute the engine.

A fixture change must retain its input provenance, license, generation command,
and intended admission or rejection behavior. Do not regenerate expected bytes
from the production codec. Do not update an expected result just because a test
fails. Historical format numbers are compatibility discriminators, not product
release names.

## Adding or moving tests

Place a case under the contract it exercises. Keep the public suite entry point
small and group shared setup at the narrowest useful scope. Retain exact case
names and bodies during a pure organization change where practical. Check module
inclusion and test discovery after a move; syntax formatting alone does not prove
that a test will run.

Shared lifecycle and query scenarios run on macOS and Linux. Native-specific
tests and stack ceilings remain platform-dependent. A successful
build, ignored test, or empty selection must not be reported as runtime coverage.

Shared scratch recovery also has [legacy namespace controls](../src/scratch/tests.rs):
constructor refusal, process termination at creation/unlink/barrier/admission,
reads during bootstrap, strict writer inspection and rejected corrupt debris.
The subprocess test invokes itself at explicit cuts and requires its child exit
status; it introduces no ignored or separately selected test.

The [analytic counter tests](../src/execution/count.rs) check zero-field admission,
the row bound, cancellation and replay. Runtime replay also exercises that owner
after partial and complete emission in the retained-output grouping tests.
