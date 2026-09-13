# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The complete 24-stage gates for `a889528` pass on matching frozen macOS and GNU
arm64 Linux inputs: 567 ordinary Rust tests per platform and 311 composition
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

## Current: INT64 quotient for analytical grouping

MOD is complete at `a889528`; the [remainder record](evidence.md#integer-remainder)
retains verified behavior, coverage and limitations. Add INT64 DIV to group
integer values by exact quotient without conversion through DOUBLE. This pairs
with remainder grouping through the existing bounded numeric program. Completed
semantic and resource repairs remain closed without a new counterexample.

1. Pin the INT64 signature, truncation direction, NULL, zero-divisor and minimum
   integer divided by minus-one overflow at the language owner's immutable
   revision. Reassess unresolved research after 30 minutes; leave disputed forms
   unsupported.
2. Extend the existing bounded call parser, binding, independent validation and
   scalar evaluator. Preserve argument errors, spans, demand, admission and
   release without another frontend, allocation owner or persistent format.
3. Cover independent literal results above 2^53, signed extremes, nullable batches,
   malformed calls/programs and composition. Extend retained admission, replay,
   cancellation and failure cases where they protect a distinct boundary.
4. Add a runnable quotient-grouping example and update contract, learning and
   test/tool maps. Run focused checks and both complete frozen platform gates;
   reconcile discovery and manifests, retain concise evidence, remove owned
   outputs and commit locally under the publication restrictions.

The pinned signature, signed fixtures, NULL evaluation and integer primitive
resolve the semantics before implementation. DIV truncates toward zero, keeps
INT64 exactness and reports minimum INT64 divided by -1 as overflow. Numeric
reference anchors are corrected against physical source lines; search-rendered
line numbers did not identify the intended source locations. Semantic research resolved within the 30-minute checkpoint.

The bounded parser, integer lane and independent validator now support DIV. Four
new focused tests pass for exact signed/extreme results, integer-only binding,
15/16-call bounds and public composition. The shared DIV/MOD NULL-lane test,
retained zero/error spans, cancellation/early drop, exact/short admission and
forced grouping replay pass. Clippy and maintenance pass. Catalog allocation
controls remain at 983 on both pathname lengths within the unchanged 1,000
ceiling; full refusal sweeps remain required. Native I/O retains a literal total
through composed DIV/MOD. The quotient-grouping example and maps are added.

The quotient example passes on fresh macOS and GNU/Linux sales databases with
exact schema, three expected groups and successful completion. The first full
gates were deliberately interrupted to incorporate comma spacing throughout
maintained code and SQL. They provide no full-gate pass. Comma separators are now spaced in maintained code, embedded SQL, generated
SQL lists and examples. Quoted data, codec/upstream fixtures and intentional
lexical cases remain intact. The spacing diff is reconciled, including two
ordinary trailing commas added by rustfmt. Maintenance and documentation pass;
fresh full gates remain required. Final discovery/manifests, evidence, cleanup
and local commits remain. DOUBLE DIV, NUMERIC types and
other new scalar calls are outside this milestone. No new platform or physical
memory qualification is claimed. No publication is authorized.

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
