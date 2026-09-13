# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The complete 24-stage gates for `f832a21` pass on matching frozen macOS and GNU
arm64 Linux inputs: 600 ordinary Rust tests per platform, 24 independent aggregate
semantic cases and 311 composition cases, plus the applicable allocation
and native campaigns. The
[checkpoint](evidence.md#full-verification-checkpoint) records exact inputs,
controls and limits. Linux retains two Darwin ACL exclusions. The
[platform matrix](../docs/testing.md#platform-status) distinguishes implementation,
execution and qualification; a passing gate does not establish release readiness.

Use the [language manifest](../docs/language.md#current-public-query-manifest),
[implementation reading path](../docs/README.md#learn-the-implementation),
[test map](../tests/README.md) and [tool map](../tools/README.md) for current
capabilities and commands. Maintained checks need no historical checkout or
archive. Completed milestones remain closed unless a concrete defect, affected
boundary or new workload changes their disposition. The
[testing/tooling review](evidence.md#testing-and-tooling-cleanup) retains its
coverage inventory and consequential deletion rationale.

## Completed: positional INTERSECT DISTINCT

Bounded positional INTERSECT DISTINCT is verified through `f832a21` on matching
complete macOS and GNU/Linux gates. The [intersection record](evidence.md#positional-intersect-distinct)
retains semantics, independent complete-row results, validators, exact admission,
spill/replay, cancellation, native failure coverage and the runnable shared-row
example. All 600 ordinary Rust tests per platform and retained campaigns pass.
The earlier EXCEPT, COALESCE, join, UNION and resource milestones remain complete
unless a concrete counterexample reopens their affected boundary.

Keep comma separators spaced in code and SQL, preserving literal data and
intentional fixtures. Resource monitoring observed normal/warning host pressure;
verification reduced Docker CPU concurrency and preserved the qualification
limits below. Owned outputs are removed; publication remains unauthorized.

## Current: positional multiset difference and intersection

Add EXCEPT ALL and INTERSECT ALL so reconciliation preserves differences in
row multiplicity. DISTINCT operators answer whether a value exists; these forms
also distinguish repeated facts. The verified sorted-set owner provides bounded
input ordering, admission, failure handling and replay. No current counterexample
requires reopening a completed semantic or resource milestone.

1. Pin syntax, positional types/names/NULLability, equivalence classes,
   representative preservation, multiplicities and demanded errors at the
   existing immutable GoogleSQL revision. Reassess unresolved research after
   30 minutes; keep disputed forms unsupported.
2. Trace the sorted-set merge and independent validators. Match equivalent rows
   one-to-one without allocating a duplicate index or retaining a group in memory.
   Preserve all DISTINCT and UNION behavior, source spans, snapshots, resource
   admission, bounded steps, replay, cancellation, failure and cleanup.
3. Check independent multiset results for duplicate counts on either side,
   empty/nested inputs, all scalar types and NULL/DOUBLE classes, original bits,
   hidden demanded errors, malformed plans, width/stack bounds, exact/short
   admission, spill, replay and healthy reuse. Extend shared schedules only where
   they cover the same owner; retain distinct controls for changed merge paths.
4. Add a runnable duplicate-reconciliation example and update contracts/maps.
   Finish focused checks and matching frozen full macOS/GNU/Linux gates,
   reconcile discovery and manifests, keep concise evidence within budget,
   remove owned outputs and commit locally with a clean tree. Keep one goal active.

Pinned decisions: at the existing revision, the
[set rules](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/query-syntax.md#set_operators)
require `MIN(m, n)` copies for INTERSECT ALL and `MAX(m - n, 0)` for EXCEPT ALL,
combined left to right. The pinned pipe grammar retains parenthesized arguments
and optional trailing comma. The
[reference lowering](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/googlesql/reference_impl/algebrizer.cc#L6048)
groups complete keys before applying multiplicities, preserving the existing
NULL/NaN/signed-zero equivalence profile. Keep exact types, left names and left
original-bit representatives. EXCEPT keeps left NULLability; INTERSECT requires
both inputs to allow NULL. These metadata and complete-input demand choices,
including LIMIT 0, remain explicit PipeSQL contracts. Research is resolved.
The merge can consume one equivalent row from each side without new storage;
ALL must still validate monotonic input while retaining duplicate left rows.

Implementation is in progress: both ALL kinds use the existing descriptor and
sorted cursors. Independent count and typed-bit/snapshot public tests pass;
shared demanded-error and full-width/small-stack controls pass. The 418-test
library run includes all four sorted-set failure schedules and duplicate-aware
hash-fallback replay. The additional nested ALL parser test passes.
The independent ownership campaign passes all 11 analytic shapes: the two ALL
cases each emit 768 rows, retain at least 11,352/11,360 bytes of usable-allocation
headroom and release completely. Existing attribution/oracle negative controls
remain effective. Maintenance passes 96 Python tests and 44 codec fixtures.

Warning-denied workspace Clippy passes. The new example produces nullable INT64
rows NULL, 1 and 3 from fresh inputs and finishes successfully. The source diff
is reviewed; the next step freezes a coherent local source commit and runs
matching full macOS/GNU/Linux gates. Then reconcile discovery,
record concise frozen evidence, remove owned outputs and commit the checkpoint.
Current observations show normal host memory pressure with two Cargo jobs (one
for the independent ownership build); no network-dependent check is needed.

Exclude name matching, coercions/new scalar types, correlated inputs, parallelism
and persistent-format changes. Preserve comma spacing, resource monitoring,
publication restrictions and unfinished platform/physical-memory qualifications.

## Next engineering priorities

Choose the next bounded milestone by README's decision order. Resolve a concrete
semantic, durability or resource counterexample before extending the affected
owner. The existing append, reader and grouped-allocation repairs remain complete;
the physical-memory qualifications below are broader obligations.

| Area | Next action and required evidence |
| --- | --- |
| Resource ownership | Identify an uncovered owner, allocation history or transient boundary with a representative public workload. Trace any requested/usable/charged discrepancy before changing admission. Preserve independent attribution and release checks; arbitrary allocators and whole-process/RSS bounds remain unqualified. |
| Native platforms | Implement the missing Windows paths, handles, traversal, locking, synchronization, CLI startup and process ownership in bounded owner-specific changes. Separate compilation from native runtime qualification; unavailable Windows hardware must not block independent macOS/GNU/Linux work. |
| Filesystems and durability | Qualify additional supported filesystem/device premises beyond process termination. Keep the tested host-shared mount excluded and identity checks fail-closed. Reopen its unresolved sharing-layer cause only when new environment evidence can change the disposition. The [identity diagnostic](../docs/testing.md#diagnose-filesystem-identity) is independently replayable. |
| Native diagnostics | Extend from qualified mutex/pathname Rust wrappers to an affected unqualified boundary with reproducible clean/fault controls. ThreadSanitizer, instrumented standard libraries, system-library internals, other native boundaries and general race freedom remain unfinished. See the [qualification limits](evidence.md#platform-and-sanitizer-limitations). |
| Product qualification | Select a useful bounded language/workload or crash/concurrency scenario from the intended analytical workload. Preserve independent result, failure and resource oracles. Deterministic overlapping-reader schedules do not establish arbitrary-race detection. Measure complete-query costs before changing an owner for performance; define compatibility before promising stable formats or interfaces. |

The shared-mount investigation found descriptor APIs agreeing and no repository
normalization defect. Native storage passed while the tested sharing layer did
not; the causal question remains unresolved. Linux campaigns use an unprivileged
user and native database storage. Broader durability, Windows runtime and
whole-process memory require separate evidence.

Partitioned hash aggregation, a shared buffer manager and parallel execution
remain possible alternatives, not scheduled rewrites. Investigate one when a
representative workload exposes a concrete limitation, with an independent
correctness oracle, resource costs and an end-to-end measurement that can reject
the proposal. The [grouping workload record](evidence.md#grouping-learning-workload)
retains the adopted research lessons and oracle-compatibility caveat.

## Publication

Review the exact tree and destination before publishing. Do not push, publish,
modify remote refs or rewrite published history until the publication decision
is explicitly resolved. Any later history replacement requires explicit
authorization and reconciliation of the actual remote state.
