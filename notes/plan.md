# Work plan

[README](../README.md) owns product requirements; [Engineering](../docs/engineering.md)
owns working method. This plan identifies unfinished product work.
[Evidence](evidence.md) records verification and consequential limitations.

## Current baseline

Use the [reading path](../docs/README.md#learn-the-implementation),
[source map](../docs/source-map.md), [test guide](../tests/README.md), and
[tool guide](../tools/README.md) to navigate the implementation and its checks.
Maintained builds, tests, and examples require no historical checkout or archive.

The complete 23-stage gates pass on macOS and GNU arm64 Linux on the same frozen
inputs. All twelve bounded-thread scenarios and their ordinary-thread
counterparts execute on both platforms, with explicit target-specific ceilings.
Two Darwin
ACL-specific allocation cells remain excluded on Linux. Windows remains
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

## Active: complete scalar MIN and MAX aggregation

COUNT(expression) and the composed execution learning path are complete. The
[example](../examples/composed.rs) verifies join multiplicity, nullable counts
and sums, descending results, and cancellation/release under two budgets on
macOS and Linux. The [evidence](evidence.md#composed-execution-example) separates
its focused verification from the unchanged engine's full gates. Existing
competing-reader coverage was reused; no duplicate harness or engine change was
needed.

The next ordinary analytical gap is MIN/MAX. Start with at most 45 minutes
reviewing the supported scalar profile, GoogleSQL semantics, argument capture,
aggregate state, independent validation, and spill records. Establish exact
NULL, NaN, signed-zero, STRING ordering, and DATE behavior from authoritative
contracts before implementation. Record the smallest coherent representation
and cheapest falsifiers here.

The primary [MIN/MAX reference](https://docs.cloud.google.com/bigquery/docs/reference/standard-sql/aggregate_functions#min)
requires input-typed results, NULL for empty/all-NULL groups, and NaN propagation.
GoogleSQL's [type rules](https://docs.cloud.google.com/bigquery/docs/reference/standard-sql/data-types)
compare strings by Unicode code points and treat signed zeros as equal. PipeSQL
preserves stored DOUBLE bits, unlike BigQuery's documented negative-zero storage
behavior. Specify the extrema tie rule locally: retain the first NaN payload;
when both zero signs occur, MIN chooses negative zero and MAX positive zero.
Continue evaluating demanded arguments after a NaN so later scalar errors remain
observable. UTF-8 byte ordering agrees with code-point ordering for valid text.

The existing argument batch owns u64 values and validity only. General grouping
sizes records as header + keys + eight bytes per argument. STRING extrema need
captured bytes that survive source-batch release, checked variable-width spill
payloads, and reusable retained result storage. Do not implement them as source
pointers or numeric conversions. DATE retains its typed day value.

Start with separate demanded MIN/MAX slots beside the existing sum state, sharing
argument evaluation across calls. Allocate no sum cells for extrema-only input.
For retained text, evaluate preallocated bounded slots before introducing an
arena/compaction protocol: each demanded string extremum needs at most 65,536
bytes per admitted group, plus length metadata. This is simple but expensive;
include it in optional hash capacity and reassess the observed cost before
accepting the representation. Disk reduction needs only one group's extrema.
Argument capture should reserve at most 65,536 bytes per retained text argument
for a batch, and flush replay batches when their byte capacity fills. A complete
single argument row must always fit the admitted minimum.

The first falsifiers are global empty/all-NULL and NaN/zero cases, shared
COUNT/SUM/MIN/MAX programs with demanded errors, and strings that grow and shrink
on successive replacements. Then force grouped replay across full-length text
records and verify exact bytes, independent corruption rejection, admission,
cancellation, and release. No public MIN/MAX support is implemented yet.

The inspection found that argument-batch construction omitted the presence-mask
tail check used by the record reader. Construction now rejects that malformed
shape before reserving memory. A focused macOS test covers each mask tail, an
empty shape with a presence bit, and valid construction/release; it passes.

Complete global, grouped, repeated, and composed MIN/MAX for supported numeric
expressions and direct STRING/DATE columns. Account for retained variable-width
values and their release; avoid allocating per input row or adding a second
aggregation engine. Preserve COUNT/SUM/AVG, demanded errors, diagnostics,
resource refusal, cancellation, independent validation, and persistent bytes.
Do not add DISTINCT aggregates, windows, new scalar types, or unrelated syntax.

Deliver understandable state ownership, independent boundary regressions,
actual memory/spill equivalence, a concise educational example, current language
and resource contracts, required macOS/Linux verification, compact evidence,
and coherent local commits. Platform qualification remains explicit. Reassess
any approach whose state or temporary representation becomes harder to explain
than the operation it implements.

## Applying DuckDB lessons

The [grouping evidence](evidence.md#grouping-learning-workload) records the primary
references and the workload already delivered. Apply further lessons through
bounded changes to existing owners:

| Lesson | PipeSQL action and acceptance check |
| --- | --- |
| Streaming alone does not bound blocking intermediates. | Retain the completed COUNT spill/refusal regressions. Apply the same complete-result and release checks to the composed workload. |
| Operators compete for memory across a complete query. | The composed example now connects these owners. Preserve the existing competing-reader checks and measure memory and temporary bytes separately. |
| Memory limits need interpretable measurements. | When addressing physical-memory qualification, reconcile existing logical counters with allocation measurements. Add diagnostics only where a missing observation prevents a decision. |
| SQL regressions should expose queries and expected results. | Keep explicit independent expectations beside each new query. Use DuckDB for optional differential checks only after reconciling NULL, overflow, floating-point, ordering, and dialect semantics. |

For any proposed algorithm change, first identify a representative workload, correctness oracle,
resource costs, and an end-to-end measurement that could reject the proposal.
DuckDB's implementation is a source of alternatives, not evidence that a change
will improve PipeSQL. Keep adopted invariants in their existing contract owners;
retain findings here or in evidence rather than creating a research archive.

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
