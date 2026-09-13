# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan identifies
unfinished work and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The complete 24-stage gates for `2263930` pass on matching frozen macOS and GNU
arm64 Linux inputs: 552 ordinary Rust tests per platform and 311 composition
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

## Current: SAFE_DIVIDE for nullable analytical ratios

Numeric division is complete at `2263930`; the
[division record](evidence.md#numeric-division) identifies verified semantics,
resources and failure coverage. Add SAFE_DIVIDE so a ratio can retain its row and
produce NULL when division itself encounters zero or finite overflow. Argument
errors must remain visible. Other scalar functions, integer DIV, NUMERIC types
and generic safe-error modes remain outside this milestone.

1. Pin signatures, NULL, zero, overflow, nonfinite results and argument-error
   handling at the language owner's upstream revision. Reassess after 45 minutes
   if authoritative evidence cannot resolve a case; leave disputed forms unsupported.
2. Extend the existing bounded expression parser and numeric program for two
   arguments. Track nullable DOUBLE results through validation and demand without
   adding an allocation owner. Preserve argument evaluation and source spans.
3. Cover nesting, mixed types, malformed arity/types/programs, NULL, signed zero,
   underflow, nonfinite values, composition, exact/short admission, cancellation,
   replay and cleanup with independent results and negative controls. Extend
   existing public failure callers where consequential.
4. Add a runnable ratio example with missing denominators and update contract,
   learning and test/tool maps. Run focused checks and both complete frozen gates,
   reconcile discovery/manifests/receipts, record evidence, remove owned outputs
   and commit locally under the publication restrictions.

The pinned
[signatures](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/googlesql/common/builtin_function_internal_3.cc#L2723),
[safe division fixtures](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/googlesql/compliance/functions_testlib_math.cc)
and [SafeInvokeBinary](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/googlesql/reference_impl/function.cc#L277)
establish nullable DOUBLE results and suppression of division-domain errors
after arguments have evaluated. The existing parser stack now represents the
first and second call arguments without recursion. Numeric evaluation clears a
lane's validity for zero-denominator or division-overflow errors, while retaining
argument failures. Constant evaluation now represents NULL explicitly; folded
numeric NULL predicates retain DOUBLE typing so STRING comparisons still reject.
The obsolete nonnull-only output helper had no remaining consumers and is removed.

Six focused SAFE_DIVIDE tests pass: two scalar tests, one binding/admission test,
two public catalog tests and one legacy test. They cover mixed types, nesting,
precedence, typed NULLs, signed zero, nonfinite values, underflow, validity reuse,
argument errors, malformed calls/programs, nullable identity mutation, 15/16-call
limits, empty/loaded legacy storage and composition. Twelve replay tests pass,
including retained output and forced hash fallback with nullable safe arguments.
Exact/short sorted execution and explicit cancellation/early-drop checks pass.

Maintenance and all-target Clippy pass; documentation resolves 532 local links.
The allocation controls retain census 982 at both pathname lengths and the same
1,000 ceiling. The existing division phase now demands a NULL safe result before
counting two rows. Native I/O adds a zero count over three NULL ratios. The macOS
safe-ratio example runs on a fresh database and matches its schema, four rows,
NULLs, DOUBLE bits and successful completion.

Implementation and test/tool maps are ready for frozen full verification. Both
complete platform gates, GNU/Linux example execution, discovery/manifests,
final evidence, cleanup and local commits remain. No publication is authorized.

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
