# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The complete 24-stage gates for `99c8755` pass on matching frozen macOS and GNU
arm64 Linux inputs: 621 ordinary Rust tests per platform, 24 independent aggregate
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

## Completed: wide positional set allocation ownership

`99c8755` adds six cases to the existing ownership caller, combining compact
STRING source demand with repeated logical positions and separately stored wide
columns. Both pathname lengths pass on macOS and GNU/Linux, including complete
rows, external storage, requested/usable charges, negative controls and final
release. No allocation deficit was found; production admission remains unchanged.
The [ownership record](evidence.md#wide-positional-set-allocation-ownership)
retains the workload, exact observations and qualification limits.

Matching complete 24-stage gates pass all 621 ordinary Rust tests per platform
and the retained semantic, allocation and native campaigns. Finalization moves
one existing comment back to its owner without changing executable code. Owned
outputs are removed. Prior language, testing/tooling and resource milestones
remain complete unless a concrete counterexample reopens their affected boundary.
Comma spacing is preserved. Verification used at most two Cargo jobs and one
Docker CPU/build job; resource monitoring observed normal/warning pressure.
Publication and the broader qualifications below remain unresolved.

## Active: numeric rounding for analytical buckets

FLOOR and CEIL let queries round measurements into analytical buckets using the
existing scalar program. Confirm CEILING as an alias before accepting it.

1. Pin numeric argument/result typing, INT64 conversion, NULL, signed zero,
   NaN/infinity and boundaries at the existing immutable GoogleSQL reference.
   Timebox semantic research to 30 minutes; keep uncertain forms unsupported.
2. Implement the confirmed profile through existing unary parser, validation and
   row/batch execution. Retain demand, spans, bounds and admission; add independent
   scalar/public outcomes and relevant shared cancellation, replay and failure
   coverage without new runtime owners or a generic expression framework.
3. Add one runnable rounding example and affected navigation/contracts. Verify
   focused behavior and matching frozen macOS/GNU/Linux full gates, reconcile
   discovery and inputs, retain concise evidence, clean owned outputs and commit.

Monitor resources with at most two Cargo jobs and one Docker CPU/build job.
Compile examples after both platform Rust test stages. Publication and broader
qualification restrictions remain unchanged.

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
