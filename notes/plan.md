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
inputs. Twelve GNU arm64 stack qualifications remain excluded, but their
ordinary-thread functional counterparts execute on both platforms. Two Darwin
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

## Next: resolve cross-platform stack qualification

Linux pathname traversal now has explicit byte, symlink, native-call, and
caller-accounted scratch bounds. Native comparisons, allocation refusal, healed
outcomes, and the complete macOS/Linux gates passed. The
[evidence record](evidence.md#linux-pathname-bounds) retains the consequential
large-suffix counterexample and qualification limits.

The next bounded milestone is the twelve excluded GNU arm64 stack scenarios.
Determine a defensible cross-platform stack contract from actual native thread
minimums, engine call paths, and retained scenarios. Distinguish allocated thread
size from engine stack consumption. Spend at most 90 minutes on initial design
and falsification, then reassess from evidence before expanding instrumentation.

Preserve every functional scenario and its independent expectations. Do not
silently increase a ceiling, relabel an ordinary-thread pass as bounded-stack
qualification, or claim live-stack use from a requested/reported thread size.
If a contract needs redesign, explain the failed premise, resulting guarantee,
per-target limits, and remaining uncertainty with its authoritative owner.
Keep instrumentation proportional and separate from production behavior.

Finish applicable native controls and regressions, complete verification on
frozen inputs, reconcile discovery and exclusions, update the existing contracts
and evidence, and commit coherent work locally. Do not add unrelated features or
make unavailable Windows runtime resources a blocker for independent progress.

## Other release work

| Concern | Required outcome |
| --- | --- |
| Windows | Implement native paths, handles, traversal, locking, synchronization, CLI startup, process ownership, and target-specific verification. |
| Stack limits | Resolve or explicitly redesign the 64-KiB contract against GNU arm64's larger native minimum; do not silently skip the combined scenarios. |
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
