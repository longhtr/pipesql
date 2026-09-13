# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The complete 24-stage gates for `849f38e` pass on matching frozen macOS and GNU
arm64 Linux inputs: 626 ordinary Rust tests per platform, 24 independent aggregate
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

## Active: natural logarithms for analytical scales

Add one-argument LN through the existing numeric parser, binder, validators,
batch scratch and demand cursor. This supports log transforms of positive
measurements without a new expression representation, allocation owner or format.
Semantic research and owner tracing have a 30-minute budget; reassess if the
pinned cases cannot establish the exceptional-value or precision contract.

The cheapest falsifiers are finite zero/negative domain cases, negative infinity,
NULL payloads, and independently rounded high-precision logarithms near one and
at binary64 extremes. The pinned compliance cases and kernel specify NaN for
negative infinity, resolving the reference prose's broader nonpositive-error
wording. Preserve input NaN bits explicitly; finite results use the native
logarithm with no cross-platform bit-identity or correct-rounding promise.

Focused numerical/domain, public composition/span, exact/short admission,
stored-value/reopen, cancellation, measured small-stack and 31-variant
forced-replay checks pass. Aggregate domain errors retain their enclosing span.
Catalog controls retain 1,056 allocations; all eight derived native-I/O census
controls pass. These focused controls do not replace the full campaigns. One
initial incomplete exact replay selector ran zero tests; the corrected selector
executes and passes. Full gates and fresh examples remain pending.

Complete public composition and demanded-error spans, exact/short admission,
cancellation, forced replay, allocation/native campaigns and a runnable sales
log-transform tutorial. Run complete sequential frozen macOS and GNU arm64 Linux
gates, then sequential fresh examples. Reconcile manifests and discovery, record
scoped evidence, clean owned outputs and commit locally. Broader qualifications
and publication restrictions remain unchanged.

## Completed: numeric rounding for analytical buckets

`2096d2f` adds FLOOR, CEIL and CEILING through the existing unary scalar path.
The [rounding record](evidence.md#numeric-rounding-for-analytical-buckets) retains
semantic provenance, independent boundary coverage and the runnable learning path.
Matching complete 24-stage macOS/GNU/Linux gates pass, with 623 ordinary Rust
tests per platform and the retained semantic, allocation and native campaigns.
Fresh tutorial outputs agree after both Rust stages complete. No allocation owner,
format or admission allowance changed. Owned outputs are removed.

The initial overlapping macOS Rust stage timed out; the standalone two-job retry
passes with unchanged inputs and deadlines. Resource monitoring and the checkpoint
retain that distinction. Prior language, testing/tooling and resource milestones
remain complete unless a concrete counterexample or new workload changes their
scope. Publication and broader qualification restrictions remain unresolved.

## Completed: wide LEFT JOIN allocation ownership

`667983f` adds a 64-column nullable STRING LEFT JOIN to the existing ownership
caller. Both pathname lengths pass on macOS and GNU/Linux with all 11 expected
pairs, step-boundary attribution and complete release. The false-attribution
control rejects. No deficit, production change or allowance increase is indicated.
The [ownership record](evidence.md#wide-left-join-allocation-ownership) retains the
trace, independent oracle, observations and limits.

Matching frozen 24-stage full gates pass sequentially, preserving the narrow join,
wide-set and other semantic, allocation and native controls. Input manifests and
discovery agree; owned outputs are removed. Broader qualifications remain unresolved.

## Completed: nearest-integer rounding for analytical buckets

`7c56cf8` implements one-argument ROUND through the existing scalar owners.
The [rounding record](evidence.md#nearest-integer-rounding) retains pinned semantics,
independent boundary oracles, composition/failure coverage and the learning path.
Both complete matching 24-stage macOS/GNU/Linux gates pass, with 624 ordinary
Rust tests per platform and all retained allocation/native campaigns. Fresh
nearest-rounding tutorial outputs agree after both full gates finish.

No expression framework, allocation owner, format or admission allowance changed.
Discovery and manifests are reconciled; owned outputs are removed. Decimal-position
and rounding-mode arguments remain unsupported. Publication and broader platform,
durability, sanitizer and physical-memory qualifications remain unresolved.

## Completed: square roots for analytical magnitudes

`849f38e` implements one-argument SQRT through the existing scalar owners with
DOUBLE promotion, NULL demand and inline typed domain errors. The
[square-root record](evidence.md#square-roots-for-analytical-magnitudes) retains
pinned semantics, independent boundary answers, preparation/runtime error spans,
stored-value, admission, cancellation, replay and allocation/native coverage.

Matching frozen 24-stage macOS/GNU/Linux gates pass sequentially, with 626 ordinary
Rust tests per platform. Fresh RMS example outputs agree after both complete
gates. Discovery, manifests and retained controls are reconciled; owned outputs
are removed. No allocation owner, persistent format or admission allowance
changed. Publication and broader qualification restrictions remain unresolved.

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
