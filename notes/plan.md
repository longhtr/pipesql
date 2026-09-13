# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The complete 24-stage gates for `b1a9570` pass on matching frozen macOS and GNU
arm64 Linux inputs: 544 ordinary Rust tests per platform and 311 composition
cases, plus the applicable allocation and native campaigns. The
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

## Current: numeric division for analytical ratios

Count-only analytic storage reduction is complete at `b1a9570`; the
[checkpoint](evidence.md#analytic-count-storage-reduction) records its verified
resource improvement. The next useful expression boundary is numeric division:
current projections can add, subtract and multiply, but cannot express ratios.
Use the existing bounded scalar representation and frontend, without adding
other scalar functions or changing persistent formats.

1. Reduce type, NULL, zero-denominator, overflow and nonfinite cases from the
   language owner's pinned GoogleSQL revision. Reassess after 45 minutes if
   authoritative evidence cannot resolve a case; keep disputed forms unsupported.
2. Trace token precedence, binding, independent expression validation, demand,
   runtime evaluation and error causes. Implement division throughout the existing
   numeric expression profile with explicit result types and source spans.
3. Preserve checked integer subexpressions, unused-expression suppression, LIMIT
   and Boolean demand, admission, cancellation and cleanup. Add independent
   literal results and mutation controls, including mixed types and nested stages.
4. Add a runnable ratio example and update language, resource and learning owners.
   Extend applicable public failure callers. Run focused checks and both complete
   frozen platform gates, reconcile discovery/manifests, record concise evidence,
   remove owned outputs and commit locally under the publication restrictions.

The pinned signatures, coercion fixtures, reference NULL handling and arithmetic
implementation establish DOUBLE results, NULL propagation, zero-denominator
errors and nonfinite behavior. The lexer, parser, binder and shared numeric stack
now implement division, with a separate zero-denominator error and captured cause.
No new allocation owner or persistent representation is introduced.

Eight division tests now cover mixed types, precedence, checked integer children,
NULL, signed zero, nonfinite values, overflow, Boolean demand, SET, derived inputs,
UNION DISTINCT, grouped ratios, source spans, terminal failure, cancellation,
early drop and healthy reuse. Seven focused tests and the separate preparation
exact/short test pass. The 34-test admission selection passes, including division
in the sorted-owner exact/short execution test with zero effects on refusal.
These are filtered checks, not complete-suite evidence.

Maintenance passes, including 44 independent codec fixtures and 529 local links.
The allocation controls pass with a catalog census of 982 at both pathname lengths;
the existing ceiling of 1,000 is unchanged. Required division preparation,
execution and stepping phases retain count two. The native I/O caller retains
its prior results and adds ratio total 4.5. The macOS ratio example runs on a fresh
database and produces north/7.5 and south/20.0 with successful completion.

Implementation and test/tool maps are ready for frozen full verification. Both
complete platform gates, GNU/Linux example execution, discovery/manifests,
final evidence, cleanup and local commits remain. No publication is authorized.

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
