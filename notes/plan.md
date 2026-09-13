# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The complete 24-stage gates for `374d312` pass on matching frozen macOS and GNU
arm64 Linux inputs: 558 ordinary Rust tests per platform and 311 composition
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

## Current: numeric ABS for analytical deviations

SAFE_DIVIDE is complete at `374d312`; the
[safe division record](evidence.md#safe-division) retains verified behavior,
coverage and limitations. Add INT64/DOUBLE ABS so analytical queries can express
absolute deviations through the existing bounded numeric program. No current
counterexample reopens the completed division or resource repairs. Windows
persistence still requires a complete native mapping and runtime evidence;
this language milestone makes no new platform qualification claim.

1. Pin signatures, NULL, minimum-INT64 overflow, signed zero and nonfinite behavior
   at the language owner's immutable revision. Reassess research after 30 minutes
   if authoritative evidence cannot resolve a case; leave disputed forms unsupported.
2. Extend the existing bounded parser and scalar evaluator, preserving input
   typing, argument errors, demand, source spans and independent validation.
   Introduce no separate frontend, allocation owner or persistent representation.
3. Cover literal expected results, malformed forms/programs, nested composition,
   exact/short admission and consequential cancellation, replay and failure
   boundaries. Reuse existing campaign mechanics where they add distinct coverage.
4. Add a runnable deviation example and update contract, learning and test/tool
   maps. Run focused checks and both complete frozen gates; reconcile discovery,
   manifests and receipts, record concise evidence, remove owned outputs and
   commit locally under the publication restrictions.

The [language owner](../docs/language.md) now pins signatures, minimum-integer
fixtures, DOUBLE absolute value and NULL evaluation at the existing immutable
GoogleSQL revision. ABS uses one parser call boundary and one unary instruction,
preserving type and validity. Its integer overflow reports absolute value;
argument failures still propagate, including inside SAFE_DIVIDE.

Four focused ABS tests pass, covering scalar lanes and nonfinite inputs,
binding/type/nullability mutations and program bounds, and public values,
malformed calls, spans, demand and composition. Retained exact/short sorted
admission, forced grouping replay and cancellation/early-drop tests pass.
All-target Clippy and maintenance pass. The documented deviation example runs
against a fresh macOS sales database with the expected schema, four rows, NULLs
and successful completion. Catalog allocation controls retain the existing
census and result; the full refusal sweep is still required.

The implementation and maps are ready for frozen verification. Both complete
platform gates, GNU/Linux example execution, discovery/manifests/receipts,
final evidence, cleanup and local commits remain. Other scalar calls and NUMERIC
types remain outside this milestone. No publication is authorized.

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
