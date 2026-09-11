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

## Current: recover functional coverage from stack exclusions

Twelve GNU arm64 exclusions currently skip complete functional scenarios because
their native threads cannot satisfy the separate 64-KiB stack check. Separate the
scenario from its stack qualification. Execute the same behavioral assertions on
ordinary threads on both platforms, while retaining the bounded-stack tests and
their explicit target exclusions. Keep shared scenario setup local to each test
owner; do not duplicate expected results or introduce a test framework.

Verify test discovery and execution on macOS and GNU/Linux, preserve the macOS
small-stack checks, and update the platform matrix and evidence. This milestone
does not enlarge or claim to satisfy the GNU arm64 stack bound. Run focused checks
and the required core gates on frozen inputs, review the final diff, and commit
coherent verified changes locally. Retain applicable runtime evidence for unchanged
production and native campaign inputs instead of repeating unrelated campaigns.

## Other release work

| Concern | Required outcome |
| --- | --- |
| Windows | Implement native paths, handles, traversal, locking, synchronization, CLI startup, process ownership, and target-specific verification. |
| Stack limits | Resolve or explicitly redesign the 64-KiB contract against GNU arm64's larger native minimum; do not silently skip the combined scenarios. |
| Filesystems and durability | Investigate the shared-mount identity counterexample and qualify supported filesystem/device premises beyond process termination. |
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
