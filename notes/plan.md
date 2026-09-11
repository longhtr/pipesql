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

## Next: complete nullable COUNT arguments

The grouping learning workload is complete: the
[runnable comparison](../docs/getting-started.md#observe-grouping-with-less-memory),
[execution path](../docs/execution.md#follow-the-grouping-example), and
[retained observations](evidence.md#grouping-learning-workload) connect the SQL
and its independently checked results to real memory and disk owners. Both full
gates pass; no algorithm replacement was needed.

The current language supports COUNT(*) but rejects COUNT(expression). Complete
that ordinary analytical operation next, using the expression forms and scalar
types already supported by PipeSQL. Start with at most 45 minutes tracing the
existing aggregate representation, demand rules, NULL counts, and spill argument
codec. Confirm GoogleSQL's contract from its primary reference and record the
smallest coherent design before implementation.

Deliver counting of non-NULL arguments through global, grouped, repeated, and
composed aggregation on the existing execution path. Keep COUNT(*) unchanged.
Test empty and all-NULL input, each supported scalar type, demanded expression
errors, memory/disk equivalence, cancellation, refusal, and release with
independent expectations. Add no DISTINCT aggregates, window functions, new
scalar types, or unrelated expression syntax. Retain diagnostic spans and
persistent bytes; keep independent validators independent.

Make the implementation traceable and add a concise example explaining why
COUNT(*) and COUNT(nullable_column) differ. Complete focused checks and the
required frozen-input macOS/Linux verification, update authoritative contracts,
consolidate evidence, and commit locally. Windows qualification remains separate;
its unavailable runtime must not block this portable work.

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
