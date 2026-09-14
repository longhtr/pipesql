# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The [allocation-capacity preflight](evidence.md#allocation-capacity-preflight)
in `51bd731`, with campaign integration in `7078729`, passes matching macOS
and GNU arm64 Linux scoped verification:
14-stage core gates, 640 ordinary Rust tests per platform, public ownership,
29 healthy allocation-control cells, 24 independent aggregate-semantic cases,
311 composition cases and fresh examples. A direct allocator observer rejects
the previous helper. Current production callers use equal request/ceiling values;
valid allocation and release behavior is preserved. These checks establish the
affected boundary; they do not constitute a complete 24-stage gate. The preceding
[blocking-controller repair](evidence.md#blocking-controller-lifetimes) retains
its independent physical-size controls against the prior implementation.
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

The [first-tutorial cleanup](evidence.md#first-tutorial-and-optional-queries) is
complete. Optional queries have their own setup, cleanup and maintained guide;
the first declared-table flow now finishes before those choices. Executable inputs
remain unchanged, and fresh setup, representative queries and cleanup pass.

The [native allocation reuse reduction](evidence.md#native-allocation-reuse) is
complete. A two-allocation C history reproduces the oversized Darwin extent
without the engine or Rust observer; GNU/Linux reports different extents and no
reuse in those cells. It does not identify the original query's freed block or
explain its entire aggregate deficit. The strict combined-query diagnostic and
unqualified usable-heap/RSS status remain; no admission padding was added.

The [query execution walkthrough](evidence.md#query-execution-walkthrough) is
complete in `7450500`. Preparation and execution follow the same runnable query,
including producer boundaries, borrowed batches, completion and terminal failure.
Fresh macOS/GNU arm64 Linux runs agree on total 38 and a demanded addition
overflow with its exact source span. Engine inputs remain unchanged.

The [snapshot and outcome walkthrough](evidence.md#snapshot-pins-and-retained-outcomes)
is complete in `45d1158`. The existing example now checks old/new rows, two
successful receipts and a separately aborted attempt across later publication,
reclamation and reopen. Its reading path distinguishes data pins from retained
success history. Fresh macOS/GNU arm64 Linux runs agree; engine inputs remain
unchanged.

The [computed projection workload](evidence.md#computed-projection-costs) is
complete in `5671e4c`. Repeated complete executions confirm the staged form's
five extra batch buffers and evaluation/copy overhead on this fixed input.
The study retains a runnable comparison and the current evaluation owners;
no general optimizer or performance guarantee follows from the narrow result.

## Active milestone: STRING byte-length projections

Add one-argument BYTE_LENGTH as a complete SELECT, EXTEND or SET expression
on a visible STRING column or bounded STRING literal. Return INT64 UTF-8 byte
length, preserve NULL and fold literal length during binding. Projected lengths
can feed arithmetic and aggregates in later stages. Nested calls and direct
aggregate arguments remain outside this bounded profile and must fail with
owned source spans.

Use one typed computation through existing dependencies, validators, batch/row
evaluation and admission. Preserve conditional demand, range identities,
materialization, replay, refusal, cancellation and cleanup; do not generalize the
numeric kernel or introduce allocation owners, formats or admission allowances.
Verify UTF-8/empty/maximal/NULL inputs, literal ownership, wrong types/arities,
legacy ASCII and representative composed public queries, plus independent
malformed-plan rejection. Complete affected focused and enclosing verification
on macOS and constrained GNU arm64 Linux, then fresh examples sequentially.
Finish learning paths, evidence, local commits and owned cleanup with broader
qualifications and publication restrictions preserved.

The corrupted-payload control distinguishes existing scan behavior from scalar
short-circuiting: Boolean branches can skip payload reads, while a selected
COALESCE expression loads potential raw dependencies before evaluating its
chosen branch. Retain both outcomes without promising new source-I/O laziness.
A post-sort STRING constant feeding length and then numeric arithmetic also
needs explicit coverage of the row resolver's constant dependencies.

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
