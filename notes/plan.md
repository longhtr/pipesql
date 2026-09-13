# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The complete 24-stage gates for `0ea0040` pass on matching frozen macOS and GNU
arm64 Linux inputs: 562 ordinary Rust tests per platform and 311 composition
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

## Current: INT64 remainder for analytical grouping

ABS is complete at `0ea0040`; the
[absolute-value record](evidence.md#absolute-value) retains verified behavior,
coverage and limitations. Add INT64 MOD for grouping integer values by remainder
through the existing bounded numeric program. The completed semantic and resource
repairs remain closed without a new counterexample. This milestone makes no new
platform or physical-memory qualification claim.

1. Pin the INT64 signature, result sign, NULL, zero-divisor and minimum-integer
   divided by minus-one behavior at the language owner's immutable revision.
   Reassess unresolved research after 30 minutes; leave disputed forms unsupported.
2. Extend the bounded call parser, binding, independent validation and scalar
   evaluator. Preserve argument errors, source spans, demand, resource admission
   and release without another frontend, allocation owner or persistent format.
3. Cover literal independent results, integer extremes, signed operands, nullable
   batches, malformed calls/programs and composition. Extend existing admission,
   replay, cancellation and failure cases where they protect a distinct boundary.
4. Add a runnable grouping example and update contract, learning and test/tool
   maps. Run focused checks and both complete frozen platform gates; reconcile
   discovery and manifests, record concise evidence, remove owned outputs and
   commit locally under the publication restrictions.

The [language owner](../docs/language.md) now pins the INT64 signature, signed
and extreme fixtures, zero/minimum-integer handling and NULL evaluation at the
existing immutable GoogleSQL revision. MOD shares bounded two-argument parser
frames with SAFE_DIVIDE and uses the existing INT64 lane. Inference distinguishes
user argument-type failures from malformed internal programs; binding retains
source spans while independent validation rejects both invalid forms.

Five focused MOD tests pass, covering signed extremes, NULL validity words and
reuse, malformed programs/calls, integer-only binding, identity mutations,
15/16-call bounds and public composition. Retained zero-divisor/argument-error
spans, terminal failure, cancellation/early drop, exact/short admission and forced
grouping replay pass. All-target Clippy and maintenance pass. Catalog allocation
controls retain count two while demanding exact oddness above 2^53; their census
is 983 at both pathname lengths, below the unchanged 1,000 ceiling. Full refusal
sweeps remain required. Native I/O retains total nine while evaluating MOD.

The documented remainder example passes on a fresh macOS sales database with
three expected groups, NULLs, schema and successful completion. Its initial
reserved alias was corrected to `n`. The implementation and maps are ready for
frozen verification. Both full gates, GNU/Linux example execution, final discovery,
manifests/receipts, evidence, cleanup and local commits remain. DOUBLE MOD, NUMERIC
types and other new scalar calls remain outside this milestone. No publication
is authorized.

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
