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
Four public catalog tests, two legacy tests, and six internal library tests
requiring a measured stack at most 64 KiB
are explicitly ignored on GNU aarch64, whose native pthread
minimum exceeds that limit. [Platform status](../docs/testing.md#platform-status)
explains the observer, exclusions, and remaining qualification.

Public fixture directories report cleanup failures after successful scenarios and
preserve the original panic during unwinding. The catalog fixture regression
checks successful cleanup, an already removed directory, a real cleanup failure,
and a subprocess unwind so a second panic cannot abort the parent harness.

## Legacy public contracts

| Modules | Concern |
| --- | --- |
| [execution/queries.rs](execution/queries.rs) | Independently enumerated key pairs, grouping, and LIMIT across batches and empty input. |
| [execution/ownership.rs](execution/ownership.rs) | Borrowed results, memory release, and diagnostic spans after source/query teardown. |
| [execution/stack.rs](execution/stack.rs), [load/stack.rs](load/stack.rs) | Native stack observation around loaded open, load, preparation, and execution. |
| [load/lifecycle.rs](load/lifecycle.rs) | Load publication, reopen, retained receipts, and token shape versus issuance. |
| [execution/cli.rs](execution/cli.rs), [load/cli.rs](load/cli.rs) | CLI query sources and sinks, load publication, and independent receipt history. |

The stack subprocess uses its full test selector and reports completion after
the measured work. Successful process exit without that marker is a failure.
Keep the query expectations and small-stack operations in their test bodies;
shared setup must not add work to the measured stack.

## Declared-table contracts

| Modules under `catalog_lifecycle/` | Concern |
| --- | --- |
| `append.rs`, `snapshots.rs` | Append failure/drop, date admission, pinned generations, reopen, reclamation, and receipts. |
| `aggregates.rs`, `grouping.rs`, `spooling.rs` | Global arithmetic, grouped keys, mixed typed results, and memory/disk output. |
| `joins.rs`, `join_corpus.rs` | Join composition, snapshots, duplicate pairs, and the independent nullable-row oracle. |
| `computed.rs`, `boolean.rs`, `text_filter.rs`, `null_predicate.rs` | Expression demand and scalar/predicate semantics. |
| `order.rs`, `distinct.rs`, `limit.rs` | Materialization, complete-row equality, ordering, and prefix boundaries. |
| `wide.rs` | Full-width schemas, late columns, repeated outputs, and scan admission. |

Shared helpers construct public fixtures and collect typed results. The test bodies
own their expectations and case-specific failure checks. Preserve independent
oracles when consolidating setup; do not replace them with engine calculations.

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
| [append.rs](../src/catalog_snapshot/tests/append.rs) | Batch ownership, refusal, abort, commit, and cleanup. |
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
The internal [legacy scan tests](../src/execution/scan/legacy/tests.rs) cover scan admission,
source effects, result ownership, and cancellation. These and the shared
execution suites run on macOS and Linux; GNU arm64 stack exclusions are explicit
in the affected tests. Aggregate mapping and
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
Legacy load tests separate [normal loading and admission](../src/load/tests/loading.rs),
[publication and recovery](../src/load/tests/publication.rs), and
[failure schedules](../src/load/tests/failures.rs). Their shared namespace reader
keeps its independent checksum and layout expectations. Staging and private-unit
readback checks live beside those owners.

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
tests and the GNU arm64 stack exclusions remain platform-dependent. A successful
build, ignored test, or empty selection must not be reported as runtime coverage.
