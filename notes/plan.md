# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The complete 24-stage gates for `8c28a1f` pass on matching frozen macOS and GNU
arm64 Linux inputs: 571 ordinary Rust tests per platform and 311 composition
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

## Current: equality LEFT JOIN for analytical facts

INT64 DIV and comma spacing are complete at `8c28a1f` and `6d2b7d5`; the
[quotient record](evidence.md#integer-quotient-and-comma-spacing) retains their
verification and limitations. Add equality LEFT JOIN so a fact remains visible
when its dimension key has no match, using the existing bounded shared-sorter
join. Completed repairs remain closed without a concrete new counterexample.

1. Pin pipe syntax, duplicate multiplicity, NULL keys, right-output NULLability
   and post-join filtering at the language owner's immutable revision. Reassess
   unresolved research after 30 minutes; leave disputed forms unsupported.
2. Trace parsing, binding, independent validation, demand, physical planning,
   join transitions and admission. Repair any concrete prerequisite defect before
   extending its owner. Preserve inner joins and bounded failure behavior.
3. Implement unmatched-left output through the existing join. Cover empty inputs,
   duplicates, NULL/unmatched keys, supported types, derived/repeated producers,
   downstream composition and malformed plans with independent expected results.
   Verify exact/short admission, spill, replay, cancellation, allocation refusal,
   interruption and cleanup without another join framework or persistent format.
4. Add a runnable fact/dimension example and update contract, learning and test/tool
   maps. Run focused checks and both complete frozen platform gates; reconcile
   discovery and manifests, retain concise evidence, remove owned outputs and
   commit locally under the publication restrictions.

The semantic profile is pinned at GoogleSQL revision
`0e7d7073ed0360be587a5efa0fa78abeee00f17b`:
[pipe JOIN](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/pipe-syntax.md#join_pipe_operator),
[LEFT JOIN](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/query-syntax.md#left_join),
[ON](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/query-syntax.md#on_clause) and
[pipe WHERE](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/pipe-syntax.md#where_pipe_operator).
Accept `LEFT JOIN` and `LEFT OUTER JOIN` with the existing equality-key profile.
Each matching pair contributes a row, including duplicate cross products. An
unmatched left row contributes one row whose right columns are NULL; a NULL ON
condition does not match. ON outputs retain left columns followed by right
columns. A following WHERE filters that result, so rejecting all matched rows
must not manufacture an unmatched row. Right output facts must permit NULL
without changing the independent right producer's input facts.

Parsing now carries the join kind through direct and nested input continuations;
binding still explicitly refuses LEFT JOIN until nullable output identities and
execution are implemented. All 56 frontend tests pass on macOS, including the
new parser case (348 unrelated library tests filtered). Parser storage remains
4,452 bytes. The next implementation must remap both visible right outputs and
range-only right columns: `available_columns` includes both, while
`relation_columns` describes only the visible row. `column_type` supplies
canonical facts globally, so changing source NULLability would corrupt the input
contract. Add a bounded descriptor of fresh nullable right identities, translate
backward demand to its original inputs, and translate physical positions in both
lowering and the independent validator. Semantic validation must check descriptor
capacity, canonical input facts, consecutive identities and the remapped range
scope. The independent aggregate-demand walk also needs this translation.
Retain the current right-group bookmark and replay schedule for matching rows;
add bounded unmatched-left emission for exhausted right input and lesser or
nonmatching NULL/NaN keys. Full implementation and verification remain.
RIGHT/FULL joins, USING, compound
or non-equality ON predicates, correlated inputs and parallelism remain outside
this milestone. Keep comma separators spaced in code and SQL, preserving literal
data and intentional fixtures. No new platform or physical-memory qualification
is claimed. No publication is authorized.

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
