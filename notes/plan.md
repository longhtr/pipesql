# Work plan

[README](../README.md) owns product requirements; [Engineering](../docs/engineering.md)
owns working method. This plan identifies unfinished product work.
[Evidence](evidence.md) records verification and consequential limitations.

## Current baseline

Use the [reading path](../docs/README.md#learn-the-implementation),
[source map](../docs/source-map.md), [test guide](../tests/README.md), and
[tool guide](../tools/README.md) to navigate the implementation and its checks.
Maintained builds, tests, and examples require no historical checkout or archive.

The complete 23-stage gates for `8aceaee` pass on macOS and GNU arm64 Linux
on matching frozen inputs. Each platform executes 457 Rust tests and 285
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

## Active: EXTEND with the current expression profile

Compact hash text storage is implemented and verified. The
[comparison](evidence.md#compact-hash-text-storage) records the short-string
reservation reduction, eliminated 256-group spill at 4 MB, and increased
transient peak for wide values. Further optional storage tuning needs a new
concrete limitation; do not reopen the completed study without one.

Implement the next column operator from the [language direction](../docs/language.md#deliberate-profile):
`EXTEND` should preserve the input row and add expressions from the currently
supported projection profile. First resolve names, aliases, identity, duplicate
names, range visibility, and order semantics against the pinned GoogleSQL
sources. Retain reduced independent cases for the accepted forms. Do not infer
those rules solely from SELECT or introduce unrelated scalar functions.

The pinned [analyzer fixtures](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/googlesql/analyzer/testdata/pipe_extend.test)
confirm input-column preservation, direct-reference identity reuse, retained
range variables, duplicate-name ambiguity, and sibling-alias rejection. Separate
EXTEND stages can use earlier aliases. The pinned resolver retains input names;
its ProjectScan definition propagates input ordering for nonanalytic projections.
This is fixture and source evidence, not a newly executed upstream analyzer.
Accept direct references and current numeric expressions, with explicit AS or
bare aliases; keep aggregate/window calls and other scalar forms unsupported.

Retain EXTEND as a contextual pipe word so existing identifiers named `extend`
keep working. Add a semantic EXTEND stage that records only new projection
entries and inherits its input columns. Expanding every inherited column into
the projection pool would violate the existing syntax-sized admission bound.
Bind all new expressions against the unchanged input scope, then append outputs
without changing existing ranges. Independent validation must check the combined
width and definitions against that original input. Reuse scalar lowering and
demand propagation; no new execution controller is needed.

Use the existing frontend, validated plan, and execution owners. Preserve
original columns and demanded-error behavior through filtering, grouping,
joins, sorting, and derived inputs. Bound output width and work; maintain exact
diagnostic spans, resource admission, cancellation, and cleanup. Unsupported
forms must fail explicitly. Add a short executable learning example and update
the language manifest and reading path without duplicating their contracts.

Implementation and focused checks now cover retained identity/ranges, typed
NULLs and empty input, alias conflicts, exact-width and one-byte-short admission,
independent validator mutations, demanded overflow spans, and cancellation
through composed operators. The shared identifier regression also preserves
STRING columns named `aggregate`. The executable SQL example reuses the declared
table walkthrough. Remaining work is frozen macOS/Linux gate verification,
compact evidence, final documentation reconciliation, commits, and cleanup.

Complete focused semantic and failure checks, required frozen full verification
on macOS and available GNU/Linux, documentation checks, and coherent local
commits. Keep Windows and production qualification limits explicit. Close the
milestone once its accepted manifest and verification are complete.

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
| Later physical-memory qualification | DuckDB's memory-management article distinguishes component memory and temporary storage measurements. | Reconcile existing logical charges with allocator observations and other process owners on one representative composed workload. Explain unexplained differences before claiming a process-memory cap; add a diagnostic only when an existing observation cannot answer the question. |

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
