# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The complete 24-stage gates for `b2236a6` pass on matching frozen macOS and GNU
arm64 Linux inputs: 577 ordinary Rust tests per platform, 648 independent join
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

## Completed: equality LEFT JOIN

Equality LEFT JOIN is verified and committed through `b2236a6`. The
[left-join record](evidence.md#equality-left-joins) retains the implementation,
independent coverage, admission and native-failure results, example outcomes and
limits. Both complete platform gates pass on matching frozen inputs. The earlier
[integer quotient and comma-spacing work](evidence.md#integer-quotient-and-comma-spacing)
remains complete. Keep comma separators spaced in code and SQL, preserving
literal data and intentional fixtures.

## Current: numeric COALESCE defaults

Add two-argument numeric COALESCE so a query can replace a missing joined amount
or a nullable aggregate with a default. Keep the existing INT64/DOUBLE profile;
variadic forms, new data types, CASE, IF and IFNULL remain outside this milestone.
Completed repairs remain closed without a concrete new counterexample.

1. Pin result selection, numeric coercion, NULLability and short-circuit error
   behavior at the language owner's immutable revision. Reassess unresolved
   research after 30 minutes and leave disputed forms unsupported.
2. Trace the bounded parser, binding, independent validators, demand analysis and
   scalar/aggregate evaluation. COALESCE must skip an unused fallback's runtime
   errors while still binding its syntax, names and types. Repair concrete
   prerequisite defects without another frontend or expression framework.
3. Implement the bounded form with independent literal results, NULL and numeric
   extremes, skipped/demanded failures, malformed programs, arity/nesting bounds,
   LEFT JOIN and aggregate composition, admission, replay, cancellation and reuse.
4. Add a runnable example and update contract and test/tool maps. Run focused
   checks and both complete matching frozen platform gates; reconcile discovery
   and manifests, retain concise evidence, remove owned outputs and commit locally.

The pinned
[conditional-expression contract](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/conditional_expressions.md#coalesce)
requires first-non-NULL selection, left-to-right evaluation and skipping later
arguments. The pinned
[numeric supertype rules](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/conversion_rules.md#supertypes)
keep two INT64 arguments INT64 and make an INT64/DOUBLE pair DOUBLE. Result
NULLability follows selection: it can be NULL only when both arguments can be
NULL. Binding still validates both arguments; an error evaluating the first
argument is not a NULL value. Existing unsupported NULL literal syntax remains
outside the numeric profile; nullable columns and SAFE_DIVIDE supply NULL values.
Research resolved these semantics within the timebox.

The parser now retains a COALESCE operation after its two ordered argument
subtrees, using the existing binary-call frames and shared operation arena.
Focused tests cover nested ordering, complete expression spans, malformed arity
and the 32-operation boundary. All 11 parser tests and the existing binding rejection selection pass in macOS
release mode; Clippy passes for all workspace targets with warnings denied.
Public binding remains fail-closed until execution is implemented; no passing
COALESCE feature verification is claimed.

The trace found two eager boundaries to repair: `scalar::Expression::evaluate_batch`
evaluates postfix operations, and `execution::computed` gathers dependencies
before evaluation. Aggregate input capture calls the same scalar kernel, while
aggregate finalization is accessed through the row-value resolver. The retained
semantic and physical demand walks conservatively admit all potential inputs;
runtime demand must skip unused fallbacks within a producer without moving
materialization boundaries. Choose the bounded branch representation together
with row and batch dependency scheduling before enabling binding. Preserve the
existing scalar operation and stack limits and account for any added scratch.
Preserve spaced comma separators, existing platform/resource limits and the
publication restrictions below.

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
