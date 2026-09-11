# Work plan

[README](../README.md) owns product requirements; [Engineering](../docs/engineering.md)
owns working method. This plan identifies unfinished product work.
[Evidence](evidence.md) records verification and consequential limitations.

## Current baseline

Use the [reading path](../docs/README.md#learn-the-implementation),
[source map](../docs/source-map.md), [test guide](../tests/README.md), and
[tool guide](../tools/README.md) to navigate the implementation and its checks.
Maintained builds, tests, and examples require no historical checkout or archive.

The complete 23-stage gates pass on macOS and GNU arm64 Linux on the same frozen
inputs. All twelve bounded-thread scenarios and their ordinary-thread
counterparts execute on both platforms, with explicit target-specific ceilings.
Two Darwin
ACL-specific allocation cells remain excluded on Linux. Windows remains
unfinished. The [platform matrix](../docs/testing.md#platform-status) distinguishes
implementation, execution, and qualification; [evidence](evidence.md) records the
checks. Linux campaigns require an unprivileged user and GNU time. A passing
local gate does not establish production readiness.

The shared-mount identity investigation is settled as an environment
qualification limit. The stock caller and raw native observations reproduce the
mismatch on the tested host-shared mount; native storage passes. Descriptor APIs
agree with each other, and no repository normalization defect was established.
Keep fail-closed behavior. The sharing-layer cause remains unresolved; current
[replay instructions](../docs/testing.md#diagnose-filesystem-identity) need no old
checkout or diagnostic binary. Reopen causal investigation when new sharing-layer
evidence can change the disposition.

## Active: complete scalar MIN and MAX aggregation

COUNT(expression) and the [composed execution example](../examples/composed.rs)
are complete. MIN/MAX implementation is in the working tree. Keep this the sole
active milestone; do not add DISTINCT aggregates, windows, scalar types,
unrelated syntax, or speculative optimization.

### Semantics and representation

The primary [MIN/MAX reference](https://docs.cloud.google.com/bigquery/docs/reference/standard-sql/aggregate_functions#min)
requires input-typed results, NULL for empty/all-NULL groups, and NaN propagation.
GoogleSQL's [type rules](https://docs.cloud.google.com/bigquery/docs/reference/standard-sql/data-types)
compare strings by Unicode code points and treat signed zeros as equal. PipeSQL
preserves DOUBLE bits: retain the first NaN payload; when both zero signs occur,
MIN chooses negative zero and MAX positive zero. Demanded arguments still execute
after NaN. Valid UTF-8 byte ordering agrees with code-point ordering. DATE extrema
retain typed, range-checked days.

Identical arguments share evaluation and nullable counts. SUM/AVG own sum cells;
MIN/MAX own separate demanded slots. Numeric extrema use one u64 each. Text
extrema reuse a bounded byte slot and store its length in that word. Legacy
pipelines admit one byte per text extremum per dense group; declared pipelines
admit 65,536 bytes. The controller supplies the source domain independently to
construction and validation. Folding rejects values outside the admitted bound.

Captured text owns one 65,536-byte arena per retained argument per batch. Packed
spans identify bytes within that arena, never source pointers. Spill records use
length words followed by a UTF-8 trailer. The reader bounds lengths before reads,
then checks the complete checksum, type layout, UTF-8, NULL payloads, and date
ranges. Replay flushes when either row or byte capacity fills. The admitted
minimum fits one complete maximum-width row. Persistent formats are unchanged.

Optional hash admission includes extrema and retained text in both capacity
selection and allocation. A nine-extrema pressure regression exposed a missing
term in capacity selection; it failed before the repair and passes afterward.
Keep the simple text slots pending the final budget review; do not introduce an
arena compactor without a measured need.

### Verification completed

- Public macOS tests pass for numeric, DATE, and STRING extrema: empty/all-NULL
  input, special numeric values, typed results, grouping, repeated aggregation,
  shared COUNT/SUM/AVG arguments, and release.
- Ownership and independent-validator tests pass for producer release, group
  reuse, direction/slot corruption, mask conflicts, maximum-length text, UTF-8
  ordering, shorter replacements, and byte-limited replay.
- Checked-record tests reject invalid lengths, UTF-8, NULL payloads, and DATE
  ranges with recomputed checksums. COUNT-only layout interpretations remain
  distinct from retained values.
- A full-length STRING workload passes through hash execution and forced disk
  fallback with exact MIN/MAX/COUNT results. Cancellation during reduction
  publishes no unfinished groups and releases all memory and temporary bytes.
- A demanded-error regression passes for global/grouped MIN and MAX: NaN in
  an earlier input unit cannot suppress multiplication overflow in a later unit.
  Errors retain the aggregate call span; undemanded extrema are not evaluated.
- Legacy global and two-key grouping tests pass under the existing 2 MB budget.
  They check STRING/DOUBLE/DATE results, exact one-byte text-slot charges, and
  rejection when validation is given the wrong source domain.
- Before the latest spill and legacy tests, the serial macOS library/catalog run
  passed 336 + 58 tests, with no failures or ignored tests. The command was
  `cargo test --release --offline --locked --lib --test catalog_lifecycle -- --test-threads=1`.
  An earlier concurrent run failed a process-wide descriptor assertion; serial
  execution passed. Two fixtures were widened to exceed the enlarged argument
  record maximum while retaining their boundary assertions.

These focused results do not replace final platform gates. The latest completed
full gates remain the earlier baseline recorded in [evidence](evidence.md).

### Remaining work, in order

1. Composed regressions now pass for joined, derived, and computed extrema;
   repeated legacy STRING extrema also pass. Demanded-error/span, exact fallback
   admission, temporary exhaustion, and cancellation checks pass. The existing
   allocation campaign now demands numeric/text extrema and verifies their
   results. Mac controls pass with 721 allocations on both short and 384-byte
   paths. The campaign work ceiling is 800; it still sweeps every measured
   prefix. Complete those sweeps in both full gates.
2. Finish the ownership/readability review. Language, execution, resource,
   source-map, and test contracts now describe extrema. The existing grouping
   tutorial checks MIN=1 and MAX=3 for all 4,096 regions. Fresh macOS runs pass
   at 2 MB (memory 1,590,745 bytes; temporary 0) and 1.2 MB (memory 1,166,088;
   temporary 803,016). Unprivileged GNU/Linux runs also pass: memory 1,590,652
   and 1,166,064 bytes, with temporary peaks 0 and 803,016. The example image is
   `pipesql-verification-rust:1.98.1-time`; its owned container and outputs were
   removed after completion. Accumulator state now lives in `accumulator.rs`;
   Clippy and all maintenance checks passed before the latest campaign extension.
3. Run focused checks for those changes, then complete required macOS and
   unprivileged GNU/Linux gates on matching frozen inputs and isolated targets.
   Reconcile executed tests, exclusions, fixtures/models, public/native campaigns,
   examples, and before/after manifests. Windows remains unqualified.
4. Consolidate self-contained evidence within the 641,696-byte historical budget,
   commit coherent verified changes locally, remove owned outputs, and finish
   with a clean tree. Do not publish or modify remote refs. Close the goal only
   after its implementation, verification, documentation, and cleanup are done.

## Applying DuckDB lessons

DuckDB's published designs inform the following work. These are PipeSQL design
choices and acceptance checks, not claims that DuckDB's results transfer here.
The [grouping evidence](evidence.md#grouping-learning-workload) records the
learning workload already delivered.

| Priority | Lesson and source | PipeSQL action and completion check |
| --- | --- | --- |
| Current MIN/MAX goal | [External aggregation](https://duckdb.org/2024/03/29/external-aggregation) makes variable-width ownership and relocation part of the spill design. | Capture owned STRING bytes before releasing source batches. Independently validate replay lengths, UTF-8, NULLs, and type identity. Exercise maximum-length values, growing/shrinking replacements, and replay after producer release. Keep checked serialized records; pointer relocation would add machinery without an established need. |
| Current MIN/MAX goal | The same external-aggregation study varies group cardinality and measures operation beyond available memory. | Compare complete results across several budgets around the hash admission boundary, with few/many groups and short/long strings. Record retained memory, temporary bytes, and elapsed time. Charge text capacity before admitting groups; reject a representation that prevents the one-group fallback from meeting its stated minimum. Fixed text slots remain a proposal until their cost is measured. |
| Current MIN/MAX goal | [Memory management](https://duckdb.org/2024/07/09/memory-management) treats blocking intermediates and their competing owners explicitly. | Extend existing composed-query checks to extrema. Exercise memory refusal, exhausted temporary capacity, cancellation, and release. Attribute spill to the relevant operator or a controlled query; total temporary bytes alone do not prove aggregation spilled. |
| Current tests | DuckDB's [SQL test guidance](https://duckdb.org/docs/lts/dev/sqllogictest/writing_tests) favors exercising behavior through SQL. | Keep queries and independent expected results visible in existing public tests. Retain internal corruption and allocation controls where SQL cannot establish the invariant. No additional test framework is needed. |
| Later physical-memory qualification | DuckDB's memory-management article distinguishes component memory and temporary storage measurements. | Reconcile existing logical charges with allocator observations and other process owners on one representative composed workload. Explain unexplained differences before claiming a process-memory cap; add a diagnostic only when an existing observation cannot answer the question. |

DuckDB is not a universal differential oracle. Its documented
[floating-point ordering](https://duckdb.org/docs/current/sql/data_types/numeric#floating-point-types)
places NaN above other numbers. Our GoogleSQL-derived MIN/MAX contract propagates
NaN in both directions. Establish compatible NULL, overflow, floating-point,
collation, and ordering semantics before comparing results. Keep explicit local
expectations for incompatible cases; never change them to match DuckDB.

After MIN/MAX, reassess the existing qualification gaps before selecting another
milestone. Partitioned hash aggregation, a shared buffer manager, parallel
execution, and compact string storage are possible alternatives, not scheduled
rewrites. Investigate one only when a representative workload exposes a concrete
limitation in the current owner. Require an independent correctness oracle,
resource costs, and an end-to-end measurement that could reject the proposal.
Keep adopted invariants in their existing contracts and replace settled plan
entries instead of accumulating a research archive.

## Remaining qualification

| Concern | Required outcome |
| --- | --- |
| Windows | Implement native paths, handles, traversal, locking, synchronization, CLI startup, process ownership, and target-specific verification. |
| Filesystems and durability | Retain the tested shared-mount exclusion; qualify supported filesystem/device premises beyond process termination. |
| Physical memory | Reconcile logical charges with allocator-usable memory and other owners without claiming a whole-process cap from engine counters. |
| Native diagnostics | Establish reproducible sanitizer controls and identify instrumentation limits before attributing reports or claiming a clean boundary. |
| Product qualification | Extend representative language/workload, crash/concurrency, and performance evidence; define compatibility before promising stable formats or interfaces. |

Choose work by README's decision order. For each milestone, identify the affected
contract, a concrete falsifier, and completion criteria. Update this plan when a
finding changes the next action; replace settled entries instead of accumulating
an implementation diary. Keep evidence self-contained and within its retention
budget.

## Publication

Review the exact tree and destination before publishing. Ordinary development
must preserve published history. Any later history replacement requires explicit
authorization and reconciliation of the actual remote state.
