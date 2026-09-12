# Work plan

[README](../README.md) owns product requirements; [Engineering](../docs/engineering.md)
owns working method. This plan identifies unfinished product work.
[Evidence](evidence.md) records verification and consequential limitations.

## Current baseline

Use the [reading path](../docs/README.md#learn-the-implementation),
[source map](../docs/source-map.md), [test guide](../tests/README.md), and
[tool guide](../tools/README.md) to navigate the implementation and its checks.
Maintained builds, tests, and examples require no historical checkout or archive.

The complete 23-stage gates for `5a3cb0a` pass on macOS and GNU arm64 Linux
on matching frozen inputs. Each platform executes 480 Rust tests and 298
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

## Completed: column transformations

SELECT, EXTEND, SET, DROP, RENAME, and AS form the current bounded
column-transformation profile. The [language manifest](../docs/language.md#current-public-query-manifest)
owns its syntax and semantics; [evidence](evidence.md#column-transformation-semantics)
records independent cases, retained qualified inputs, typed-copy identity,
resource/failure coverage, and the verified cross-platform tutorial. Both complete
gates pass on frozen source `5a3cb0a`. The 64-column public bound remains intact;
internal payloads admit up to 128 visible and retained values. Larger inline
owners retain their existing resource charges. Compact hash storage and EXTEND
remain complete. Reopen these only for a concrete defect or limitation.

## Next: reconcile composed memory ownership

Explain the difference between logical reservations, requested allocations, and
allocator-usable extents for one representative composed query. The current
[ownership checkpoints](evidence.md#composed-query-owners) prove release and
coexistence, but do not attribute every difference or establish a physical-memory
bound. This is the next resource qualification task, not a new query feature.

1. Trace existing counters and owners before adding instrumentation. Choose a
   reproducible workload with overlapping blocking owners and an independent
   complete-result oracle. Use the existing caller and build/subprocess tools.
2. Reconcile stable checkpoints through preparation, execution, cancellation,
   completion, and release. Separate engine allocations, caller/runtime owners,
   allocator rounding, reserved capacity, and unsampled transitions. Include a
   constrained-resource case and a negative control that detects incorrect
   attribution. Repair a concrete accounting defect across its consumers.
3. Verify the maintained diagnostic on macOS and available native GNU/Linux
   storage. Keep unsupported platform and observer limits explicit. Update the
   resource explanation and concise evidence so a learner can reproduce and
   interpret the result. Run verification required by retained changes, commit
   locally, and remove owned outputs.

Limit initial investigation to four hours before reassessing scope and evidence.
Do not infer a whole-process cap from engine counters, add a new monitoring
framework, or redesign scheduling without a concrete failure. Finish a useful,
reproducible reconciliation; identify any remaining qualification precisely.

Do not push or modify remote refs. Windows remains unfinished; unavailable
platform resources do not block the available verification.

## Applying DuckDB lessons

DuckDB's published designs inform the following work. These are PipeSQL design
choices and acceptance checks, not claims that DuckDB's results transfer here.
The [MIN/MAX evidence](evidence.md#typed-min-and-max) and
[grouping workload](evidence.md#grouping-learning-workload) record the delivered
implementation, examples, checks, and accepted costs.

| Priority | Lesson and source | PipeSQL action and completion check |
| --- | --- | --- |
| Applied in MIN/MAX | [External aggregation](https://duckdb.org/2024/03/29/external-aggregation) makes variable-width ownership and relocation part of the spill design. | Capture owned STRING bytes before releasing source batches. Independently validate replay lengths, UTF-8, NULLs, and type identity. Exercise maximum-length values, growing/shrinking replacements, and replay after producer release. Keep checked serialized records; pointer relocation would add machinery without an established need. |
| Applied in compact hash storage | The same external-aggregation study varies group cardinality and measures operation beyond available memory. | Existing workloads check cardinality, budget pressure, and full-width text replay; exact admission checks cover the fallback minimum. The completed STRING study establishes unused capacity and short-string spill costs. Compact spans and admitted arena growth remove short-string spill in the maintained 4 MB workload. Retain the recorded transient growth cost for wide values; no speedup is claimed. |
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
