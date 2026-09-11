# Verification evidence and open limitations

This record identifies checked behavior and consequential limitations. Product
and implementation contracts live in [docs](../docs/README.md); current work
lives in [the plan](plan.md). Maintained fixtures and callers provide replay inputs.
No build, test, or investigation below requires a retired project checkout.

## Latest completed gate checkpoint

On September 11, 2026, the full macOS gate passed all 23 stages on arm64 Darwin
25.6.0 with Rust 1.98.1, Python 3.14.7, and the native Apple toolchain. Checks used
release artifacts, offline locked dependencies, and warnings-denied compilation
and documentation. The run covered formatting, maintenance, filesystem ABI,
rounding vectors, attempt models, Clippy, Rust tests, rustdoc, doc tests, aggregate
semantics/composition, public/CLI allocation, native initialization/synchronization/
byte I/O, catalog interruption, and independent graph inspection.

The Rust suites executed 417 tests with no failures or exclusions. The lease
subprocess additionally executed one selected test; it is not counted twice.
The public directory cleanup regression executed, including its isolated unwind
control. Maintenance checked 81 tooling tests, 39 independent codec fixtures,
and documentation links. The declared-table example printed `north 15` and
`south 20`; reuse of its database path returned `AlreadyExists` with exit 1.

Both platform runs used the same frozen 656-file export. Their before/after
manifests matched, all stage exit statuses were zero, and finalization reported
no errors. Only the two notes files were finalized after runtime verification.
The remaining 654 inputs match the final source. Their manifest fingerprint is
`00ad2c8d519c3be28401e0e39990a4c6f64f1400881299de2b8975cfeeb17633`.
Recompute it from the repository root:

```sh
python3 -B tools/source-manifest.py | python3 -c 'import hashlib, sys; print(hashlib.sha256("".join(line for line in sys.stdin if not line.split("  ", 1)[1].startswith("notes/")).encode()).hexdigest())'
```

This fingerprint identifies maintained source, not reproducible binaries. The
final documentation check covers the finalized notes. Raw successful logs and
retired source exports are not required inputs; the current gate reconstructs
its generated cases from maintained callers and fixtures.

Run `sh tools/check.sh --output /absolute/new-result-directory` with the
[documented prerequisites](../docs/testing.md#prerequisites). The gate preserves
stage logs and a JSON receipt, checks before/after source manifests, and removes
its owned build target. Preserve failure context before disposing of a run.

## Linux native verification

The maintained Linux core and native campaigns exercise dynamically linked
64-bit GNU/Linux callers. The reviewed environment is arm64 Linux
7.0.12-linuxkit, glibc 2.36, Rust 1.98.1, and Python 3.11.2, with database files
on native `overlayfs` and sources mounted read-only. The compiler image starts
from `rust@sha256:9a73a5088750b4c95158ab26629c854c3d6fc4b173cb7bc8079ad252d8ed7bfa`
(arm64 manifest `09e98f39fa15751de9476fefafe4be0e4ef92b292d608410595bbbde9ebdd375`),
with Clippy and rustfmt provisioned before disabling networking.

Set `RUSTUP_TOOLCHAIN=1.98.1-aarch64-unknown-linux-gnu`,
`RUSTFLAGS='-D warnings'`, and `RUSTDOCFLAGS='-D warnings'`. Run the core gate,
then synchronization, byte-I/O, and catalog interruption sequentially using the
[documented commands](../docs/testing.md#complete-local-gate).
Keep database/output directories separate from a host-shared source mount.
The September 11 run passed all 14 core stages and the three native campaigns
on the same frozen inputs described above. It executed 397 Rust tests, with 12
explicit stack exclusions and one additional selected lease-subprocess execution.
Both native platforms passed 241 synchronization cells, 1,088 byte-I/O cells,
and the interruption cases described below.
These checks do not qualify other libc implementations, static linking, all
filesystems, native concurrency, or power-loss durability.

## Persisted graphs and interruption

Maintained owners: [graph inspector](../tools/catalog_graph.py),
[graph campaign](../tools/check-catalog-graph.py), and
[interruption campaign](../tools/check-catalog-interruption.py).
Their [contract](../docs/verification.md#independent-catalog-inspection) owns
budgets, output interpretation, and exclusions.

The independent graph checker reconstructs namespace-format-7 rows and success
history without production decoders or fixture encoders. Its stock seed has two
tables, one empty, 18 committed rows, issued prefix 6, and successes `[1, 2, 4, 5]`.
Inputs cover full-width integers, raw NaN payloads, signed zero/infinities,
UTF-8/NUL text, duplicates, NULLs, bitmap boundaries, and DATE endpoints.
A separate encoded case reverses physical column order and uses IDs 29 and 3;
expected rows retain declared schema order.

The campaign checks budget refusal, checksum-valid structural corruption,
payload/type violations, receipt history, root/fence conflicts, corrupt newer
graphs, unreferenced extents, aliases, symlinks, oversized files, and lease
contention. Wrong-row and wrong-receipt controls challenge the observers.

The interruption campaign terminates the stock process before/after observed
write, positional write, native synchronization, rename, and unlink calls.
Each cut must reach the expected trace prefix. Candidate attempt 4 is NotFound
before issuance replacement, Aborted after issuance but before data replacement,
and Durable after data replacement; corresponding generations are 2, 2, and 3.
Later appends must preserve old rows and receipts without reusing an aborted
identity. Wrong-generation, row, and receipt controls must fail.

The macOS graph campaign passed 43 cases, two oracle controls, three CLI limits,
genesis, lease contention, and independent column-order checks. Both platforms'
interruption campaigns passed 76 append cuts, 46 recovery cuts, and 249 independent
graph checks, including the wrong-history, wrong-row, and wrong-receipt controls.
Re-run with fresh outputs to obtain results for changed inputs.
Process termination retains host-visible writes; it does not model lost,
reordered, or torn writes, device power loss, kernel failure, or arbitrary
concurrent schedules. The graph inspector is an offline diagnostic, not repair
or backup software.

## Resource ownership and admission

The [composed ownership caller](../tools/fixtures/composed-ownership.rs) is run by
`python3 tools/check-diagnostic-allocation.py --ownership-only`.
Two reader threads hold ORDER BY and DISTINCT results over 4,096 numeric rows
while a writer stages and commits one key. An independent count array checks
complete old/new results. Synchronized checkpoints reconcile logical reservations,
requested allocations, usable allocator extents, temporary bytes, and descriptors.
Cancellation, completion, drop, allocation refusal, and temporary refusal must
preserve other live owners and release the departing owner's resources. A wrong
count for the committed key is rejected by the completed-row oracle.

The held hash-grouping control exposes a consequential limitation: a 4 MB logical
budget can admit allocations whose allocator-usable extents exceed 4 MB. Optional
hash admission can also consume nearly the remaining budget even for few groups,
causing a competing query to refuse while the held result remains valid. This is
not a whole-process memory cap or a resolved admission policy. The caller reports
requested, usable, and charged bytes separately so the current values can be
measured without an old workload or source archive.

Thread stacks, runtime state, allocator metadata, caller barriers, and foreign
owners remain separate. Returning engine allocations to baseline does not require
RSS to return to baseline. Serialized transitions with overlapping owners do not
qualify every interleaving or allocation-failure position. [Resources](../docs/resources.md)
owns current equations and the outstanding physical-memory obligation.

## Native boundaries and diagnostics

The maintained [allocation and native callers](../tools/README.md#native-and-allocation-callers)
distinguish definite abort, cleanup debt, and ambiguous publication, then check
healed reopen. Inline error facts and fixed-buffer rendering avoid retained
diagnostic heap owners in the exercised cases. Error layout is not a public ABI;
caller-owned strings and custom error formatting remain separate owners.

The full macOS run also passed 30 native initialization cells and 547 CLI
allocation-prefix cases. Public allocation exercised diagnostics, composed
ownership, and short/long lifecycle, load, query, catalog, and recovery refusals.
The aggregate semantics campaign passed 24 cases; composition checked complete
results against its maintained independent expectations.

Native wrappers make bounded attempts instead of hiding interruption retry loops.
Exact byte I/O requires positive progress or terminal failure. Campaigns cross
short transfers and returned errors with healed public outcomes. They do not
bound kernel-call latency or native runtime memory. Small-stack regressions check
native-reported stack size; requested size, mapped memory, resident pages, and
live frames are different measurements.

## Platform and sanitizer limitations

The [platform matrix](../docs/testing.md#platform-status) distinguishes implemented,
exercised, excluded, and unfinished behavior. Windows remains unimplemented.
GNU arm64 reports a native stack larger than the 64-KiB ceiling even for a 48-KiB
request: the reviewed pthread minimum is 131,072 bytes, and the observed stack
was 137,152 bytes. Four public catalog, two legacy integration, and six internal
library tests retain their 65,536-byte ceiling and explicit exclusions. Their
combined scenarios remain unqualified on that target; `--ignored` does not make
the native minimum satisfy the contract.

A Docker host-shared mount reported as `fuseblk` exposed inconsistent pathname
and opened-file identities during catalog creation. The stock caller returned
`RecoveryRequired` with "metadata file changed while opening". One captured
ROOT.B observation changed from pathname inode 5566 to opened inode 5567 on device
47; an immediate pathname recheck also reported 5567. The cause remains unresolved.
Successful runs on native `overlayfs` and simpler replacement probes do not clear
that failure, and no database invariant was relaxed.

Investigate with the maintained [catalog graph caller](../tools/fixtures/catalog-graph.rs)
and campaign on fresh directories on the affected mount. Record the OS, kernel,
libc, database filesystem, and pathname/opened identities; compare with a native
filesystem using the same stock executable. The failure was intermittent, so a
single successful setup is insufficient. Old diagnostic builds and their failure
frequencies are not retained qualification claims.

Sanitizer qualification is incomplete. Earlier diagnostic compiler/standard-library
combinations disagreed about reports and error kinds in safe-std/native controls;
the responsible difference was not isolated, and libSystem was not instrumented.
Those retired diagnostic results establish neither a current engine defect nor
race freedom. A new investigation must provision and identify its toolchain,
standard library, sanitizer runtime, native dependencies, and a reproducible
control before attributing a report or claiming a clean boundary.

## Query semantics and accepted costs

Current semantics and witnesses belong to [Language](../docs/language.md),
[tests](../tests/README.md), and the maintained independent semantic campaigns.
The physical validator retains a combined malformed-graph regression: a forward
producer edge into a later row with an invalid column count must return corruption
without indexing unvalidated state. The producer and validator remain independent.
Shared sorter checks do not qualify every joining, grouping, DISTINCT, or ordering
transition; use the public composition and failure cases for those boundaries.

Old benchmark comparisons against retired implementations and unavailable external
workloads are withdrawn. No performance improvement or current throughput claim
is made for this tree. The engineering costs that still need measurement are
external grouping I/O, separately evaluated computed expressions, sorting for
DISTINCT, narrow join batches, and wide-text join admission. Correct bounded
execution does not establish competitive performance or equivalence between
algebraically equivalent queries. New measurements must identify current source,
reconstructible data, configured resources, complete results, and host context.

## Retired implementations and gate isolation

Formats 1/2/3/5 remain explicit rejection fixtures with maintained independent
encoders. They are test inputs, not retained implementations or accepted product
interfaces. Prototype syntax and operations do not become supported by deleting
historical code; the language manifest owns accepted behavior.

The complete gate remains sequential. A child process can outlive its leader;
healthy parallel execution alone does not demonstrate cancellation or cleanup.
The maintained process-group tests exercise live descendants, timeout, and
interruption. Any future parallel supervisor must preserve owned descendants,
bounded outputs, failure propagation, and cleanup before it can replace this
simpler execution order.
