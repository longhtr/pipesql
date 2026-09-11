# Work plan

[README](../README.md) owns product requirements; [Engineering](../docs/engineering.md)
owns working method. This plan identifies unfinished product work.
[Evidence](evidence.md) records verification and consequential limitations.

## Current baseline

Use the [reading path](../docs/README.md#learn-the-implementation),
[source map](../docs/source-map.md), [test guide](../tests/README.md), and
[tool guide](../tools/README.md) to navigate the implementation and its checks.
Maintained builds, tests, and examples require no historical checkout or archive.

The full macOS gate and Linux core, synchronization, byte-I/O, and interruption
checks passed on the inputs identified in the evidence record. This establishes
those checks, not production readiness. Windows remains unfinished, and twelve
GNU arm64 stack scenarios remain explicitly excluded. The
[platform matrix](../docs/testing.md#platform-status) owns the coverage details.

## Next: close Linux verification gaps

Complete the remaining Linux allocation and initialization campaigns before
claiming coverage equivalent to macOS. Inspect their native premises, use the
existing callers and independent expectations, and preserve resource accounting,
commit outcomes, cleanup, and durability contracts. Implement prerequisite
portability repairs where needed; do not convert unavailable observations into
passing exclusions.

Completion requires target-native execution on identified filesystems, exercised
negative controls, exact source identities, honest test discovery, and current
platform documentation. Keep the GNU arm64 stack minimum and host-shared filesystem
identity failure visible until separate evidence resolves them. Add CI using the
same maintained commands once the required runners are available.

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
