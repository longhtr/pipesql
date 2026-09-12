# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The complete 24-stage gates for `8a7b1ee` pass on matching frozen macOS and GNU
arm64 Linux inputs: 525 ordinary Rust tests per platform and 311 composition
cases, plus the applicable allocation and native campaigns. The
[checkpoint](evidence.md#full-verification-checkpoint) records exact inputs,
controls and limits. Linux retains two Darwin ACL exclusions. The
[platform matrix](../docs/testing.md#platform-status) distinguishes implementation,
execution and qualification; a passing gate does not establish release readiness.

The prepared-aggregate caller follow-up in `63c3c2d` passes the complete ownership
selection on both platforms, with unchanged engine inputs. Its retained-family
inventory and scoped observations are recorded in the evidence.

Use the [language manifest](../docs/language.md#current-public-query-manifest),
[implementation reading path](../docs/README.md#learn-the-implementation),
[test map](../tests/README.md) and [tool map](../tools/README.md) for current
capabilities and commands. Maintained checks need no historical checkout or
archive. Completed milestones remain closed unless a concrete defect, affected
boundary or new workload changes their disposition. The
[testing/tooling review](evidence.md#testing-and-tooling-cleanup) retains its
coverage inventory and consequential deletion rationale.

## Current: full-partition analytic row count

Implement the first bounded analytic profile from the language contract:
`COUNT(*) OVER ()` in SELECT and EXTEND. It preserves input cardinality and adds
a nonnullable INT64 count of the complete input partition. Keep other analytic
signatures, partition/order specifications and explicit frames rejected until
their own complete implementation.

1. Trace parser/binder identities, demand analysis, independent validators,
   physical planning and existing blocking storage. Choose a bounded execution
   owner after checking the empty-input, downstream LIMIT and demanded-error
   cases; do not fake the result with an aggregate that collapses input rows.
2. Implement the complete accepted profile across current producer composition,
   including mixed ordinary projections and original range visibility. Analytic
   expressions clear semantic relation order as the target contract requires.
3. Preserve admission before effects, bounded memory/work, cancellation, failure,
   scratch cleanup, snapshots and healthy reuse. Keep independent expected rows,
   semantic/physical mutations and exact/short resource controls.
4. Add a small example and current language/resource/learning documentation.
   Run focused checks, both complete macOS/GNU/Linux gates and the documented
   example with fresh outputs. Reconcile discovery, remove owned artifacts and
   commit locally before closing the milestone.

Parser, binding, semantic/physical mappings and runtime now implement the bounded
count profile. The analytic projection owns evaluation of its ordinary
expressions while their definitions retain the original binding scope. Shared
sorted input captures demanded fields with ordinals and supplies the complete
count during emission. Four focused public catalog tests and the legacy count
regression pass on macOS. The initial broad development run passed 380 library
tests; it found a grouped-cardinality defect before the later repairs, so it is
not verification of the current tree.

Concrete integration repairs remain part of this milestone:

- Grouped results must carry cardinality when no output value is demanded. Their
  zero-field internal frames retain row/header checks and do not demand unused
  aggregate arithmetic. The public aggregate/count composition now passes.
- Legacy sources require shared scratch eligibility plus explicit recovery of
  empty, single-link SCRATCH.A/B debris. Query inspection must tolerate live
  bootstrap names; writer inspection and exclusive recovery stay separate.
  Four macOS controls now pass, including every constructor effect, fourteen
  process-death cuts, reads during construction and rejected corrupt debris. Authoritative file codecs are unchanged.
- A later STRING constant can give an earlier legacy producer a UTF-8 batch.
  That producer now uses bounded cell writes; the legacy count/constant case
  passes. A direct LIMIT/constant regression retains this separate boundary.
- The public catalog allocation census grew from 859 to 926 calls. Its harness
  ceiling is now 1,000 so the complete prefix sweep remains required. This
  changes no engine memory limit. The native derived-join census with analytic
  count observes 26 reads and three writes; controls are not failure coverage.

The zero-field grouped-result regression now passes with forced hash fallback.
Warnings-denied Clippy passes for all targets, maintenance passes with 44 codec
fixtures and 520 local links, and the documented count example returns the three
expected rows on macOS. Run both complete frozen gates to cover final inputs,
including the extended grouping replay variant and the complete failure campaigns.
Verify the example on GNU/Linux, reconcile discovery, review the final diff,
record concise evidence, remove owned outputs and commit locally. No full gate
has run for this milestone, and none of its new platform behavior is qualified.

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
