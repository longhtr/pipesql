# Work plan

[README](../README.md) owns product requirements; [Engineering](../docs/engineering.md)
owns working method. This plan identifies unfinished product work.
[Evidence](evidence.md) records verification and consequential limitations.

## Current baseline

Use the [reading path](../docs/README.md#learn-the-implementation),
[source map](../docs/source-map.md), [test guide](../tests/README.md), and
[tool guide](../tools/README.md) to navigate the implementation and its checks.
Maintained builds, tests, and examples require no historical checkout or archive.

The current core gates pass on macOS and GNU arm64 Linux. The full 23-stage
campaign baseline at `b1c8f34` remains applicable to unchanged production and
campaign sources; [evidence](evidence.md) separates the two scopes. Twelve GNU
arm64 stack qualifications remain excluded, but their ordinary-thread functional
counterparts now execute on both platforms. Two Darwin ACL-specific allocation
cells remain excluded on Linux. Windows remains unfinished. The
[platform matrix](../docs/testing.md#platform-status) distinguishes implementation,
execution, and qualification. Linux campaigns require an unprivileged user and
GNU time. A passing local gate does not establish production readiness.

## Current: resolve the shared-mount identity counterexample

The retained Linux `fuseblk` observation reported a pathname inode changing from
5566 to 5567 when opened; the DBMS refused it as changed metadata. Investigate
this concrete mismatch before extending filesystem qualification. Compare the
same maintained public catalog operation on the shared mount and native
`overlayfs`, then distinguish native pathname/descriptor API disagreement from
actual replacement, caching, or unstable filesystem identities.

Limit initial reproduction and causal investigation to 90 minutes, then choose
a disposition from the observations. Preserve a small current-source reproducer,
exact mount/toolchain context, and the violated identity invariant. Repair the
native boundary only when evidence establishes a defect, with independent
regressions and applicable verification. If the mount cannot provide the required
identity stability, retain fail-closed behavior and document that precise
qualification limit. A successful rerun does not clear the counterexample.

Use existing metadata, ABI, and graph callers where practical. Do not add a
workflow framework, historical archive, blanket mount blacklist, weaker identity
comparison, silent retries, or an unrelated feature. Finish with a verified
implementation or environment disposition, current contracts/evidence, a coherent
local commit, and owned scratch removal. Reassess the next milestone afterward.

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
