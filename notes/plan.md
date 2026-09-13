# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The complete 24-stage gates for `7b03e84` pass on matching frozen macOS and GNU
arm64 Linux inputs: 618 ordinary Rust tests per platform, 24 independent aggregate
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

## Completed: infix negated membership and ranges

`NOT IN` and `NOT BETWEEN` are implemented in `7b03e84` through the existing
Boolean parser and verified by matching complete macOS/GNU/Linux gates. The
[predicate record](evidence.md#infix-negated-membership-and-ranges) retains pinned
semantics, independent typed/model outcomes, demand and error checks, validators,
admission, stage limits, legacy scans, replay and failure campaigns. All 618
ordinary Rust tests per platform and the retained campaigns pass. The fresh
tutorial returns only `south, 20` on both platforms.

The prior null-safe, scalar, set, join, testing/tooling and resource milestones
remain complete unless a concrete counterexample reopens their affected boundary.
Comma spacing is preserved. Resource monitoring reduced Docker's CPU quota after
warning pressure; pressure was normal at final verification. Example compilation
followed both platforms' Rust tests. Owned outputs are removed. Publication and
the broader qualifications below remain unresolved.

## Active: numeric sign classification

Add `SIGN(value)` for INT64 and DOUBLE expressions so queries can classify and
group negative, zero and positive measurements. The current scalar manifest does
not admit SIGN; the existing unary numeric program is the candidate owner.

1. Pin result types, NULL, signed zero, NaN, infinities and integer extremes at
   the existing GoogleSQL revision. Timebox uncertain semantic research to 30
   minutes; preserve unsupported forms if a rule remains disputed.
2. Extend the existing parser, typed scalar program and independent validators.
   Preserve demand, spans, numeric exactness and resource/stack bounds. Add
   independent scalar/public outcomes, rejected forms, composition and
   exact/short admission checks; use existing failure and replay owners.
3. Add a runnable classification example and update current contracts/navigation.
   Run focused checks and matching frozen macOS/GNU/Linux full gates, reconcile
   coverage and inputs, retain concise evidence, clean owned outputs and commit.

Monitor resources with at most two Cargo jobs. Compile examples after platform
Rust builds and reduce Docker concurrency when memory pressure warrants it.
Publication restrictions and broader qualification limits remain unchanged.

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
