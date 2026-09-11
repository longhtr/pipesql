# Work plan

[README](../README.md) owns product requirements; [Engineering](../docs/engineering.md)
owns working method. This plan identifies unfinished product work.
[Evidence](evidence.md) records verification and consequential limitations.

## Current baseline

Use the [reading path](../docs/README.md#learn-the-implementation),
[source map](../docs/source-map.md), [test guide](../tests/README.md), and
[tool guide](../tools/README.md) to navigate the implementation and its checks.
Maintained builds, tests, and examples require no historical checkout or archive.

The full 23-stage gates pass on macOS and GNU arm64 Linux for the inputs in the
[evidence record](evidence.md). Windows remains unfinished. Twelve GNU arm64 stack
scenarios and two Darwin ACL-specific allocation cells remain excluded on Linux;
the [platform matrix](../docs/testing.md#platform-status) distinguishes those
limits from exercised behavior. The Linux campaigns require an unprivileged user
and GNU time. No production-readiness claim follows from a passing local gate.

## Next: make grouping admission proportionate to its input

The composed ownership caller shows that optional hash grouping can reserve nearly
the remaining memory budget even for few groups, refusing a competing reader.
Trace admission through the grouping controller, hash storage, and resource
reservation owner. Establish a public regression with overlapping readers and
independent complete-row expectations before choosing a repair.

Bound optional allocation using justified input information or incremental growth
while preserving the admitted spill minimum. Do not add an arbitrary fairness
percentage, an unbounded estimate, or a separate execution path. Keep NULL/key
semantics, aggregate results, refusal, cancellation, spill/replay, and cleanup
intact. Explain the chosen representation and admission invariant beside their
owners so the implementation remains useful for learning.

Completion requires the competing-reader regression, relevant grouping and
allocation/failure tests, an updated resource contract and evidence record, and
coherent verified local commits. Assess whether the first candidate changes the
counterexample before extending the design. Whole-process memory qualification,
Windows, and general scheduling fairness remain separate work.

## Other release work

| Concern | Required outcome |
| --- | --- |
| Windows | Implement native paths, handles, traversal, locking, synchronization, CLI startup, process ownership, and target-specific verification. |
| Stack limits | Resolve or explicitly redesign the 64-KiB contract against GNU arm64's larger native minimum; do not silently skip the combined scenarios. |
| Filesystems and durability | Investigate the shared-mount identity counterexample and qualify supported filesystem/device premises beyond process termination. |
| Grouping admission | Prevent optional hash allocation from unnecessarily excluding competing readers while preserving bounded execution and complete results. |
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
