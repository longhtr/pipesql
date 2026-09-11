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

## Next: learn from DuckDB through one complete analytical workload

Trace a declared-table query through grouping and
ordering under both comfortable and forced-spill memory budgets. Use maintained,
locally generated inputs and an independent result oracle. Connect the runnable
case to preparation, physical operators, reservations, spill files, and cleanup
in the existing execution guide. This should help a reader explain why each
owner exists and what changes when memory runs short.

| DuckDB lesson | PipeSQL application and acceptance criteria |
| --- | --- |
| [Streaming, spilling, and shared memory](https://duckdb.org/2024/07/09/memory-management) | Compare operator admission under one database authority, including a held reader. Record logical/requested/usable bytes separately and verify complete results, refusal, and release. Identify any starvation or avoidable reservation before proposing scheduling changes. |
| [External aggregation](https://duckdb.org/2024/03/29/external-aggregation) | Challenge few/many groups, skew, and wide keys across the memory-to-spill transition. Measure spill bytes and complete-query time before considering a different grouping layout. DuckDB's unified page management and pointer relocation are design alternatives, not requirements for our safe engine. |
| [Readable SQL result tests](https://duckdb.org/docs/current/dev/sqllogictest/intro) | Keep the query and independently established expected rows visible in the walkthrough and regression. Reuse current runners; introduce no test language or framework without a concrete maintenance benefit. |

These are study inputs and planned checks, not verified improvements. Start with
one workload and a bounded investigation; finish its explanation, checks, and
any demonstrated prerequisite repair before adding another algorithm. Preserve
PipeSQL's own NULL, numeric, ordering, error, and cancellation contracts rather
than treating another database's answer as automatically authoritative.

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
