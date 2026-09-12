# Work plan

[README](../README.md) owns product requirements; [Engineering](../docs/engineering.md)
owns working method. This plan identifies unfinished product work.
[Evidence](evidence.md) records verification and consequential limitations.

## Current baseline

Use the [reading path](../docs/README.md#learn-the-implementation),
[source map](../docs/source-map.md), [test guide](../tests/README.md), and
[tool guide](../tools/README.md) to navigate the implementation and its checks.
Maintained builds, tests, and examples require no historical checkout or archive.

The complete 23-stage gates for `5a3cb0a` pass on macOS and GNU arm64 Linux
on matching frozen inputs. Each platform executes 480 Rust tests and 298
composition cases, plus its applicable native and allocation campaigns. All twelve bounded-thread scenarios and their ordinary-thread
counterparts execute on both platforms, with explicit target-specific ceilings.
Two Darwin ACL-specific allocation cells remain excluded on Linux. Windows remains
unfinished. The [platform matrix](../docs/testing.md#platform-status) distinguishes
implementation, execution, and qualification; [evidence](evidence.md) records the
checks. Linux campaigns require an unprivileged user and GNU time. A passing
local gate does not establish production readiness.

The shared-mount identity investigation is settled as an environment
qualification limit. The stock caller and raw native observations reproduce the
mismatch on the tested host-shared mount; native storage passes. Descriptor APIs
agree with each other, and no repository normalization defect was established.
Keep fail-closed behavior. The sharing-layer cause remains unresolved; current
[replay instructions](../docs/testing.md#diagnose-filesystem-identity) need no old
checkout or diagnostic binary. Reopen causal investigation when new sharing-layer
evidence can change the disposition.

## Completed: column transformations

SELECT, EXTEND, SET, DROP, RENAME, and AS form the current bounded
column-transformation profile. The [language manifest](../docs/language.md#current-public-query-manifest)
owns its syntax and semantics; [evidence](evidence.md#column-transformation-semantics)
records independent cases, retained qualified inputs, typed-copy identity,
resource/failure coverage, and the verified cross-platform tutorial. Both complete
gates pass on frozen source `5a3cb0a`. The 64-column public bound remains intact;
internal payloads admit up to 128 visible and retained values. Larger inline
owners retain their existing resource charges. Compact hash storage and EXTEND
remain complete. Reopen these only for a concrete defect or limitation.

## Completed: composed memory attribution

The maintained ownership caller reconciles prepared plans, two parked readers,
and an append against requested and allocator-usable bytes on macOS and GNU/Linux.
The [resource equations](../docs/resources.md#interpret-composed-memory-observations)
explain inline handles, allocation allowances, and unused pathname capacity.
Independent wrong-row and wrong-attribution controls fail at their intended
checks. Physical allocation refusal, logical admission refusal, temporary refusal,
cancellation, completion, and final release preserve the other owners.

Caller synchronization storage is separately destroyed and observed. macOS
allocator rounding can exceed an engine owner's logical charge, so physical
memory remains a release obligation. The [evidence](evidence.md#attribution-of-composed-memory)
records the measured scope; it does not establish a whole-process cap.

## Completed: native mutex sanitizer baseline

The maintained [sanitizer command](../docs/testing.md#qualify-native-sanitizer-observations)
passes on macOS and GNU/Linux. Clean/fault controls establish AddressSanitizer
heap-bounds detection and its expected failure exit. The pinned compiler,
uninstrumented nightly, and instrumented nightly each execute all four mutex
tests. Receipts record unchanged inputs, compiler/runtime/artifact identities,
linked native libraries, and cleanup. Verifier negative controls reject empty
selections, wrong failures, and ambiguous artifacts.

This qualifies the exercised Rust wrapper paths. Prebuilt standard libraries
and native pthread implementations remain uninstrumented; ThreadSanitizer,
other native boundaries, Windows, and general race freedom remain unfinished.
The [evidence](evidence.md#platform-and-sanitizer-limitations) records the limits.
Do not repeat this investigation without a concrete new report or affected change.

## Active: positional UNION ALL

Finish and qualify the implemented positional union profile from baseline
`fc94fa1`. The [language contract](../docs/language.md#union-all) owns accepted
syntax, correspondence, names, types, scope, ordering, and demand. The
[resource contract](../docs/resources.md#union-all-admission) owns admission and
replay. The [tutorial](../docs/getting-started.md#combine-pipeline-results) follows
the implementation through a working sales query.

The parser uses bounded child continuations and normalizes arguments to binary
nodes. Admitted semantic descriptors assign identities to positions and retain
both input mappings. Independent semantic and physical validators check these
mappings and graph edges. The runtime streams branches through one output batch
and uses the existing scheduler; replay resets visited branches on demand.
There is no full-result union spool or second query pipeline.

The pinned GoogleSQL parser and analyzer fixtures are
[`pipe_set_operation.test`](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/googlesql/parser/testdata/pipe_set_operation.test)
and the corresponding
[analyzer fixture](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/googlesql/analyzer/testdata/pipe_set_operation.test).
Their SHA-256 values are `444ef8e948a1f1c9515c29760d63073bd9c9e045b311252bd4c5a605b1401751`
and `4ba1033c2e770b5e7df93fb35cb476fa1b41a94ba36292ba84a9d624073dea3f`.
The same revision's `resolver_query.cc` establishes argument scope, positional
width checks, fresh outputs, first-input names, and removal of input ranges.
GoogleSQL supports common-supertype coercion; PipeSQL's identical-type restriction
is local. This is source/fixture inspection, not an upstream analyzer execution.

Development verification completed:

- A macOS release-profile checkpoint passed 489 Rust tests and the selected lease
  subprocess before subsequent tests were added. The broad debug run aborted on
  a small-stack test; it is not passing evidence. Release stack limits are unchanged.
- Focused tests cover positional duplicates, fresh identities, NULLability, scope,
  nested branches, transforms, joins, grouping, DISTINCT, ordering, LIMIT, demanded
  overflow spans, all scalar types, snapshots, cancellation, and partial/full replay.
  Independent mutations reject invalid semantic and physical mappings and edges.
- Source-pool and 64-column output boundaries pass on ordinary and small-stack
  threads. Exact prepared/execution admission rejects one byte short and releases
  reservations; execution refusal precedes source I/O, including for LIMIT 0.
- Forced sorting/grouping spill and temporary-space refusal preserve results and
  release owners. The allocation campaign includes a typed union/sort/count query
  and retries: both path lengths pass every prefix of the 790-allocation census,
  including required union preparation, execution, and stepping refusals.
- The tutorial produces its documented two groups, completes successfully, and
  removes its temporary database. Maintenance passes 90 tooling tests, 39 codec
  fixtures, and 466 local links. Release Clippy and formatting pass.

These checks establish development progress, not a completed gate on final inputs.
The existing GNU arm64 image provides the pinned compiler, Python, GNU time, and
unprivileged execution. Database files must use native container storage, not the
host-shared mount with the documented identity qualification limit.

Next actions:

1. Freeze the reviewed implementation commit for complete macOS and GNU/Linux
   gates. Reconcile discovered tests, exclusions, and before/after input manifests.
   Preserve failures and repair prerequisites before accepting either gate.
2. Verify the tutorial on Linux and complete required independent/public/native
   campaigns. The legacy independent composition corpus remains separate;
   union's independent expectations live in public Rust contract tests.
3. Consolidate final provenance and limitations in existing evidence, replace this
   development checklist with the remaining product work, and commit locally.
   Remove owned outputs and finish with a clean tree. Retained evidence must stay
   within 641,696 bytes. Do not push or modify remote refs.

Do not add DISTINCT set operations, name-based correspondence, unrelated
functions, speculative optimization, or another workflow framework.

## Applying DuckDB lessons

DuckDB's published designs inform the following work. These are PipeSQL design
choices and acceptance checks, not claims that DuckDB's results transfer here.
The [MIN/MAX evidence](evidence.md#typed-min-and-max) and
[grouping workload](evidence.md#grouping-learning-workload) record the delivered
implementation, examples, checks, and accepted costs.

| Priority | Lesson and source | PipeSQL action and completion check |
| --- | --- | --- |
| Applied in MIN/MAX | [External aggregation](https://duckdb.org/2024/03/29/external-aggregation) makes variable-width ownership and relocation part of the spill design. | Capture owned STRING bytes before releasing source batches. Independently validate replay lengths, UTF-8, NULLs, and type identity. Exercise maximum-length values, growing/shrinking replacements, and replay after producer release. Keep checked serialized records; pointer relocation would add machinery without an established need. |
| Applied in compact hash storage | The same external-aggregation study varies group cardinality and measures operation beyond available memory. | Existing workloads check cardinality, budget pressure, and full-width text replay; exact admission checks cover the fallback minimum. The completed STRING study establishes unused capacity and short-string spill costs. Compact spans and admitted arena growth remove short-string spill in the maintained 4 MB workload. Retain the recorded transient growth cost for wide values; no speedup is claimed. |
| Applied in MIN/MAX | [Memory management](https://duckdb.org/2024/07/09/memory-management) treats blocking intermediates and their competing owners explicitly. | Extend existing composed-query checks to extrema. Exercise memory refusal, exhausted temporary capacity, cancellation, and release. Attribute spill to the relevant operator or a controlled query; total temporary bytes alone do not prove aggregation spilled. |
| Current tests | DuckDB's [SQL test guidance](https://duckdb.org/docs/lts/dev/sqllogictest/writing_tests) favors exercising behavior through SQL. | Keep queries and independent expected results visible in existing public tests. Retain internal corruption and allocation controls where SQL cannot establish the invariant. No additional test framework is needed. |
| Applied in ownership attribution; physical cap remains open | DuckDB's memory-management article distinguishes component memory and temporary storage measurements. | The maintained caller attributes prepared/query/append charges and caller synchronization on macOS and GNU/Linux. Allocator rounding can exceed an owner's charge; this does not establish a usable-heap or process-memory cap. |

DuckDB is not a universal differential oracle. Its documented
[floating-point ordering](https://duckdb.org/docs/current/sql/data_types/numeric#floating-point-types)
places NaN above other numbers. Our GoogleSQL-derived MIN/MAX contract propagates
NaN in both directions. Establish compatible NULL, overflow, floating-point,
collation, and ordering semantics before comparing results. Keep explicit local
expectations for incompatible cases; never change them to match DuckDB.

The qualification gaps below remain separate obligations. Partitioned hash
aggregation, a shared buffer manager, and parallel execution remain possible
alternatives, not scheduled rewrites. Investigate one only when a representative
workload exposes a concrete limitation in the current owner. Require an independent correctness oracle,
resource costs, and an end-to-end measurement that could reject the proposal.
Keep adopted invariants in their existing contracts and replace settled plan
entries instead of accumulating a research archive.

## Remaining qualification

| Concern | Required outcome |
| --- | --- |
| Windows | Implement native paths, handles, traversal, locking, synchronization, CLI startup, process ownership, and target-specific verification. |
| Filesystems and durability | Retain the tested shared-mount exclusion; qualify supported filesystem/device premises beyond process termination. |
| Physical memory | Reconcile logical charges with allocator-usable memory and other owners without claiming a whole-process cap from engine counters. |
| Native diagnostics | Establish reproducible sanitizer controls and identify instrumentation limits before attributing reports or claiming a clean boundary. |
| Product qualification | Extend representative language/workload, crash/concurrency, and performance evidence; define compatibility before promising stable formats or interfaces. |

Choose work by README's decision order. For each milestone, identify the affected
contract, a concrete falsifier, and completion criteria. Update this plan when a
finding changes the next action; replace settled entries instead of accumulating
an implementation diary. Keep evidence self-contained and within its retention
budget.

## Publication

Review the exact tree and destination before publishing. Ordinary development
must preserve published history. Any later history replacement requires explicit
authorization and reconciliation of the actual remote state.
