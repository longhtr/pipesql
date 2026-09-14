# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The [blocking-controller lifetime repair](evidence.md#blocking-controller-lifetimes)
in `df0cace` passes matching macOS and GNU arm64 Linux scoped verification:
14-stage core gates, 640 ordinary Rust tests per platform, public ownership,
24 independent aggregate-semantic cases, 311 composition cases and fresh examples.
Independent physical-size controls reject the prior implementation. These checks
establish the affected boundary; they do not constitute a complete 24-stage gate.
The previous [full checkpoint](evidence.md#full-verification-checkpoint) remains
attached to `1e5cfdc`. The [platform matrix](../docs/testing.md#platform-status)
distinguishes implementation, execution and qualification.

Use the [language manifest](../docs/language.md#current-public-query-manifest),
[implementation reading path](../docs/README.md#learn-the-implementation),
[test map](../tests/README.md) and [tool map](../tools/README.md) for current
capabilities and commands. Maintained checks need no historical checkout or
archive. Completed milestones remain closed unless a concrete defect, affected
boundary or new workload changes their disposition. The
[testing/tooling review](evidence.md#testing-and-tooling-cleanup) retains its
coverage inventory and consequential deletion rationale.

The [bounded power expressions](evidence.md#bounded-power-expressions) are
complete. POW and POWER share the existing binary numeric path, with explicit
exceptional values, owned failures and approximate finite results. Both full
gates, independent discovery and record reconciliation, and sequential fresh
compounding examples pass. Prior language and ownership milestones remain closed
unless a concrete affected defect or new workload changes their scope. Exhaustive
execution-allocation histories, arbitrary allocator/concurrency behavior,
whole-process/RSS, Windows and broader durability/sanitizer qualification remain
unfinished.

The [failed wide-join construction boundary](evidence.md#failed-wide-join-construction)
is complete. Existing inline charges now outlive physical controller release;
no allowance or payload allocation was added. All 353 construction allocation
prefixes and the healthy control pass at both pathname lengths on both platforms,
alongside independent negative controls, complete gates and fresh LEFT JOIN
examples. The prepared query remains independently owned while errors are live.

The combined construction-refusal and changed-width demanded-expression history
still exposes a macOS native usable-heap deficit, with requested ownership covered.
Its fresh Linux replay completes. Keep that strict diagnostic available without
a passing or general physical-memory claim. Join, ordering and sorted-set
controllers now retain their inline charges through physical release. Broader
allocator histories still need their own evidence.

## Active milestone: complete the first tutorial before optional examples

The declared-table walkthrough delays its finish and cleanup section until after
roughly 500 lines of optional query examples. Move that collection into a guide
with its own setup and cleanup, and link it from the completed first tutorial and
documentation index. Preserve query text, results, reading paths, qualifications
and current inbound links. Keep the other independent workloads in place.

Verify links and preserved commands, then run a fresh declared-table and
representative-query walkthrough. Confirm executable inputs are unchanged before
reusing their runtime evidence; document movement does not require repeating
engine or native campaigns. Review and commit locally and remove owned outputs.

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
