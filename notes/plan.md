# Work plan

[README](../README.md) owns product requirements; [Engineering](../docs/engineering.md)
owns working method. This plan identifies unfinished product work.
[Evidence](evidence.md) records verification and consequential limitations.

## Current baseline

Use the [reading path](../docs/README.md#learn-the-implementation),
[source map](../docs/source-map.md), [test guide](../tests/README.md), and
[tool guide](../tools/README.md) to navigate the implementation and its checks.
Maintained builds, tests, and examples require no historical checkout or archive.

The complete 23-stage gates for `827cad5` pass on macOS and GNU arm64 Linux
on matching frozen inputs. Each platform executes 454 Rust tests and 285
composition cases, plus its applicable native and allocation campaigns. All twelve bounded-thread scenarios and their ordinary-thread
counterparts execute on both platforms, with explicit target-specific ceilings.
Two Darwin ACL-specific allocation cells remain excluded on Linux. Windows remains
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

## Active: compact optional hash text storage

The [STRING grouping study](evidence.md#string-grouping-costs) is complete.
On both tested platforms, four groups of eight-byte extrema reserve about
41 MB at an 80 MB query budget. At 4 MB, 256 short-string groups spill despite
having only 4,096 bytes of useful extrema. Maximum-width values still need
substantial capacity. The study establishes a representation cost; it found no
wrong result or resource-release defect.

Replace maximum-width STRING slots in the optional hash owner with bounded
variable-width storage and explicit spans. Keep the one-group disk fallback,
source replay, persistent formats, and independent validators intact. Before
implementation, make growth, replacement, transient old/new allocations, and
exhaustion locally understandable and fully charged. Avoid per-row allocation,
unbounded compaction, and new shared-memory frameworks.

The design review is bounded to 30 minutes before reassessment. Hash grouping
currently reuses the fixed accumulator representation and folds captured arguments
only after assigning every row in a batch to a group. On hash exhaustion, the
controller destroys optional state and replays the pinned source through the
already-admitted disk owner. Preserve that transition.

Two costs must be separated: maximum-width text per slot and eager allocation
of slots from the entire available budget. Merely packing text while admitting
many more slots could retain the same unused reservation. Compare a small initial
allocation with bounded growth against a fixed optional capacity policy before
choosing the smaller complete implementation. Any growth must occur outside row
folding, charge simultaneous old/new buffers, and preserve bounded cancellation
points. Repeated replacements must reuse capacity or have a proved space bound;
an arena that accumulates every discarded value is insufficient. The first
falsifier is the existing eight-case workload plus an increasing-length replacement
case; a lower short-string reservation alone cannot excuse a broken wide-value
or fallback path.

The candidate uses explicit `(start, capacity)` spans for hash text slots; the
existing extremum word continues to hold length. A replacement reuses its span
when it fits. Otherwise it receives a power-of-two region in an append-only
arena. For each slot, superseded region capacities sum to less than its current
capacity, even after repeated shrinking and regrowth. Thus discarded regions
remain bounded by retained capacities rather than input row count. UTF-8, source
width, span extents, and lengths still require independent checks.

Allocate or grow the arena before folding a captured batch. Charge the complete
new buffer while the old buffer remains live, copy in bounded cancellable steps,
and release the old charge only after destroying its buffer. If admission fails,
destroy optional state and use the existing pinned-source replay. Do not mutate
the fallback accumulator. The candidate sizing policy caps initial STRING hash
metadata at 4,096 groups (the existing run-row bound), subject to available
memory, while leaving numeric-only sizing unchanged. This is a policy to test,
not an additional semantic group limit: larger inputs retain the disk path.
Reject or revise it if the workload comparison reveals an unjustified regression.

Implementation checkpoint: hash grouping now uses compact spans, pre-fold
admission, cancellable arena copying, and fixed-layout finalization. The release
aggregation selection passed 58 tests. Warnings-denied Clippy passed for the
library, tests, and examples. The eight-case macOS workload completed with full
results and resource release: short-string reservations are about 2.49 MB at
both budgets, and 256 short-string groups no longer spill at 4 MB. Wide values
still spill at 4 MB and stay in memory at 80 MB; growth overlap raises that case's
sampled peak to 52.82 MB. Treat these as initial observations, pending final
verification and allocator comparison.

The earlier debug aggregation selection aborted in the bounded-stack test;
the required release version passes. A new fixture initially selected legacy
one-byte text storage and was corrected to declared UTF-8 storage. A follow-up
focused test now checks cancellation after the first 65,536-byte copy while
both arena buffers remain live. That two-test focused selection passes. All ten public aggregate tests pass,
including NULL/empty/Unicode behavior and the new 256-group no-spill regression.
Next, complete frozen macOS/Linux gates and final workload/allocator comparisons,
review any failures, and consolidate the final evidence and plan.

Completion requires unchanged query semantics and failure/cleanup contracts,
meaningfully lower short-string reservations, and no spill for the maintained
256-group short-string workload at 4 MB. Keep maximum-width values correct
through admission and fallback; do not obtain the short-string improvement by
silently reducing supported widths or weakening resource accounting. Exercise
replacements, NULLs, replay after producer release, refusal, cancellation,
allocation boundaries, and complete results. Compare the maintained workload
against its recorded baseline without treating single timings as a speed claim.
Run the affected focused checks, then required complete verification on frozen
inputs. Update resource contracts and examples, commit locally, and close the
milestone once these criteria are met.

## Applying DuckDB lessons

DuckDB's published designs inform the following work. These are PipeSQL design
choices and acceptance checks, not claims that DuckDB's results transfer here.
The [MIN/MAX evidence](evidence.md#typed-min-and-max) and
[grouping workload](evidence.md#grouping-learning-workload) record the delivered
implementation, examples, checks, and accepted costs.

| Priority | Lesson and source | PipeSQL action and completion check |
| --- | --- | --- |
| Applied in MIN/MAX | [External aggregation](https://duckdb.org/2024/03/29/external-aggregation) makes variable-width ownership and relocation part of the spill design. | Capture owned STRING bytes before releasing source batches. Independently validate replay lengths, UTF-8, NULLs, and type identity. Exercise maximum-length values, growing/shrinking replacements, and replay after producer release. Keep checked serialized records; pointer relocation would add machinery without an established need. |
| Storage change selected | The same external-aggregation study varies group cardinality and measures operation beyond available memory. | Existing workloads check cardinality, budget pressure, and full-width text replay; exact admission checks cover the fallback minimum. The completed STRING study establishes unused capacity and short-string spill costs. Implement the bounded representation change above and repeat the same complete-result workload to test its benefit. |
| Applied in MIN/MAX | [Memory management](https://duckdb.org/2024/07/09/memory-management) treats blocking intermediates and their competing owners explicitly. | Extend existing composed-query checks to extrema. Exercise memory refusal, exhausted temporary capacity, cancellation, and release. Attribute spill to the relevant operator or a controlled query; total temporary bytes alone do not prove aggregation spilled. |
| Current tests | DuckDB's [SQL test guidance](https://duckdb.org/docs/lts/dev/sqllogictest/writing_tests) favors exercising behavior through SQL. | Keep queries and independent expected results visible in existing public tests. Retain internal corruption and allocation controls where SQL cannot establish the invariant. No additional test framework is needed. |
| Later physical-memory qualification | DuckDB's memory-management article distinguishes component memory and temporary storage measurements. | Reconcile existing logical charges with allocator observations and other process owners on one representative composed workload. Explain unexplained differences before claiming a process-memory cap; add a diagnostic only when an existing observation cannot answer the question. |

DuckDB is not a universal differential oracle. Its documented
[floating-point ordering](https://duckdb.org/docs/current/sql/data_types/numeric#floating-point-types)
places NaN above other numbers. Our GoogleSQL-derived MIN/MAX contract propagates
NaN in both directions. Establish compatible NULL, overflow, floating-point,
collation, and ordering semantics before comparing results. Keep explicit local
expectations for incompatible cases; never change them to match DuckDB.

The qualification gaps below remain separate obligations. Partitioned hash
aggregation, a shared buffer manager, and parallel execution remain possible
alternatives, not scheduled rewrites. Investigate one only when a representative
workload exposes a concrete limitation in the current owner. Require an independent correctness oracle,
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
