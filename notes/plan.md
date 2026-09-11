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

## Next: bound Linux pathname resolution

Linux canonicalization currently supplies bounded input/output buffers to
`libc::realpath`, but its internal allocation, stack, and traversal work remain
unqualified. Inspect that owner and establish explicit byte, symlink, native-call,
and scratch limits for admitted absolute paths. First test whether the existing
boundary can satisfy the resource contract with a defensible argument; replace
opaque traversal only where needed. Limit initial design and falsification to
60 minutes, then reassess the implementation approach.

Preserve Linux naming and error behavior, macOS's separate native semantics,
fallible construction, and filesystem identity checks. Use independent native
comparisons and boundary/refusal controls; do not move an independent oracle into
production or weaken limits to obtain passing checks. Keep the design with the
native owners, without a general filesystem framework. Finish the implementation
or supported resource argument, applicable full gates on frozen inputs, current
contracts/evidence, and coherent local commits. Windows runtime availability must
not block this work, and Linux success must not imply Windows qualification.

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
