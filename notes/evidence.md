# Verification evidence and open limitations

This record identifies checked behavior and consequential limitations. Product
and implementation contracts live in [docs](../docs/README.md); current work
lives in [the plan](plan.md). Maintained fixtures and callers provide replay inputs.
No build, test, or investigation below requires a retired project checkout.

## Full verification checkpoint

The September 11, 2026 complete gates passed all 23 stages on macOS and GNU
arm64 Linux. The macOS environment was arm64 Darwin 25.6.0, Rust 1.98.1,
Python 3.14.7, and the native Apple toolchain. The Linux environment is identified
below. Checks used release artifacts, offline locked dependencies, and
warnings-denied compilation and documentation.

The gates covered formatting, maintenance, filesystem ABI, rounding vectors,
attempt models, Clippy, Rust tests, rustdoc, doctests, aggregate semantics and
composition, public/CLI allocation, native initialization/synchronization/byte
I/O, catalog interruption, and independent graph inspection. Maintenance passed
84 tooling tests, 39 independent codec fixtures, and local documentation links.
The seed-only graph command previously passed on both platforms; reuse of its
output directory was rejected. Seed failure propagation and artifact identities
have tooling regressions independent of engine execution.

Rust suites executed 430 tests on macOS and 418 on GNU arm64. GNU arm64 explicitly
excluded twelve 64-KiB stack qualifications; macOS excluded none. Each suite also
executed one selected lease subprocess, excluded from these totals. The twelve
ordinary-thread counterparts passed on both platforms and share expectations
with their bounded-stack variants. Their functional success does not qualify
GNU arm64 stack headroom. The public directory cleanup regression executed,
including its isolated unwind control.

Both gates used one frozen 660-file export containing 659 manifested inputs.
All stage statuses were zero, before/after manifests matched across both runs,
and finalization reported no errors and removed owned build targets. The frozen manifest SHA-256 is
`80b97c89c74f0975c8b64d3d91864ecf7e5383328ecb34648421f6df88a7cfe8`.
The two notes files and the concurrency guide's description of Linux pathname
ownership were finalized afterward; runtime inputs did not change. The final
657 non-notes inputs have fingerprint
`751a521fde97e246fb2f581ce986243611c39db1c05c67db36fd04ec034ac14a`.
Recompute it from the repository root:

```sh
python3 -B tools/source-manifest.py | python3 -c 'import hashlib, sys; print(hashlib.sha256("".join(line for line in sys.stdin if not line.split("  ", 1)[1].startswith("notes/")).encode()).hexdigest())'
```

This fingerprint identifies maintained source, not reproducible binaries. Final
documentation checks cover the finalized notes. The unchanged declared-table
example was previously exercised at baseline `36b7823`: it printed `north 15`
and `south 20`; reuse of its database path returned `AlreadyExists` with exit 1.
Raw successful logs and retired source exports are not required inputs; current
callers and fixtures reconstruct the generated cases.

Run `sh tools/check.sh --output /absolute/new-result-directory` with the
[documented prerequisites](../docs/testing.md#complete-local-gate). The gate keeps
stage logs and a JSON receipt, checks before/after source manifests, and removes
its owned build target. Preserve failure context before disposing of a run.

## Linux native verification

The maintained full Linux gate exercises dynamically linked
64-bit GNU/Linux callers. The reviewed environment is arm64 Linux
7.0.12-linuxkit, glibc 2.36, Rust 1.98.1, and Python 3.11.2, with database files
on native `overlayfs` and sources mounted read-only. The compiler image starts
from `rust@sha256:9a73a5088750b4c95158ab26629c854c3d6fc4b173cb7bc8079ad252d8ed7bfa`
(arm64 manifest `09e98f39fa15751de9476fefafe4be0e4ef92b292d608410595bbbde9ebdd375`),
with Clippy, rustfmt, and Debian GNU time 1.9-0.2 provisioned before disabling
networking. The provisioned local image ID was
`sha256:520be9ff830f944e49a3319cbf6f8ccfb2c1f21631947de50290efb98038e282`.
Campaigns ran as UID/GID 1000, with writable temporary output on the
container filesystem. A root-run allocation control had correctly failed because
root could bypass read-only directory permissions; it is not passing evidence.

Set `RUSTUP_TOOLCHAIN=1.98.1-aarch64-unknown-linux-gnu` and run the
[full gate](../docs/testing.md#complete-local-gate) as an unprivileged user. The
gate sets warnings-denied Rust and documentation flags. Keep database/output
directories separate from a host-shared source mount.

The September 11 run passed all 23 stages on the same frozen inputs described
above. It executed 418 Rust tests, with 12 explicit stack exclusions and one
additional selected lease-subprocess execution. Both platforms passed 547 CLI
allocation-prefix cases, 83 parser control/deny pairs, ambiguous publication
resolving to aborted and durable outcomes, and closed/broken output sinks.
Public allocation passed its complete applicable prefix sweeps, short/long
ownership controls, timeout cleanup, and wrong-row negative control. Linux omits
the two Darwin ACL-specific recovery cells; it does exercise ordinary read-only
construction refusal as an unprivileged user.

Initialization passed 30 Darwin and 80 Linux cells. Darwin observes root stat in
its traversal, including data-mount spelling and a 33-link chain. Linux observes
root/component `lstat` and `readlink`, with short, 33-link, and forty-link
expanded-suffix paths. It rejects calls to libc `realpath`. Both check scheduled
overlap, injected refusal, correct names, and byte-preserving healed reopen.
Both platforms passed 241 native synchronization cells and 1,028 byte-I/O cells,
plus the graph/interruption cases below.
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

Both graph campaigns passed 43 cases, two oracle controls, three CLI limits,
genesis, lease contention, and independent column-order checks. Both platforms'
interruption campaigns passed 76 append cuts, 46 recovery cuts, and 249 independent
graph checks, including the wrong-history, wrong-row, and wrong-receipt controls.
Re-run with fresh outputs to obtain results for changed inputs.
Process termination retains host-visible writes; it does not model lost,
reordered, or torn writes, device power loss, kernel failure, or arbitrary
concurrent schedules. The graph inspector is an offline diagnostic, not repair
or backup software.

## Resource ownership and admission

### Linux pathname bounds

Linux canonicalization uses an explicit native traversal with caller-admitted
overflow. The [resource contract](../docs/resources.md#native-paths-stack-and-io)
owns its byte, link, work, and allocation limits. A fixed output buffer did not
bound the previous libc resolver's private growable scratch.

The maintained regression starts with a short pathname whose forty symlinks add
152,000 pending suffix bytes before resolving to a short final name. It prevents
replacing that accepted behavior with a single 4-KiB pending buffer. Each link is
read once; overflow retains the suffix rather than replaying a namespace that
may have changed. Independent native comparisons cover names and errors, joined
workers, permission refusal, and byte-ceiling error precedence. The GNU/Linux
filesystem suite passed all eighteen tests. A mode-000 directory's `/.` and
`/..` cases retain native behavior; a named child returns `EACCES`.

Public creation and reopen checks exercise logical scratch refusal before
namespace mutation, successful retry, and unchanged authoritative reopen bytes.
The focused allocation campaign passed 38 create and 30 open refusal positions,
two censuses, and two full-prefix controls. It requires typed scratch-allocation
failure, released requested/usable allocations, and successful retry. A separate
unit regression checks old/new buffer overlap admission at 32,767 and 32,768
bytes and preserved state on refusal.

The pathname-specific GNU arm64 thread reported 137,152 bytes with a 128-KiB
request and passed creation, refusal, and reopen. This is within that test's
144-KiB ceiling; it does not satisfy the twelve existing 64-KiB qualifications
or measure live stack use. No pathname performance improvement, whole-process
memory cap, or other-platform qualification follows from these checks.

### Composed query owners

The [composed ownership caller](../tools/fixtures/composed-ownership.rs) is run by
`python3 tools/check-diagnostic-allocation.py --ownership-only`.
Two reader threads hold ORDER BY and DISTINCT results over 4,096 numeric rows
while a writer stages and commits one key. An independent count array checks
complete old/new results. Synchronized checkpoints reconcile logical reservations,
requested allocations, usable allocator extents, temporary bytes, and descriptors.
Cancellation, completion, drop, allocation refusal, and temporary refusal must
preserve other live owners and release the departing owner's resources. A wrong
count for the committed key is rejected by the completed-row oracle.

At baseline `99164f2`, the held grouping case charged 3,991,744 bytes against a
4,000,000-byte budget and excluded a competing DISTINCT reader during setup.
The key arena reserved all remaining memory even when the bounded group slots
could not store that many key bytes. The maintained caller now requires both
readers to complete against independent old/new row counts and checks that the
competing reader releases its owners without disturbing the held group.

The repair bounds the arena by group-slot capacity times maximum encoded key
width. Each inserted group stores one key; this removes unusable capacity without
reducing what those slots can hold. The completed macOS gate observed charges of
2,693,132 bytes on the short path and 2,693,291 bytes on the long path; GNU/Linux
observed 2,693,101 and 2,693,275 bytes. Both readers completed on both platforms,
and the departing reader restored the held owner's allocation totals.

The repeated-aggregation cancellation test explicitly selects downstream disk
execution before any input runs. The public native-I/O fixture uses a 1.1-MB
budget that admits the query's blocking minimum and forces spill. Its census
requires both positional reads and writes: the completed runs observed 19 reads
and five writes. This preserves failure coverage without relying on an earlier
optional allocation starving the downstream controller. The census caught the
missing writes before this fixture repair; that failed run and the superseded,
interrupted macOS run are not full-gate evidence.

This is not cardinality-based slot sizing or general scheduling fairness. Wide
keys may still use the available key budget. Requested, usable, and logically
charged bytes remain separate measurements; the repair does not turn the logical
limit into a whole-process memory cap.

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
ordinary-thread counterparts exercise the functional scenarios on that target.
Their 64-KiB qualification remains unsatisfied; `--ignored` does not make the
native minimum satisfy the contract.

The September 11 shared-mount investigation reproduced the failure using the
unchanged stock catalog caller from `4931770`: 7 of 20 fresh setups failed on the
Mac-hosted `fuseblk` mount; all 20 native `overlayfs` setups passed. Failures
returned `RecoveryRequired` with "metadata file changed while opening". The GNU
arm64 caller SHA-256 was
`20cb51e778a6af8a555f20b433146e9765b5d26b601d62064a7028253d540e09`.
Execution used UID 1000, glibc 2.36, Rust 1.98.1, and Linux
7.0.12-linuxkit in the image identified above, on an arm64 Darwin 25.6.0 host.

The maintained [identity observer](../tools/fixtures/filesystem-identity.c)
reproduced 4 failures in 20 additional shared-mount setups and none in 20 native
setups. One ROOT.B witness was:

```text
before=46:282 fstat=46:286 statx=46:286 after=46:286
```

The fields are device:inode pairs. Raw libc pathname inspection disagreed with
both raw descriptor APIs; `fstat64` ran before `statx`. The immediate pathname
recheck agreed with the descriptor. This is not merely Rust metadata
normalization or a difference between those descriptor APIs. A separate trace
recorded no application rename or writable open between the disagreeing calls.
A synchronized host-side observation retained the same host inode, size, and
nanosecond modification/change timestamps across a guest mismatch. These
observations do not identify the responsible bridge or kernel behavior.

The independent observer control passed with a stable file and with intentional
replacement. Omitting the observer removed the expected replacement witness.
Simpler C publication loops did not reproduce the stock caller's mismatch; they
do not clear it. The observed failure fractions describe these runs, not a
reliability estimate. No production identity check, retry, or filesystem blacklist
changed. This tested shared mount remains unqualified; that disposition does not
exclude every FUSE filesystem or every container configuration.

[Replay instructions](../docs/testing.md#diagnose-filesystem-identity) use current
source, fresh paths, bounded attempts, the stock seed, and optional observation.
Run without observation first, retain actual failures, and compare with native
storage. Old diagnostic binaries, host scripts, and raw successful logs are not
required inputs. Further root-cause work needs evidence about the sharing layer;
repeatedly passing a simpler probe cannot establish the missing identity premise.

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
