# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The complete 24-stage gates for `5146e72` pass on matching frozen macOS and GNU
arm64 Linux inputs: 595 ordinary Rust tests per platform, 24 independent aggregate
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

## Completed: positional EXCEPT DISTINCT

Bounded positional EXCEPT DISTINCT is verified and committed through `5146e72`.
The [EXCEPT record](evidence.md#positional-except-distinct) retains the pinned
profile, complete-row oracles, independent validators, admission, spill/replay,
cancellation, native failure coverage and runnable comparison example. Both
complete platform gates pass on matching frozen inputs. Earlier COALESCE, joins,
set operators and resource repairs remain complete unless a concrete defect
reopens their affected boundary. Keep comma separators spaced in code and SQL,
preserving literal data and intentional fixtures.

## Current: positional INTERSECT DISTINCT

Add bounded complete-row intersection for declared-table analytical queries.
Together with EXCEPT, this lets callers distinguish shared and missing rows when
reconciling inputs. The verified sorted-input and positional-mapping owners make
this a concrete extension without another frontend or a general set framework.
No known semantic or resource counterexample from the EXCEPT gates requires a
prerequisite repair; reopen an affected owner if new evidence finds one.

1. Pin syntax, positional type compatibility, names, NULLability, equality,
   representatives, argument association and demanded-error behavior at the
   existing immutable GoogleSQL revision. Reassess unresolved research after
   30 minutes and keep disputed forms unsupported.
2. Trace the parser, semantic/physical validators, complete-row demand, sorted
   inputs and scheduler before selecting the representation. Share mechanics
   only where their ownership and failure contracts coincide. Preserve UNION
   and EXCEPT behavior, original value bits, pinned snapshots and resource bounds.
3. Verify independent complete-row results and negative controls for duplicates,
   empty inputs, all scalar types and typed NULLs, DOUBLE edges, nesting, source
   spans, demanded errors, admission, spill/replay, allocation refusal,
   cancellation and healthy reuse. Extend shared campaigns when they cover the
   same boundary; add distinct failure schedules only where the new path needs
   them. Keep SQL and expected results locally understandable.
4. Add a runnable shared-row example and update contracts/maps. Finish focused
   checks and matching complete macOS/GNU/Linux gates, reconcile discovery and
   inputs, retain concise evidence within budget, remove owned scratch and
   commit coherent changes locally. Keep exactly one active goal.

Pinned decisions (research completed): the existing GoogleSQL revision's
[pipe syntax](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/pipe-syntax.md#intersect_pipe_operator)
and [set rules](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/query-syntax.md#set_operators)
define positional equal widths, left names, one shared complete row and
left-to-right argument combination. Retain exact types without coercion.
[Grouping equality](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/data-types.md#groupable-data-types)
provides NULL, NaN and signed-zero classes. PipeSQL infers nullable output only
when both inputs are nullable, preserves a left representative's original bits
without promising which representative, and retains EXCEPT's complete-input
demand and LIMIT 0 behavior. These metadata and evaluation choices are local
contracts, not upstream guarantees. The parser's existing bounded continuation,
independent semantic and physical demand checks, and two-sort owner carry the
operation; only merge selection differs. No new buffers or failure authority
are needed.

Implementation and focused verification are complete. Five public checks cover
50 independent set comparisons, all current scalar types and typed NULLs,
original DOUBLE representative bits, pinned inputs, names/NULLability, nested
composition and shared EXCEPT/INTERSECT demanded errors. Nine parser/binding/
validator checks pass, as do both full-width/small-stack scenarios, forced
aggregate replay and all four owner schedules on both operation kinds. The
ownership campaign passes at both pathname lengths: 256 INTERSECT rows,
11,600 bytes minimum usable headroom and complete release, with retained negative
controls. Clippy is clean; 573 local links resolve. The fresh shared-regions
example returns required INT64 values 1 and 2 on macOS.

The EXCEPT owner and tests moved to `sorted_set`; shared demanded-error checks
moved from the public EXCEPT module to INTERSECT and now run both operations.
No protected schedule was deleted. The same constructors and effect owners keep
the allocation-prefix/native-I/O campaigns applicable; independent ownership
checks explicitly execute the new kind. Finish matching frozen full gates and
the GNU example, then reconcile discovery/manifests, record concise evidence,
remove owned outputs and commit the verified checkpoint.

Exclude INTERSECT ALL, name matching, new data types/coercions, correlated inputs,
parallelism and persistent-format changes. Preserve comma spacing. Monitor
relevant resource use and limit concurrent builds when observations warrant it.
Publication restrictions and platform/physical-memory qualifications remain.

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
