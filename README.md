# PipeSQL

PipeSQL is an embedded, single-node analytical database written in Rust. Queries
use a restricted profile of GoogleSQL pipe syntax: start with `FROM`, then read
transformations from top to bottom with `|>`. Traditional `SELECT ... FROM ...`
query blocks are rejected.

PipeSQL is intended to run on **macOS, Linux, and Windows**. It is pre-release:
formats and interfaces are unstable, and platform qualification is incomplete.
Runtime checks cover the reviewed macOS path and a subset of Linux behavior.
Linux integration coverage and durability qualification remain incomplete;
Windows implementation remains unfinished.

## What works today

The library creates declared tables, appends typed batches, and queries
immutable snapshots. One serialized writer can coexist with snapshot readers.
Queries support numeric projections, STRING/NULL/Boolean filters, COUNT/SUM/AVG,
grouped and repeated aggregation, full-row DISTINCT, equality joins, ORDER BY,
LIMIT/OFFSET, and independent FROM/JOIN subqueries. Grouping has memory and
spill paths; joins, ordering, and DISTINCT use shared sorting. Reclamation
preserves pinned queries and commit receipts.

The legacy fixed-schema `lineitem` loader also supports Q1/Q6 through the shared
query engine. These capabilities do not imply general GoogleSQL or TPC-H
support. The [language manifest](docs/language.md#current-public-query-manifest)
specifies exact accepted forms and limits. The [work plan](notes/plan.md)
records unfinished work; [retained evidence](notes/evidence.md) distinguishes
observed results from open qualification requirements.

## Build, test, and use

Install the pinned Rust 1.98.1 toolchain, Clippy, rustfmt, Python 3.11 or newer,
and the native compiler/SDK required by the target's checks. Dependencies are
locked; libc 0.2.186 is vendored. Builds are offline after toolchain
provisioning.

```sh
cargo build --release --offline --locked
python3 tools/check-maintenance.py
sh tools/check.sh
```

[Build and test](docs/testing.md) explains prerequisites, focused commands, full
verification, and failure diagnosis. The full gate currently requires macOS;
Linux can run its documented core subset. The full gate includes independent
fixtures, warnings-denied compilation, Rust tests, and semantic, resource, and
native failure campaigns. A green local gate is not production certification.

Start with [Create and query a declared table](docs/getting-started.md) for a
complete library example. After loading the documented `lineitem` input, this
query groups selected rows and filters the aggregate result:

```sql
FROM lineitem
|> SELECT l_quantity AS qty, l_returnflag AS flag
|> WHERE qty < 25
|> AGGREGATE SUM(qty) AS total, COUNT(*) AS n GROUP AND ORDER BY flag
|> WHERE n > 1000
|> SELECT flag, total;
```

Save it to a file and run:

```sh
target/release/pipesql query --database /absolute/database --query-file /absolute/query.sql \
  --memory-limit-bytes 2000000 --temp-limit-bytes 1000000
```

Require successful completion: schema or partial rows alone do not establish a
complete result. After an uncertain load, resolve the printed transaction token
using the [CLI recovery
instructions](docs/interfaces.md#resolve-a-cli-load-outcome).

## Product requirements

These requirements govern implementation and design decisions.

1. **Pipe-only language.** Queries and subqueries use the accepted PipeSQL
   profile. There is one parser, binder, typed relational plan, and execution
   path. Unsupported syntax is rejected, not translated through another frontend.
2. **Embedded, single-node OLAP.** PipeSQL is a library with a local CLI.
   Projected scans, joins, aggregation, sorting, windows, bulk ingestion, and
   analytical transformations define the intended workload. There is no server,
   network protocol, replication, consensus, sharding, or distributed scheduler.
3. **Snapshot concurrency.** The initial model is one serialized writer with
   concurrent immutable readers, not row-at-a-time multiwriter transactions.
4. **Bounded external-memory execution.** Every supported blocking operator has
   a bounded path below its natural working set. Memory pressure may reduce
   concurrency or performance; it must not silently cause wrong results or
   unbounded consumption. Input text, nesting, memory, stack, queues, retries,
   work quanta, temporary data, recovery, and snapshot retention have explicit
   limits, owners, and terminal behavior.
5. **Truthful persistence.** Durable success follows the event it acknowledges.
   Definite abort, success, and possibly committed outcomes remain distinct.
   Recovery validates authoritative state and never silently falls back from a
   known newer commit to an older snapshot. Durability depends on documented OS,
   filesystem, and device synchronization premises; independent power-loss
   certification requires separate evidence.
6. **Cross-platform boundaries.** Database semantics and algorithms are portable.
   Native implementations for macOS, Linux, and Windows must preserve the same
   product contracts using their documented platform primitives. Implementation,
   compilation, native execution, and durability qualification are separate claims.
7. **Complete features.** A feature includes semantics, ownership, resources,
   cancellation, failure, cleanup, observability, and stock-artifact tests.
   A successful demonstration of its normal path is insufficient.
8. **Compact, maintainable implementation.** Shipped first-party production
   behavior, including generated code and official adapters, stays below 500,000
   nonblank, noncomment source lines. Tests and tools are counted separately.
   Engineers must be able to understand, build, test, operate, and change the
   system using the repository and ordinary tools. Complexity must repay its
   maintenance and verification cost.

The intended workload is local analytics and ELT over small files through data
larger than configured memory: append-heavy facts, smaller dimensions,
analytical queries, bulk import/export, and multiple readers alongside one bulk
writer. The first release excludes distributed availability, remote object
storage as transactional storage, row locks, high-rate singleton updates,
triggers, stored procedures, foreign keys, locale-sensitive collation,
unrestricted native extensions, and a service control plane.

## Decision order

When feasible designs conflict, decide in this order:

1. semantic and durability correctness;
2. bounded resources and failure containment;
3. usefulness for the intended workload;
4. measured end-to-end performance;
5. simplicity and maintainability;
6. dependency and source cost; and
7. extensibility.

A later objective cannot compensate for violating an earlier one.

## Architecture and release standard

The current design uses a hand-owned frontend, immutable typed plans,
interpreted vector execution, immutable column chunks, and bounded
WAL/checkpoint publication. Stable column identities remain distinct from names
and ordinals. Physical planning consumes validated semantic plans and a pinned
snapshot, not source text. DOUBLE storage preserves raw IEEE-754 bits. The
engine forbids unsafe code; required native operations are confined to explicit
filesystem and CLI boundaries.

The legacy single-load path uses format 4; declared tables and repeated appends
use namespace format 7. Both share publication and retain successful-attempt
history within admitted bounds. [Storage](docs/storage.md) and
[Transactions](docs/transactions.md) own exact representations and recovery
rules. These formats are not stable compatibility promises.

Production status requires the stock library and CLI to satisfy the product
contract on every advertised platform, including semantic, forced-memory,
resource-refusal, crash/corruption, recovery, concurrency, dependency, and
representative performance evidence. See the [release
requirements](docs/verification.md#release-criteria) and [consolidation
gate](docs/verification.md#production-consolidation-gate).

Releases are identified by immutable build provenance and supported capability
contracts, rather than numeric preview milestones. Production names describe
capabilities. Persistent-format revision numbers remain mandatory fail-closed
compatibility discriminators.

## Find your way around

- [Documentation index](docs/README.md): usage and authoritative contracts.
- [Reading path](docs/README.md#learn-the-implementation): learn through a working
  example and the functions that implement it.
- [Source map](docs/source-map.md): implementation owners and data flow.
- [Engineering guide](docs/engineering.md): how to change and verify the system.
- [Test map](tests/README.md) and [tools](tools/README.md): checks and their scope.
- [Work plan](notes/plan.md): current decisions, gaps, and publication restrictions.
- [Retained evidence](notes/evidence.md): current verification and unresolved limitations.
- [Third-party attribution](THIRD_PARTY.md): dependency and adapted-source terms.

README owns product scope and decision order. Technical contracts under `docs/`
are authoritative within their stated concerns. Historical evidence cannot amend
those contracts. Active fixtures and reference implementations live with tests
and tools; builds do not depend on historical notes or temporary archives.
