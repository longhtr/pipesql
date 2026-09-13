# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The complete 24-stage gates for `2d41739` pass on matching frozen macOS and GNU
arm64 Linux inputs: 583 ordinary Rust tests per platform, 648 independent join
compositions and 311 aggregate composition cases, plus the applicable allocation
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

## Completed: numeric COALESCE defaults

Two-argument numeric COALESCE is verified and committed through `2d41739`.
The [COALESCE record](evidence.md#numeric-coalesce-defaults) retains the bounded
implementation, independent selection/error oracles, admission and failure
coverage, example outcomes and consequential repairs. Both complete platform
gates pass on matching frozen inputs. Equality LEFT JOIN and the earlier
[integer quotient and comma-spacing work](evidence.md#integer-quotient-and-comma-spacing)
remain complete. Keep comma separators spaced in code and SQL, preserving
literal data and intentional fixtures.

## Current: positional EXCEPT DISTINCT

Add bounded positional set difference for declared-table analytical queries,
allowing callers to compare complete result rows across two snapshot-pinned
branches. Pin column matching, type compatibility, duplicate/NULL equality,
representative selection and demanded-error behavior at the language owner's
immutable revision before choosing a representation. Reassess unresolved
research after 30 minutes and keep disputed forms unsupported.

Trace the existing UNION DISTINCT, row comparison, independent validators,
shared sorter and branch scheduler. Reuse admitted owners when their invariants
fit; do not add another frontend or a generic set-operation framework. Cover
independent complete-row results, duplicates, empty branches, typed NULLs,
DOUBLE edge values, nesting, demanded failures, exact/short admission, spill,
replay, allocation refusal, cancellation and cleanup. Add a runnable example and
update contracts/maps. Finish focused checks and matching complete native macOS
and GNU/Linux gates, concise evidence, cleanup and clean local commits.
Keep EXCEPT ALL, name matching, new data types, correlated inputs, parallelism
and persistent-format changes outside this milestone. Monitor relevant host and
verification resources and reduce build concurrency when measured pressure
warrants it. Publication restrictions below remain in force.

The pinned [pipe syntax](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/pipe-syntax.md#except_pipe_operator)
and [set rules](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/query-syntax.md#except)
retain a left row once exactly when no equivalent right row exists. Multiple
arguments combine from left to right; nested parentheses retain their own scope.
The [parser grammar](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/googlesql/parser/googlesql.tm)
also permits a trailing comma. The
[analyzer fixtures](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/googlesql/analyzer/testdata/pipe_set_operation.test)
protect positional width, first-input names and isolated input scopes.
PipeSQL retains its existing identical-type restriction rather than adding the
upstream common-supertype coercions in this milestone. Since each result is a
left-input value, output NULLability follows the left column; this is an inference
from set difference, not an upstream NULLability metadata promise.

The pinned [reference lowering](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/googlesql/reference_impl/algebrizer.cc)
implements EXCEPT through complete-row grouping and branch-presence counts.
The [grouping rules](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/data-types.md#grouping-with-floating-point-types)
make NULLs, NaNs and signed zeros equivalent within their respective groups.
Retain the existing unspecified order and representative choice, preserving the
selected left value's stored bits. A requested result demands comparison fields
in both branches, including fields later projected away; consume both branches
before emitting rows, even if the left branch is empty. LIMIT zero can remain
undemanded under the existing execution contract. Research resolved these choices
within the 30-minute timebox.

Implementation worklist:
1. Extend the existing child-continuation parser for EXCEPT DISTINCT, preserving
   UNION ALL/DISTINCT grouping and the shared argument/stage limits. The semantic
   and execution path now binds EXCEPT DISTINCT through the shared descriptor.
   The parser checkpoint passes all 13 focused release tests, including nested
   set modes, source spans, trailing commas and the exact stage boundary. All-target
   release Clippy also passes with warnings denied; build concurrency was two
   jobs and sampled host memory pressure remained normal.
2. Add positional semantic mapping and independent physical validation/demand.
   Trace all existing union consumers before sharing the mapping owner. Output
   values come only from the left, while comparison demands cover both inputs.
   The shared semantic, physical and preparation selection passes all seven tests,
   retaining existing UNION controls and checking complete EXCEPT input demand.
3. Implement a bounded merge difference over two existing sorted-input owners.
   Retain each side's own nullable layout when decoding/comparing rows. Deduplicate
   left rows without join cross products or another heap index; reuse checked
   runs, merge buffers and replay. Cover the complete result/failure/admission
   cases above with independent literal and reference results. Six public tests
   pass, including a 50-case standard-library set oracle, NULLs in all four scalar
   positions, NaN/signed-zero exclusion, exact INT64 values, pinned inputs and
   complete-row demanded errors. The 64-column output/source-pool scenario now
   lives with wide tests and covers UNION ALL/DISTINCT and EXCEPT; both ordinary
   and bounded-thread variants pass without changing their stack limits.
   Four owner tests pass: actual-capacity reconciliation and exact/short admission,
   replay, all 17 cancellation phases, corruption/truncation/read failure in both
   inputs, temporary refusal and healthy reuse. The fixture uses 180 left and
   176 right rows to cross the independently traced 174-record first-run boundary.
   Seven retained join-owner tests pass after sharing only their database fixture.
   Forced grouping fallback confirms EXCEPT replay without reopening sources.
   Catalog healthy controls count 1,056 allocations at both pathname lengths;
   the campaign work ceiling is 1,100 and all EXCEPT phases are required outcomes.
   Full refusal sweeps remain part of the complete gates. Public ownership checks
   pass eight analytic shapes, including 256 typed EXCEPT survivors with minimum
   observed usable-byte headroom of 11,600 bytes and complete release. Retained
   attribution and row-oracle negative controls fail as intended.
4. Finish the example, contracts/maps, focused and complete platform verification,
   manifest/discovery audit, concise evidence, cleanup and clean local commits.
   The fresh macOS missing-regions example returns NULL and 3 as documented;
   documentation verification passes 566 local links. Complete matching platform
   gates have not run for this implementation. The focused native-I/O campaign
   passes all 490 derived-query cells with EXCEPT and retained earlier queries.
   All-target release Clippy and 96 tooling tests plus 44 independent codec
   fixtures pass. Freeze this checkpoint for matching complete platform gates.

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
