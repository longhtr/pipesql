# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The complete 24-stage gates for `2096d2f` pass on matching frozen macOS and GNU
arm64 Linux inputs: 623 ordinary Rust tests per platform, 24 independent aggregate
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

## Active: wide LEFT JOIN allocation ownership

The retained joined ownership workload reduces a narrow nullable INT64 self-join
to four aggregate columns. Wide set ownership covers another producer boundary.
Neither directly measures a wide LEFT JOIN's STRING payloads and null extension.

1. Tracing resolved before implementation within 30 minutes. Null-extension
   descriptors retain fresh nullable identities; unmatched rows reuse the join
   output batch while both sorted inputs keep separate buffers and charges.
   Select three left columns and 61 right columns, six source rows per side,
   nullable 65,536-byte STRING cells and an explicit 11-pair output oracle. The
   pairs cover 2-by-3 and 1-by-2 duplicate groups, unmatched keys and NULL keys.
   Check every output field without relying on equal-key output order.
2. Extend the existing ownership caller at short and 384-byte paths. Sample
   requested/usable bytes against charges through execute, every step and final
   release; require external storage and the existing wrong-attribution control.
   Focused macOS runs pass both paths: 11 pairs, 624 steps, 11,866,809 temporary
   bytes and minimum sampled usable headroom of 7,608 bytes. The false-attribution
   control rejects after rows and release. No deficit or admission change is
   indicated. The complete existing macOS ownership selection also passes,
   preserving narrow join, wide-set and other owner controls. Maintenance passes
   96 tooling tests, 44 independent codec fixtures and 639 local links.
3. Update affected resource/tool maps, run focused controls and matching frozen
   macOS/GNU/Linux full gates, reconcile discovery and manifests, retain concise
   evidence, remove owned outputs and commit locally. Run the gates sequentially
   after the observed overlapping-run timeout, with resource monitoring and the
   existing build limits. Broader qualifications remain unresolved.

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
