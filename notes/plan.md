# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The complete 24-stage gates for `60bf34d` pass on matching frozen macOS and GNU
arm64 Linux inputs: 629 ordinary Rust tests per platform, 24 independent aggregate
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

## Active: exponential transforms and geometric means

Add one-argument EXP for INT64/DOUBLE through the existing bounded numeric
program, parser, binder, independent validators, batch scratch and demand cursor.
This makes log-scale aggregates usable in their original units: the learning
example will average LN of positive amounts, then apply EXP in a later pipe
stage to obtain their geometric mean.

The semantic and ownership trace completed within its 30-minute budget. Pinned GoogleSQL
signatures, compliance cases and the reference kernel agree on DOUBLE promotion,
NULL propagation, finite overflow errors, positive infinity and negative-infinity
zero. Preserve input NaN bits as a PipeSQL choice. Retain representable subnormal
results; results too small to remain nonzero become positive zero. Finite
exponentials use the native approximate primitive without a universal precision
or repeated/cross-platform bit-identity promise.

Implementation and focused checks pass through both batch and demand paths.
Independent Decimal answers cover normal, subnormal, zero and overflow boundaries;
public checks retain finite-overflow spans after source/plan teardown, constant
predicate failures, integer-child errors, both SAFE_DIVIDE argument positions,
skipped COALESCE/Boolean branches, empty geometric means, exact/short admission,
stored bits/reopen, cancellation and 32 replay variants. The catalog census stays
at 1,056 allocations on both pathname lengths, and eight derived I/O controls
pass; these controls alone do not constitute failure campaigns. Three nearby
SQRT/LN source-link anchors were corrected against the pinned revision without
changing their semantics.

Complete both sequential frozen platform gates, fresh examples, discovery and
semantic/allocation reconciliation, evidence, cleanup and coherent local commits.
Broader qualification and publication restrictions remain unchanged.

## Completed: transient allocation ownership in wide LEFT JOIN

`60bf34d` observes live requested/usable Rust allocations inside public execute
and step calls for the retained 64-column nullable STRING LEFT JOIN. The
[transient ownership record](evidence.md#transient-ownership-in-wide-left-join)
retains event counts, headroom, calibration, independent controls and limits.
No engine accounting defect was exposed; production owners and admission
allowances remain unchanged.

Both complete 24-stage macOS/GNU/Linux gates pass on matching frozen inputs.
Discovery, ordered allocation schedules and semantic records are reconciled.
Fresh LEFT JOIN examples pass sequentially on both platforms after both gates.
Owned outputs are removed while pre-existing artifacts and toolchains remain.
Completed language work, including [natural logarithms](evidence.md#natural-logarithms-for-analytical-scales),
remains closed. Arbitrary allocator histories, concurrent allocation observation,
whole-process/RSS, Windows and broader durability remain unfinished. Publication
restrictions are unchanged. Reassess README's decision order before activating
the next bounded milestone.

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
