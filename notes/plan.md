# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The complete 24-stage gates for `4ae3a67` pass on matching frozen macOS and GNU
arm64 Linux inputs: 604 ordinary Rust tests per platform, 24 independent aggregate
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

## Completed: positional multiset difference and intersection

EXCEPT ALL and INTERSECT ALL are implemented in `4ae3a67` and verified by matching
complete macOS/GNU/Linux gates. The [multiset record](evidence.md#positional-except-all-and-intersect-all)
retains pinned semantics, independent counts and original-bit results, validators,
resource admission, spill/replay, failure schedules and the runnable example.
All 604 ordinary Rust tests per platform and the retained campaigns pass.
The prior set, join, scalar, testing/tooling and resource milestones remain
complete unless a concrete counterexample reopens their affected boundary.

Comma spacing is preserved in code and SQL. Resource monitoring reduced the
Docker CPU quota after warning pressure; host pressure was normal at completion.
Owned scratch outputs are removed. Publication remains unauthorized, and the
platform, durability and physical-memory qualifications below remain unfinished.

## Current: numeric NULLIF for sentinel normalization

Add two-argument numeric NULLIF to the existing scalar profile so queries can
exclude sentinel amounts from aggregates without dropping their complete rows.
COALESCE already provides numeric conditional demand; NULLIF needs its own pinned
equality and evaluation contract. The completed multiset and resource milestones
have no new counterexample and remain closed.

1. Pin arity, numeric common typing, NULLability, NULL/NaN/signed-zero equality,
   original bits, evaluation order and demanded errors at the existing immutable
   GoogleSQL revision. Reassess unresolved research after 30 minutes.
2. Trace the scalar program, binding, independent validators, demand evaluator,
   row/batch consumers and aggregate arguments. Reuse their bounded owners;
   preserve COALESCE, spans, admission, replay, failure and cleanup.
3. Add independent numeric cases, conditional-error controls, malformed-program
   checks and relevant composed/resource/failure boundaries. Keep SQL and expected
   results visible and reuse unchanged owner campaigns.
4. Add a runnable sentinel example, update contracts/maps, run focused checks and
   matching frozen macOS/GNU/Linux gates, reconcile evidence, remove owned outputs
   and commit locally. Preserve resource monitoring and comma spacing.

Exclude text/DATE/Boolean-valued NULLIF, new types, general CASE/IF, unrelated
coercions and persistent-format changes. Research precedes implementation;
publication and broader qualification restrictions remain unchanged.

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
