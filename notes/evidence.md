# Verification evidence and open limitations

This record identifies checked behavior and consequential limitations. Product
and implementation contracts live in [docs](../docs/README.md); current work
lives in [the plan](plan.md). Maintained fixtures and callers provide replay inputs.
No build, test, or investigation below requires a retired project checkout.

## Full verification checkpoint

The September 12, 2026 (local time) complete gates for `827cad5` passed all 23
stages on macOS and GNU arm64 Linux. The macOS environment was arm64 Darwin 25.6.0, Rust 1.98.1,
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

Rust suites executed 454 tests on each platform, with no failed or ignored
tests. Each suite also executed one selected lease subprocess, excluded from
these totals. The twelve bounded-thread scenarios and their ordinary-thread
counterparts passed on both targets. The independent native stack control and
the Rust oversized-thread negative control passed. The public directory cleanup
regression executed, including its isolated unwind control.

Both gates used one frozen 663-file export containing 662 manifested inputs.
All stage statuses were zero, before/after manifests matched across both runs,
and finalization reported no errors and removed owned build targets. The frozen
manifest SHA-256 is
`56fe5841848f6efed996eb231aec967d4ac2706baec25abb46f7e39a355b3b73`.
At that checkpoint, only the two notes files were finalized afterward. The
other 660 manifested inputs have fingerprint
`b0087231c7cef7c5ad444f12bce9579ccad484a51d315e69daf156a8de1a2e79`.
To fingerprint the currently checked-out inputs:

```sh
python3 -B tools/source-manifest.py | python3 -c 'import hashlib, sys; print(hashlib.sha256("".join(line for line in sys.stdin if not line.split("  ", 1)[1].startswith("notes/")).encode()).hexdigest())'
```

This fingerprint identifies maintained source, not reproducible binaries. Final
documentation checks cover the finalized notes. The declared-table example ran
on both platforms and printed `north total=15 rows=3 present=2` and
`south total=20 rows=1 present=1`. The frontend walkthrough also produced its
complete INT64 result, 38, on macOS. Gate stages took 1,558 seconds on macOS and
774 seconds on Linux; these are verification costs, not query benchmarks.
Raw successful logs and retired source exports are not required inputs; current
callers and fixtures reconstruct the generated cases.

Run `sh tools/check.sh --output /absolute/new-result-directory` with the
[documented prerequisites](../docs/testing.md#complete-local-gate). The gate keeps
stage logs and a JSON receipt, checks before/after source manifests, and removes
its owned build target. Preserve failure context before disposing of a run.

## Nullable COUNT arguments

COUNT(expression) uses the existing aggregate engine for global, grouped,
repeated, and composed queries. Numeric programs still evaluate demanded scalar
errors; direct STRING/DATE arguments use typed validity. Count-only states own
no sum cells. Identical demanded COUNT/SUM/AVG programs share evaluation and
state. Only the new temporary argument layout carries presence-only payloads;
persistent formats remain unchanged.

The full gates execute seven added Rust regressions. They cover all four scalar
types, empty/all-NULL input, empty strings, NaN, large integers, exact diagnostic
spans, hidden versus demanded overflow, legacy execution, join multiplicity,
and repeated/derived inputs. A 4,096-group fixture checks complete ordered counts
at 4,000,000 bytes without spill and 1,600,000 bytes with observed spill. It
checks cancellation after spill begins, workspace refusal at 1,200,000 bytes,
temporary-space refusal at one byte, repeated execution, and owner release.
These budgets describe that fixture, not universal query minima.

Internal controls reject invalid count-only value slots and checksum-valid
nonzero presence payloads or changed layout interpretation. Semantic mutations
reject wrong validity-input types, NULLability, identity, and aggregate kind.
Both CLI composition campaigns passed 285 cases, including numeric/STRING/DATE
counts over empty and nonempty input while retaining COUNT(DISTINCT ...) refusal.
The [tutorial](../docs/getting-started.md) explains the different results of
COUNT(*) and COUNT(nullable_column) using the maintained declared-table example.

## Typed MIN and MAX

The implementation through `827cad5` adds numeric-expression and direct
STRING/DATE extrema to the existing global, grouped, repeated, and composed
execution paths. The full gates above execute the regressions and allocation
campaigns described here.

The design follows Google's [MIN/MAX rules](https://docs.cloud.google.com/bigquery/docs/reference/standard-sql/aggregate_functions#min)
and [type ordering](https://docs.cloud.google.com/bigquery/docs/reference/standard-sql/data-types).
PipeSQL's signed-zero and NaN-payload choices are specified in the
[language contract](../docs/language.md#current-declared-table-queries); they are not guarantees
about other GoogleSQL implementations.

The full gates cover empty/all-NULL input, empty text, Unicode ordering,
typed DATE results, INT64 extremes, infinities, signed zeros, and first-NaN
payload retention. Joined, derived, computed, and repeated legacy inputs pass.
Global and grouped demanded-error tests place NaN in one input unit and an
overflowing multiplication in a later unit: MIN/MAX still report the exact
aggregate-call span. Hidden extrema do not introduce undemanded failures.

The full-length text regression exercises memory grouping and forced disk
fallback with 65,536-byte values, empty strings, Unicode, and all-NULL groups.
It observes opened scratch storage and reduction, checks complete independent
results, and verifies cancellation and exhausted temporary capacity without
publishing unfinished groups. Every path releases its query-owned reservations.
Capture/replay controls cover producer release, full byte arenas, shorter
replacements, and group reuse. Independent controls reject incompatible slots,
source domains, mask tails, and checksum-valid invalid UTF-8, lengths, NULL
payloads, or DATE ranges.
Two wide-row fixtures were expanded to remain above the enlarged temporary
argument-record maximum; their boundary assertions remain intact.

Admission checks compare constructed owners with their required bytes, including
one-byte-shortfall refusal. Nine numeric extrema exposed an omitted capacity
term; the repaired hash admission passes at 8,000, 32,000, 128,000, and 1,000,000
available bytes. Legacy STRING extrema retain one-byte slots and pass global,
two-key, and repeated aggregation under 2 MB. Declared STRING extrema reserve
65,536 bytes per slot per group regardless of actual length. This is an accepted
capacity cost for allocation-free replacement and explicit bounded fallback,
not a compact-string or performance claim. Persistent formats are unchanged.

The extended public allocation caller checks numeric and text extrema alongside
its existing COUNT/SUM/AVG results. Both platform gates execute exactly 721
allocation-refusal prefixes and the full healthy prefix on each short and
384-byte path. The campaign ceiling increased from 710 to 800 to admit that
control; it does not truncate the measured sweep. Refusal paths retain recovery,
same-handle retry, complete-result, and release checks.

## Composed execution example

[examples/composed.rs](../examples/composed.rs) constructs two rows per integer
key, self-joins them, counts rows and present amounts, sums nullable amounts,
and orders the 4,096 groups descending. The
[tutorial](../docs/getting-started.md#follow-a-join-through-grouping-and-sorting)
owns fresh-input commands and the independent expected results. The
[reading path](../docs/execution.md#follow-the-composed-example) follows source
occurrences, scheduling, admission, shared sorting, replay, and cleanup.

Release runs on the macOS and unprivileged GNU arm64 Linux environments below
checked all rows, successful completion, and reservation release. Each run also
cancelled a second execution after temporary storage was reserved and checked
release again. The source SHA-256 was
`16aa6d6370051ae59586f218951f4e07105754ced12ed512d0bf190837c6f01f`.

| Platform | Configured memory bytes | Maximum sampled logical memory | Maximum sampled temporary bytes |
| --- | ---: | ---: | ---: |
| macOS | 12,000,000 | 7,571,276 | 1,263,448 |
| macOS | 2,200,000 | 2,162,705 | 2,336,640 |
| GNU/Linux | 12,000,000 | 7,571,173 | 1,263,448 |
| GNU/Linux | 2,200,000 | 2,162,659 | 2,336,640 |

The join and final sort use scratch even at the larger budget. These totals do
not isolate grouping spills, count transferred bytes, bound physical memory, or
establish performance. A macOS negative control inserted
`WHERE copies.amount IS NOT NULL` after the join while preserving the expected
results; the example rejected the changed multiplicity. Its temporary source,
executable, and input were removed after the check.

These observations use the current `827cad5` frozen source. Fresh executions of
the declared-table example and both composed-example budgets pass on both
platforms after the full gates. The example checks completion, cancellation,
and release itself. Its owned databases, build targets, and container were
removed afterward. The full gates cover warnings-denied Clippy for all examples,
formatting, tooling tests, independent fixtures, and documentation links.

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

The September 12 run passed all 23 stages on the same frozen inputs described
above. It executed 454 Rust tests, with no ignored tests and one
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
filesystem suite passed all nineteen tests, including the stack observer control. A mode-000 directory's `/.` and
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
144-KiB ceiling; it does not establish a 64-KiB Linux engine-frame bound
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

The current gate's held-group checkpoints also distinguish logical database
charges from the caller's live requested and allocator-usable totals. Every row
below has zero temporary bytes and six observed descriptors. Requested/usable
totals include other allocations visible to the caller's allocator observer;
they are not an isolated grouping allocation or a whole-process memory bound.

| Platform and path | Logical charge | Requested bytes | Usable bytes |
| --- | ---: | ---: | ---: |
| macOS, short | 2,693,132 | 2,645,212 | 2,694,720 |
| macOS, long | 2,693,291 | 2,646,451 | 2,696,112 |
| GNU/Linux, short | 2,693,101 | 2,644,845 | 2,653,480 |
| GNU/Linux, long | 2,693,275 | 2,646,307 | 2,655,064 |

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

### Grouping learning workload

DuckDB's discussions of [shared memory and spilling](https://duckdb.org/2024/07/09/memory-management)
and [external aggregation](https://duckdb.org/2024/03/29/external-aggregation)
motivated this workload: vary group cardinality and skew, observe actual disk
use, and check competing owners before considering a new algorithm. Its
[SQL result tests](https://duckdb.org/docs/current/dev/sqllogictest/intro) also
reinforce keeping queries and independent expected rows visible. PipeSQL reuses
its existing test runners and safe engine; DuckDB's page management and pointer
relocation are alternatives, not required architecture. The checks below found
no prerequisite engine defect. Measure complete-query time and I/O before
proposing a performance change; preserve PipeSQL's own semantic contracts.

[The runnable example](../examples/grouping.rs) generates 8,192 declared-table
rows across 4,096 integer keys. Each key occurs with amounts 1 and 3. It verifies
every ordered key, count 2, sum 4, minimum 1, and maximum 3. It requires
successful completion and checks that dropping the result restores the
prepared-query reservation baseline.
[The walkthrough](../docs/getting-started.md#observe-grouping-with-less-memory)
contains the fresh-input commands and cleanup instructions.

Focused release runs of the extended MIN/MAX example on the macOS and GNU arm64
Linux environments above observed these logical database counters, sampled after
query steps. The example source SHA-256 was
`4fe28dba8a1837f3ee24ad622dd8b01811d8c112551c3eefbd29ccc2ca2cc86a`.

| Platform | Query memory limit | Maximum sampled memory | Maximum sampled temporary bytes |
| --- | ---: | ---: | ---: |
| macOS | 2,000,000 | 1,590,745 | 0 |
| macOS | 1,200,000 | 1,166,088 | 803,016 |
| GNU/Linux | 2,000,000 | 1,590,652 | 0 |
| GNU/Linux | 1,200,000 | 1,166,064 | 803,016 |

Both runs returned all 4,096 expected groups on each platform. Grouping supplies
the requested order itself, so a separate ORDER BY operator cannot account for
the temporary bytes. Scratch reserves extents before writes; these counters
are not filesystem block usage, cumulative I/O, allocator-usable memory, or RSS.
No timing comparison or algorithm improvement is claimed.

The maintained public regression
`grouping::ordered_grouping_preserves_few_many_and_skewed_groups_across_memory_budgets`
passes eight combinations on each platform: 32 or 4,096 groups, uniform or skewed
input, and both budgets. It independently derives counts and sums from the two
input passes, verifies every ordered row and completion, asserts disk use only
for the high-cardinality low-budget cases, and checks release. Internal grouping
tests retain wide-text and cancellation-at-each-phase coverage; the composed
ownership caller above checks competing readers and allocator observations.

Two isolated copies of the example challenge its result checker against the
stock library. Replacing `SUM(amount)` with `SUM(amount+1)` rejects a wrong sum.
Appending `|> LIMIT 4095` to the query rejects a missing tail after successful
query completion. Both callers exit unsuccessfully without printing `verified`.
These source substitutions reconstruct the controls; no modified library,
historical fixture, or retained temporary caller is required.

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
At `b6737e3`, twelve GNU arm64 scenarios were excluded because a 48-KiB request
produced a 137,152-byte reported thread, exceeding their 64-KiB ceiling. The
independent native control now confirms that GNU arm64 rejects both 48-KiB and
64-KiB pthread requests with `EINVAL`; its native minimum is 131,072 bytes.
The revised [stack contract](../docs/resources.md#native-paths-stack-and-io)
keeps the 48-KiB Rust request, retains macOS's 64-KiB ceiling, and explicitly
qualifies GNU arm64 against a 144-KiB reported extent. This allows at most
16 KiB above the native minimum for runtime overhead. It is a changed native
thread envelope, not evidence that Linux engine frames fit within 64 KiB.

Focused execution passed all twelve scenarios on both platforms, with unchanged
functional expectations and their ordinary-thread counterparts retained. Native
reports were 61,440 bytes on macOS and 137,152 on GNU arm64. The independent C
control and the Rust scenario helper both rejected an oversized 2-MiB request;
macOS reported 2,109,440 bytes and Linux reported 2,097,152. The C observer also
checked that its local variable lay inside the returned native stack interval.
Observation failures fail the tests. These controls do not measure peak frames,
guard residency, or a whole-process cap. The full gates for this change passed; the checkpoint above identifies their
verified inputs.

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
