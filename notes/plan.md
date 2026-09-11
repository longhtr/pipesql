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

## Next: trace composed blocking execution

COUNT(expression) is complete for the current scalar profile. Both full gates
pass all 23 stages on matching frozen inputs, with 439 Rust tests per platform,
no ignored tests, and one separate lease subprocess. The updated tutorial runs
on both platforms. [Evidence](evidence.md#nullable-count-arguments) records
semantics, failure coverage, source identity, and qualification limits.

Next make a join-to-group-to-order query understandable as a complete operation,
including competing memory owners. Spend at most 45 minutes reviewing the
existing joined aggregation, sorting, and composed-ownership checks before
choosing the smallest missing learning workload. Do not reopen accepted work
without a concrete defect or missing contract boundary.

Use a generated dataset with independently calculable join multiplicity and
nullable aggregate results. Connect one runnable example to preparation,
operator scheduling, memory admission, spill, cancellation, and release. Compare
comfortable and constrained memory with observed temporary use; a small budget
alone does not establish spill. Reuse existing fixtures and failure campaigns,
adding a regression only where current coverage does not exercise a consequential
boundary. Keep the explanation in the existing tutorial and execution owners.

Finish with complete results, visible ownership and cleanup checks, applicable
macOS/Linux verification, concise evidence, current documentation, and a local
commit. Repair any concrete prerequisite defect without weakening contracts.
Do not add new SQL features, speculative optimization, profiling infrastructure,
or another test framework. Reassess after the bounded inspection if existing
coverage already supplies the required operation and learning path.

## Applying DuckDB lessons

The [grouping evidence](evidence.md#grouping-learning-workload) records the primary
references and the workload already delivered. Apply further lessons through
bounded changes to existing owners:

| Lesson | PipeSQL action and acceptance check |
| --- | --- |
| Streaming alone does not bound blocking intermediates. | Retain the completed COUNT spill/refusal regressions. Apply the same complete-result and release checks to the composed workload. |
| Operators compete for memory across a complete query. | Review an existing join-to-group-to-order workload with overlapping owners. Add a regression only for a concrete missing boundary; measure memory and temporary bytes separately. |
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
