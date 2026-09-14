# Verification evidence and open limitations

This record identifies checked behavior and consequential limitations. Product
and implementation contracts live in [docs](../docs/README.md); current work
lives in [the plan](plan.md). Maintained fixtures and callers provide replay inputs.
No build, test, or investigation below requires a retired project checkout.

## Mixed report append interruption and recovery

`a002bb1` extends the existing [interruption campaign](../tools/check-catalog-interruption.py)
with a [report caller](../tools/fixtures/catalog-report-interruption.rs). The native
observer and production engine are unchanged. The same cut loop now checks the
original facts history and the composed report separately, with one stock library
and caller build. Literal grouped answers remain independent of input construction;
the Python graph oracle separately checks both tables, column identities, types,
NULLability, complete raw DATE offsets and DOUBLE bits, including signed zero.

Report setup commits two declarations, the dimension rows and eight events, then
aborts attempt 5. The observed append issues attempt 6 and adds eight events in
two writes. Its uninterrupted control retains and reruns the old eight-event plan
before checking the new report. Terminated processes retain no live reader pins.
A healthy continuation appends event 8 again, producing nine or seventeen raw
events depending on whether the interrupted append published. Literal grouped
answers and receipt history are checked before retry, after retry and after a
second reopen, with memory/temp release at every report boundary.

The first targeted probe terminated at event 77, immediately before data
publication at event 78. The independent graph retained eight events, generation
4 and issued attempt 6; public recovery resolved that attempt as aborted. Healthy
retry published attempt 7 at generation 5 and produced the exact nine-event graph.
The final campaign covers every observed before/after cut:

| History | Append cuts | Recovery cuts | Independent graph checks | Issuance/data publication after events |
| --- | ---: | ---: | ---: | --- |
| Original facts | 76 | 46 | 249 | 10 / 66 |
| Composed report | 88 | 46 | 273 | 10 / 78 |

Recovery cuts start from both unpublished and mixed-root published states.
Each terminated recovery is followed by healthy reopen, exact history checks,
a successful append and another reopen. Both platforms reconcile 528 unique case
records: 256 intentional process terminations, six expected oracle failures and
266 successful processes, including 522 independent graph inspections. Each
workload's wrong-generation, wrong-row and wrong-receipt controls fail at the
intended assertion. The report graph unit test also rejects twelve field/value
mutations, three wrong state/retry interpretations and an altered healthy-retry
amount. Missing completion output cannot count as a successful caller.

The selected graphs, receipts, outcomes and all 528 native traces agree across
macOS and GNU arm64 Linux. Sixteen intermediate unpublished-recovery cuts
(16–23 in each workload) retain different obsolete filenames, with matching
counts and reachable graphs. [Recovery construction](../src/catalog_snapshot/construction.rs)
collects uncommitted IDs in directory order before unlinking them; it promises no
cross-platform deletion order. Every healed inventory agrees. No comparison
removed a reachable-object, row, identity or receipt difference.

Frozen sequential platform verification passes caller formatting, optimized
warnings-denied Rust/native caller builds, 102 tooling tests, 44 independent codec
fixtures, both interruption histories and the independent graph campaign. The
latter retains 43 cases, two oracle controls, three CLI limit checks, genesis,
lease contention and independent column order. The scoped sequences took
119.344 seconds on macOS and 69.298 seconds on Linux. This tooling/documentation
change does not replace the preceding complete ordinary Rust or native gate
records; no production code, format, allowance or failure contract changed.

After both platform sequences, eight fresh lessons ran on macOS then Linux:
even/skewed reports at both budgets, empty/small reports, the original report and
calendar lesson. Every process succeeded with empty stderr and byte-for-byte
stdout agreement, including complete rows and DOUBLE bits. Lesson execution took
21.256/12.173 seconds, excluding fresh builds; these are verification timings.

The frozen 743-input manifest SHA-256 is
`b245ea949fc0b152dad4831ddc1c7f90406473b3dff33d5508d4c27f42f9c3e1`.
Only the two notes files changed afterward; the other 741 inputs retain fingerprint
`3e19efed22b8a9ba593ebda11a1dd00fdec6cb7e6a41da1f01b469739101979d`.

| Artifact | macOS SHA-256 | GNU arm64 Linux SHA-256 |
| --- | --- | --- |
| Scoped verification receipt | `f70d177d171ebd6583832670d217c53fa37f175351e739cd6ddb67bba38b45e2` | `d6c5f46ed44537cd2039866718ad28e4c427e850d975f7667a1b0a0b81cf2c45` |
| Interruption receipt | `f36f817970b83398edc721ba0cd0ef53428516d09a43b2428e123c1e2b1deef7` | `022a96f7464d4d15f889af90733a72c9345aa6618176bcc445a4f715859ddd58` |
| Fresh lesson receipt | `6cbc601794ab561d6c5e6ca1f364ed695179dfa652bd6f8e01ad0032c50e5f0d` | `42d5d5d8316bccc372a1b5cf904e670f0bc14e76f1144c342be5c177ac500b30` |

Host monitoring sampled CPU/RSS, memory pressure/swap, disk capacity/I/O and
network. Pressure was normal/warning, swap ranged from 1,321.75 to 3,095.25 MiB,
and free disk stayed above 187.46 GiB. Sampled disk throughput reached 174.32 MB/s;
compiler/owned-process samples reached 100.3% CPU and 452,064 KiB RSS. Container
samples reached 898.0 MiB with no OOM kill and zero network traffic. One Mac Cargo
job and one Linux CPU/job were used. Linux retained uid/gid 1000, 2 GiB without
extra swap, no network and native database storage. These are resource observations,
not physical-memory qualification.

This remains process-termination evidence with host-visible writes preserved.
It does not simulate discarded writes, torn storage, kernel/device failure or
power loss. General concurrency, Windows, broader durability and the separate
macOS combined-query usable-allocation deficit remain unfinished.

Final documentation verification passed 940 local links. Owned probe/cut fixtures,
logs, manifests, source exports, targets, lesson databases, monitoring outputs and
the verification container were removed. No owned verification process remained;
the original workspace target and preserved Linux image are unchanged.

## Overlapping report lifetimes

`3e7156a` extends the existing [snapshot tests](../tests/catalog_lifecycle/snapshots.rs)
with the retained event report and independent literal answers for eight,
sixteen and twenty-four events. Two real reader threads stop after each adds
its own spill storage, before rows are emitted. An admitted writer retains the
third append. At 64 MB, a third report refuses shared memory admission with typed
`native query workspace` fields; partial construction releases all its charges
without changing the parked owners. Reclamation while the writer is held returns
contention without advancing the generation.

Four schedules vary publication before/after the first reader finishes and which
reader finishes first. The older reader cancels; the newer reader completes.
Both reopen their retained plans with fresh tokens. Exact schemas, complete
nullable groups, commit receipts and generation increments agree. Each terminal
transition reconciles memory against the other live owners; cancellation and
completion release each parked reader's temporary storage separately. Reclamation
can proceed between reader releases, and all reservations return to the resident
baseline afterward. Fresh preparation and close/reopen produce the twenty-four
event answer and retain all three durable receipts.

A fifth scenario challenges pin protection. It identifies the old fixture catalog
through a directory difference and the literal format tag, independently of the
engine's reclamation traversal. After publication and reclamation, it deliberately
unlinks that pinned catalog. The older reader's reopened cursor must return
`Io(NotFound)`; restoring the object must let the same plan succeed. This control
rejects the false positive in which an already open descriptor survives an
erroneous unlink. Every wait has a thirty-second deadline; report progress loops
are bounded. These are enumerated schedules, not arbitrary-race or scheduler
coverage. The [lesson](../docs/event-report.md#follow-overlapping-readers) explains
those lifetime and observation boundaries.

Initial probes found that budgets of 8–32 MB admitted one parked report but could
refuse a second. The 64 MB schedule admits both plus the writer and still refuses
a third report. The second reader's spill checkpoint is measured after including
the writer's temporary reservation, so writer storage cannot satisfy that check.
No production engine code, API, format, allowance or validator changed.

Sequential optimized macOS/GNU arm64 Linux verification passed the full catalog
suite and both report examples: 175 catalog tests and four example tests on each
platform, independently discovered across three executable artifacts. No test
failed, was ignored or was filtered. Both new tests execute, covering the four
healthy schedules and the unlink control. All-target warnings-denied Clippy,
formatting, rustdoc, 101 tooling tests and 44 independent codec fixtures pass.
The scoped runs took 185.620 seconds on macOS and 88.442 seconds on Linux.
This test/documentation-only change does not replace the preceding full
native/persistence gate record or broaden its qualifications.

After both platform suites, eight fresh lessons ran sequentially on macOS then
Linux: even/skewed reports at both budgets, empty/small reports, the original
report and calendar lesson. Every process succeeded with empty stderr and exact
complete stdout agreement. Rows, DOUBLE bits and sampled temporary quantities
match the retained report checkpoints. Lesson execution took 20.971/12.504 seconds,
excluding fresh builds; these are verification timings, not performance claims.

The frozen 742-input manifest SHA-256 is
`8fc5b674156535586329adb4601ecc5ef078b43e5ee69db60f0cc329adbb96d2`.
Only the two notes files changed afterward; the other 740 inputs retain fingerprint
`ee27ca18ed8ceca31b2ef83aad9219577ab445569d7c9ba69650b556557290d6`.

| Artifact | macOS SHA-256 | GNU arm64 Linux SHA-256 |
| --- | --- | --- |
| Scoped verification receipt | `5acb2d184fd24d6e147fb579fb597688928ce571cbade293a7189627341093c2` | `47da54bec0e0db6a6c5d2d5a7996def131419c0f9c8af3e65cafe54025cba2cd` |
| Fresh lesson receipt | `fd08f7d15f2c40a8609b62c4a37201aa4b8a38083554870a030b220573ee2e10` | `9a872f85b3d1dfbece90857c47ff74c9340ddc148efc87ac4e4d83b3b870d3cd` |
| Fresh scaled-report binary | `abb5f0e9fa3289ffb4a6e35b171c93e84c9fdc8c9e9e7bf9b2ac2345110e9348` | `69911b611314d93a40633807f042add22e554803a832595877e6a939a1ad33f6` |

Monitoring sampled CPU/RSS, memory pressure/swap, disk capacity/I/O and network.
Host pressure was normal/warning; swap ranged from 1,697.25 to 3,490.44 MiB and
available disk stayed above 185.21 GiB. Sampled disk throughput reached 220.81 MB/s;
compiler/owned-process samples reached 96.8% CPU and 428,208 KiB RSS. Container
samples reached 883.5 MiB with no OOM kill and zero network traffic. One Mac Cargo
job and one Linux CPU/job were used; Linux retained uid/gid 1000, 2 GiB without
extra swap, no network and native database storage. These observations do not
qualify physical/RSS bounds. General concurrency, sanitizer coverage, Windows,
broader durability and the combined-query macOS usable deficit remain unfinished.

Final documentation verification passed 932 local links. Owned probes, databases,
logs, source exports, manifests, build targets, monitoring outputs and the
verification container were removed. No owned verification process remained;
the original workspace target and preserved Linux image are unchanged.

## Composed report allocation histories

`c78e814` repairs `General::assemble` and extends the existing
[ownership caller](../tools/fixtures/composed-ownership.rs), observer and campaign.
The retained sixteen-event report produces thirteen literal groups. Three complete
histories include cold preparation, partial-result drop, spill cancellation,
retained-plan reuse and healthy continuation after preparation/construction
refusal. Both short and 384-byte paths run through the stock library on macOS
and GNU arm64 Linux. Caller row storage stays outside the armed observer.

The first constructor sweep exposed a real requested-ownership defect at prefix
97, refusing the `hash group owner` allocation. All 97 successful allocations
were freed and endpoint counters balanced, but transient requested headroom was
-25,186 bytes and usable headroom was -36,085 bytes. `General::assemble` had
unpacked `Minimum` before fallible optional construction; reverse local drop
order released the reservation while fallback buffers remained live. The repair
keeps that owner intact until all fallible construction finishes. It changes no
allowance, allocation framework, validator, public API or persistent format.
The [lesson](../docs/event-report.md#follow-a-failed-allocation) follows this
ownership boundary beside the implementation.

Both platforms now pass preparation prefixes 0–12 with healthy control 13 and
construction prefixes 0–106 with healthy control 107, at both pathname lengths.
Errors remain live while heap counters, reservations, descriptors and formatting
under allocation denial are checked. Successful allocation and pre-free samples
reconcile separately from terminal counters. Minimum constructor headroom is
4,096 requested bytes on both platforms, and 2,488/4,096 usable bytes on
macOS/Linux. Complete and interrupted executions balance allocation/free events;
prepared-owner drop and final descriptors, memory and temporary release agree.

Four added controls reject wrong resident attribution and missing preparation,
construction or cancelled-terminal observation. The terminal control initially
exposed a harness hole: checking for some frees accepted an incomplete trace.
Requiring balanced execution allocation/free counts closes it. The independent
campaign parser requires complete prefix sequences, phase order, exact row counts,
headroom and release; its unit test rejects eleven damaged records. All twenty
ownership controls fail at their intended oracle on both platforms.

The complete sequential 24-stage gates pass on identical frozen inputs:

- 685 ordinary Rust tests per platform, independently discovered across 21
  targets; none failed, ignored or filtered. macOS/Linux library counts are
  449/451 and filesystem counts 21/19. Both include 173 catalog tests. The separate
  lease subprocess passes one test with six intentionally filtered siblings.
- 101 tooling tests, 44 independently reproduced codec fixtures, 24 independent
  semantic cases and 326 composition records. Cross-platform records agree after
  removing only database-path stdout lines and composition digest fields.
- Catalog allocation prefixes 0–1055 and healthy control 1056 at both pathname
  lengths; initialization 30 macOS/80 Linux cells, synchronization 241 cells and
  native I/O 1,394 cells per platform. Linux retains two Darwin ACL exclusions.
- 76 append cuts, 46 recovery cuts, 249 independent graph checks, 43 graph cases,
  two oracle controls and retained CLI, genesis, lease and column-order checks.

Both receipts confirm unchanged inputs and successful target/case cleanup.
The gates took 1,825.462 seconds on macOS and 512.602 seconds on Linux.
After both gates, eight fresh lessons ran sequentially on macOS then Linux:
even/skewed 131,072-event reports at both budgets, empty/small reports, the original
report and calendar lesson. Every process succeeded with empty stderr and exact
cross-platform stdout agreement, including DOUBLE bits. Results and sampled temp
quantities match the preceding scaled-report record. Execution of these lessons
took 20.335/12.261 seconds, excluding fresh builds; these are verification timings.

The frozen 742-input manifest SHA-256 is
`02a5a68b3fefa0b9039a2c54a2c109971216df2baf59c967a95e3333fbf7a58e`.
Only the two notes files changed afterward; the other 740 inputs retain fingerprint
`9522ba94f8bd0f59aa38711232df7fdfd565baa5ac655c5477ac3bbcc6814094`.

| Artifact | macOS SHA-256 | GNU arm64 Linux SHA-256 |
| --- | --- | --- |
| Full gate receipt | `fa68387e8378142886b036e539d1848e5d5c24879dbec7be007bb8d8667bf435` | `e536de85b6f7a33eccd918243aa558e10663fb17af3caa497aff37a902c054b6` |
| Fresh lesson receipt | `572aa58d2a76c4631e6bb5a06889ce84090020b9468c2fef583a545d00c2d79e` | `3a6cb1704c2e06a909c3004c35a1301f2caba5a603349431c9640b488ca38183` |
| Fresh scaled-report binary | `48971416fe7715bae0233f838331163488bcb36f93ec457a6d6523a964b74dcb` | `69911b611314d93a40633807f042add22e554803a832595877e6a939a1ad33f6` |

Monitoring sampled host CPU/RSS, memory pressure/swap, disk capacity/I/O and
network during sustained work. Pressure was normal/warning, swap ranged from
1,217.38 to 2,187.12 MiB and available disk stayed above 185.71 GiB. Sampled disk
throughput reached 221.96 MB/s; compiler/owned-process samples reached 99.9% CPU
and 556,992 KiB RSS. Container samples reached 1.300 GiB with no OOM kill and zero
network traffic. One Mac Cargo job and one Linux CPU/job were used. Linux retained
uid/gid 1000, 2 GiB without extra swap, no network and native database storage.

These observations qualify the bounded report histories, not arbitrary allocator
reuse or physical/RSS caps. The separate combined-query macOS usable deficit
remains unqualified; its strict diagnostic is unchanged. General concurrency,
Windows and broader durability limits remain in the [plan](plan.md).

Final documentation verification passed 927 local links. Owned logs, manifests,
source exports, diagnostic/build targets, tutorial databases, monitoring outputs
and the verification container were removed. No owned verification process
remained. The original workspace target and preserved Linux image are unchanged.

## Scaled report spill, refusal and replay

`8eeaa08` adds [scaled_report.rs](../examples/scaled_report.rs) and extends the
[event-report lesson](../docs/event-report.md#scale-the-report-and-observe-spill).
The caller writes 131,072 events in 512 bounded batches using the retained input
helper, with different NULL intervals and even/skewed dimension distributions.
The skewed profile retains rare missing keys and empty labels as well as the
frequent duplicated key. An independent nested-loop join and literal DATE/year
mapping construct complete nullable integer expectations; the ordered DOUBLE
projection has separate bit checks. Small and empty profiles remain explicit.
The original sixteen-event literal oracle and negative control are unchanged.

Two new public tests compare all four profiles at 32 MB and 8 MB query budgets,
require spill for both scaled profiles, cancel after observed temporary storage,
and rerun the retained plan. They check result schema, every row, Finished and
reservation release. A 500 KB memory budget refuses the native query workspace;
a one-byte temp budget refuses database temporary storage. Both retain typed
Resource fields and release ownership before close and healthy reopen. An altered
reference count must fail, followed by a successful rerun. Abandoning a result
after its first batch also releases execution state and preserves the plan.

The [internal replay test](../src/execution/aggregation/grouping/tests/replay.rs)
uses the same report SQL over four facts and three dimensions. It admits one
hash group, then observes disk fallback and replay of the retained join. Five
literal rows cover NULL years/labels, duplicate dimensions and all-NULL totals.
Every step reconciles result ownership, and completion releases all pressure,
plan, result and temporary reservations. No production engine code, allocation
allowance, format, validator or failure contract changed.

Sequential optimized macOS/GNU arm64 Linux verification independently discovered
and passed 449/451 library tests, 173 catalog tests and four example tests per
platform: 626/628 tests respectively, with zero ignored or filtered tests. Platform-specific
library discovery accounts for the difference. All-target
Clippy with warnings denied, formatting, stock example builds and documentation
checks passed. This scoped library/catalog/example verification does not replace
the earlier complete core or native/persistence gate records.

After both platform suites, eight fresh lessons ran first on macOS and then on
Linux. Every process succeeded with empty stderr and identical complete stdout:

| Profile | Query memory bytes | Events | Groups | Sampled temporary bytes |
| --- | --- | --- | --- | --- |
| Even | 32,000,000 and 8,000,000 | 131,072 | 20 | 13,893,440 |
| Skewed | 32,000,000 and 8,000,000 | 131,072 | 16 | 13,893,440 |
| Empty | 8,000,000 | 0 | 0 | 237 |
| Small | 8,000,000 | 16 | 13 | 2,065 |

The original report and calendar lessons also matched. These observations show
that the high-budget report spills too; they do not attribute all temporary bytes
to grouping. The separate internal observation establishes grouping replay.
The independent oracle rejected its altered count on both platforms.

The frozen 742-input manifest SHA-256 was
`a67c19ddd7e5def20752f2606a2c12d63762cc0f3b3d89e8f4366e5216f283ee`.
Only the two notes files changed afterward; the other 740 inputs retain
fingerprint `8709ae8730b2ed56278f673f43c66d6d8864690ee28bbca74c5553532dcc6bb8`.

| Artifact | macOS SHA-256 | GNU arm64 Linux SHA-256 |
| --- | --- | --- |
| Scoped command receipt | `b656ce2350e8505065fc427e113a2b87a0cda10385c642e6d7418a801efce26d` | `e36a2e0c87452d3b166ed9b49884e0e78ee8157041889cad5fccbb239cc96665` |
| Fresh-example receipt | `b94afb487514e32c7361f4ce2d2d6037effac056bdc0f3c6876d47d28de5e3f9` | `b41590a90e294732f686ae5b3e4c39b78f37f14765624c3994ca166f53f119d2` |
| Optimized scaled-report binary | `9a473a28d622293ec1498f2efd19250455769ff091403f89efd1b86d94749204` | `e8b7240748701d808d21bf064fdc0d233310d962bd6ea31035f01fc958d3309d` |

The common even-profile stdout digest is
`c31f844f8a1cc0548e3c5cc7dda0d48b6fda5b12459f01fa248eeeb4bb0fd40c`;
the skewed digest is
`00b83a5c2d30af631514a20e2b392e007ce08c690147b9d775fefa2255465b15`.
The scoped sequences took 473.94 seconds on macOS and 162.56 seconds on Linux.
Fresh examples took another 21.21 and 12.36 seconds. These include validation and
filesystem setup; they are verification timings, not a query performance claim.

Host monitoring covered CPU, RSS, memory pressure/swap, disk capacity/I/O and
network. Observed pressure was normal/warning, swap 1,676.56–2,174.25 MiB, and
available disk space stayed above 185.84 GiB. The largest compiler/owned-process
samples were 100% CPU and 832,080 KiB RSS; observed host disk throughput reached
172.49 MB/s. Container samples reached 1.195 GiB, with no OOM kill and zero
network traffic. One Mac Cargo job and one Linux CPU/job were used; Linux kept
uid/gid 1000, 2 GiB without extra swap, no network and native database storage.
These sampled observations do not establish physical-memory bounds. Broader
[qualification limits](plan.md#qualifications-that-remain-outside-this-internal-claim)
remain unchanged.

Final documentation verification passed 919 local links. Owned build targets,
source exports, input manifests, logs, monitoring outputs, tutorial databases and
the verification container were removed. No owned verification process remained;
the original workspace target and preserved Linux image were unchanged.

## Composed report across appends

`890b5c9` adds the [sixteen-event report lesson](../docs/event-report.md) and
[public example](../examples/event_report.rs). The bounded caller input writer
is separate from literal expected groups, DATE offsets and DOUBLE bit patterns.
Two event appends produce seven old groups and thirteen new groups; duplicate
key-2 dimensions account for ten and twenty joined rows. Missing dimensions,
empty STRING labels, nullable dates and all-NULL totals remain distinct.
A query prepared before the second append retains the complete old answer.
A fresh query and a query prepared after close/reopen produce the complete new
answer. The separate numeric projection checks identities, original DATEs,
nullable measurements and doubled bits, including negative zero.

Two example tests use the existing public test-directory cleanup owner. One runs
the complete workflow; the other changes expected total 30 to 31, requires
`unexpected report group 3`, checks partial-result release and reruns the retained
plan successfully. Both success and rejection reconcile memory/temp reservations.
Preparation releases its temporary SQL source before execution. No engine,
allocation allowance, persistent format, validator or general framework changed.

Sequential optimized macOS and GNU arm64 Linux checks independently discovered
and passed all 173 catalog tests and both example tests per platform, with zero
failures, ignored or filtered tests. Formatting, example Clippy with warnings
denied, four example builds and documentation checks passed. Fresh report,
calendar-year, snapshot and left-join examples ran sequentially on macOS then
Linux after verification; all exited successfully with empty stderr and identical
stdout across platforms. The report's 17-line output ends in `status=finished`.
This is a public integration/example checkpoint; the retained core and complete
native/persistence gate claims remain attached to their earlier revisions.

The frozen 741-input manifest SHA-256 was
`15357a9189009c5cb2399c64c1ee35d1af96059945ef4cc4c655f22212d31da1`.
Only the two notes files changed afterward; the other 739 inputs retain
fingerprint `c75bda36554eb6ae56a96d9a4aae40debe00860ddbe65a3531e9f219e483ef0c`.
The scoped command receipts cover format, Clippy, separate build/discovery/run
steps, fresh examples and documentation:

| Artifact | macOS SHA-256 | GNU arm64 Linux SHA-256 |
| --- | --- | --- |
| Command receipt | `c1db1e42d44f90a80edb4a31f4ffca70a87936483b301bbdab3f4b9052f51539` | `55dc6e7030768ebc4be21e1768da3dcf3f70dcd34cfc964cc1cf9f574b084297` |
| Optimized report binary | `e6ff7fa5c439e1e8f7ad31f8413460fa637e508ffd54b6ce403b1eeae0b9dbf3` | `d1cfa06f0337e94051da2208e2cad59ffd8352bc3329ed02461adf2e4ceccd06` |

The common report stdout digest is
`afd1069f6f149c9b389afd53b1d891b7f5258a3589f7f61f69a2ca7a74e60657`.
The scoped sequences took 112.65 seconds on macOS and 52.57 seconds on Linux,
excluding preliminary implementation checks and the final fresh-example reruns.
An initial missing borrow in the example test failed compilation and was repaired;
a lifetime-notation warning was also corrected before these frozen checks.
No engine defect was exposed. Final documentation verification passed 909 local
links. Owned source exports, build targets, logs, manifests, tutorial databases,
monitoring outputs and the verification container were removed; no owned process
remained. The original workspace target and preserved image were unchanged.

The host monitor sampled CPU, RSS, memory/swap, disk capacity/I/O and network.
Observed swap stayed at 1,844.62 MiB; minimum available space exceeded 186 GiB.
Observed compiler/owned-process CPU reached 95.1%, RSS 456,480 KiB, and host disk
throughput 127.49 MB/s. The final memory-pressure level was normal. Host `en0`
counters include unrelated traffic. Sampled container memory reached 426.7 MiB;
container network counters remained zero and no OOM kill occurred. These are
sampled observations, not peak bounds or engine admission qualification.
Linux used the preserved Rust image, uid/gid 1000, one CPU/job, 2 GiB without
extra swap, networking disabled and native container database storage. The
[broader limitations](plan.md#qualifications-that-remain-outside-this-internal-claim)
remain open.

## Calendar-year projections

`a2fed93` adds bounded EXTRACT(YEAR FROM date) projections. The
[calendar lesson](../docs/calendar-year.md) follows stored DATE input through
its existing Gregorian decoder, typed INT64 projection and grouped execution.
The parser and binder reuse constant DATE syntax and shifts; runtime reads a
checked source/materialized DATE or owned DATE constant. Literal extraction
folds during preparation. The shared typed-input lookup keeps both DATE and
STRING out of numeric input buffers. No calendar algorithm, allocation owner,
admission allowance, persistent format or general expression framework was added.

Eight public tests cover literal years 0001 and 9999, negative epoch offsets,
leap/century boundaries, the Gregorian/ISO-year distinction, NULLs, retained DATE
values and reopen. Constant tests release SQL before execution, retain the eight
DATE-shift nesting limit and reject the ninth shift and invalid intermediates.
Typed identities compose with arithmetic, predicates, aggregation, joins,
ordering and set materialization. Demanded arithmetic keeps its owned span;
COALESCE and Boolean skips retain their distinct evaluation/loading behavior.
Cancellation, abandoned results and a literal logical-plan report are checked.
Independent semantic/physical mutations reject wrong types, nullability, scope,
producer provenance and raw DATE positions used as extracted INT64 outputs.

The corrupted-DATE fixture keeps metadata/checksums and changes a demanded
payload. A false earlier filter and skipped Boolean branch complete without
reading it; direct extraction, numeric conversion, selected COALESCE dependencies
and NULL tests fail before rows. The native admission fixture checks the existing
147,424-byte DATE payload capacity and a 6,176-byte runtime extraction workspace,
including exact admission and one-byte-short refusal before I/O. Two added
forced-replay variants retain a DATE constant across sorting or create it after
sorting, then extract its year and feed grouping; literal totals are 4007 and
2007. All 42 retained replay variants pass. Focused checks caught an ambiguous
fixture alias and a stale ownership-census marker; their corrections preserved
name resolution and independent missing-marker controls.

Sequential macOS and GNU arm64 Linux 14-stage core gates pass 680 ordinary Rust
tests each across nineteen discovered targets: seven nonempty suites and twelve
empty example harnesses. macOS has 448 library, 15 CLI, 173 catalog, 10 execution,
seven lifecycle, six load and 21 filesystem tests. Linux has 450 library and
19 filesystem tests; other counts agree. No ordinary test is ignored or filtered.
The separate lease subprocess passes one test with six intentional filtered
siblings. Both platforms pass 100 tooling tests, 44 independent codec fixtures,
warnings-denied Clippy and Rustdoc, and the zero-case doc-test harnesses.
Stage totals are 473.066 s on macOS and 175.712 s on Linux.

Both complete ownership selections pass nineteen analytic cases and six wide-set
cases at each pathname length, construction-refusal prefixes 0–352 and healthy
control 353, partial-result/formatter observations and all sixteen negative
controls. The two added analytic shapes check year 1969 before and after spooling
with complete rows, requested/usable attribution and release. Stock CLIs pass
24 independent semantic cases and 326 composition records; only the documented
database-path and output-digest fields are normalized for cross-platform
comparison. The year corpus uses independent Python calendar expectations and
literal boundary/shift results. Native fault/interruption campaigns were not
rerun; their preceding checkpoint and broader qualifications remain separate.

Fresh examples run after both platform campaigns, macOS then Linux. Calendar
input yields four groups: NULL/40/1, 1999/10/1, 2000/25/2 and 2001/30/1, followed
by `status=finished`. Declared-table output remains north 15/3/2 and south 20/1/1;
the logical-plan example retains its literal report and total 38. BYTE_LENGTH
returns 5/5/12; the character lesson returns 2/1/3/2/4/1/11/3. All executions exit
zero with empty stderr, exact expected results and terminal completion.

| Receipt/artifact | macOS SHA-256 | GNU arm64 Linux SHA-256 |
| --- | --- | --- |
| Core receipt | `ec5befab63970a65d0f7274f2167fba08590b4b9d7c1dae373c9e75a99392ce1` | `9ae3fd8679021f1aa9f7219b5010efaff4876b0e0da7fb17ca69d7ca1fa27347` |
| Ownership driver | `740a30c8496f17fbd80da00f9890e7ae5882b6078a5fcf71d2269a0bdbe06891` | `b5066604514bea8c44d621ad2ae74c7511b27de32c05f3f127ee74b29f2539b9` |
| Ownership log | `af5c728496f236f0a3e28cc1d89bc8807fa51ec49df9ded02775a8c447bccef6` | `bdbf1540b7c01b4482839a1a2cc617f65fb5b1f350c308da40fa313035067917` |
| Stock CLI | `34c02f5414535132d9f778561d016654672565e569558032a5856d9b9b780fde` | `e0315e1a34fbb5b2fff910d28899685b6069b3620222156c8572fdb3a908a64b` |
| Semantic records | `cb2a492fdb3b94612b168935c9abd3f9dfeb0a2ee70f31478455f5b37bfc51be` | `db060c05a5ecf80f4fc3208c2cd186030c97b1ec2d3f7f903e3e6d516ede48f0` |
| Composition records | `093bfd9613dd1e0e997331a7655778bdfc9dfe33943372fcf63a4eacc51ac6b3` | `47410590779d00f8d5596291a48ad66c6789e349e9dfa7a929e0f0713f1244a3` |
| Calendar example | `bfa0b1a63acc5a5777ffd81f2359d3c4d767edd7a2d43153841ea95bb868f0f2` | `1d34a00a0a0d681a13513e8e0e404ec799e292261f441042875a9b00abaf8cb6` |

Calendar stdout is identical at
`717f9259b7a91ac02a731a8718eef5f71cdea7d2a501cf524921df6f92f2fd75`.
Both gates use frozen 737-input manifest
`36631a08fa2aa47f80217efc84c736d38ab7f8499694c96ce73a9d12e0db52ac`;
before/after manifests match. Final changes are limited to both notes files,
the language summary entry and resource prose distinguishing runtime column
extraction from folded constants. Those documentation changes receive a fresh
link check. The other 733 inputs retain fingerprint
`b41aba08586de9bb8eb002b795e957bbd2f741745a880c09b2008c22c6eb0947`.

Verification uses Rust 1.98.1, one Mac Cargo job and the preserved Linux image
`sha256:520be9ff830f944e49a3319cbf6f8ccfb2c1f21631947de50290efb98038e282`,
uid/gid 1000, one CPU/job, 2 GiB memory without additional swap, networking
disabled and native container database storage. The monitor records 124 host
and 28 Docker samples: normal/warning pressure, swap 1738.44–1964.62 MiB, free
disk at least 185.55 GiB and sampled disk traffic 0–147.25 MB/s. Observed rustc
peaks are 99.4% CPU and 730,096 KiB RSS. Docker reaches 100.78% CPU and 1.231 GiB,
with no OOM kill or network traffic. Host en0 increases by 201,828,822 inbound
and 38,599,250 outbound bytes, including unrelated traffic. These observations
do not qualify engine admission, arbitrary allocator histories or whole-process
RSS. The existing usable-heap, concurrency, durability and Windows limitations
remain open.

Owned logs, targets, exports, example databases, monitor and verification
container are removed after recording results. The original target tree,
installed toolchains and preserved image remain. No package is published and no
history is rewritten. The verified repository checkpoint may be synchronized
under the current publication policy.

## Borrowed logical-plan explanations

`2244948` adds `PreparedQuery::logical_plan()` and the public borrowed
`LogicalPlan` formatter. The [runnable preparation example](../docs/frontend.md#inspect-the-prepared-plan)
uses the existing sales setup and `query-flow.sql`. It prints the actual logical
relations, source occurrences, input edges, visible identities, postfix scalar
program and final output position, then verifies one nullable INT64 total of 38.
Logical structure remains distinct from physical scheduling, demand and costs.
Names absent from the retained plan are not reconstructed; diagnostic text and
labels remain unstable.

The formatter owns only a shared Plan reference and writes directly to its sink.
It has no database, execution or catalog effect authority and adds no allocation
owner, pin, allowance or format change. Source comparison with `61ce436` retains
every pre-existing production file except the public entry point and exports in
`src/frontend.rs` and `src/lib.rs`. Plan and PreparedQuery storage fields, parser,
binder, independent validators, scalar evaluation, execution and native owners
remain unchanged. The new formatting module handles existing semantic variants
directly, without a visitor framework or another query representation.

Six public tests retain literal complete reports for projections, grouping,
repeated output positions, joins, positional union and legacy input. Additional
literal lines cover scalar operations, typed constants, STRING measurement,
nullable LEFT JOIN outputs, structural operators and lowered filter decisions.
Every byte-capacity cut of one report propagates sink failure immediately;
the complete sink is its control. Formatting preserves logical reservations and
the prepared snapshot across append, followed by normal execution and release.
One initial join oracle incorrectly counted a source alias as a node. Existing
binder inspection confirms that FROM aliases enter source scope directly; the
corrected expectation follows retained nodes rather than original syntax.

Sequential macOS and GNU arm64 Linux 14-stage core gates pass 669 ordinary Rust
tests per platform, discovered across eighteen targets: seven nonempty suites
and eleven empty example harnesses. macOS has 445 library, 15 CLI, 165 catalog,
10 execution, seven lifecycle, six load and 21 filesystem tests. Linux has 447
library and 19 filesystem tests, with the remaining counts equal. No ordinary
test is ignored or filtered. The separate lease subprocess passes one test with
six intentional filtered siblings. Each platform also passes 100 tooling tests,
44 independent codec fixtures, warnings-denied Clippy, Rustdoc and doc tests.
Stage times total 469.692 s on macOS and 185.344 s on Linux.

The maintained ownership selection passes on both platforms and pathname lengths,
including seventeen analytic cases, six wide-set cases, construction prefixes
0–352 and healthy control 353, and all sixteen negative controls. The existing
partial-result observer now covers successful and failed formatting into fixed
caller storage. The healthy selection's twelve formatting intervals per platform report zero
allocation and free events; nonzero preparation events provide positive controls
for the same observer. No arbitrary allocator-history or process/RSS claim follows.
The separate aggregate-semantic/composition and native fault campaigns are not
repeated; their preceding checkpoints retain their recorded scope.

Fresh examples run after verification, macOS then Linux. Declared setup returns
the expected north/south totals. The logical-plan example produces all ten
expected lines, empty stderr and exit 0 on both platforms. Its complete stdout
SHA-256 is `b76051675e8902a55a7124159f6768425cda4f407a9d1033c98eaec0bc8c2d1a`.
The example checks schema, one complete row, Finished and logical owner release.

Both platforms use frozen 733-input manifest
`85f4def06c54ca5efe2ee7164d120fdbe079d6a9783a5d19d9b360c0e13db671`.
Only the two notes files change afterward. The other 731 inputs retain fingerprint
`9366445815f0868fb2da0d448dc41e98278a2a7f37d9b1a9fddd12b61af799f4`.

| Artifact SHA-256 | macOS | GNU arm64 Linux |
| --- | --- | --- |
| Core receipt | `039907f3d48d198bb53d80f69bf05b3a3a5cd2c3e54b4ef26c2867dfa1511eab` | `2450765d8777dd73d695699a5a46b4764de89fa16cdeb5c2a9c2beeb1c6daf7a` |
| Ownership driver | `55c315fc3755cc64c21380871ca778c259a7a8c75a33e1ff17832f3a7f853db8` | `f96c1488a460a77fa56775cd27464886dcef18c5157f18cc94f6462d23a877c3` |
| Ownership log | `b8fce7262e4846392bf78b7bc47c4b62a82dfdb4ee7a488c387c8274f65ebe8a` | `3c3ff0790f90c01574fb5f01779bd6e4757f92f3a7d1240bbe54113ad429c423` |
| Logical-plan example | `b9b2d6c5d37cac79b9f520eaf4869d854cc8deeffadcad79a2f7a11866b71272` | `d7d9c86ddf162279838e9c26ef0bef988dfd16f116faf5ca91978e6b1930f474` |

Monitoring records 144 host and 29 Docker samples. Host pressure is normal/warning,
swap spans 1,174.94–2,012.69 MiB, free disk stays above 186.16 GiB and sampled
I/O spans 0–170.26 MB/s. Relevant Rust/build processes peak at 100% CPU and
810,880 KiB RSS. Docker peaks at 104.12% sampled CPU and 1.252 GiB memory, with
no OOM and zero network traffic. Host en0 counters grow by 395,455,722 input and
45,423,071 output bytes, including unrelated traffic. Both platforms use one
Cargo job; Linux uses uid/gid 1000, one CPU, 2 GiB without extra swap, read-only
source and native database storage. These are observations, not admission or
whole-process bounds.

All owned logs, manifests, exports, targets, tutorial databases, monitoring and
the verification container are removed. No owned verification process remains;
the original workspace target, toolchains and image remain. The unresolved
allocator, concurrency, durability and Windows qualifications are unchanged.
Final documentation verification passes 875 local links.

## SQL equality and stored DOUBLE bits

`55c6216` adds [a runnable equality tutorial](../docs/equality.md) and
[its public-library example](../examples/equality.rs). Eight literal rows contain
both signed zeros, two distinct NaN representations, two NULLs with different
ignored payloads and two copies of `1.0`. Reopen preserves every present input's
bits. Counts distinguish eight rows from six present values; grouping yields
four classes of two, and DISTINCT retains one actual input representative per
class. Four predicates retain exact literal ID lists. A self-join emits exactly
the eight zero/one pairs, with no NULL or NaN match.

The example checks names, types, nullability, complete rows, Finished and logical
execution/preparation release before printing. Its private numeric copies compare
DOUBLE bits directly. The fixture oracle accepts either original zero or NaN
representation without deriving equivalence from production or promising an
incidental representative or order. The reading path connects ordinary predicates,
key comparison and hashing, duplicate removal and the join's match guard.

Warnings-denied release Clippy and example builds pass sequentially on macOS and
GNU arm64 Linux. Fresh wrong-expectation controls on both platforms change only
`EXPECTED_GROUP_ROWS` from `[2, 2, 2, 2]` to `[3, 2, 2, 2]`. Each exits 1 with
empty stdout and exactly `Error: "group counts differ from the literal oracle"`
on stderr. These variants compile with `rustc --edition=2024 -O -D warnings`,
an explicit crate name, `--extern pipesql` pointing at the stock release rlib and
`-L dependency` pointing at its release dependency directory. No production
source is changed for the control.

After these checks, fresh healthy runs execute macOS then Linux. Each checks all
27 report lines, empty stderr and exit 0. Output SHA-256 agrees at
`ddd81d45d23611d580e370c62c548d76f1d4a3768f2de31ad5ed1b22c568d291`.
The maintained checks pass 100 tooling tests and 44 independent codec fixtures.
All 700 pre-existing tracked non-Markdown files are unchanged from `5812a8a`.
Ordinary Rust suites and native fault/ownership campaigns are not rerun for this
example-only change; their preceding checkpoints remain separately scoped.

During development, compilation exposed an unnecessary Clone requirement when
copying an array into a vector; moving the array resolves it without a new trait.
The first SQL run used reserved `rows` as an alias. Changing the example's alias
to `entries` preserves the existing grammar. Neither issue required an engine
change, allocation owner, allowance or format revision.

Both platforms use frozen 730-input manifest
`8e9f7c8bf54b652e57be5f61c4c054b48db2e56212c853e1d79bf92e49d844a3`.
Only the two notes files change afterward; the other 728 inputs retain fingerprint
`1ce84bffdcf498840ef3a8ea885317fab087a4675d383ae293579297b072591d`.

| Artifact SHA-256 | macOS | GNU arm64 Linux |
| --- | --- | --- |
| Stock equality example | `6dd457d6bc4bb514585737e207252b458864a3645a4222196c7c7042051d19f1` | `3084f67d99474ed292a1b722d67866c284b2e92b7877f77cb82f8b1a11085b7c` |
| Healthy receipt | `d96415c7a260b671460fb48e3fcb9d811c0fa41a725ede7a2244b0f1033a67d9` | `e02d937032b1d6fb986e862f0262949316c24f9654eeea954833481ccfe8154c` |

Monitoring records 43 host and six Docker samples. Host pressure stays normal,
swap spans 1,350.94–1,462.94 MiB, free disk stays above 187.70 GiB and sampled
I/O spans 0–65.98 MB/s. Relevant host processes peak at 96.7% CPU and 396,624 KiB
RSS. Docker peaks at 99.63% CPU and 805.1 MiB, with no OOM and zero network traffic.
Host en0 counters grow by 4,507,657 input and 2,605,340 output bytes, including
unrelated traffic. Both platforms use one Cargo job; Linux uses uid/gid 1000,
one CPU, 2 GiB without extra swap, read-only source and native database storage.
These observations do not qualify engine admission or process/RSS bounds.

All owned outputs, databases, exports, builds, monitoring and the container are
removed. The existing workspace target, toolchains and verification image remain.
Final documentation verification passes 864 local links.
Broader allocator, concurrency, durability and Windows qualifications remain open.

## Numeric call recognition

`4ce634c` replaces the numeric parser's duplicate acceptance and frame-selection
chains with read-only `Parser::pending_numeric_call`. Recognition checks token
kind, the following left parenthesis and one mapping for all eighteen supported
spellings. Numeric parsing still owns token consumption and stack admission.
Known-call mappings are unchanged; an unknown name explicitly returns None
instead of relying on the previous guarded SAFE_DIVIDE fallback. The
[preparation explanation](../docs/frontend.md#expressions-and-predicates) follows
nested CAST, COALESCE, ABS and MOD through their postfix operand boundaries.

Source review retains all previous recognized-call branches and spellings,
including aliases and LOG10. Only the parser changes in production; the lexer,
binder, typed program, independent validators, execution, resource/native owners,
allowances and formats are byte-for-byte unchanged from `e7688ab`. No registry,
mutable parser state or new allocation owner is introduced. This is a readability
change, with no performance claim or new accepted syntax.

All eighteen focused parser tests pass. Two new tests retain a literal nested
postfix program across mixed case and POW/POWER and CEIL/CEILING aliases, ordinary
bare function names as columns, reserved CAST rejection and exact unknown-call
diagnostics. The first unknown-call assertion incorrectly expected a span on
`(`. Inspection of the retained parser shows the existing pipe-loop stop and
end-of-query trailing-syntax error; the corrected test preserves that message
and empty end span. Existing arity, nesting, token and operation-bound tests remain.

Sequential macOS and GNU arm64 Linux 14-stage core gates pass 663 ordinary Rust
tests per platform, independently discovered across sixteen targets: seven
nonempty suites and nine empty example harnesses. macOS has 445 library, 15 CLI,
159 catalog, 10 execution, seven lifecycle, six load and 21 filesystem tests;
Linux has 447 library and 19 filesystem tests with the other counts equal. No
ordinary test is ignored or filtered. The separate lease subprocess passes one
test with six intentional filtered siblings. Each platform also passes 100
tooling tests, 44 independent codec fixtures, warnings-denied Clippy, Rustdoc
and doc tests. Stage times total 467.937 s on macOS and 167.165 s on Linux.
All 24 independent semantic cases and 322 composition records agree after only
the documented database-path stdout and composition-digest normalization.

Fresh examples run after the campaigns, macOS then Linux. Declared setup returns
the documented north/south totals. Numeric CAST returns required DOUBLE bits
`4340000000000000`, `3ff0000000000000` and `0000000000000000`. SQRT returns
nullable DOUBLE bits `402a751f9447b724`; query-flow returns nullable INT64 38.
Every query has its expected schema, one complete row, successful status and exit.
The [CAST ownership checkpoint](#explicit-numeric-double-casts) and earlier native
fault evidence remain distinct; their campaigns are not repeated for unchanged
runtime and native owners.

Both gates use frozen 728-input manifest
`05767f31c32a6359b1432f1262cc0c7f40f6bbcf36e4e684e84fcfe2fb223541`.
Only the two notes files change afterward. The other 726 inputs retain fingerprint
`7f1289be32bc2318ee60e69e62a0607451cc21efceb7b5709ab0d6b483df0ee6`.

| Artifact SHA-256 | macOS | GNU arm64 Linux |
| --- | --- | --- |
| Core receipt | `6f5e78b8786da7375b346b71d187788c9cadfa5bd81b76c364cd3c8fc55c0f65` | `7057a95655f4bbb6f3dedd5dd4bb92e05b68aacf780b935495eeaf85820f7b2b` |
| Stock CLI | `77ba1933a21c46dc4a1d4c20ee2f998c22f73a21587c92c582541dfb82281da8` | `c15c788ddc921a21a020a03445d63c651e12ad56066f49d16ef072162fa4a55e` |

Monitoring records 73 host and 21 Docker samples. Host pressure is normal/warning,
swap spans 1,197.62–1,814.81 MiB, free disk stays above 186.60 GiB and sampled
I/O spans 0–136.32 MB/s. Relevant host processes peak at 99.1% CPU and 776,528 KiB
RSS. Docker peaks at 99.93% CPU and 1.237 GiB memory, with no OOM and zero network
traffic. Host en0 counters grow by 179,280,998 input and 11,235,839 output bytes,
including unrelated traffic. Each platform uses one Cargo job; Linux uses uid/gid
1000, one CPU, 2 GiB without extra swap, no network and native database storage.
Sampling can miss short peaks and does not qualify engine admission or RSS.
Broader allocator histories, durability, concurrency, sanitizers and Windows
qualification remain open.

Owned targets, databases, exports, manifests, logs, monitor outputs and the
verification container were removed. The existing workspace target, installed
toolchains and preserved image are unchanged; no owned verification process remains.
Final documentation verification passes 847 local links.

## Explicit numeric DOUBLE casts

`1fcd7bd` adds CAST of a bounded numeric expression to FLOAT64 or DOUBLE.
The [language contract](../docs/language.md#current-public-query-manifest) owns
accepted spellings and conversion semantics. The iterative parser closes a CAST
frame at AS and emits one unary ToDouble operation. Existing inference, independent
validators, batch evaluation and the demand cursor preserve typed input evaluation,
NULLability and DOUBLE bits without a new allocation owner, allowance or format.
The [exercise](../docs/query-examples.md#choose-where-integer-arithmetic-becomes-approximate)
contrasts exact subtraction before conversion with subtraction after rounding.

Literal bit oracles cover 2^53 neighbors, negative ties, INT64 extrema, signed
zeros, subnormals, finite extrema, infinities and NaN payloads. Public tests cover
owned preparation, nullable schema, nested calls, typed replacements, joins,
sets, grouping collisions, materialization, reopen, invalid source/target spans,
input overflow, skipped COALESCE and Boolean branches, SAFE_DIVIDE argument errors,
cancellation, abandonment and release. Constant predicate overflow remains a
preparation error. Parser checks admit 31 nested casts within the existing
32-operation/160-token bounds and reject excess operations or tokens. Both small
stack scenarios execute nested CAST with a skipped overflowing fallback.

Focused checks pass for both evaluators, all six public cast tests, exact native
admission/refusal, corrupted STRING dependencies consumed through numeric CAST,
forced grouping replay and small-stack execution. The admission oracle retains
278,496 native payload bytes and independently requires 10,304 computed bytes
for a direct numeric CAST; a one-byte shortfall refuses before native I/O. Two
new replay variants convert before or after a sorted producer and still require
fallback replay without reopening sources. Initial test-only failures were a
Debug formatter for QueryStep and an unsupported arithmetic WHERE left operand;
the latter test now names its EXTEND result before comparing it. No language
boundary was broadened to accommodate the test.

Sequential matching macOS and GNU arm64 Linux 14-stage core gates pass:
661 ordinary Rust tests on each platform, discovered across sixteen targets
(seven nonempty suites and nine empty example harnesses). macOS has 443 library,
15 CLI, 159 catalog, 10 execution, seven lifecycle, six load and 21 filesystem
tests; Linux has 445 library and 19 filesystem tests with the other counts equal.
No ordinary test is ignored or filtered. The separate lease subprocess passes
one test with its six intentional filtered siblings. Each gate also passes
100 tooling tests, 44 independent codec fixtures, warnings-denied Clippy,
Rustdoc and doc tests. Stage times total 467.710 s on macOS and 175.409 s on Linux.

The complete ownership selection passes seventeen analytic and six wide-set
cases at both pathname lengths, construction refusal prefixes 0–352 and healthy
control 353, preparation refusal prefixes and sixteen negative controls. CAST
analytic cells 15/16 each return 512 rows in 8,133 steps, observe 42,184 temporary
bytes and release all owners. Minimum usable headroom is 7,648 bytes on macOS
and 8,880 on Linux at both pathname lengths. All 29 healthy allocation-control
cells pass per platform, including catalog control 1056 at both lengths; these
controls do not repeat the catalog allocation-prefix sweep. Linux retains its
two Darwin ACL exclusions. All 24 independent aggregate-semantic cases and 322
composition records agree after removing only database-path stdout lines from
semantic records and artifact digests from composition records.

Fresh examples run after the campaigns, sequentially on macOS then Linux.
Declared-table setup returns the documented north/south totals. Numeric CAST
returns one row of three required DOUBLE values: 9007199254740992, 1 and 0,
with bits `4340000000000000`, `3ff0000000000000` and `0000000000000000`.
BYTE_LENGTH returns nullable INT64 5, 5 and 12; the character-length example
returns eight required INT64 values 2, 1, 3, 2, 4, 1, 11 and 3. Query-flow
returns nullable INT64 38. Every invocation completes with the expected schema,
rows, Finished-equivalent CLI status and successful exit.

Both gates use the frozen 728-input manifest
`3f4901e824b24a9efcf8090a9ca796a1ce1876b554e16e507a242d644aa78823`.
Only the two notes files change afterward; the other 726 inputs retain fingerprint
`7b132df79b459435f475c59ee4b155b30c10ab460835d4da12c9c6efdddc080f`.

| Artifact SHA-256 | macOS | GNU arm64 Linux |
| --- | --- | --- |
| Core receipt | `392b32c48e7e98c50da52227e807c21880dcd2240906ad2a54873112cb1ab3ee` | `5671f4a42c028e717725080ff4ee1bea68f1d70918ac375acdaf1ef29408ff2f` |
| Stock CLI | `0b7b9998c3fb4f2158745e3c8e2a3c49eaaadd55a1b1d53de872df4ef69c58ed` | `5eec924de8789425291dd38d660901fa03235721662630682e7eab5d29714fcb` |
| Ownership driver | `499df543ed129803aa02fcf8c1e0cb19c1e9d7cfc64edd5001fb9b08a56ef45c` | `b7e577f573b61f6412bff4535a5e371d9a21631b53ae9c035a8173b7f46ae7ea` |
| Ownership log | `c9e9b1ebed8505eddfa2592db202153921a38424a0dc80771db0898f6c278fc5` | `0cd459037373090d54e8132c98ce4d8497c953987dc4e18d6d84eda9bd82401d` |

Monitoring records 130 host and 25 Docker samples. Host pressure is normal/warning,
swap spans 1,626.38–1,826.25 MiB, free disk stays above 186.33 GiB and sampled
I/O spans 0–197.09 MB/s. Relevant host processes peak at 100% CPU and 713,552 KiB
RSS; Docker samples peak at 108.56% CPU and 1.24 GiB memory, with no OOM and zero
network traffic. Host en0 counters grow by 451,587,395 input and 19,284,005 output
bytes, including unrelated traffic. Each platform uses one Cargo job; Linux uses
uid/gid 1000, one CPU, 2 GiB without extra swap, networking disabled and native
database storage. Sampling can miss short peaks and does not qualify engine
admission or whole-process RSS. Arbitrary allocator histories, concurrency,
sanitizers, broader durability and Windows qualification remain open. The earlier
[full native checkpoint](#full-verification-checkpoint) remains distinct from
these scoped checks; native mutation and persistent formats are unchanged.

Owned targets, databases, exports, manifests, logs, monitoring outputs and the
verification container were removed. The existing workspace target, installed
toolchains and preserved verification image are unchanged; no owned verification
process remains. Final documentation verification passes 842 local links.

## Stored text measurement costs

`529306d` adds [text_cost.rs](../examples/text_cost.rs) and its
[learning path](../docs/getting-started.md#compare-stored-text-measurements).
The fixture repeats eight explicit nullable text classes across 4,096 rows.
Each eight-row cycle contains six 128-byte values, empty STRING and NULL. Literal
oracles require 3,584 present values, 393,216 text bytes and 246,784 Unicode
scalars. The two queries project BYTE_LENGTH or CHAR_LENGTH before COUNT(*),
COUNT(length) and SUM(length) aggregation. The [execution explanation](../docs/execution.md#stored-text-measurement-costs)
traces the shared payload/checksum/UTF-8 checks, checked borrowed `str`, length
measurement and global aggregation. A local constant-time byte measurement does
not make the complete stored-text query constant-time in input size.

Both prepared queries remain live on a common baseline. Each variant runs ten
warmups and ten samples of fifty complete executions, with sample order
alternating. All 1,020 executions per platform verify schema, literal row counts
and total, Finished and release. Timing includes result construction, source
reads, computation, aggregation, result checks and destruction; setup/open,
preparation, printing and the final reservation check are separate. No cache
eviction is attempted. Both variants observe 610,696 additional logical bytes
and zero temporary reservations on both platforms. These are sampled logical
charges, not allocator extents, RSS or I/O measurements.

Warnings-denied example Clippy and release builds pass on macOS and GNU arm64
Linux. Fresh wrong-total callers change only byte total 393,216 to 393,215;
both reject the first result with exit 1, empty stdout and the intended literal
oracle error. Fresh healthy runs then complete sequentially on macOS and Linux.
The maintained local checks pass 100 tooling tests and 44 independent codec
fixtures. All 696 pre-existing non-Markdown inputs are unchanged from `83fcd3a`;
production code, ordinary Rust tests and the earlier projection-cost example are
unchanged. The engine gates are not rerun for this example-only workload.

Per-execution sample means in milliseconds are:

| Sample | macOS bytes | macOS scalars | Linux bytes | Linux scalars |
| --- | --- | --- | --- | --- |
| 1 | 1.288 | 1.032 | 0.832 | 0.861 |
| 2 | 1.047 | 1.005 | 0.829 | 0.861 |
| 3 | 1.130 | 1.248 | 0.831 | 0.879 |
| 4 | 0.974 | 1.014 | 0.832 | 0.880 |
| 5 | 0.968 | 0.993 | 0.833 | 0.871 |
| 6 | 1.233 | 1.075 | 0.842 | 0.862 |
| 7 | 1.385 | 1.012 | 0.848 | 0.888 |
| 8 | 0.967 | 0.995 | 0.833 | 0.863 |
| 9 | 0.982 | 0.999 | 0.852 | 0.893 |
| 10 | 0.996 | 1.066 | 0.828 | 0.885 |

macOS byte/scalar medians are 1.0215/1.013 ms, with scalar counting slower in six
pairs. Linux medians are 0.8325/0.875 ms, with scalar counting slower in all ten
pairs. macOS ranges overlap and pair ordering changes; Linux has a consistent
ordering in this run. These complete-query observations neither isolate kernel
cost nor establish a general performance or platform ranking. They justify no
engine optimization. The reading path retains the distinction between obtaining
valid text and measuring it without adding benchmark machinery to production.

The frozen 726-input manifest is
`9e941f6dc595d5e387da2c1777fd06efbc1e38df0105ff8cada3be650301cd02`.
After verification, the tutorial clarifies that each combining sequence repeats
the pair (`e`, U+0301); its commands and expected values do not change. Alongside
the two notes files, this is the only later input change. The other 723 inputs
retain fingerprint `ae6709c2684804c7faf4c89d81381404968f756a3bf98d5e28a8a63d68ee1c1a`.
All 697 non-Markdown inputs, including the new example, retain fingerprint
`b63fc7bfd8c592cf08d1e38e1ccfb9de9eb7c7ce6f0b69e4424b2dabc24d02c8`.
The example source SHA-256 is
`5b28e40c19b6566f7d39bb4a400221b9707867c5db89dfc8ee46325c200c6aaf`.

| Artifact SHA-256 | macOS | GNU arm64 Linux |
| --- | --- | --- |
| Example executable | `3537fd2be4de9f662c60543054e7255053ef3cfe91338493a499ca40c201db68` | `1343a0b2fb44dcacbd84a3fcf756b0b162b287e32f8fc7ea16302ff2fd6f3eb1` |
| Healthy stdout | `765943c6ef87d4345017548706f2af3143209bfddc0df13334ed312f233dc3dd` | `18eeb4cfabff42a60259cfe4dff3334fc80fc5966277b23be939f8ea75db88da` |

Monitoring records 26 host and nine Docker samples. Host pressure is
normal/warning, swap spans 1,762.38–1,778.38 MiB, free disk stays above 186.92 GiB
and sampled I/O spans 0–28.24 MB/s. Docker peaks at 103.04% CPU and 666.4 MiB,
with zero network traffic and no OOM event. Host en0 deltas are 157,187,048 inbound
and 5,995,311 outbound bytes, including unrelated activity. Heavy work and fresh
examples run sequentially with one Cargo job. Linux uses uid/gid 1000, one CPU,
2 GiB without extra swap, no network and native database storage. Sampling can
miss short peaks; these observations do not qualify engine admission or RSS.
Broader allocator histories, durability, concurrency, sanitizer and Windows
qualifications remain open.

Owned builds, callers, databases, export, manifests, logs, monitoring outputs and
the verification container were removed. The original workspace target and
preserved image are unchanged; no owned verification process remains. Final
documentation verification passes 826 local links.

## STRING character-length projections

`de6d92b` adds bounded CHAR_LENGTH through `Computation::StringLength` and the
pure [measurement unit](../src/string_length.rs). BYTE_LENGTH and CHAR_LENGTH
share typed input identity, literal folding, independent validators and borrowed
runtime text. CHAR_LENGTH counts Unicode scalar values, including combining
marks, joiners and NUL. Both return INT64, propagate NULL and accept a visible
STRING column or bounded literal as a complete SELECT, EXTEND or SET expression.
The [language contract](../docs/language.md#current-public-query-manifest) retains
upstream primary references and unsupported forms. No allocation owner,
admission allowance, persistent format or generic expression framework changes.

The [paired example](../examples/character_length.sql) and
[reading path](../docs/query-examples.md#compare-bytes-and-unicode-scalars) contrast
bytes, scalars and displayed characters. The
[execution trace](../docs/execution.md#string-length-evaluation) follows one typed
input through the binder, measurement and numeric output. Literal expectations
remain independent of the production counting function.

Focused checks pass ten catalog tests, both independent validator mutations,
exact/short admission, source corruption/demand and forced grouped replay. The
expanded corruption fixture first retained twelve prepared queries and correctly
hit the eight-pin limit. It now runs each unit with its own seven-pin fixture;
the admission rule is unchanged. Four new catalog tests cover ASCII, composed
and decomposed Unicode, supplementary characters, a two-scalar flag, a joined
emoji, NUL, empty/maximal/NULL values, literal ownership, scope, composition and
reopen. Existing rejected-form and cancellation controls now exercise both units.

Matching frozen macOS and GNU arm64 Linux verification passes sequentially:

- All fourteen core stages, including warnings-denied release Clippy, rustdoc
  and doc tests. Independent discovery finds 652 ordinary Rust tests per platform
  across fifteen targets: seven nonempty suites and eight zero-test Rust examples.
  macOS counts are 440 library, 15 CLI, 153 catalog, 10 execution, 7 lifecycle,
  6 load and 21 filesystem tests. Linux has 442 library and 19 filesystem tests;
  other counts agree. No ordinary tests are ignored or filtered. The separate
  lease subprocess passes one test with its six intentional filtered siblings.
- 100 tooling tests and 44 independent codec fixtures per platform.
- Complete public ownership selection at both pathname lengths: fifteen analytic
  cases, six wide-set shapes, all construction prefixes 0–352 and healthy control
  353, retained reader/append/partial-result controls and sixteen negative controls.
  New character-length cases each return 512 complete nullable rows. Before/after
  analytic spooling require 8,121/7,712 steps and peak temporary charges
  38,088/101,496 bytes. Minimum usable headroom is 7,648 bytes on macOS and 8,880
  on Linux in both new cases at both pathname lengths; all owners release.
- Twenty-nine healthy allocation-control cells per platform, including catalog
  census 1,056 at both pathname lengths. Linux retains two Darwin ACL exclusions
  and its expanded-path controls. This selection does not claim a full catalog
  allocation-prefix sweep.
- 24 independent aggregate-semantic cases and 317 composition records. Records
  agree after removing only semantic stdout database paths and composition
  `sha256` fields. The added composition cases cover ASCII input, a Unicode
  literal followed by arithmetic, and an empty aggregate.
- Sequential fresh declared-table setup and BYTE_LENGTH, CHAR_LENGTH and query-flow
  examples on macOS then Linux. Setup returns the documented north/south totals.
  BYTE_LENGTH returns nullable INT64 values `5, 5, 12`; CHAR_LENGTH returns eight
  required INT64 values `2, 1, 3, 2, 4, 1, 11, 3`; query-flow returns nullable INT64
  38. Schemas, complete rows, successful terminal status, exit and empty stderr
  agree with literal expectations.

Core gates take 482.559 seconds on macOS and 172.200 seconds on Linux. The frozen
725-input manifest is `1f3cebb625bf9c9e0f8af7764bd833ec5bcdcc71f96847d8905ae6ac51a24e63`.
Only the two notes files change after verification; the other 723 inputs retain
fingerprint `ca79035fa4dc23982858fa0134f311f713fe851dc45990e57ee10cd5d134e5b6`.
Retained artifact identities are:

| Artifact SHA-256 | macOS | GNU arm64 Linux |
| --- | --- | --- |
| Core receipt | `4dc242cd7d0564d3cdd6d6d80febef3345f615b1ee74004454f6bb5ac68bc545` | `c1ae1fec1127c9e146cba9307002243c867503d4c0b5576d187211c25fa60f0c` |
| Stock CLI | `08bf5010106141bf61a0214512d738ee8cc1b1d2e09f602f2c52fbdc52ac1dcc` | `95b40b163f98ce1002a935ad98ccf08854a3d8943c218ec6457852e33274f313` |
| Ownership caller | `b9eda4b3c4a4f3001a68571cd78ca287a1f8d9e81ae948209eeb9f676936dccb` | `332069db90732dd6ae2a7814e12f55eb29a0ee0efae9e768a52a56673693ca4d` |
| Ownership log | `b560e0f2dd7592e874106f2578c44128ca04f6ea48cc360fd905823581ba2345` | `a017c08865e19e49cad93d72418d5c94b29ffa437ccccd02636409d504e29fd2` |

Monitoring records 111 host samples and 34 Docker samples. Host pressure spot
checks are normal/warning; swap spans 1,403.69–1,840.81 MiB, free disk stays above
186.61 GiB and sampled disk I/O spans 0–456.31 MB/s. The observed host compiler
peaks at 100.1% CPU and 777,424 KiB RSS. Docker peaks at 100.67% CPU and 1.294 GiB,
with zero network traffic and no OOM event. Host en0 deltas are 470,805,091 inbound
and 23,953,817 outbound bytes, including unrelated activity. Verification uses one
Cargo job and sequential platforms; Linux runs as uid/gid 1000 with one CPU,
2 GiB without extra swap, disabled networking and native database storage.
These observations do not establish engine admission or whole-process/RSS bounds.

The earlier full native/persistence checkpoint remains attached to `1e5cfdc`;
these scoped checks do not replace its separate source identity or claim a new
24-stage gate. Arbitrary allocator histories, the retained combined macOS
usable-heap counterexample, broader durability/concurrency/sanitizer coverage,
host-shared database identity and Windows qualification remain open.

Owned logs, manifests, export, targets, tutorial databases, monitoring outputs
and the verification container were removed. Final documentation verification
passes 816 local links. No owned verification process remains; the original
workspace target and preserved image are unchanged.

## Partial-result allocation ownership

`b9b3fcc` adds [result-ownership.rs](../tools/fixtures/result-ownership.rs) under
the existing composed-ownership caller. It reuses the allocation-event observer
and heap/descriptor snapshots. Its independent ordered input is 0–255 followed
by INT64 maximum. The three cases require a 256-row addition prefix before
`ArithmeticOverflow` at bytes 40–50, cancellation after a nonempty partial prefix,
and all 257 healthy rows followed by Finished. No production code, ordinary
Rust tests or executable examples change from `64af0e6`.

Each case observes preparation, construction, individual execution steps and
prepared release. The terminal step must free runtime allocations while their
charges remain live. Repeated terminal steps and consuming the remaining result
handle must allocate and free nothing. The result's retained charge is checked
against its independent physical size. Owned errors retain their category/span
and remain formattable after prepared release; heap, descriptor, memory and
temporary baselines must be restored while the error remains alive.

Focused macOS cases pass. Changing the expected prefix count rejects 256 rows
against 257; leaving steps unobserved after the prefix rejects missing terminal
free events. Both negative controls exit 101 at the intended assertion. The
Python completion checker independently requires all three distinct records,
correct row bounds, nonzero phase events, nonnegative headroom and complete
release. Its new interpretation test challenges missing, duplicated, malformed,
unknown, incorrectly counted and unobserved records.

Complete ownership selections pass sequentially on macOS and GNU arm64 Linux
at short and 384-byte database paths. New case rows/steps are overflow 256/3,783,
cancelled 1/3,016 and finished 257/3,785 on both platforms. Each construction
allocates 26 times and each terminal step frees 18 allocations. Overflow
preparation allocates seven times and prepared release frees two; the other
cases allocate six and free one. Across these new cells, minimum requested/usable
headroom is 4,240/3,208 bytes on macOS and 4,240/4,240 on Linux. These are aggregate
allocation-event observations for these histories, not arbitrary allocator or
whole-process/RSS bounds.

Both enclosing selections retain thirteen analytic shapes, six wide-set shapes,
353 construction refusal prefixes plus healthy control 353 at both paths, and
all existing caller controls. All sixteen negative controls reject as intended.
Warnings-denied caller builds and formatting pass. Maintenance passes 100 tooling
tests, 44 independent codec fixtures and 804 local links. The BYTE_LENGTH core
engine baseline remains applicable; these tool-only changes do not repeat or
supersede its core gate or the earlier full native/persistence checkpoint.

The frozen 722-input manifest and read-only Linux export agree, SHA-256
`d3aba43b89f9ad3803a60b998d60860d5562d63a60e2a5cf4b51940b967433e0`.
Only the two notes files change afterward; the other 720 inputs retain fingerprint
`a31858ba8be3b97bd1c8f85734c0e571cfb723f74eff9bb79231269165011164`.
Final driver hashes are macOS
`6fd011600a325f636e944f53541e700c1e1cc020ce53a3ae11d4cb75644bcf49`
and Linux `71656ad327d77166bfe735569407ca0af34f9bf9eef6225350268d009e52d2a5`.
Complete ownership-log hashes are macOS
`30f850142647c17effacbb7ec6a9d3c50f40bf666737f8a1bf08cba16c8e958b`
and Linux `249280d9c181c3630dac1068b0cc3bf7c9230576d2e5284de84666dc8b744599`.

Rust 1.98.1 uses one Cargo job per platform on the 8-GiB Apple M1/macOS 26.6.2
host. The preserved Linux image uses uid/gid 1000, one CPU, 2 GiB without extra
swap, disabled networking and native database storage. Twenty-two host samples
show normal pressure, 1,531.69 MiB swap, over 188.00 GiB free disk and sampled
I/O 0–105.44 MB/s. Five container samples show at most 100.42% reported CPU,
824.2 MiB and zero network traffic; final inspection confirms no OOM kill.
Host en0 counters increase by 64,083,591 received and 3,833,132 sent bytes,
including unrelated activity. These measurements do not establish peak bounds.

Owned focused/campaign outputs, targets, source export, monitoring records,
logs, manifests and container are removed. No owned verification process remains.
The existing target, image and toolchains are preserved. Final documentation
verification passes 806 local links. Broader physical-memory, concurrency,
durability and Windows qualifications remain open. Nothing is pushed or published.

## Partial query results and terminal ownership

`3c7e5d2` adds [query_results.rs](../examples/query_results.rs) and the optional
[partial-result tutorial](../docs/query-results.md). A fresh table contains
INT64 amounts 0–255 and INT64 maximum. Explicit ORDER BY makes the expected
sequence independent of scan order. The projection returns 256 checked values
1–256 before a demanded addition overflow. The owned error retains operation
`addition` and source bytes 40–50 after the result, prepared query and caller SQL
string are released. A direct ordered query is cancelled after its first
nonempty partial batch; its prepared plan then executes successfully with a
fresh token, returning all 257 literal expected values and terminal Finished.

The first trial incorrectly assumes every Rows batch has 256 rows. The direct
sorted producer returns one row initially; the example now accepts a nonempty
partial prefix and verifies its values before cancellation. It retains repeated
terminal-state checks, result/plan reservation baselines and zero temporary debt.
The [owner trace](../docs/execution.md#trace-a-terminal-query-result) distinguishes
workspace destruction, the remaining result handle, the owned error and the
prepared plan. No production interface or engine behavior changes.

Warnings-denied release example Clippy/builds and fresh sequential macOS/GNU
arm64 Linux runs pass. Both return exactly the tutorial's three lines, including
cancellation prefix 1, with identical stdout SHA-256
`ad9a84302be0eb71abab0ea2841961914fc734c00cedc1a3bbfc989aff62f5be`.
A separately compiled caller changes only the expected overflow-prefix count to
255; each platform rejects it with exit one, the prefix diagnostic and no success
output. Maintenance passes 99 tooling tests, 44 independent codec fixtures and
796 local links. All 691 prior non-Markdown inputs remain byte-identical to
`66b90ea`; the BYTE_LENGTH engine baseline remains applicable without another
core/native campaign. This example does not qualify arbitrary concurrent
cancellation or general physical-memory bounds.

The frozen 721-input manifest and read-only export agree, SHA-256
`740fa716b4a088f70f7ac66e53bf678439760cf4c6364b1af4c3ef0b3b27a964`.
Only the two notes files change afterward; the other 719 inputs retain fingerprint
`063bb783e67542eced9c08e9723ad1579a4662a85c9fbbcf55e24cb410fb65e7`.
Example source SHA-256 is
`865c2705b72225d306e49915bf16265fa48c692d9ac321ffc7c219e4dae107cb`.
Executable hashes are macOS
`5cc6b95c17c3b5e7b6d716effc8db2585cd1659b07bdab01bbba86c9eddab807`
and Linux `8f369375070de93a26d9e75a317d1419d61b935b00c1276837924ba2e2d8b167`.

Rust 1.98.1 uses one Cargo job per platform on the 8-GiB Apple M1/macOS 26.6.2
host. The preserved Linux image runs with uid/gid 1000, one CPU, 2 GiB without
extra swap, no network and native database storage. Twenty-one sustained host
samples show normal/warning pressure, swap 1,539.69–1,555.69 MiB, over 188.14 GiB
free disk and sampled I/O 0–14.72 MB/s. Five container samples show at most
100.03% reported CPU, 784 MiB and zero network traffic; final inspection confirms
no OOM kill. Host en0 counters increase by 106,607,895 received and 5,065,949 sent
bytes, including unrelated activity. These samples are observations, not peaks
or engine resource qualification.

Owned trials, final databases, targets, negative callers, export, monitoring
records, logs, manifests and container are removed. No owned verification process
remains. The existing target, image and toolchains are preserved. Final
documentation verification passes 800 local links. Broader qualifications remain
open. Nothing is pushed or published.

## STRING byte-length projections

`ffa73b3` adds bounded BYTE_LENGTH through `Computation::ByteLength`: one typed
STRING input produces INT64 with the input's nullability. Literal arguments fold
into ordinary integer programs. The shared evaluator borrows checked text;
numeric kernels receive only the resulting integer. Independent semantic and
physical validators retain their own rejection rules. No allocation owner,
format, admission allowance or numeric-expression framework changes. The
[learning query](../docs/query-examples.md#measure-text-in-bytes) and
[evaluation trace](../docs/execution.md#string-length-evaluation) connect the
public result to these owners.

Focused checks cover UTF-8 bytes versus characters, empty and 65,536-byte text,
NULLs, bounded decoded literals, owned rejection spans, aliases/ranges, STRING
constants, batch and row evaluation, joins, grouping, sets, cancellation,
abandonment and source corruption. Two added forced hash-fallback variants
require disk use, replay of a sorted producer, literal aggregate results and
complete release for stored text and retained text constants. Independent
mutation checks reject invalid type, nullability, scope and physical positions.
The existing exact/one-byte-short admission control also checks the literal
6,176-byte computed workspace, separately from the source STRING allocation.

Development checks caught invalid test aliases and unsupported standalone legacy
sorting; those fixtures now use accepted names and grouped materialization.
A deliberate post-sort constant-to-length-to-arithmetic case exposed a missing
constant branch in the new row dependency loop. The repaired resolver reads
retained text directly instead of sending it to a numeric kernel. A corrupted
payload control also disproved an initial test assumption: COALESCE skips scalar
evaluation after potential source payloads load, while an earlier Boolean branch
can skip the entire expression and its payload. Both outcomes are retained and
explained at the execution-contract owner. The final enclosing suites pass.

Matching frozen inputs pass sequential 14-stage macOS and GNU arm64 Linux core
gates in 473.954 and 177.140 seconds. Independent test listings contain 648
ordinary tests per platform across fourteen targets, seven of them zero-test
examples. The catalog suite contains 149 tests. No ordinary tests are ignored or
filtered; the separate lease subprocess passes one test with six intentional
sibling filters. Both maintenance stages pass 99 tooling tests, 44 independent
codec fixtures and 783 local links. These are scoped core gates; the previous
complete 24-stage native/persistence checkpoint remains attached to `1e5cfdc`.

Public ownership selections pass thirteen analytic shapes and six wide-set
shapes at both pathname lengths, fourteen negative controls, and construction
prefixes 0–352 plus healthy control 353. The two new analytic shapes each return
512 independently checked rows, alternating byte length 128 and NULL, with
partition count 512. They require nonzero temporary storage, step ownership and
release. Minimum observed requested/usable headroom across the retained ownership
histories is 4,096/1,400 bytes on macOS and 4,096/3,648 on Linux. macOS ownership
ran before the final replay-test additions and prose wrapping; its production and
caller inputs match the frozen revision. Both platforms also pass 29 healthy
allocation-control cells, including catalog control 1,056 at both path lengths.
Healthy controls do not repeat the allocation-prefix sweeps. Linux retains the
two Darwin ACL exclusions, with its expanded-path create/open controls.

The stock CLI passes 24 independent aggregate-semantic cases and 314 composition
records on each platform. Records agree after removing only semantic stdout
`database=` lines and composition stdout-digest fields. Sequential fresh declared
examples then complete with the documented setup rows. BYTE_LENGTH returns one
nullable INT64 row `5, 5, 12`; the existing query-flow example returns 38. Schema,
row count, terminal status and exit are checked, with platform outputs agreeing
after pathname normalization.

The frozen 719-input manifest and read-only Linux export share SHA-256
`5eef16591fabf84ac4258ecf4abff59195e4bdc714abc89c14db572a3f9319db`.
Only the two notes files change afterward; the other 717 inputs retain fingerprint
`0295f87f8ca43ba3531816634395734d2112a65f07811e3e100e8fac7d50ae51`.
Core receipt hashes are macOS
`4216ef93a57e03e4e83e3dc9ec7e7c2aa9870ff9babc439f4e4b439da4676948`
and Linux `1ff41d7f5d4dcdef64e8f5993c94f9d24c7eaa1bf9870717200b1ddae9b39dca`.
Stock CLI hashes are macOS
`f64a84d2f46bbb1a4fd153fedfc3f688db7503607144bc4a779e2036d8c1e003`
and Linux `0fb33704bc6a3cfe18d10011f7dbfb8b67acde95c873d98bf726b307f47d3191`.

Rust 1.98.1 uses one Cargo job per platform on the 8-GiB Apple M1/macOS 26.6.2
host. Linux uses the preserved image, LinuxKit 7.0.12 aarch64, uid/gid 1000,
one CPU, 2 GiB without extra swap, disabled networking and native database storage.
Eighty-eight sustained host samples show normal/warning pressure, swap
1,126.88–1,595.69 MiB, over 187.66 GiB free disk and sampled I/O 0–163.26 MB/s.
Forty-three container samples show at most 103.69% reported CPU and 1.256 GiB
memory, zero network traffic and no OOM kill. Host en0 counters increase by
469,370,630 received and 22,360,034 sent bytes, including unrelated host activity.
These samples do not qualify physical-memory bounds or general concurrency.

Owned focused/gate targets, source export, monitoring records, logs, manifests,
tutorial databases and container are removed. No owned verification process
remains. The existing target, image and toolchains are preserved. Final
documentation verification passes 786 local links; the two notes files remain
below the retained evidence ceiling. Broader allocator-history/RSS, durability,
sanitizer and general concurrency qualifications remain open, as does Windows
implementation/runtime qualification. Nothing is pushed or published.

## Computed projection costs

`5671e4c` adds [projection_cost.rs](../examples/projection_cost.rs), with
[replay instructions](../docs/getting-started.md#compare-computed-projection-costs)
and the [evaluation explanation](../docs/execution.md#computed-projection-costs).
It compares six successive SELECT additions with the same six left-to-right
additions inside one SELECT expression. Both use the same scan/global-aggregate
producer structure and stored input: 32,768 nonnullable INT64 amounts, repeating
0 through 99 in 256-row append batches. COUNT is 32,768 and SUM after adding six
is 1,817,536. Every execution checks both literal outputs, complete termination,
zero observed temporary reservations and release to the shared logical baseline.

The source hypothesis is five extra computed batch buffers and five extra scalar
program evaluations/result copies per selection. Each 256-row buffer contains
2,048 payload bytes and 32 validity bytes. Fresh macOS and GNU arm64 Linux runs
both observe additional logical memory of 362,996 bytes for staged evaluation
and 352,596 for the single expression: exactly the predicted 10,400-byte gap.
Both prepared queries remain live, and query/temporary limits are 8,000,000 bytes.
No transient allocator, physical-memory or I/O conclusion follows from these
step-boundary logical observations.

The initial macOS trial has descending millisecond timings despite two warmups;
it is not the retained timing comparison. The final example warms each variant
twenty times and averages one hundred checked executions per sample, alternating
which variant runs first across ten sample pairs. Each platform completes 2,040
checked executions. Timing includes result construction, execution, row validation,
completion and destruction, excluding printing and the final reservation check.
No cache eviction is attempted. The table summarizes the ten per-sample averages
in milliseconds; these are not individual-query tail latencies.

| Platform | Staged minimum / median / maximum | Single minimum / median / maximum |
| --- | ---: | ---: |
| macOS | 3.465 / 3.527 / 3.559 | 3.046 / 3.056 / 3.107 |
| GNU arm64 Linux | 3.026 / 3.0365 / 3.061 | 2.369 / 2.3805 / 2.770 |

The single expression is faster in all ten pairs on each platform. Median paired
relative reductions are 12.80% and 21.66%, respectively. These observations locate
a cost in this narrow repeated workload, not a general query-speed or platform
comparison. Retain the existing owners: automatic substitution would need to
preserve each definition's demanded failure span, shared uses, conditional work,
materialization and admission. This small case alone does not repay that broader
semantic and maintenance obligation. No optimizer or admission change is retained.

An initial Clippy check rejects two indexed loops; the example now iterates its
sample pairs directly. Final warnings-denied example Clippy and optimized builds
pass on both platforms. A separately compiled caller changes only the expected
SUM to 1,817,537 and must reject the correct result: both platforms exit one with
the literal-oracle failure and no success output. Maintenance passes 99 tooling
tests, 44 independent codec fixtures and 769 local links. All 688 prior
non-Markdown inputs remain byte-identical to `85cd497`; production inputs retain
the allocation-preflight engine baseline without another core gate.

The frozen 717-input manifest and Linux source export agree, SHA-256
`342e0058eabf76168b8833cfca1a01eaee3d1246dd14a2087b7a67282d7ef53d`.
Only the notes files change afterward; the other 715 inputs retain fingerprint
`fc679ce356832c1addd46beb53c5f8d8f1e59807fdd469deb89f155c07bd0da1`.
The example source hash is
`383a0377267324256654075ce0ca6529d3b7a3087d1c69da521b4ef0a3698e5b`.
Executable hashes are macOS
`b7cd5988e1f957531b8510f318ac6df30b7a283e568b265f679e888777a1383c`
and Linux `5d528b05862a784eea172a785d4943811e286f6a8167e81325cbe52fc7d8d3df`.
Final stdout hashes, which include timings, are macOS
`1a4fa2b15338f44c73e84f7073924cd524cdb3fda186069bceec8ba55a5472ca`
and Linux `cc70cce4921ade5f7a04ea5e5e4ad994b26e0b1aec190b486782e14b5d19e77e`.

The host is an 8-GiB Apple M1 running macOS 26.6.2. Rust 1.98.1 uses one Cargo
job per platform. Linux uses the preserved image, LinuxKit 7.0.12 aarch64,
uid/gid 1000, one CPU, 2 GiB without extra swap, disabled networking and native
database storage. macOS setup/open takes 0.720269 seconds and preparation takes
0.000550; Linux takes 0.077467 and 0.000103. Whole-example process observations
include setup and all repetitions: macOS 8.05 seconds elapsed, 3.69 user and
3.14 system, maximum RSS 3,293,184 bytes; Linux 5.67 elapsed, 5.01 user and
0.57 system, maximum RSS 3,032 KiB. Linux reports 1,224 filesystem-output units;
these process counters do not measure per-query bytes or qualify resource bounds.

Four sparse host observations show normal/warning pressure, swap
1,374.50–1,382.50 MiB, over 187 GiB free disk and sampled I/O 0.06–34.17 MB/s.
The container observation shows 100.64% reported CPU, 381.3 MiB memory and zero
network traffic; inspection confirms no OOM kill. These are not peak bounds.
Owned trial/final databases, targets, control callers, exports, logs, manifests
and container are removed. Existing target, image and toolchains are preserved.
Final documentation verification passes 773 local links. Broader durability,
allocation-history, concurrency and platform qualifications remain open. Nothing
is pushed or published.

## Snapshot pins and retained outcomes

`45d1158` extends [snapshots.rs](../examples/snapshots.rs) and its
[transaction reading path](../docs/transactions.md#follow-snapshot-pins-and-retained-outcomes).
The example keeps both successful `Commit` values, explicitly aborts a separate
empty append and checks all three outcomes after later publication, reclamation,
old-plan release and close/reopen. Existing literal row checks remain. Expected
successful generations are 2 and 3; declaration publishes generation 1 and the
aborted issuance adds no data generation. Copied receipts do not pin reader data.

Sequential fresh macOS and GNU arm64 Linux runs exit zero and produce exactly
the eight documented lines, with identical stdout SHA-256
`1243874d94f0316562ef170e9898a00f8d7452fdd5a9de6620015c46b7715df3`.
Both warnings-denied example Clippy checks pass. Maintenance passes 99 tooling
tests, 44 independent codec fixtures and 760 local links; formatting and diff
checks pass. All 687 non-Markdown manifest inputs other than `snapshots.rs`
remain byte-identical to `a6a1652`. The allocation-preflight engine baseline
therefore remains applicable without repeating its gates.

The frozen 716-input manifest and Linux source export agree, with SHA-256
`d96c21ce4c4c01e769be0e9398fab503055bf2bc4d4bcc85d0e4146caca81c9f`.
Only the two notes files change afterward; the other 714 inputs retain fingerprint
`589047390b693d19615ea97c56fb1c4d864f9fa59a208ddf7a7e0e6fc85bec4b`.
The example source SHA-256 is
`fc77fe031728a8a1a70f7169f8f3b821cf2d606f1b79e3c539f27ee13b0713e1`.
Example executable hashes are macOS
`e8644a5cf227c8ed183ce8ce02da43146a8347760786667c39c57c3cd0e90182`
and Linux `cb73e806f6eb3fb25d5f152ff3244b2eeac35ea0e828d7e81e05db11f2b9d8a4`.

Both builds use Rust 1.98.1 and one Cargo job. Linux uses the preserved image,
uid/gid 1000, one CPU, 2 GiB memory without extra swap, disabled networking and
native database storage. Two sparse host observations show normal memory pressure,
swap 1,462.50–1,478.50 MiB, over 187 GiB free disk and sampled I/O 4.52–16.69 MB/s.
The container observation shows 100.20% reported CPU, 810.9 MiB memory and zero
network traffic; inspection confirms no OOM kill. These are observations, not
peaks or resource qualification. Final documentation verification passes 763 local
links. Owned examples, targets, exports, logs, manifests
and container are removed; existing target, toolchains and image are preserved.
The example establishes healthy sequential behavior, not new crash, concurrent
scheduling or power-loss evidence. Broader qualifications remain open. Nothing
is pushed or published.

## Allocation-capacity preflight

`51bd731` makes `resources::allocate` reject requested elements above their
admitted ceiling before `Vec::try_reserve_exact`. Requested-byte overflow reports
the existing maximal required-byte diagnostic. Valid geometry, allocator refusal
and the returned-capacity check retain their behavior. No allowance, allocation
owner, public API or persistent format changes. Reviewed production calls supply
equal request and ceiling values, including the legacy scan's constant aliases.
This closes an invalid-internal-geometry boundary, not a demonstrated public-query
failure. The [resource contract](../docs/resources.md#admission-protocol) separates
reservation, preflight, physical allocation and release.

The [capacity probe](../tools/fixtures/allocation-capacity.rs) includes the actual
production helper and uses the existing diagnostic allocator. A request for two
`u64` elements under a one-element ceiling reaches the old helper's allocator
once. The observer denies that attempt, and the zero-call assertion rejects the
old helper with exit 101. The final probe is checked against the `88fd4ca`
resource source and then the repaired source, using the same allocator and
unchanged public-library error type. Three repaired cases refuse before any
allocator call: two elements under ceiling one, one under zero, and overflowing
requested-byte geometry. Four controls cover an empty vector, real allocator
denial, exact capacity and a valid request below its ceiling. Returned vectors
are dropped before their reservations. The existing four resource tests also
pass; 434 unrelated library tests are filtered only in that focused run.

The first core attempt stops at maintenance because the mocked pathname campaign
supplies only its former mutex marker. It runs no ordinary Rust tests and is not
passing evidence. `7078729` updates that integration test to require both common
controls and to reject missing capacity/mutex evidence or a nonzero capacity exit
even when a success marker is printed. All fifteen campaign-interpretation tests
pass, including these additional subcases. The final core gates pass all fourteen
stages, 99 tooling tests and 44 independent codec fixtures on each platform.
Independent executable discovery confirms 640 ordinary Rust tests, including
143 catalog tests, across thirteen targets: seven nonempty and six zero-test
examples. No ordinary test is ignored or filtered. The separate lease subprocess
passes its one test with six intentional sibling filters.

Both public ownership campaigns pass eleven analytic and six wide-set shapes at
both pathname lengths, the retained wide-join lifecycle/failure observations,
overlapping owners and fourteen negative controls. Join construction still has
353 allocations: every refusal prefix 0–352 and healthy control 353 pass at both
pathname lengths. Both `--ownership-only` and `--controls-only` execute all seven
new capacity-probe cases. The latter selection passes 29 healthy filesystem,
lifecycle, load, query, catalog and recovery control cells per platform. Catalog
construction still counts 1,056 allocations. Linux's two expanded-path controls
replace the two unavailable Darwin ACL recovery cells. These healthy controls
are not another complete catalog/lifecycle allocation-prefix sweep.

The unchanged stock CLI on each platform passes 24 independent aggregate-semantic
cases and 311 composition records. Records agree after removing only semantic
stdout database-path lines and composition stdout-digest fields. Sequential fresh
declared setup and [query-flow.sql](../examples/query-flow.sql) return one nullable
INT64 row, 38, complete status and exit zero. Its demanded-overflow variant has
no row or success marker, exits one and retains the same addition span, bytes
50..80. Fresh normal/failure records agree after database-path normalization.

Replay uses the existing core gate, diagnostic-allocation `--ownership-only` and
`--controls-only` selections, then a stock CLI build, semantic/composition
commands and query-flow setup. Frozen inputs at `7078729` total 716 files, with
manifest SHA-256
`58409c1b600b0048a7bce8fe7d36695178f15774df26c6aed4469963ebdf2683`.
Both final core gates and public campaigns have matching before/after manifests;
the Linux Git export matches. Only the two notes files change afterward; the
other 714 inputs retain fingerprint
`4ed6dddde7df2a1adbd34c41b4dff6e400088179f3524b7829cf888e70744e69`.

| Record | macOS | GNU arm64 Linux |
| --- | --- | --- |
| Core elapsed seconds | 453.148 | 168.439 |
| Core receipt SHA-256 | `4150f1e6a38ac50682d439757c325a2c1f7673e093dbd298659acbd995780087` | `3d43fd691eabe83ea4ad5a79c7287794fb626fb355fadbb679d4f7b42316162d` |
| Public campaigns/build elapsed seconds | 131.270 | 118.096 |
| Public receipt SHA-256 | `6137d0b4f47af1b517f39313d107ebb5381556bfe94fb8fd21f95251ad66c2eb` | `dbba34ed5f19b0f0ed27dd968cc590fde3f6f326a05ff877813a58da124bbbf3` |
| Stock CLI SHA-256 | `9048fb525a208e0dfd8fadb03556df7930f60bac344b2d0990429020ebb7af69` | `b9fb374e503c89f07554c345053dac428ea0b1c76d8f9788b379dbe9403ce1ed` |

Verification uses Rust 1.98.1, one Cargo job per platform and the preserved image
`sha256:520be9ff830f944e49a3319cbf6f8ccfb2c1f21631947de50290efb98038e282`.
Linux uses uid/gid 1000, one CPU, 2 GiB memory without additional swap, networking
disabled and native database storage. Ninety-seven host samples observe normal/
warning pressure, swap 1,450.94–2,342.50 MiB ending at 2,189.62 MiB, over
186.85 GiB free disk and sampled I/O 0–187.67 MB/s. Twenty-three container samples
reach 104.4% reported CPU and 1.208 GiB memory, with zero network traffic and no
OOM kill. These observations do not qualify engine admission or whole-process
bounds. Owned logs, manifests, source exports, targets, databases, monitoring and
container outputs are removed; existing target, toolchains and image are preserved.
Final documentation verification passes 750 local links.

This is scoped verification. Earlier native/persistence and full allocation-prefix
evidence retain their original inputs and limits. The combined native usable-heap
deficit, arbitrary allocator/concurrency and whole-process/RSS bounds, broader
durability, Windows and general sanitizer/race qualification remain open. Nothing
is pushed or published.

## Query execution walkthrough

`7450500` connects the preparation example to a concrete
[execution trace](../docs/execution.md#trace-a-query-through-execution).
[query-flow.sql](../examples/query-flow.sql) preserves the existing query:
rename `amount` to `subtotal`, compute `subtotal + 1`, then sum the result.
The guide follows its two physical producers, backwards input requests, parent
handoffs, retained batches, global result checking/emission and terminal cleanup.
It distinguishes those events from exact step counts and keeps implementation
entry points beside the explanation. No engine instrumentation or API changes.

Sequential fresh macOS and GNU arm64 Linux stock builds and declared-table setup
pass. Running the exact SQL through the CLI with 16,000,000 bytes memory and
8,000,000 temporary capacity returns one nullable INT64 row, 38, followed by
`row_count=1`, `status=queried` and exit zero. This agrees with the independent
sum 11 + 21 + 6; the fourth input remains NULL. Replacing only `subtotal + 1`
with `subtotal + 9223372036854775807` produces a schema but no result row or
success marker, exits one and reports addition overflow at bytes 50..80. The
byte slice independently resolves to the complete demanded addition. Both
platforms' normal/failure records agree after removing only the database path.
Setup and replay commands remain in the
[query guide](../docs/query-examples.md#follow-one-query-from-names-to-results).

The SQL SHA-256 is
`9a18d9015a08462eef80875684e1be48e98c2fb69e6aaddd48f52f953ecca477`.
Build plus setup and both queries take 23.762 seconds on macOS and 21.133 seconds
on Linux. Stock CLI hashes are
`fd25e134c7a1e0667962073df1af86ee8d6c7e6be76b3976765f28e1a8a1951c`
and `449425181d66379d64688fdda495a0cf574f30dafe5b6f1da9b3db01c45718f2`,
respectively. All 695 prior non-documentary inputs remain byte-identical to
`5975b71`; the only new executable example input is this SQL file. Its verified
engine baseline therefore remains applicable without another core gate.
Maintenance passes 99 tooling tests and 44 independent codec fixtures.
Final documentation verification passes 746 local links.

The 715-input manifest is
`f8ec598d4bf9f18fe1a1ceb9b850c8f95d61d185ac94caa4d69d095c2e07e1e3`;
the native Linux export matches exactly. Only the two notes files change after
verification; the other 713 inputs retain fingerprint
`519a0769f60542119cf4e26d129e563d67f98025538fc0bf67391ddcc7334335`.
Both builds use the pinned Rust 1.98.1 with one Cargo job. Linux retains the
preserved image, uid/gid 1000, one CPU, 2 GiB limit, networking disabled and native
database storage. Four host snapshots observe normal/warning pressure, swap
1,595 MiB and over 188.24 GiB free disk; sampled I/O ranges from 0.19 to
4.61 MB/s. Two container snapshots observe up to 674.3 MiB and zero network
traffic; no OOM kill occurs. These sparse observations do not establish peaks
or memory bounds. Owned targets, databases, source export, logs and container
are removed; existing targets, image and toolchains are preserved. Nothing is
pushed or published, and broader qualifications remain unchanged.

## Explicit producer admission

`5975b71` replaces the classification chain and implicit aggregate fallthrough
in `Runtime::open_native` with `Owner::admit_native`, an exhaustive match on
`Producer`. The interface admits memory and controllers without an effects
recorder. The runtime operation visibly reserves catalog scratch, collects all
owners, prepares aggregates, then opens pending sources. Each non-scan output
still precedes its controller allocation. Normal construction and a later
controller refusal were reviewed through ownership transfer and physical drop:
earlier owners release before source opening, and the runtime reservation outlives
transferred controller storage. No public API, payload allocation, admission
allowance, execution state or persistent format changes. The
[execution reading path](../docs/execution.md#producer-execution) explains the
decision and phase boundary beside the real implementation.

Focused execution verification passes 124 tests, with 314 unrelated library
tests deliberately filtered. Subsequent complete core runs discover and execute
640 ordinary Rust tests per platform, including 143 catalog tests, with no
ordinary ignored or filtered tests. The separate lease subprocess passes one
test with its six intentional sibling filters. Independent executable listing
confirms 13 targets: seven nonempty and six zero-test examples. Both 14-stage
core gates also pass warnings-denied compilation, rustdoc/doctests, 99 tooling
tests, 44 independent codec fixtures and 723 local links. Existing exact-minimum,
one-byte-short before-I/O refusal, independent capacity/lifetime, cancellation,
replay, demanded-error and small-stack regressions remain passing.

Sequential public ownership campaigns pass on both platforms: eleven analytic
shapes and six wide-set shapes at both pathname lengths; complete wide join,
preparation/execution failure, lifetime, abandonment and overlapping-owner
checks; and all fourteen negative controls. Construction retains its census of
353 allocations: refusal prefixes 0–352 and healthy control 353 pass at both
pathname lengths. Minimum sampled requested/usable construction headroom is
4,096/1,400 bytes on macOS and 4,096/3,648 bytes on Linux. These samples retain
their workload-specific meaning and do not establish arbitrary-history bounds.

The unchanged stock CLI on each platform passes all 24 independent
aggregate-semantic cases and 311 composition records. Cross-platform records
agree after removing only database-path output lines from semantic stdout and
ambient stdout-digest fields from composition records. Sequential fresh declared
and LEFT JOIN examples produce their documented complete totals; fresh
`window-count.sql` and `repeated-regions.sql` queries each return the expected
three rows, successful exit and `status=queried`. Their exact row records agree.

Replay uses `tools/check.py --scope core --output NEW_DIRECTORY`, followed by
`tools/check-diagnostic-allocation.py --ownership-only`, a fresh stock
`cargo build --release --offline --locked --workspace --bins --examples`, and
the [independent semantic/composition commands](../tools/README.md). Fresh
examples use the [declared setup](../docs/query-examples.md#set-up-the-sales-table)
and [LEFT JOIN setup](../docs/getting-started.md#retain-facts-with-missing-dimensions).
The frozen 714-input manifest SHA-256 is
`7d54612cc5f4e7d46901ae39402cdc87c52ab7fd6e041684e5394ed12f558403`.
Both core gates and public campaigns retain matching before/after manifests,
including an exact Git export for Linux. Only this record and the plan change
afterward; the other 712 inputs retain fingerprint
`7546b227efa75378499c0c3f4b98ddf924c59b2f6d163097e906d5d2bc23a18a`.

| Record | macOS | GNU arm64 Linux |
| --- | --- | --- |
| Core elapsed seconds | 456.274 | 154.641 |
| Core receipt SHA-256 | `0097badf33e7df3474eb8c59156d9e4c3aa3a4a7923ecd75b7b6defe260c9cb5` | `5ac6420eaa3332e0dcab222a10685ed4799cf34dc1b438a7e8915c0f24b5ee80` |
| Public campaigns/build elapsed seconds | 101.475 | 97.065 |
| Public receipt SHA-256 | `81c59acda87f529dbaad2266048cc392a32059b14b13a2aeed0aa6e713a17451` | `7d7e3d21046636724682b4ef6d27f4eb3e27605817e4a949df830a53cf6fee67` |
| Stock CLI SHA-256 | `dba6b3a91dc3ef2a0df34697fcaf6fbf41a1a62fb82f4beaa0e251814e8fee3b` | `449425181d66379d64688fdda495a0cf574f30dafe5b6f1da9b3db01c45718f2` |

Verification uses Rust 1.98.1, one Cargo job per platform, macOS arm64 and the
preserved GNU arm64 image
`sha256:520be9ff830f944e49a3319cbf6f8ccfb2c1f21631947de50290efb98038e282`.
The Linux container uses uid/gid 1000, one CPU, 2 GiB memory with no additional
swap, networking disabled and native database storage. Fifty-eight host samples
observe normal/warning memory pressure, swap 1,249.94–1,643.00 MiB ending at
1,627.00 MiB, over 187.90 GiB free disk and sampled disk I/O 0–137.21 MB/s.
Nineteen container samples reach 100.58% CPU and 1.297 GiB memory, with zero
network traffic and no OOM kill. These are resource observations, not engine
admission or whole-process bounds. Owned logs, manifests, exports, targets,
databases, monitoring and container outputs are removed; the pre-existing target,
toolchains and verification image are preserved.
Final documentation verification passes 728 local links.

This is scoped verification, not another complete 24-stage checkpoint. The
unchanged native/persistence campaigns retain their earlier evidence. The strict
combined-allocation-history deficit, arbitrary allocator/concurrency and
whole-process/RSS qualification, broader durability, Windows and general
sanitizer/race coverage remain open. Nothing is pushed or published.

## Native allocation reuse

`0c78252` adds the independent native diagnostic. The ordinary Rust measuring
allocator forwards allocation layouts unchanged to System and samples after
allocation/before physical free. It adds no headers,
padding or per-allocation payload. The independent native C control uses only
`malloc`, `free` and the platform's usable-size query. Its larger seed is a
controlled input matching an observed extent, not a complete reconstruction of
the original query history.

Three fresh macOS cold cells each request 3,817,440 bytes and observe 3,817,472
usable bytes. Three reuse cells first allocate/free 3,899,392 bytes, then request
3,817,440; each receives the same address and all 3,899,392 usable bytes. Thus a
controlled native reuse history reproduces the observed oversized allocation
without the engine or Rust observer. It supports that explanation for the query's
extent but does not identify its original freed block or explain every byte of
its aggregate deficit. Three invalid-invocation controls reject, and closing
stdout rejects with exit one instead of reporting success after a failed flush.
The inherited macOS environment sets `MallocNanoZone=0`. Repeating all six native
cells with that variable unset produces the same extents and address-reuse results.
This checks those two configurations, not every allocator setting.

The unmodified combined-history caller still fails on macOS with exit 101,
349 allocations, nine frees, requested headroom 16,528 and usable headroom
-219,998 bytes. The guard remains strict. No engine or observer defect has been
demonstrated by this reduction, and no admission allowance changes. GNU/Linux's
combined history completes with the full eleven-pair result. Its three cold C
cells report 3,817,456 usable bytes; three seeded cells report 3,817,448 bytes and
no address reuse. Each seed occupies 3,903,472 usable bytes. Invalid-invocation
controls reject on both platforms; a full output sink on Linux returns exit one.
These are observed allocator configurations, not fixed cross-platform constants.

Both native callers compile with the documented C11, optimized, warnings-denied
command; macOS uses Apple clang 21.0.0. The experiment runs sequentially on
macOS arm64 Darwin 25.6.0 and GNU
arm64 Linux 7.0.12-linuxkit/glibc 2.36. The stock Rust caller uses Rust 1.98.1 and
unchanged production/observer inputs; no engine or existing caller test source
changes from `df0cace`. Maintenance passes 99 tooling tests, 44 independent codec
fixtures and 719 local links. Final link verification passes 721 links. No broader
runtime gate is repeated for this optional independent diagnostic.

The 714-input manifest and exact Linux source export have SHA-256
`f65368a0c86ebe37745107f75c8b7d7ff7b164ed4ed13ed68c9326eba9efe713`.
Only the two notes files change afterward; the other 712 inputs retain fingerprint
`ccc09b8ca69c8a1be82538e148366c0f0c3a23fa9a9de1c92f316aa8f0592fee`.
The native C source has SHA-256
`516d5904364a01217dda1835a17ead60bf4a2da6f4fb0ff14d4b7722409db013`.
The maintained [replay commands](../tools/README.md#isolate-native-allocation-reuse)
reconstruct both controlled histories; the separate combined-query command keeps
its original failure condition. The first native reduction completed within the
initial 30-minute investigation.

Twenty-two host samples observe normal and warning pressure, swap between
1,345.94 and 1,433.94 MiB, ending at 1,345.94, and at least 188.43 GiB free disk.
Sampled disk I/O ranges from 0.02 to 57.71 MB/s. Rust builds use one job. Linux
uses uid/gid 1000, one CPU, a 2 GiB memory/memory-plus-swap limit and no network.
Twenty container samples observe up to 100.07% CPU, 846.6 MiB and zero network
traffic. No monitoring command fails and no container OOM kill occurs. These
observations include unrelated host activity and do not qualify engine/RSS bounds.
Owned probes, stock builds, databases, source export, logs, monitoring outputs
and the container are removed. The existing target, toolchains and verification
image remain. Broader allocator, physical-memory and RSS qualification remains
unresolved.

## First tutorial and optional queries

The first declared-table walkthrough now reaches cleanup directly after its main
example. Seventeen optional query sections move intact to
[the query guide](../docs/query-examples.md), which creates its own sample database
and cleans it up independently. The documentation index and current inbound links
follow that owner; the other independent workloads remain in place.

The moved collection matches all 22,986 bytes from `c35003d`, including query
commands, expected results, reading paths and qualifications. Its SHA-256 is
`8bc60d8c9e06ddf3aa6b2fe7460659d829d66e6138d24cf63fe25f14a237886c`.
Only Markdown changes; executable inputs remain unchanged from `df0cace`.
Its scoped platform evidence below is retained without a new full-gate claim.

Fresh macOS setup runs for the first tutorial and query guide both return the two
documented region totals. EXTEND and partition count each return all three
expected rows with exact row counts, successful exit and `status=queried`.
Both temporary database directories are removed by the documented cleanup command.
The isolated stock build and walkthrough take 46.751 seconds with one Cargo job.
Two host observations show normal pressure, swap use of 1,561.94 then 1,553.94 MiB,
and at least 188.46 GiB free disk. These samples are not peak resource bounds.
Maintenance passes 99 tooling tests and 44 codec fixtures; final documentation
verification passes 713 local links. Owned outputs are removed; the pre-existing
target and installed toolchains remain. No runtime campaign is repeated for this
movement of unchanged examples.

## Blocking controller lifetimes

`df0cace` repairs order and sorted-set controllers, which had the same inline-charge
lifetime mismatch as joins: their reservations belonged to fields inside the allocation they
charged. The runtime now retains those existing charges after construction and
until physical controller release. Order's shared constructor covers ORDER BY,
DISTINCT, UNION DISTINCT and retained-row partition count. The runtime forwards
its reservation through declared admission and legacy partition-count admission.
No allowance, payload allocation, persistent format or
operator algorithm changes.

The existing physical-capacity regressions independently require a 3,000-byte
order transfer or a 6,096-byte sorted-set transfer on the tested 64-bit target,
alongside actual buffer capacities at every retained transition. A control built
from `59cd4d9` with only the two updated test files rejects the old implementation:
one order check and three sorted-set checks fail with zero transferred bytes;
the other 23 blocking tests pass. With the repair, all 27 blocking tests pass
sequentially. These focused runs intentionally filter the other 411 library tests.
The control is reconstructable from that retained production revision and the
test changes; no source archive is required.

Both platforms pass the 14-stage core gate, the public allocator caller's
`--ownership-only` selection, and the independent aggregate-semantic and composition
campaigns against one stock CLI per platform. These are scoped checks, not a
complete 24-stage gate. The [tool map](../tools/README.md) owns the commands;
`cargo build --release --offline --locked --workspace --bins --examples` builds
the CLI and fresh example artifacts with warnings denied. macOS arm64 Darwin
25.6.0 and GNU arm64 Linux 7.0.12-linuxkit use Rust 1.98.1 and offline locked inputs.
Linux uses uid/gid 1000, glibc 2.36 and native overlay database storage.

The 712-input manifest is
`a83d43847f75d0ba43cf178f45ecc4184f561cd8f71893db9a09d16698530f78`.
The exported source and every before/after manifest agree. Only the two notes
files change afterward; the other 710 inputs retain fingerprint
`6409dfe52a513c8a9b58eecdffddbd69d3fba02b615cacdd1b662ef01d33f8ba`.
Each platform executes all 640 ordinary Rust tests, including 143 catalog tests,
with none ignored or filtered. Independent discovery lists seven nonempty targets
and six zero-test examples. The separate lease child passes with its six
intentional filtered siblings. Maintenance passes 99 tooling tests, 44 independent
codec fixtures and 700 local links. Final documentation verification passes 703
local links.

At both pathname lengths on both platforms, public ownership retains all eleven
analytical shapes, six wide set cases, complete join results and lifecycle
checks. All fourteen negative controls reject. Join construction retains its
353-allocation census, prefixes 0–352 and healthy control 353. Minimum requested
and usable headroom remains 4,096/1,400 bytes on macOS and 4,096/3,648 on Linux.
Preparation refusals, demanded arithmetic failures, overlapping owners and the
other retained ownership shapes also pass. This does not add exhaustive
allocation-refusal coverage for ordering or sorted set construction. Their
independent transfer/capacity checks, exact minima, failure/cancellation paths and
successful public workloads establish the repaired enclosing lifetime.

The 24 semantic records and 311 composition records agree across platforms after
removing only ambient database output paths and composition stdout digests. All
expected failure outcomes remain part of that comparison. Core stage times total
397.678 seconds on macOS and 159.324 seconds on Linux; public ownership, stock
build and semantic/composition checks total 108.880 and 94.240 seconds respectively.
Core receipt SHA-256 values are
`4a902dc5d3c66c2a51894bec251c18b17016a83f285cedc19dcb3d9151e2492c` and
`a908b608bf88f185df15f25ae47ba11a4e176063ca2da8805d1da5fd33ce2e2c`.
Public-check receipt SHA-256 values are
`a00833d16d8cbdbd423ca99116db0e2e29be3413fa87c6be655312e96e3331a8` and
`5ee6a68a1f2badd8f8f1738958fdbc2e92a31e2b7fce7159a340a9bb9aee9cfa`.
These elapsed times are verification costs, not performance benchmarks.

After both platform checks, fresh examples run sequentially on macOS and Linux.
Declared-table setup and LEFT JOIN complete with their documented rows. Partition
count returns `(north, 5, 3)`, `(north, 10, 3)` and `(south, 20, 3)`; repeated-region
difference returns NULL, 1 and 3. Both CLI queries require exact row counts,
`status=queried` and successful exit. The [reading path](../docs/query-examples.md#count-the-complete-input-beside-each-row)
connects this behavior to controller storage and independently owned sort buffers.

Forty resource samples observe normal and warning host pressure. Swap ranges
from 1,432 to 1,648 MiB and ends at 1,432. macOS uses at most two Cargo jobs;
Linux uses one CPU, one job, a 2 GiB memory/memory-plus-swap limit and disabled
networking. Eleven container samples observe CPU up to 100.12%, memory up to
1.214 GiB and zero network traffic. Free host disk remains above 188.34 GiB;
sampled disk I/O ranges from zero to 131.95 MB/s. Completed development builds
are removed before platform verification. Monitoring commands have no failures
or timeouts, and no container OOM kill occurs. These observations include other
host applications and do not qualify engine admission, usable heap or RSS.

Owned builds, exports, databases, logs, monitoring outputs and the verification
container are removed. The pre-existing target, installed toolchains and preserved
verification image remain. Native effect wrappers and persistent publication are
unchanged; their prior full-gate evidence remains attached to its exact inputs.
The combined-history usable-heap counterexample, broader durability, concurrency,
sanitizer, physical-memory and Windows qualifications remain unresolved.

## Full verification checkpoint

This complete checkpoint predates the blocking-controller extension above;
changed source requires its own verification record.

Both complete 24-stage gates verify the 712 frozen inputs retained in `1e5cfdc`
on macOS arm64 Darwin 25.6.0 and GNU arm64 Linux 7.0.12-linuxkit. Both use Rust
1.98.1, release artifacts, locked offline builds and warnings-denied compilation
and documentation. Linux uses uid/gid 1000, glibc 2.36 and native overlay storage
with an exact Git source export. Input manifests match before/after and across
gates: `e1b8ba30a2d7e9ab3b3bd1299a365b852cefddcffbf251725ff5bc8359fe943a`.
Finalization changes only the two notes files. The other 710 inputs retain
fingerprint `2ecfe1b669db279c217f9f34b6c641d6b5dedc0102630c0e8da885266d11fd00`;
all manifested inputs are tracked. Final local-link verification passes 699 links.

Each platform executes 640 ordinary Rust tests, including all 143 public catalog
tests, plus the separate lease subprocess. Discovery independently lists those
640 tests across thirteen targets per platform: seven nonempty test targets and
six examples with no tests. No ordinary test is ignored or filtered; the selected
lease child reports six filtered siblings. Maintenance passes 99 tooling tests,
44 independent codec fixtures and 698 local links. Independent aggregate semantics
pass 24 cases and composition passes 311 scenarios. Their complete records agree
across platforms after excluding ambient database paths and composition stdout
digests. Those digests are not portable semantic hashes; expected nonzero
semantic-case exits remain part of the comparison.

Both allocation campaigns retain positions 0–1,055 plus healthy control 1,056
at both short and 384-byte database paths. All four ordered lists were reconciled
explicitly. The caller's work ceiling remains 1,100. The retained preparation
census and complete prefixes 0–10 agree in both fresh ownership callers at both
lengths on both platforms. Join construction adds a census of 353: prefixes
0–352 refuse and prefix 353 succeeds, at both pathnames on both platforms.
Minimum requested/usable construction headroom is 4,096/1,400 bytes on macOS
and 4,096/3,648 bytes on Linux. The missing-observer control rejects, and every
successful caller retains the full 11-pair, 64-column healthy result and lifecycle
checks. The combined-history diagnostic remains separately unqualified below.

Both platforms pass 552 CLI allocation prefix cases. Native initialization passes
30 macOS and 80 GNU/Linux cells; synchronization passes 241 cells and I/O passes
1,394 cells per platform. Interruption retains 76 append cuts, 46 recovery cuts
and 249 independent graph checks. All 43 graph cases, two oracle controls, three
CLI limits, genesis, lease contention and independent column order pass. Linux
retains two Darwin ACL repair-rename exclusions, one at each pathname length.

Both receipts have zero finalization errors. The full gates run sequentially;
stage times total 1,598.832 seconds on macOS and 512.610 seconds on Linux. Receipt
SHA-256 values are respectively
`7b80e44ae6cfcf71c5386a8bd54954e366b22f938f72f9a332b4f5ec6dee5d16` and
`db7af84381c3857663003f56ea4f2b159cb4cd93ed5445e583fb3e4180607172`.
These runs are verification observations, not performance benchmarks.

One hundred seventeen periodic resource samples observed normal and warning
memory pressure on an 8 GiB host. Swap use ranged from 737.69 to 1,562.88 MiB,
ending at 1,546.88. macOS used at most two Cargo jobs and one for fresh examples
and diagnostics. Docker used one CPU, one build job and a 2 GiB memory and
memory-plus-swap limit. Twenty-nine container samples observed CPU up to 101.49%
and memory up to 1.260 GiB. Networking was disabled and sampled container network
traffic was zero. Host disk samples ranged from zero to 143.99 MB/s; free disk
stayed above 188.22 GiB. Completed development builds were removed during warning
pressure; platform work remained sequential. Host process, disk and network
observations include unrelated applications. Monitoring commands reported no
failures or timeouts, and no container OOM kill occurred. These observations do
not qualify engine admission, usable-heap limits or whole-process/RSS bounds.

The Linux image remains `pipesql-verification-rust:1.98.1-time`, digest
`sha256:520be9ff830f944e49a3319cbf6f8ccfb2c1f21631947de50290efb98038e282`.
Fresh LEFT JOIN examples run sequentially on macOS and Linux after both complete
gates. Each creates and reopens its database, completes the query and returns
`unmatched total=90 rows=2`, `north total=30 rows=2` and `south total=30 rows=1`
in that order with successful exit. Owned gate outputs, source exports, logs,
monitor, databases, isolated build outputs and the verification container are
removed. Pre-existing target artifacts, the image and installed toolchains remain.
Windows, broader durability, physical-memory and sanitizer qualification remain
unfinished.

### Failed wide-join construction

The initial 64-column nullable STRING LEFT JOIN execute-only census observed
353 allocations. On macOS, refusing prefix 345 returned a typed filesystem-path
out-of-memory error and fully released construction at return, but the scoped
observer found requested headroom -1,344 and usable headroom -2,760. The first
deficit occurred before freeing the 5,992-byte join controller vector, after its
nested inline reservations had dropped. Entry/exit reconciliation alone missed
this lifetime error.

The repair transfers existing controller and sorter inline charges to the runtime
reservation, which outlives the node and controller vectors. It adds no allowance
or payload allocation. Run-buffer release returns only freed payload capacity;
inline charge can remain local or belong to the runtime. The bounded prefix
fixture keeps preparation outside fault injection, checks typed live errors and
physical-free ordering, and retains the complete 11-pair healthy oracle. Focused
short/384-byte sweeps and their missing-observer control pass, followed by both
complete gates and fresh examples above. An initial library run used default
parallel test threads and failed the process-global descriptor oracle in
reclamation; that run is not passing evidence. Its join-account oracle also exposed the moved inline
charge and now checks both the runtime transfer and independent physical sizes.
All seven join tests and the selected reclamation test then pass in the retained
sequential mode. Maintenance passes 99 tooling tests, 44 independent codec
fixtures and 698 local links. The sequential test mode remains required.

A second experiment combined the refusal sweep with the existing changed-width
LOG10 failure query in one caller. Requested ownership remained covered, but
Darwin usable headroom reached -219,998: one 3,817,440-byte request occupied
3,899,392 usable bytes. This allocator-history counterexample remains unresolved;
no hard usable-heap/RSS bound is claimed. Construction and demanded-error gates
retain separate fresh callers. The strict combined history remains replayable as
`wide-left-join-construction-sequence`, with instructions in the
[tool map](../tools/README.md). It is a diagnostic with no passing qualification
claim. A fresh post-gate macOS replay fails with exit 101, 349 allocations,
nine frees, requested headroom 16,528 and usable headroom -220,030 bytes. The same
fresh Linux history completes with exit zero; that observation does not establish
a general bound. The affected ordering and sorted-set lifetimes are repaired in
the blocking-controller extension above.

### Bounded power expressions

`38f2332` adds two-argument POW and its POWER alias through the existing binary
call frames, binder, independent validators, batch scratch and demanded cursor.
Both numeric inputs promote to DOUBLE before the power operation. NULLs remain
NULL even for unity or zero-exponent identities. Explicit exceptional-value
branches handle NaN precedence, infinities, signed zeros and represented exponent
parity before finite native power. Finite domain and overflow failures retain
operation `power` and owned source spans. No allocation owner, scratch allowance,
persistent format or admission padding changes. The
[language contract](../docs/language.md#current-public-query-manifest) owns the
complete accepted form and approximate-precision limits.

The pinned GoogleSQL signature, alias, compliance cases and kernel were inspected
and agree on the accepted behavior. No upstream execution is claimed. Eleven
finite references use 100-digit Decimal powers of the exact binary64 inputs,
rounded once to binary64, with a two-ULP regression threshold. Forty-five literal
vectors check identities, NaNs, infinities, parity, signed zeros and underflow;
nine failure vectors distinguish finite domain errors from overflow. Batch and
cursor paths both execute these checks, including NULL lanes with invalid hidden
power payloads. An initial fixture supplied nonfinite
literals and failed the existing validator; typed-column inputs corrected the
fixture without weakening literal validation.

Public checks exercise both names, mixed numeric types, conversion beyond 2^53
and at INT64 extremes, NULL operands, malformed calls,
INT64-only refusal, exact program bounds and prior argument errors. Demand checks
retain discarded expressions, Boolean/COALESCE short-circuiting, SAFE_DIVIDE
argument errors and owned UTF-8 spans after caller source and query release.
Grouped, joined and set results, empty aggregates, cancellation, early drop and
stored exceptional values across producers and reopen remain checked. A complete
compounding query compares four results with independent exact rational answers,
using a six-ULP bound for that query rather than a universal precision promise.

Focused release verification passes 29 scalar, 48 binder, six replay and 143
public catalog tests. Replay includes 34 literal-result fallback variants. Clippy
passes with warnings denied. Allocation development controls pass, including
fixed-buffer power error/cause formatting under denial; these controls alone
make no refusal-sweep claim. Both complete gates above execute the full retained
schedules. The catalog query now evaluates a finite power inside LOG10 while
retaining its count of two. The native I/O query also evaluates a finite power
while retaining its independent aggregate answer of nine. Both catalog censuses
remain 1,056 and native I/O coverage remains 1,394 cells per platform.

The [compounding tutorial](../docs/query-examples.md#compound-a-rate-over-several-periods)
uses an initial quantity of 1,000 and three periods at rates of 5, 10 and 20 percent.
Fresh runs on both platforms return nullable INT64/DOUBLE columns and exactly
three rows in order: 1157.6250000000002, 1331.0000000000005 and 1727.9999999999998.
Observed DOUBLE bits are `4092168000000001`, `4094cc0000000002` and
`409affffffffffff`. Schema, row count/order, decimal/bit agreement, the independent
mathematical bounds and successful completion are checked. Matching observations
do not promise bit-identical repeated or cross-platform powers or exact decimal
money arithmetic. Owned outputs are removed; broader platform, durability,
concurrency and physical-memory qualifications remain unfinished.

### Demanded execution failure ownership

`1145480` observes late arithmetic failure through the maintained public wide
nullable STRING LEFT JOIN caller. The owner trace found no accounting or release
defect: replacing the running result drops runtime buffers and temporary owners
before their charges; clearing its physical plan then leaves only the result's
handle reservation. The [runtime contract](../docs/resources.md#runtime-and-result-admission)
owns that flow. Production behavior and admission allowances are unchanged.

Three histories demand `LOG10(ABS(r.id-3))` directly, as a SAFE_DIVIDE argument,
and as a COALESCE fallback after NULLIF returns NULL. Replacing one STRING payload
keeps 64 output fields. Caller SQL is freed before execution; the inline error
retains operation `base-ten logarithm` and its exact UTF-8 byte span through
repeated failure, extraction and prepared-query release.

At both pathname lengths on both platforms, every history fails on public step
586 after observing 11,604,603 temporary bytes. Each failing call has zero
allocations and 334 observed frees. Its minimum requested headroom is 16,936
bytes; usable headroom is 14,136 bytes on macOS and 15,944 on GNU/Linux. Heap and
descriptors already equal the post-preparation baseline when the call returns,
temporary charges are zero, and the result retains exactly its handle-size
charge. These are measurements of the exercised histories, not fixed platform
constants or universal allocator bounds.

Repeated failed steps and error extraction have no heap events. Repetition
retains the same owned error and charges; extraction releases the handle charge.
Prepared-query release observes three frees, with minimum requested/usable
headroom of 12,432/11,048 bytes on macOS and 12,432/12,432 on GNU/Linux. Heap,
descriptors and reservations then match the original resident baseline while the
error remains live, after fixed-buffer formatting and after error release.

The scoped observer remains unchanged and is fresh for each public step. A new
negative control observes construction but omits step observation; missing
failure frees reject it. Fourteen focused campaign-oracle tests include missing,
duplicate, reordered and invalid histories, absent external work, missing frees,
negative headroom and step bounds. The complete focused ownership selection
passes at both pathname lengths before the full gates. Existing allocation/free
calibration, preparation failure prefixes, lifecycle and attribution controls,
independent nonheap equations and all eleven literal healthy pairs across 64
columns remain checked.

Fresh [LEFT JOIN examples](../examples/left_join.rs) on both platforms return
`unmatched total=90 rows=2`, `north total=30 rows=2` and `south total=30 rows=1`
in that order. These successful complete results accompany the failure evidence;
partial output would not establish success. Owned outputs are removed. Exhaustive
execution-allocation refusals, arbitrary allocators and concurrent histories,
whole-process/RSS, Windows and broader durability/sanitizer qualification remain
outside this completed boundary.

### Base-ten logarithms

`0f4c3b3` adds one-argument LOG10 through the bounded unary parser, binder and
independent validators. INT64 promotes before evaluation, NULL propagates and
finite nonpositive inputs produce `ArithmeticDomain` with operation
`base-ten logarithm` and an owned source span. Negative infinity produces the
profile's quiet NaN; positive infinity and input NaN bits retain their specified
behavior. The shared logarithm kernel keeps LN's existing domain decisions.
There is no new allocation owner, scratch buffer, admission allowance or persistent
format. The [language contract](../docs/language.md#current-public-query-manifest)
owns exact behavior, unsupported forms and approximate-precision limits.

Pinned GoogleSQL prose, signature, compliance cases and kernel agree on these
exceptional values. They were inspected, not executed upstream. Eleven independent
finite LOG10 references use Python Decimal at precision 100 on the exact binary64
inputs. The regression threshold is two ULPs for those values, without a universal
accuracy promise. Five exact vectors check unity, infinities and NaN bits; five
finite nonpositive inputs must error. Both batch evaluation and the demand cursor
execute these checks, including a NULL lane with a hidden negative payload.
LN retains its own literal reference values through the shared test driver.

Public checks retain INT64 promotion boundaries, rejected types and arities,
INT64-only argument refusal, skipped Boolean/COALESCE branches, demanded argument
errors, UTF-8 byte spans after source release, empty aggregates and composition
through grouping, joins and sets. A complete power-ratio query checks four literal
Decimal references with a three-ULP regression threshold and exact positive zero.
Shared tests cover stored exceptional bits across producers and reopen,
cancellation, early drop, independent descriptor rejection and 31/32-call program
admission. Forced grouping fallback now exercises 33 literal-result variants.

Catalog allocation retains its count of two while LOG10(1) contributes zero.
Native I/O retains its aggregate answer of nine while LOG10(n/n) contributes zero.
The complete frozen campaigns preserve their independent refusal schedules,
recovery checks and healthy controls. Fixed-buffer diagnostics also render the
new operation and its matching cause kind while allocation is denied.

Focused release checks pass 26 scalar, 141 public catalog, 48 binder and 13 replay
tests. Initial development runs included an incorrect hand-transcribed power-ratio
reference and a broad debug-profile selection that aborted with stack overflow;
neither contributes passing evidence. Decimal calculation corrected the reference
before release verification. Eleven temporary test directories left by the debug
abort were identified and removed. The complete release gates above pass all
ordinary tests and bounded-stack scenarios; debug-profile qualification is not
claimed.

The [decibel tutorial](../docs/query-examples.md#express-a-power-ratio-in-decibels)
uses amounts 5, 10 and 20 as power measurements relative to 10 in the same units.
Both fresh platform runs return nullable INT64/DOUBLE columns and exactly three
rows: -3.010299956639812, positive zero and 3.010299956639812. Observed DOUBLE bits
are `c008151824c7587f`, `0000000000000000` and `4008151824c7587f`. Schema, complete
row order, row count, decimal/bit agreement and successful completion were checked.
These matching observations do not promise bit-identical repeated or cross-platform
logarithms. Owned examples and verification outputs are removed; publication
restrictions are unchanged.

### Failed preparation ownership

`7eee0be` extends the maintained wide nullable STRING LEFT JOIN caller to failed
preparation, using the existing allocator harness and scoped observer. No new
accounting defect was exposed. Engine behavior, admission allowances and
persistent formats are unchanged; the binding owner now explains why its shared
reservation precedes partial plan and descriptor locals. The
[preparation contract](../docs/resources.md#query-preparation) owns that flow.

Each platform and pathname length observes a ten-allocation healthy preparation
census. Prefixes 0–9 each return a typed allocation failure; prefix 10 succeeds
without a refusal. For each nonzero refused prefix, every successful allocation
has a corresponding observed free before preparation returns. Heap and
reservation counters must already match the resident baseline while allocation
refusal remains armed. The caller then suspends faults for descriptor enumeration
and reporting, without another engine operation. The error stays live through
that complete reconciliation, followed by another release check after its drop.
Prefix zero intentionally has no successful events and reports no headroom sample.

A constant predicate, `l.id > EXP(1000)`, fails after the join's partial plan and
null-extension descriptor have been allocated. All ten allocations are freed,
and the error retains operation `exponentiation` and exact span 80..89 after the
caller's source text has been freed. Error formatting and subsequent release
also pass. The complete healthy join then returns all eleven literal pairs and
checks every one of its 64 columns. The retained normal preparation, execute/step,
finished release and two abandonment observations also pass.

The minimum requested/usable headrooms over nonzero refused prefixes are shown
below in bytes. The late arithmetic error observes the same minima for each
platform/path pair. These are aggregate observations of the exercised histories,
not per-pointer attribution or universal allocator bounds.

| Path | macOS headroom | GNU/Linux headroom |
| --- | --- | --- |
| Short | 7,938 / 7,920 | 8,016 / 7,992 |
| 384 bytes | 7,382 / 7,296 | 7,382 / 7,368 |

A negative control suppresses observation of failed prefix one and must fail
coverage even though its returned resources reconcile. The supervisor checks the
complete ordered prefix trace, real refusal records and the healthy final
control. Its independent interpretation tests reject missing census/completion
markers, omitted zero or full-prefix cases, duplicates, reordering, absent
refusals and a mismatched control. The existing hidden-allocation/free-only
calibration, false-attribution controls and independent nonheap equations remain
intact. A 32-allocation caller ceiling bounds work without changing engine
admission. Both full gates above include these checks and the two complete
ordered traces per platform. Foreign allocations, arbitrary allocator histories,
concurrent schedules and whole-process/RSS remain outside this observation.

### Transient preparation and release ownership

`d84c30b` repairs uncharged pathnames exposed by observing the maintained wide
nullable STRING LEFT JOIN during preparation. Before the repair, the short and
384-byte paths exceeded the contemporaneous charge by 242 and 810 requested
bytes, respectively; usable deficits were 272 and 896 bytes on macOS. Catalog
scratch owned its complete buffer while reads also held a units directory path
and an object path. Preparation now admits those two existing 4,096-byte bounds
before catalog I/O and releases them before binding. Retained plan charges,
allocator allowances, catalog allocation census and persistent formats are unchanged.
The [preparation contract](../docs/resources.md#query-preparation) owns the peak
and destruction flow.

The independent scope fixture includes both path bounds in the public preparation
peak, tests its exact limit and one byte below, and checks refusal with zero or
8,191 bytes available for the path owner. A focused run passes all 48 binder
tests; its other library tests are intentionally filtered. The first full macOS
gate on `d84c30b` stopped after one Rust failure: the open/resolve fixture assumed
that its exact opening budget also admitted preparation. `a72e3c6` preserves that
opening minimum, requires preparation refusal there, then reopens with both
paths admitted and requires the original unknown-table error. Its focused test
passes. The failed gate is not passing evidence for its unrun later stages;
both complete gates above use the corrected frozen tree.

The existing scoped caller now records preparation and release separately.
After checking every field of all eleven literal joined pairs, it drops the
finished result and prepared plan. Two fresh repetitions abandon an unfinished
result immediately after execute and after a Progress step with live scratch
extents. Each restores the original heap, descriptors and reservations; result
drop alone must release temporary storage and leave only the prepared charge.
The final step has already freed a finished result's heap, so its drop observes
zero allocation/free events and independently checks the remaining charge release.

The following are minimum requested/usable headrooms in bytes for the exercised
histories. Preparation counts include temporary catalog and binding owners;
prepared release frees the plan and null-extension vector.

| Phase | Allocation/free events | macOS headroom | GNU/Linux headroom |
| --- | --- | --- | --- |
| Preparation, short path | 10 / 8 | 7,950 / 7,920 | 8,016 / 7,992 |
| Preparation, 384-byte path | 10 / 8 | 7,382 / 7,296 | 7,382 / 7,368 |
| Prepared release, either path | 0 / 2 | 8,336 / 7,056 | 8,336 / 8,336 |
| Immediate result abandonment, either path | 0 / 344 | 2,896 / 960 | 2,896 / 1,896 |
| Scratch-owning result abandonment, either path | 0 / 344 | 2,896 / 960 | 2,896 / 1,896 |

Both platforms retain the prior execute/step event counts and headrooms in the
[earlier observation](#transient-ownership-in-wide-left-join). Calibration still
detects the hidden uncharged allocation and the free-only live owner. The
false-attribution and disabled-calibration controls remain required. A new
negative control disables preparation observation and must fail phase coverage;
the supervisor also rejects missing lifecycle completion output even when the
ordinary join and calibration markers are present. These are single-threaded
aggregate observations of the exercised Rust allocation events, not individual
pointer attribution, arbitrary allocator histories, foreign allocation coverage,
concurrent schedules or whole-process/RSS qualification.

### Exponential transforms and geometric means

`22a62bf` adds one-argument EXP through the existing bounded parser, binder,
independent validators, batch scratch and demand cursor. The semantic and owner
trace completed within the 30-minute budget against the language guide's pinned
GoogleSQL signatures, compliance cases and reference kernel. This is source
inspection, not an executed upstream conformance run. Three neighboring SQRT/LN
source-link anchors were corrected against that revision; their semantics remain
unchanged. The [language contract](../docs/language.md#current-public-query-manifest)
owns promotion, exceptional values, overflow and approximate precision limits.

Independent Decimal exponentials at precision 100 supply rounded binary64
answers for fourteen finite inputs, including near-zero values, normal/subnormal
results and the finite side of the overflow boundary. Both evaluation paths
allow two ULPs for these particular answers; this is not a universal accuracy
bound. Eleven further cases check exact tiny/zero transitions, zeros, infinities
and NaN bits. Four finite inputs, including the adjacent value beyond the finite
overflow boundary, must return exponential overflow in both paths. A NULL batch
payload containing 1000 must remain unevaluated, and the cursor preserves NULL.

Public cases cover INT64/DOUBLE promotion, very negative integer underflow,
empty/NULL input, grouped results, LEFT JOIN, DISTINCT, union, skipped Boolean
and COALESCE branches, malformed arity/type, and integer-child overflow. EXP
failures in either SAFE_DIVIDE argument remain visible. Constant predicates can
fail during preparation. Demanded overflow preserves its UTF-8 byte span after
source and prepared-plan teardown, including an enclosing SUM. Shared checks
retain exact/short admission, independent NULLability validation, the 31/32-call
bound, cancellation, early drop, stored exceptional bits through producers and
reopen, and 32 forced sort/group replay variants. The ordinary and bounded-thread
Boolean scenarios execute LN and EXP through a computed dependency and aggregate.

The public geometric-mean answer for 10, 20, 30 and 40 comes from two successive
Decimal square roots of 240,000, with an eight-ULP regression threshold for the
composed result. The [runnable query](../examples/geometric_mean.sql) instead uses
the tutorial's positive amounts 5, 10 and 20: AVG consumes their logarithms, then
EXP in a later pipe stage restores the original units. The mathematical answer
is exactly 10. Both fresh native runs are one ULP above it; those matching bits
do not establish repeated or cross-platform bit identity. The
[learning path](../docs/query-examples.md#compute-a-geometric-mean) explains the
NULL behavior, complete result and implementation owners.

Catalog allocation still requires count two, preserving the original 1.75 ratio
and exact large integer before EXP restores the rounded/logged fallback to one.
Native I/O retains total nine with EXP(LN(n/n))-1 contributing zero. Fixed-buffer
formatting checks exponential overflow and its captured cause under allocation
denial. Both complete campaigns retain their fault schedules and healed outcomes.
The prior transient join observer also retains 371 allocations/frees and its
nonnegative headroom at both pathname lengths on both platforms. No allocation
owner, expression framework, persistent format or admission allowance changed.

### Transient ownership in wide LEFT JOIN

`60bf34d` extends the maintained allocator caller at a previously unobserved
boundary: allocations created and freed inside a public execute or step call.
The initial owner/observer trace completed within its 30-minute budget. The
public database charge report is an atomic load, allowing a scoped, borrowed
thread-local observer to compare each successful allocation and pending physical
free without allocating, locking or adding an engine hook. Caller storage stays
fixed while armed. The [resource contract](../docs/resources.md#join-ordering-and-distinct-admission)
owns the observation model and exclusions; the [tool map](../tools/README.md)
owns invocation and control details.

Both pathname lengths pass on both platforms. Each run observes 371 allocations
and 371 frees inside execute/step. Minimum requested headroom is 6,992 bytes;
minimum usable headroom is 4,296 bytes on macOS and 5,992 bytes on GNU/Linux.
These are observations of the exercised allocation histories, not universal
allocator allowances or per-pointer attribution. No engine accounting defect
was exposed, and no production implementation, admission allowance or persistent
format changed.

The caller retains the literal eleven-pair oracle across all 64 join columns,
including nullable maximum-length STRING values, external-storage use,
return-boundary ownership checks, and final heap, descriptor and reservation
release. A temporary uncharged 65,536-byte
allocation is detected despite identical entry/exit heap totals. A second
calibration observes only its free and requires the negative live-byte headroom;
disabling the observer is rejected. The supervisor rejects missing calibration
output even when the ordinary join completion marker is present. Existing
false-attribution, independent nonheap equations and failure/recovery controls
remain intact. Arbitrary allocator histories, concurrent allocation schedules,
foreign allocations, allocator metadata/retained pages, other mappings and
whole-process/RSS bounds remain unqualified.

### Natural logarithms for analytical scales

`5f769c5` adds one-argument LN through the existing parser, binder, independent
scalar validation, batch scratch and demand cursor. Research and tracing resolved
within the 30-minute budget against the language guide's pinned GoogleSQL revision.
The compliance cases and reference kernel return NaN for negative infinity,
resolving the documentation's broader nonpositive-error wording. This is source
inspection, not an executed upstream conformance run. The
[language contract](../docs/language.md#current-public-query-manifest) owns the
accepted semantics, profile choices and native logarithm precision limits.

Independent 100-digit Decimal logarithms supply literal rounded binary64 answers
for ten positive inputs, including the neighbors of one, minimum subnormal,
minimum normal, maximum finite, and converted large integers. Batch and cursor
checks allow two ULPs for these particular finite answers, while checking exact
bits for one, infinities and NaNs. This threshold is a regression check, not a
universal accuracy bound. NULL payloads containing negative numbers must remain
unevaluated. Finite zero, negative zero and negative values must return domain
errors in both evaluation paths.

Public cases cover promotion, empty/NULL input, grouping, LEFT JOIN, DISTINCT,
union, skipped Boolean and COALESCE branches, rejected types and arities, and
argument overflow. Domain errors preserve UTF-8 spans after source and prepared
query teardown, including an enclosing SUM call. Existing checks cover exact/short
admission, malformed NULLability, the 31/32-call boundary, cancellation, early
drop, stored exceptional values through producers and reopen, and 31 forced
sort/group replay variants. The ordinary and bounded-thread Boolean scenarios
also evaluate LN through a computed dependency and aggregate inside the observed
thread. Platform-specific stack ceilings remain unchanged.

The allocation query still requires the original 1.75 ratio and exact large
integer, then LN of the rounded fallback must be zero. Its complete result
remains count two. Native I/O retains total nine after adding LN(n/n), with all
prior arithmetic controls. Fixed-buffer domain-error and cause formatting runs
under allocation denial. Both complete campaigns preserve their refusal/I/O
schedules and healed outcomes. No allocation owner, expression framework,
persistent format or admission allowance changed.

The [logarithm example](../examples/logarithm.sql) and
[tutorial](../docs/query-examples.md#compare-amounts-on-a-logarithmic-scale) filter
positive sales amounts and average their natural logarithms. The values 5, 10
and 20 have mean logarithm ln(10). Both fresh native-storage runs verify table
creation, one nullable DOUBLE row, and `status=queried`, observing
2.302585092994046 with bits `40026bb1bbb55516`. These matching observed bits do
not establish a bit-identical repeated or cross-platform logarithm contract.

### Square roots for analytical magnitudes

`849f38e` adds one-argument SQRT through the existing parser, binder, independent
scalar validation, batch scratch and demand cursor. Research and tracing resolved
within 30 minutes against the language guide's pinned GoogleSQL revision. INT64
converts to DOUBLE before square root. NULL propagates; positive infinity remains
unchanged; PipeSQL explicitly preserves signed-zero and NaN input bits.
Negative inputs, including negative infinity, produce the inline
`ArithmeticDomain` error/cause with operation `square root` and an owned span.
Constant predicate arguments can fail during preparation; projected expressions
retain runtime demand, including skipped COALESCE and Boolean branches.

Literal IEEE answers cover exact powers of two, the square root of two,
subnormals, minimum normal and maximum finite values, exceptional bits and
integer conversion near 2^53 and the INT64 maximum. The promotion and RMS answers
were checked separately with 100-digit Decimal square roots of explicit numeric
inputs. No expected answer calls the production square-root primitive.
Public checks cover source-text release, argument errors through SAFE_DIVIDE,
minimum-INT64 domain failure, invalid types/arity, INT64-only consumers, NULLs,
stored bits through multiple producers and reopen, cancellation and early drop.
Existing binder checks retain exact/short admission, nullability mutation
rejection and the 31/32-call boundary. All 30 forced-grouping-replay variants pass,
including SQRT inside NULLIF with independent integer counts and complete release.

The catalog allocation query retains its exact 1.75 ratio, large-integer result
and count of two after SQRT of the existing oddness expression. Native I/O retains
the expected total of nine after SQRT(n*n) recovers each count of three; the
original division and rounding checks remain. Fixed-buffer controls construct
and render the domain error and cause under allocation denial. Both full campaigns
retain every refusal/I/O position. No allocation owner, expression framework,
persistent format or admission allowance changed.

The [square-root example](../examples/square_root.sql) and
[tutorial](../docs/query-examples.md#compute-a-root-mean-square-amount) use the
ordinary sales database. Squares 25, 100 and 400 have mean 175; SQRT returns one
nullable DOUBLE row with value 13.228756555322953 and bits `402a751f9447b724`.
Fresh native-storage runs verify table creation, complete schema/row output and
`status=queried` on both platforms. Required inputs are tracked and owned outputs
are removed.

### Nearest-integer rounding

`7c56cf8` implements one-argument ROUND through the existing parser, binder,
independent scalar validation, batch scratch and demand cursor. Semantic research
and tracing resolved within 30 minutes against the language guide's pinned
GoogleSQL revision. INT64 converts to DOUBLE before nearest-integer rounding;
halfway values round away from zero. NULL propagates, infinities remain unchanged,
and PipeSQL explicitly preserves signed-zero and NaN input bits. Decimal-position
and rounding-mode arguments remain unsupported.

Literal expected answers cover both sides of halfway boundaries, adjacent binary64
values, subnormals, maximum finite values, integer conversion near 2^53 and the
INT64 extrema. Existing shared tests verify result-type mutation rejection,
unary underflow, stored bits through multiple producers and reopen, demanded
error spans, skipped COALESCE branches, invalid types/arity, exact/short admission,
31/32-call bounds, cancellation, early drop and all 29 forced-replay variants.
The full-width stack scenario includes ROUND without changing its allowance.

The catalog allocation query retains its exact 1.75 ratio, large-integer result
and complete row count while adding ROUND to the existing unary expression.
Native I/O keeps the independently expected rounding total of nine, now exercising
ROUND before CEIL; FLOOR and the original 4.5 ratio remain. Both full campaigns
retain every allocation/I/O position. No new allocation owner, expression framework,
persistent format or admission allowance was introduced.

The [nearest-rounding example](../examples/nearest_rounding.sql) and
[tutorial](../docs/query-examples.md#group-measurements-into-buckets) use the ordinary
sales database and return three groups: NULL with a NULL total and count one,
zero with total five and count one, and one with total 30 and count two. Fresh native-storage runs on both platforms
verify successful creation, schema, exact DOUBLE bits, complete rows and
`status=queried`. Required inputs are tracked; owned outputs are removed.

### Wide LEFT JOIN allocation ownership

`667983f` extends the existing [public ownership caller](../tools/fixtures/composed-ownership.rs)
with a 64-column LEFT JOIN at short and 384-byte database paths. Tracing resolved
within 30 minutes: null extension retains fresh nullable column identities,
both sorted inputs own separate buffers, and unmatched rows reuse the ordinary
output batch. No production code, admission allowance or persistent format changed.

Six rows per side contain nullable keys and STRING cells, including empty,
embedded-NUL UTF-8 and 65,536-byte values. Eleven literal expected pairs specify
unequal duplicate groups, unmatched left rows and nonmatching NULL keys. Every
output field is checked without assuming equal-key order, including NULL extension
of the right side's required id and all its STRING fields. The case requires
external storage and measures requested/allocator-usable bytes against actual
prepared/result charges after execute and every returned step through Finished.
Dropping both owners restores heap, descriptors, memory and temporary charges.

Both pathname lengths on both platforms return 11 pairs in 624 steps and use
11,866,809 temporary bytes. Minimum sampled usable headroom is 7,608 bytes on
macOS and 8,888 bytes on GNU/Linux. The existing false-attribution mechanism fails
the same usable-byte guard after complete rows and release. Runner tests also
reject missing completion markers. Narrow joined, wide-set and other ownership
cases remain in the full campaign. No deficit was observed; transient allocations
inside a step, other workloads and whole-process/RSS bounds remain unqualified.

### Numeric rounding for analytical buckets

`2096d2f` implements FLOOR, CEIL and the CEILING alias. The
[language contract](../docs/language.md#current-public-query-manifest) pins the
GoogleSQL result types and conversion-before-rounding rule. Research resolved
before implementation within 30 minutes. Both numeric input types return DOUBLE;
large-integer conversion is deliberately observable, not exact integer bucketing.
PipeSQL explicitly preserves signed-zero and NaN input bits.

The existing [parser](../src/frontend/parser.rs),
[scalar program](../src/scalar.rs) and [demand cursor](../src/scalar/evaluation.rs)
carry two unary operations. Validation promotes their result type; row and batch
evaluation reuse the existing stack and scratch slots. There is no new allocation
owner, expression framework, type or persistent format.

Independent literal answers cover signed fractional values, subnormals, infinities,
NaN payloads, signed zeros, NULLs and INT64 conversion around 2^53 and both integer
extremes. Mutations reject an INT64 result descriptor and unary underflow.
Public queries check invalid arguments/arity, source spans, skipped/demanded
errors, COALESCE promotion, NULLIF and grouped buckets. Existing stored-value
fixtures now check rounding through multiple producers and reopen. Shared
cancellation, 28 forced-replay variants, exact/short admission and full-width
small-stack cases retain their original controls.

The catalog allocation query keeps its exact 1.75 ratio and large-integer checks,
then exercises CEIL and FLOOR through the existing SELECT/EXTEND path. Native I/O
retains its original 4.5 ratio and adds an independently expected rounding total
of nine. All allocation prefixes and observed I/O failure positions execute;
healthy controls alone were not treated as failure coverage.

The [rounding example](../examples/rounding.sql) and
[tutorial](../docs/query-examples.md#group-measurements-into-buckets) return three
groups: NULL with count one, bucket zero with total 15/count two, and bucket one
with total 20/count one. Both platforms verify schema, exact DOUBLE bits, complete
rows, successful exit and `status=queried` on fresh databases.

The preceding `2096d2f` checkpoint passed complete matching gates and fresh
rounding tutorials on both platforms. Its initial overlapping macOS run with one
Cargo job timed out at the unchanged 600-second Rust-stage deadline; unchanged
inputs and successful cleanup did not make it a passing gate. After Linux
completed, a standalone two-job macOS retry compiled tests in 59.42 seconds
instead of 124 seconds and completed the Rust stage in 416.345 seconds. No deadline
or test was weakened. That observation motivates sequential full gates.

### Wide positional set allocation ownership

`99c8755` extends the existing [public ownership caller](../tools/fixtures/composed-ownership.rs)
with six set-operation cases at short and 384-byte database paths. Tracing and
workload selection resolved within 30 minutes. Two source columns on the left
and 62 on the right reach the existing 64-source-column bound. The left projection
repeats one STRING into 61 logical positions; the right stores each separately.
The input fits the existing token bound. No production code, allocation allowance,
format or admission limit changed. The cases reuse the existing runner.

Literal expected id sequences establish all duplicate multiplicities for UNION,
EXCEPT and INTERSECT, with ALL and DISTINCT. Every returned STRING position is
checked, including NULL, empty, embedded-NUL UTF-8 and 65,536-byte cells. The
expanded records exercise external sorting while compact source demand and
62-column output retain their separate owners. The observer samples requested
and usable Rust allocations against prepared/result charges after execute and
every returned step, including Finished. Each case restores heap, descriptors,
memory and temporary charges after dropping both owners. A nonexistent measured
owner fails the same usable-byte guard; runner tests reject a missing completion
marker even when the process reports success.

Both pathname lengths pass on both platforms. UNION ALL returns 24 rows without
temporary storage; UNION DISTINCT returns eight and peaks at 39,994,742 temporary
bytes. EXCEPT DISTINCT, INTERSECT DISTINCT, EXCEPT ALL and INTERSECT ALL return
2, 4, 7 and 5 rows respectively and each peaks at 31,995,362 temporary bytes.
Step counts agree across platforms: 918, 1,316, 1,205, 1,207, 1,205 and 1,203 in
that order. Minimum sampled usable headroom is 7,608 bytes on macOS and 8,888 on
GNU/Linux; UNION DISTINCT has 11,648 and 12,984 bytes respectively. No deficit
was observed, so no admission repair was justified. All cells fit the existing
20-second subprocess deadline.

The [resource contract](../docs/resources.md#except-distinct-admission) and
[tool map](../tools/README.md) describe the boundary and invocation. These cases
establish neither transient allocation peaks between steps nor arbitrary
allocator, foreign-allocation or whole-process/RSS bounds. The retained failure,
cancellation, replay and corruption campaigns pass with unchanged schedules.

### Numeric sign classification

`670e7bc` implements SIGN; `6823ba6` completes its execution-boundary coverage.
The [language contract](../docs/language.md#current-public-query-manifest) pins
INT64/DOUBLE typing, NULL propagation, positive zero for either DOUBLE zero,
unchanged NaN payloads and signed one for nonzero values, including infinities and
integer extremes. Semantics were resolved before implementation at the existing
immutable GoogleSQL revision, within the 30-minute research bound.

The [parser](../src/frontend/parser.rs) emits one unary operation into the existing
[scalar program](../src/scalar.rs). Batch and [ordered row evaluation](../src/scalar/evaluation.rs)
share the DOUBLE classification rule. No buffer, allocation owner, type, runtime
framework or persistent format was added. Independent literal scalar expectations
cover both numeric types, subnormals, extremes, NULLs, zero and NaN bits, reused
batch scratch and row demand. A malformed unary program remains rejected.

[Public cases](../tests/catalog_lifecycle/computed.rs) cover classification and
grouping, nullable keys, nested expressions, stored DOUBLE bits across producers
and reopen, skipped division/overflow and demanded error categories and spans.
Wrong arity, missing names and unsupported argument types are rejected. Shared
unary checks retain type/NULLability mutation controls, 31/32-call nesting boundaries
and exact/one-byte-short preparation admission. Cancellation, early drop,
full-width/small-stack execution and forced sorted-producer replay pass.

Existing allocation and native-I/O queries now classify their integer remainder
or quotient with SIGN. Their independently expected counts remain two and three,
respectively; exact large-integer behavior and all failure schedules remain
covered. No extra campaign runner was added. The full catalog allocation census
remains 1,056 at both pathname lengths.

The fresh [classification example](../examples/sign.sql) and
[tutorial](../docs/query-examples.md#classify-measurements-by-sign) return the four
documented NULL, negative, zero and positive groups on both platforms. Setup,
query schema, every row, count, successful exit and terminal `status=queried`
were checked before removing the example builds and databases.

### Infix negated membership and ranges

`7b03e84` adds `name NOT IN (...)` and `name NOT BETWEEN lower AND upper` with
one local change in the [Boolean parser](../src/frontend/parser/boolean.rs).
The [pinned GoogleSQL operators](https://github.com/google/zetasql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/operators.md)
define their comparison precedence and negation semantics. The parser adds the
existing NOT syntax node around the complete membership OR or range AND.
Runtime predicates, forward decisions, literal ownership, validators, admission
and persistent formats are unchanged. The [language contract](../docs/language.md#literal-list-membership)
keeps the bounded column/literal profile and PipeSQL's ordered demand guarantees.

The [membership tests](../tests/catalog_lifecycle/membership.rs) retain independent
nullable-set expectations and add infix and enclosing negation. Literal row-id
oracles cover all four types, duplicates, NULL candidates, NaN, zero, precedence
and composed producers. [Range tests](../tests/catalog_lifecycle/boolean.rs) cover
inclusive and reversed bounds, typed NULL bounds, NaN, enclosing NOT and skipped
or demanded overflow. The obsolete NOT IN rejection becomes positive coverage;
empty lists, malformed NOT, column/subquery candidates and incompatible literals
remain rejected. A new demand test initially omitted id from its SELECT; the
corrected input projection passes.

Exact and one-byte-short preparation admission, stage limits, physical mutation
controls, cancellation, early drop, legacy count/payload expectations and forced
sorted-producer replay pass. Existing allocation and I/O queries exercise nested
NOT IN and NOT BETWEEN while retaining their independent row/count outcomes and
failure schedules. No extra runner or runtime representation was introduced.

The fresh [exclusion example](../examples/negated-membership.sql) and
[tutorial](../docs/query-examples.md#filter-by-membership) return only `south, 20`
on both platforms. Setup output, query schema, row, count, successful process
exit and terminal `status=queried` match; the NULL amount is excluded. Example
builds and databases were removed after these checks.

### Null-safe column and literal predicates

`4d77e98` adds `IS [NOT] DISTINCT FROM` through two comparison variants in the
existing one-column/owned-literal predicate. The
[language contract](../docs/language.md#current-public-query-manifest) pins NULL,
NaN, signed-zero and numeric common-typing rules. Research resolved within its
30-minute bound. The [shared decision](../src/execution/predicate.rs) handles
NULL before ordinary comparison can yield UNKNOWN, then respects enclosing
Boolean negation. Legacy scan kernels reuse the same comparisons. No new field,
buffer, allocation owner, Boolean representation or persistent format was added.

Three [public tests](../tests/catalog_lifecycle/null_safe.rs) check typed two-valued
truth tables, literal compatibility, exact INT64 and mixed DOUBLE boundaries,
NaN/signed-zero stored bits, prepared snapshots and reopen. Literal row-id oracles
cover enclosing NOT, nested sources, ordered producers, grouping, joins and set
composition. Invalid names, types, incomplete predicate syntax, nonfinite/range
literals and column-to-column forms remain rejected. Shared demanded-error tests
retain arithmetic spans, terminal failure, LIMIT 0, and ownership release even
when NULLability predicts a predicate's result. Independent validator mutations
retain healthy controls; exact/short binding admission and stage bounds pass.

The legacy scan test checks eight literal count/sum outcomes over 130 rows,
including DOUBLE, STRING, DATE and NULL-literal paths. Shared cancellation and
full-width/small-stack tests run the new predicates. Forced grouping fallback
replays a sorted producer with NULLIF-generated NULLs that must survive the new
filter. Allocation and native-I/O queries use null-safe NULL/present-value tests
within their retained failure schedules; the full allocation census stays 1,056.

The fresh [region filter](../examples/null-safe-region.sql) and its
[tutorial](../docs/getting-started.md#keep-null-rows-when-excluding-a-sentinel)
produce `(110, 4)` on both platforms, while ordinary inequality produces `(60, 3)`.
Both report nullable INT64 total, required INT64 nrows, one row, successful process
exit and `status=queried`. The complete gates above retain ordinary comparison
UNKNOWN, COALESCE/NULLIF demand, storage, replay and publication coverage.

### Numeric NULLIF sentinel normalization

`0e13c43` adds two-argument numeric NULLIF through the existing bounded scalar
program. The [language contract](../docs/language.md#current-public-query-manifest)
pins numeric common typing, conservative NULLability and ordinary numeric
equality. Research resolved within its 30-minute bound. The
[demand cursor](../src/scalar/evaluation.rs) evaluates both arguments in order,
including the second when the first is NULL, and retains the first coerced value
unless equality is TRUE. Its existing static type table handles a NULL DOUBLE
argument. An outer COALESCE can still skip the whole call. No new frontend,
expression representation, allocation owner, buffer or admission allowance was
introduced.

Independent literal cases check exact INT64 values above 2^53, mixed rounded
equality, typed NULL coercion, NaNs, infinities and signed-zero bits. Validity
word boundaries and reused scratch lanes retain independent expected values.
The [public tests](../tests/catalog_lifecycle/nullif.rs) check sentinel aggregates,
empty inputs, nesting, joins, ordering and set composition; they preserve stored
NaN payloads and signed-zero outcomes across snapshots and reopen. Distinct
error controls require the first arithmetic failure before a later computed
dependency's failure and require a second-argument error even after a NULL first
value. Hidden expressions, outer COALESCE and LIMIT 0 preserve demand boundaries,
original spans and terminal cleanup.

Shared independent program mutations reject malformed NULLIF programs and
incorrect metadata. Exact/short admission, parser bounds, wide/small-stack
execution and forced grouping replay pass. The public allocation query retains
a present ratio when NULLIF's second value is NULL. Native I/O converts COALESCE
defaults back to NULL and requires COUNT zero, while retaining all existing
failure and healthy-reuse schedules. The full allocation census remains 1,056.

The fresh [sentinel query](../examples/sentinel-amounts.sql) and its
[tutorial](../docs/getting-started.md#exclude-a-sentinel-from-an-aggregate) return
`(130, 4, 5)` on both platforms: nullable INT64 total, required INT64 measured and
nrows, one row, successful process exit and `status=queried`. Source inspection,
focused checks and the complete gates above preserve the existing COALESCE,
arithmetic, storage, replay, cancellation and publication contracts.

### Positional EXCEPT ALL and INTERSECT ALL

`4ae3a67` adds duplicate reconciliation through the existing positional descriptor,
independent validators, complete-row demand, sorted-input owners and scheduler.
The [language contract](../docs/language.md#except-all-and-intersect-all) pins
multiplicities and grouping equivalence at the existing GoogleSQL revision.
Research resolved within its 30-minute bound. EXCEPT ALL emits `max(m - n, 0)`
left occurrences and INTERSECT ALL emits `min(m, n)`; arguments combine left to
right. Exact types, left names, fresh identities, pinned inputs, operator-specific
NULLability and the existing demanded-error/LIMIT 0 rules remain explicit.

The merge pairs equal occurrences by advancing both checked cursors. It adds no
buffer, duplicate index, group counter, frontend, persistent format or allowance.
Monotonic record checks remain active while ALL retains duplicate left rows.
The existing four sorted-set schedules run both quantifiers: exact/short
admission, spill on both sides, prefix replay, all cancellation phases, corrupt
and truncated readers, I/O refusal, temporary refusal and healthy reuse. Grouping
fallback observes replay and checks literal duplicate-sensitive aggregate results.

Three [public tests](../tests/catalog_lifecycle/multiset.rs) include 100 independent
complete-row count cases, unequal multiplicities, empty/nested/multiple arguments,
joins and aggregation, names/types/NULLability, all scalar types and typed NULLs,
exact INT64 and DATE boundaries, NaN/signed-zero classes and pinned snapshots.
Original-bit checks require both distinct left NaN payloads and zero signs when
both occurrences survive. Shared tests retain hidden demanded errors and original
spans, independent malformed-plan mutations, full width and small stacks.

The unchanged constructors and file owners remain covered by public allocation
and native-I/O sweeps. Independent per-step ownership adds two ALL cases at both
pathname lengths; each emits 768 rows and releases completely. EXCEPT uses
21,611 steps and 231,664 temporary bytes, with minimum usable-allocation headroom
11,352 bytes on macOS and 12,976 on GNU/Linux. INTERSECT uses 29,337 steps and
289,520 temporary bytes, with headroom 11,360 and 12,984 bytes respectively.
All 11 analytic shapes and existing result/attribution negative controls pass.

The fresh [repeated-regions example](../examples/repeated-regions.sql) and its
[tutorial](../docs/getting-started.md#reconcile-repeated-facts) produce nullable
INT64 region rows NULL, 1 and 3, `row_count=3`, successful exit and `status=queried`
on both platforms. The complete gates above reconcile discovery and retained
failure coverage. No skipped or filtered check is counted as broader runtime
qualification.

### Positional INTERSECT DISTINCT

`f832a21` adds complete-row intersection; its dedicated full-gate checkpoint is
retained in `255bff6`. It uses the existing bounded parser,
positional descriptor, independent semantic/physical validators and scheduler.
The [language contract](../docs/language.md#intersect-distinct) pins the accepted
GoogleSQL profile. Research resolved the profile within its 30-minute bound.
Each output has the left name and a fresh identity; NULLability requires both
input columns to be nullable. That inference and eager complete-input error
demand are explicit PipeSQL contracts. Original left representative bits survive;
selection among equivalent representatives and incidental output order remain
unspecified. That checkpoint excluded ALL, name matching, coercions and correlated inputs.

The [sorted-set owner](../src/execution/blocking/sorted_set.rs) now handles EXCEPT
and INTERSECT through the same two sorted inputs, inline position maps, checked
cursors, admission and replay. Only the merge selection changes. No new buffers,
index, frontend, persistent format or allowance inflation are introduced.
The former EXCEPT owner tests moved with this owner. All four schedules now run
both operations, including exact/one-byte-short admission, spills on both sides,
replay after a prefix, all 17 cancellation phases, corruption, truncated input,
read failures, temporary refusal and healthy reuse. Independent validators retain
malformed-plan controls. Full-width and small-stack checks exercise both empty
and nonempty intersection outputs; grouping fallback proves retained replay.

Five [public checks](../tests/catalog_lifecycle/intersect.rs) include 50 independent
standard-library set comparisons, all current scalar types and typed NULLs,
NaN/signed-zero equality with original-bit checks, exact large INT64 values,
names/NULLability, repeated positions, empty and nested inputs, joins, aggregation
and pinned snapshots. The moved demanded-error check runs EXCEPT and INTERSECT,
including a hidden failing field, empty left input, original spans, repeated
failure, healthy reuse and the LIMIT 0 exception. No protected EXCEPT check was
removed. The allocation-prefix and native-I/O campaigns cover the unchanged
shared constructors and effect owners. The independent ownership campaign also
runs INTERSECT through analytic count at both pathname lengths: 256 rows,
11,072 steps, 115,832 temporary bytes and minimum allocator-usable headroom of
11,600 bytes on macOS and 12,984 on GNU/Linux, with complete release. Existing
attribution and result negative controls remain effective.

The fresh [shared-regions example](../examples/shared-regions.sql) produces
required INT64 `region`, rows `1` and `2`, `row_count=2` and `status=queried` on
both platforms. Its [tutorial](../docs/getting-started.md#retain-facts-with-missing-dimensions)
uses the actual LEFT JOIN example database and explains why the result is required.
The full gates above reconcile discovery and retained campaign coverage.

### Positional EXCEPT DISTINCT

`950b5fe` and `5146e72` add bounded positional EXCEPT DISTINCT; its dedicated
full-gate checkpoint is retained in `fb1061c`. The
[language contract](../docs/language.md#except-distinct) pins syntax, positional
typing, NULL/NaN/signed-zero equivalence, left association and complete-input
error demand at the existing immutable GoogleSQL revision. Research resolved
these choices within the 30-minute bound. Left-only output NULLability follows
from set difference; representative choice and output order remain unspecified.
Unsupported ALL, name matching, coercion and correlated forms remain rejected.

The positional descriptor now serves UNION and EXCEPT with independent semantic
and physical validators. The
[controller](../src/execution/blocking/sorted_set.rs) consumes both branches through
the existing scheduler, sorts their complete rows, and emits surviving left
representatives. Each input keeps its own nullable record layout. Inline mappings
preserve repeated semantic positions even when child payload slots are shared.
The existing checked runs and cursors supply bounded spill and replay; there is
no second frontend, hash index or persistent-format change.

Six public tests include 50 independently materialized standard-library set
comparisons, complete rows, empty and nested inputs, NULLs in every scalar type,
exact INT64 values, floating-point bits, pinned snapshots and demanded errors
with original spans. Seven selected preparation/validator tests retain mutation
controls and exact/one-byte-short admission. Four owner tests reconcile actual
allocation capacities, execution admission, replay, both readers' corruption and
I/O failures, all 17 cancellation phases, temporary refusal and healthy reuse.
The 180/176-row fixture crosses the independently traced 174-record first-run
boundary on both sides; missing a spill phase fails the test. The shared fixture
retains all seven join-owner checks. Grouping fallback proves retained EXCEPT
replay, and ordinary/bounded-thread width scenarios preserve UNION coverage while
adding EXCEPT without enlarging stack limits.

The allocation sequence adds 71 observed allocations, changing its healthy
census from 985 to 1,056; its work ceiling increases to 1,100 without changing
engine admission allowances. Native I/O retains a literal result of 90 after
excluding matching complete rows. Public ownership checks also consume 256 typed
EXCEPT survivors through window count. At returned steps their minimum observed
usable-byte headroom is 11,600 bytes on macOS and 12,984 bytes on GNU/Linux;
complete rows, release and the existing attribution/row negative controls pass.
These observations do not bound unobserved transient allocations or RSS.

The [missing-regions query](../examples/missing-regions.sql) and its
[tutorial](../docs/getting-started.md) run from fresh native databases on both
platforms. Both return nullable INT64 `region`, rows NULL and 3, `row_count=2`
and successful completion. The NULL survives because the dimension has no NULL
identifier. The earlier dedicated COALESCE full checkpoint remains in `424edd1`.

### Numeric COALESCE defaults

`4d0550e`, `df7d912`, `1c46016`, `f9a0da1` and `2d41739` implement and verify
two-argument numeric COALESCE. The [language contract](../docs/language.md)
pins first-non-NULL selection, common INT64/DOUBLE typing, NULLability and
left-to-right short-circuit evaluation. Both branches still bind, and a first
argument error propagates. The existing postfix representation gains one binary
operation; a bounded cursor derives conditional edges without a second expression
arena or prepared descriptor. Computed dependencies descend only to earlier
definitions and use the existing row cache and output buffers. Conservative
admission and materialization boundaries remain intact.

Independent literal results cover NULL combinations, exact INT64 extremes and
values beyond DOUBLE's exact range, mixed coercion, signed zero, nonfinite bits,
word-boundary validity and scratch reuse. Tests distinguish skipped and demanded
arithmetic/dependency/aggregate-finalization errors, retain diagnostic spans and
repeated terminal errors, and exercise malformed types, NULLability, arity and the
32-operation bound. LEFT JOIN defaults pass exact/one-byte-short admission,
20 cancellation phases and forced grouping replay alongside unchanged inner-join
and NULL-extended controls. Ordinary and observed small-stack queries pass.
Catalog allocation and native I/O callers demand defaults and skip faulting
fallbacks while retaining their literal result oracles.

Final review repaired a missing rejection of NULL for a required cursor input.
Type and NULLability are now checked before cursor state changes; negative
controls reject invalid inputs and then complete a valid retry. The independent
ownership equation was also missing the new cursor payload. It now accounts for
744 bytes through the cursor's fields, construction arrays and pending indices,
without importing admission constants or changing its equality/negative controls.
The first two gates were deliberately interrupted for the input repair. A later
GNU gate rejected the stale ownership equation; its matching macOS run was
stopped before changing inputs. One subsequent GNU run ended when Docker was
accidentally stopped, with exit 137 and OOMKilled=false. None of these incomplete
runs supplies complete-gate evidence; the dedicated matching full passes in
`424edd1` supersede them. An initial native fixture used a function on WHERE's unsupported left side;
explicit projection repaired the fixture without widening the language profile.

The [default-region query](../examples/default-region.sql) and its
[tutorial](../docs/getting-started.md) run from fresh native databases on both
platforms. The result has required INT64 region/count, nullable INT64 total and
literal rows `(0, 90, 2)`, `(1, 30, 2)`, `(2, 30, 1)`.
A complete-CLI observation on the composed example's 8,192-row nullable sales
table compares `FROM sales |> SELECT amount+0 AS base |> SELECT base+0 AS next
|> AGGREGATE SUM(next) AS total` with `COALESCE(amount, 0)` replacing `amount+0`.
Both return literal sum 11,264. Seven alternating runs per query have medians
13.625 ms and 9.535 ms, including process startup and open/prepare/execute/close.
This short local sample establishes no speed ranking or reason to change owners.
Variadic forms, other data types, NULL literals, CASE, IF and IFNULL remain outside
this milestone. No persistent representation changes.

### Equality left joins

`160ccb7`, `ee9b83e` and `b2236a6` add equality LEFT JOIN through the existing
parser, binder, independent validators, demand analysis and shared-sorter join.
The [language owner](../docs/language.md) pins LEFT/LEFT OUTER spelling, duplicate
multiplicity, NULL keys, nullable right outputs and subsequent WHERE behavior.
A prepared descriptor gives right outputs fresh nullable identities while
preserving the independent right producer's original facts. Its exact vector
reservation uses the existing per-allocation allowance; queries without LEFT JOIN
allocate no descriptor vector. The query identity ceiling is unchanged.
Unmatched rows reuse the join's output batch and retained sorters, without a
separate queue, match bitmap or persistent-format change.

Independent literal results and a nested-loop row oracle protect unmatched and
NULL keys, empty inputs, duplicate cross products, post-join filtering, nested
and repeated producers, grouping and ordering. Typed cases cover NULL, NaN,
signed zero, dates and strings through spill. Snapshot checks run both inner and
left joins across publication on ordinary and bounded stacks. Malformed semantic
mappings and physical join-kind/identity mutations reject with healthy controls.
Nested and eight-join preparation chains pass exact/one-byte-short admission;
execution admission, all 20 cancellation phases, forced grouping replay and
healthy reuse retain their resource and cleanup assertions. An earlier debug
small-stack run aborted and provides no passing evidence; the documented release
selection and both complete release gates pass.

The catalog allocation caller retains four matching pairs plus one unmatched
row, with literal count five; a separate inner-join ordering control retains
count eight. Its census rises from 983 to 985 within the unchanged ceiling.
Native I/O retains two left groups but only one matched group contributes to
AVG, whose literal result is 60. The focused derived campaign passes 340 cells;
the complete campaigns above retain all 1,196 cells. Demanded aggregate overflow
keeps its complete call span and terminal failure; unused expressions remain
undemanded, and NULL-extended arithmetic does not evaluate absent right values.

The [fact/dimension example](../examples/left_join.rs) and its stock CLI query
run from fresh native databases on both platforms. Both return nullable name
and total, required count, and exact rows NULL/90/2, north/30/2 and south/30/1.
RIGHT/FULL joins, USING, compound/non-equality predicates, correlated inputs and
parallelism remain unsupported. The retained comma-spacing convention applies
to code and SQL without changing quoted data or intentional lexical fixtures.

### Integer quotient and comma spacing

`8c28a1f` adds INT64 DIV through the existing bounded call parser, binder,
independent validators and scalar evaluator. The quotient truncates toward zero
without conversion through DOUBLE. Either NULL argument yields NULL; otherwise
zero raises division-by-zero and minimum INT64 divided by -1 raises division
overflow. Argument errors remain visible, including inside SAFE_DIVIDE. The
[language owner](../docs/language.md) pins signatures, signed/extreme fixtures,
the integer primitive and NULL evaluation at the immutable upstream revision.
Numeric reference anchors now use actual source lines. No allocation owner,
persistent format or admission allowance changes.

Four new tests protect literal signed/extreme results, exact values above 2^53,
integer-only binding, identity mutation, malformed calls/programs, 15/16-call
bounds, source spans and composition. The retained nullable-lane test now checks
both DIV and MOD across validity words and buffer reuse with separate literal
results. Existing failure cases cover zero through SAFE_DIVIDE, repeated terminal
failure, cancellation, early drop and healthy reuse. Exact/one-byte-short
preparation and sorted execution admission pass; forced grouping fallback replays
DIV arguments with independent grouped totals.

The catalog allocation caller requires DIV by one to preserve 9,007,199,254,740,993
exactly before filtering, while retaining count two and its existing census.
Native I/O composes DIV with MOD over count-only output and checks literal total
three without another query or runner. Both complete gates include these callers.
The formatted quotient example runs on fresh sales databases on both platforms:
nullable INT64 bucket and total, required INT64 n, rows NULL/1/NULL, 0/2/15 and
1/1/20. Exact schema, all three rows and successful completion match. DOUBLE DIV,
NUMERIC types and other new scalar calls remain outside this milestone.

`6d2b7d5` and the expression changes normalize comma separators in maintained
code, embedded/generated SQL, examples and documentation. Quoted data, codec
fixtures, upstream source bytes, linker flags and intentional lexical fixtures
remain intact. The spacing changes were reconciled against the pre-format DIV
inputs, including two ordinary trailing commas introduced by rustfmt. No runner,
formatter dependency or permanent formatting framework was added. The fresh gates
above verify the formatted inputs. Earlier gates deliberately interrupted before
the spacing change provide no complete-gate pass.

### Integer remainder

`a889528` adds INT64 MOD through the existing bounded parser, binder, independent
validators and scalar evaluator. It shares two-argument call frames with
SAFE_DIVIDE and the existing integer lane, without another allocation owner or
persistent format. Either NULL argument yields NULL; otherwise zero raises a
source-spanned division-by-zero error. Nonzero remainders have the dividend's
sign, and minimum INT64 modulo -1 is zero. The [language owner](../docs/language.md)
retains the immutable signature, fixture, primitive and NULL-evaluation sources.
Binding distinguishes unsupported argument types from malformed internal
programs; independent validation rejects both invalid forms.

Five new tests retain literal signed/extreme results, NULLs across validity words
and buffer reuse, malformed calls/programs, INT64-only binding, identity mutation,
15/16-call bounds and composition through projection, predicates, SET, subqueries,
union, grouping and analytic count. Retained tests check UTF-8 source spans,
argument failure through SAFE_DIVIDE, repeated terminal failure, cancellation,
early drop and healthy reuse. Exact/one-byte-short preparation and sorted
execution admission pass; forced grouping fallback replays MOD with literal
expected totals. Both full gates execute these checks.

The catalog allocation caller demands exact oddness above 2^53 while preserving
its count-two oracle. Its census increases from 982 to 983 within the unchanged
1,000 ceiling; both pathname sweeps cover every refusal prefix and healthy control.
Native I/O retains total nine while applying MOD to count-only output, preserving
its 1,196 failure cells without another query or runner. The documented remainder
example executes on fresh sales databases on both platforms: nullable INT64
remainder and total, required INT64 n, rows NULL/1/NULL, 0/2/30 and 5/1/5. Exact
schema, three rows and successful completion match. DOUBLE MOD, NUMERIC types and
other new scalar calls remain outside this milestone.

### Absolute value

`0ea0040` adds INT64/DOUBLE ABS through the existing bounded parser, binder,
validated numeric program and unary evaluator. It preserves input type and
NULLability. Minimum INT64 raises a source-spanned absolute-value overflow;
argument errors remain visible, including when ABS is inside SAFE_DIVIDE.
DOUBLE maps signed zero to positive zero and infinities to positive infinity;
NaN remains NaN. The [language owner](../docs/language.md) retains the pinned
signatures, fixtures, primitive and NULL-evaluation evidence. No allocation
owner, persistent codec or admission allowance changes.

Four new tests protect literal values, numeric types, NULLs across validity
words and buffer reuse, signed zero, nonfinite values, subnormals, overflow spans,
malformed calls/programs, type/nullability mutations, 31/32-call bounds, unused
expression and Boolean/LIMIT demand, and SET, derived, union and analytic-count
composition. Retained sorted execution checks exact/one-byte-short admission
and zero effects on refusal. Forced grouping fallback replays ABS arguments
with literal grouped totals. Cancellation and early drop release owners and
permit healthy reuse. These cases execute in both complete gates above.

The catalog allocation caller evaluates ABS of a negated division result while
retaining its count-two oracle and SAFE_DIVIDE NULL demand. Native I/O similarly
retains its literal aggregate 4.5 through ABS, without another query or runner.
Both campaigns retain their previous census and distinct failure schedules.
The documented deviation example runs on fresh sales databases on both platforms:
required region, nullable INT64 amount and deviation, north/NULL/NULL, north/5/5,
north/10/0 and south/20/10. Exact schema, all four rows and successful completion
match. INT64 ABS constants work in LIMIT; DOUBLE still fails its type requirement.
Other scalar calls and NUMERIC types remain outside this milestone.

### Safe division

`374d312` adds SAFE_DIVIDE through the existing bounded parser and numeric
program. Two parser argument phases emit one binary instruction without
recursion. The evaluator uses the existing validity bitmap for NULL results
from zero denominators or finite division overflow; argument failures still
propagate. Folded NULL constants preserve DOUBLE comparison typing. The
[language owner](../docs/language.md) pins the upstream signatures, fixtures and
error boundary. No persistent codec, allocation owner or admission allowance
changes; the obsolete nonnull-only output helper had no remaining consumers.

Six new tests protect mixed types, nesting, precedence, NULL, signed zero,
nonfinite values, underflow, validity reuse, argument failures, malformed calls
and programs, nullable identity mutation, 15/16-call limits, legacy empty/loaded
storage and composition. Retained replay tests now exercise nullable safe
arguments through forced hash fallback and retained output. Preparation and
sorted execution pass exact/one-byte-short admission, with zero effects on
refusal. Cancellation and early drop release owners and permit healthy reuse.
Expected values remain literal, and malformed-plan controls remain independent.

The catalog allocation caller retains its division phases and count-two oracle,
now demanding a NULL SAFE_DIVIDE result before counting. Native I/O retains its
previous results and adds count zero over three NULL ratios. Both full gates
include these callers. The documented safe-ratio example runs on fresh declared
sales databases on both platforms: required region, nullable INT64 denominator,
nullable DOUBLE ratio, north/NULL/NULL, north/5/2.0, north/10/1.0 and south/20/0.5.
Both runs match exact DOUBLE bits, four rows and successful final status. Generic
safe-error modes, integer DIV, NUMERIC types and other scalar calls remain outside
this milestone.

### Numeric division

`2263930` adds division through the existing lexer, precedence parser, binding,
validated numeric program and lane evaluator. Division returns DOUBLE, including
for two INT64 inputs; checked integer children run before coercion. NULL lanes
skip the operation. Both signed-zero denominators raise `DivisionByZero`, even
with a nonfinite numerator. Finite overflow retains `ArithmeticOverflow`; other
nonfinite results and underflow follow the pinned arithmetic rules. The
[language owner](../docs/language.md) links the immutable signatures, coercion
fixtures, NULL evaluation and implementation used to resolve these decisions.
No persistent codec or allocation owner changes. Error causes preserve the new
category and source span without retaining source text or allocating a message.

Eight new tests protect mixed types, precedence, integer-child overflow, NULL,
signed zero, nonfinite values, underflow, result-type mutation, source spans,
demand suppression, Boolean short-circuiting, SET, derived inputs, UNION DISTINCT,
grouped ratios, cancellation, early drop, terminal failure and healthy reuse.
Preparation and sorted execution exercise exact/one-byte-short admission; refused
execution performs zero effects. Existing semantic and physical validators,
independent codec fixtures and allocation attribution remain unchanged.

The catalog refusal caller requires division preparation, execution and stepping;
its nullable ratio/filter query independently expects count two. Native I/O adds
a complete division aggregate with literal result 4.5 while retaining previous
controls. Fixed-buffer diagnostic control and denial modes render both the new
error and its captured cause. Both complete frozen gates above include these
checks. The documented ratio example runs against fresh declared sales databases
on both platforms: required region, nullable DOUBLE mean_amount, north/7.5 and
south/20.0, exact DOUBLE bits, two rows and successful final status. Division in
LIMIT still fails its INT64 type requirement; analytic count remains a complete
projection expression and composes with division through a subsequent stage.

### Full-partition analytic count

`5697961` implements `COUNT(*) OVER ()` in SELECT and EXTEND through the existing
parser, binder, independent validators, physical planner and scheduler. One
checked sorted-input owner captures demanded fields and ordinals, then emits
rows with the complete count. Ordinary expressions in that projection retain
the original input scope and evaluate during emission, preserving downstream
LIMIT's demanded-error boundary. Repeated counts share storage but receive
separate semantic identities. Analytic evaluation clears semantic relation order.

Literal row oracles cover empty input, repeated counts, mixed expressions,
original ranges, predicates, LIMIT before/after, DISTINCT, grouping, joins,
unions, nested count stages, snapshots and typed spilled rows. Semantic and
physical mutation controls remain independent of lowering. Existing exact/short
admission, ten cancellation phases, seven scratch failure/corruption cases and
forced grouping replay now exercise analytic count too. The catalog allocation
caller checks count preparation, execution and stepping. The native derived-join
caller includes count while retaining its independent expected result of 120.0.

Integration exposed three repaired boundaries. Internal grouped results can
carry zero fields when their consumer demands only cardinality; checked frames
and unused-aggregate error suppression remain intact, including forced hash
fallback. Legacy queries can use shared scratch with recovery limited to empty,
single-link disposable names. Tests cover every constructor effect, fourteen
process-death cuts, live readers, strict writer admission and corrupt debris.
Authoritative persistent codecs are unchanged. Finally, an earlier legacy scan
uses bounded cell writes when a later STRING constant requires UTF-8 batches;
a direct LIMIT/constant regression protects this independently of analytic count.

The documented `examples/window-count.sql` flow runs against fresh declared sales
databases on both platforms. Schema, encoded rows, row count and final successful
status match exactly: north/5/3, north/10/3 and south/20/3. Full gates compile the
examples; these separate CLI executions establish the example's runtime result.
The initial implementation retained ordinal records even for count-only
projections; the storage reduction below removes that cost. Neither establishes optimal execution,
arbitrary-allocator bounds or whole-process/RSS limits.

### Analytic count storage reduction

`b1a9570` selects an inline counter when validated physical input has zero fields.
It consumes complete input in bounded batches, then emits the original cardinality
with the final count. Demanded values retain the checked ordinal spool. The
counter inherits the existing 134,217,728-row bound and receives no file authority.
Its inline state belongs to the admitted runtime node; output and shared row
scratch remain separately admitted. Persistent codecs and independent validators
are unchanged.

Both frozen ownership campaigns observe 529 public steps and zero temporary bytes
for 512 count-only rows, versus 7,710 steps and 32,888 bytes at `0e94fa7`. The
nineteen-count case also uses no scratch file. Typed, wide, consecutive and grouped
cases retain their explicit spill expectations and independent ownership equations.
These complete-query counts are resource observations, not elapsed-time benchmarks.

Four counter tests cover literal cardinality, malformed batches, exact/short
admission, the row bound, cancellation and replay. Public tests exercise one-byte
temporary limits, demanded-value refusal, suppressed and demanded expression
errors, empty input, UNION, snapshots, early drop and healthy reuse. Runtime replay
checks prefix and complete emission without further I/O. Legacy expectations now
state storage and width explicitly instead of inferring spill from SQL spelling.
Allocation campaigns require all three count-only phases with independent total
16; native I/O retains the original 120.0 oracle and adds count-only total 9.
The documented window example runs on fresh databases on both platforms and
produces north/5/3, north/10/3 and south/20/3. The full checkpoint above includes
all retained tests and failure campaigns.

### Analytic allocation ownership

`e6538ee` and `0e94fa7` extend the existing public ownership caller without changing
engine inputs from `5697961`. Both complete `--ownership-only` selections pass on
stock macOS and unprivileged native-storage GNU arm64 Linux. Each pathname length
runs seven analytic cases: empty input, one count, nineteen repeated counts,
INT64/DATE/nullable UTF-8 rows, 64 output columns, consecutive analytic stages and
grouped composition. Twenty repeated calls reject at the 160-token bound and
release heap and logical preparation ownership. Literal expected results require
512 rows with count 512, or one composed total of 262,144; empty input emits none.

Preparation equations account for the plan/computation allocations and, for the
grouped composition, its controller and two entry vectors. Nonaggregating cases
reconcile execution admission, first spill and emission against independently
derived nonheap charges: the result handle, a 4,096-byte physical-plan allowance,
8,192 source-path bytes less the retained pathname request, and 5,104 bytes of
row-evaluation arrays. Each pending analytic scratch constructor adds 8,192 bytes.
Terminal results retain only their handle charge. Every public step checks the
complete prepared/result charge against requested and allocator-usable extents.
All cases check final heap, descriptor, memory-reservation and scratch release.

The minimum observed usable headroom is 7,624 bytes on macOS and 8,872 on Linux,
with the same values at both pathname lengths. No discrepancy required an engine
repair. The count-only 512-row case uses 32,888 temporary bytes and 7,710 public
steps. These are complete-query resource observations, not elapsed-time benchmarks.
The grouped case checks admission headroom through its public steps; it does not
claim to force hash fallback. Replay retains the checked run and full count while
resetting its cursor; the existing forced-replay controls remain in the complete
engine checkpoint. No new within-step peak, arbitrary-allocator, Windows or RSS
qualification follows from these parked public boundaries.

A one-byte nonheap attribution error rejects through the same equation. The
runner's interpretation test rejects a missing analytic completion marker even
after successful process exit. Both platforms pass maintenance with 96 tooling
tests, 44 independently reproduced codec fixtures and 521 local links. The final
679 inputs match across platforms and are retained at `0e94fa7`, with manifest
SHA-256 `ab925e1bcaeb55a3c401cb5403fa804f62b06b8e5d65e36c3e93c65aecd7f70e`.
The final ownership log hashes are
`1fb67cac3585826cce83f895bcb43e64b277b06809edd1c9999466774d8763e0` (macOS) and
`f71af940d06d14ab97be8d292f8013f367d07ee32c0fcdbd94dbe74de80d1d19` (Linux).
Only notes change afterward. Complete engine gates were not repeated for this
tooling-only change. Owned probes, builds, databases, exports, logs and the
container are removed; the verification image and toolchains remain.

### Snapshot lifetime learning example

`db66f28` adds `examples/snapshots.rs` and its linked walkthrough. Literal amounts
10 and 20 belong to the first prepared generation; appending 30 produces a new
view. The example verifies the old rows after reclamation while the old plan is
pinned, verifies the new three-row view, drops the old plan, reclaims again and
verifies the latest rows after close/reopen. Each result must finish successfully
with exactly the expected ordered values. Reclaimed filenames/bytes are not
predicted. The reading path connects preparation's retained snapshot to the
reclamation walk's captured current and pinned views.

The documented flow produces identical five-line output on stock macOS and
unprivileged native-storage GNU arm64 Linux. Both compile/run the release example
and pass warnings-denied example Clippy. Cargo discovers the new example target;
the existing all-target gate includes it automatically. Missing/extra arguments,
a relative path and an existing database path reject on macOS; the existing-path
control also runs on GNU/Linux. The macOS existing database's file hashes remain
unchanged after rejection. Linux executes the documented directory cleanup.

Maintenance passes 96 tooling tests, 44 independent codec fixtures and 566
local links. Final
formatting and documentation checks pass, including 509 local links. Engine
sources are unchanged; full engine gates were not repeated for this example and
walkthrough. The source example matches the file executed on both platforms;
the final documentation-only edit adds direct owner links. All owned example
outputs, builds, exports, logs and the verification container are removed. The
user-owned verification image/toolchains remain. This is a sequential lifetime
example, not additional arbitrary-concurrency, Windows or durability qualification.

### Prepared aggregate descriptor ownership

Tool-only follow-up `63c3c2d` preserves the engine inputs of the frozen checkpoint at `8a7b1ee`. The complete ownership selection passes on stock macOS and unprivileged
GNU arm64 Linux with native database storage. Each pathname length executes
widths 1–10, partitions `[1,9]`, `[5,5]`, `[9,1]` and ten single-entry stages,
and rejection controls for widths 11–64. The query-wide aggregate budget remains
ten outputs, including grouping keys. COUNT results are explicitly 512 over the
literal source rows and one after another reducing stage. All cases check
preparation attribution and release; accepted cases also check usable extents,
complete results and result release. A one-byte preparation allowance error fails.
The runner rejects missing completion markers even after a successful exit.

The retained-family inventory is:

| Owner | Source bound | Maintained consumers and checks |
| --- | --- | --- |
| Owned plan | One allocation; other fixed plan arrays live inside it. | Every prepared query; fixed-key and composed ownership controls. |
| Computations | At most 80 syntax-pool entries; physical capacity is padded separately. | Projection binding/validation; constant ownership and exact/short preparation checks. |
| Aggregate plans | At most ten, sharing the aggregate-output budget. | Aggregate binding and execution; the ten-stage case observes the maximum controller vector. |
| Aggregate entries | At most ten across all plans, less any grouping keys. | Independent semantic validation checks each vector's length/capacity; the new complete width census and partitions observe allocation and release. |
| DISTINCT descriptors | At most sixteen normalized stages. | Full-row distinct validation/execution, typed reader ownership and public catalog tests. |
| UNION descriptors | At most eight: each adds a branch source and union node to the sixteen-stage pool. | Positional binding/validation, union scope admission and public union/composition cases. |

`BindingBudget::calculate` reserves one fixed 4,096-byte allowance for the plan,
one for each nonempty descriptor vector, and one per aggregate-entry vector.
For the new COUNT queries, the independent caller uses exactly `2 + stages`
allocations. Name-scope/catalog scratch is transient and outside the retained
observation; the existing scope tests own its exact/short admission coverage.
No production allocation function is used to calculate this attribution term.

The ten-entry plan charges 27,648 bytes and requests 15,216. Usable extents are
16,832 on macOS and 15,240 on GNU/Linux. Ten single-entry stages charge 68,400
and request 19,104, with usable extents of 21,760 and 19,216 respectively. No gap
was found and no engine allowance, capacity, validator or language limit changed.
The family inventory identifies bounds and consumers; it does not turn the
existing representative DISTINCT/UNION cases into exhaustive native qualification.

Verification includes both complete ownership selections, all their retained
controls, 96 tooling tests, 44 independent codec fixtures, caller formatting and
502 local documentation links. Full engine gates were not repeated for these
caller-only changes. The 675-input manifest at `63c3c2d` has SHA-256
`578af6ce17fa7554af45c7fdbdb7757bbf960926b370b81eef4041414086ce8d`.
Caller hashes are `836032ceebe248ddd97540d4944b39760f36fe7a85e0e8c6bf6ec3353b0cceb3`
(macOS) and `cb136bd524b63cfab058a900c0e131412940946a8f6d3617bccd93cc5e9bff56`
(GNU/Linux). Owned builds, fixture databases, exports, logs and the container are
removed; the existing image/toolchains remain. Arbitrary allocators, transient
whole-process peaks, RSS and Windows runtime remain unqualified.

### Legacy constant allocation ownership

`21ba9c5` extends the existing public allocation caller with six direct legacy
cases and four text-extrema cases at both pathname lengths. Fixed-key controls,
empty and 32-byte UTF-8 constants, one/64 columns, two full reused batches and
MIN/MAX/COUNT results remain explicit. Admission, output and terminal observations
use requested-plus-nonheap-equals-charged equations derived from scan, physical
plan, aggregate and pending scratch-path reservations. A one-byte attribution
error must fail. The prepared and result owners also check usable extents and
release independently. Both complete gates execute every case and the control.

The maximum-length grouped case exposed a 16,384-byte reporting omission. Hash
text growth already reserved memory, but its retained and replacement reservations
were absent from the public query report. The report now includes both. The
existing growth test reconciles it against the database's independent account at
each step, admits an exact 262,144-byte replacement, refuses at 262,143 bytes,
checks cancellation during overlap, and verifies values, release and healthy reuse.

A 64-constant prepared plan separately requested 34,304 computation bytes that
occupied 49,152 usable bytes on macOS. Its complete charge was 51,824 bytes against
59,392 usable bytes. Computation vectors now admit real padded slot capacity using
the existing buffer geometry; their logical definition limit and 4,096-byte
allocation allowance remain unchanged. Independent semantic validation checks
capacity separately from logical definitions. Preparation at 30, 31 and 64 columns
passes exact admission, one-byte-short refusal and release checks.

The wide prepared owner now charges 66,296 bytes and requests 57,960 on both
platforms. Usable extents are 59,392 on macOS and 57,968 on GNU/Linux. This keeps
macOS usable storage unchanged while increasing actual requested capacity by
14,472 bytes; it is a capacity tradeoff, not an RSS reduction. Final allocation
caller hashes are `0f893d5b364814f55b4e19987f89db593293ee98624f5064af36ba6d196538b4`
(macOS) and `a5217504d86868cce59ce911387f99281b10ae5563ff7ed0df999dc2f2bbfef5`
(GNU/Linux). These finite profiles do not qualify every prepared descriptor family,
arbitrary allocators or whole-process memory.

### STRING and DATE projection constants

Commit `8c7ec59` connects bounded constants to SELECT, EXTEND and SET through the
existing computed descriptors. STRING values own decoded bytes; DATE folding
shares predicate calendar rules. Constants have fresh identities, no input
dependencies and no numeric scratch buffers. Legacy queries with text constants
use UTF-8 batches and general grouping; the conservative query-wide domain,
including dead constants, is specified in [resources](../docs/resources.md#limit-admission).
Persistent formats remain unchanged.

The first three public cases failed at the numeric parser before implementation.
All seven retained constant tests now execute on both platforms, covering exact
values, range-preserving SET, grouping, sorting, union, malformed unused inputs,
source-text release and early drop. The maximum-width case verifies 64 columns
of 32-byte text over two full declared batches. Legacy checks retain 600-row
batch/group/extrema values and add 64 maximum literals to both ordinary and
bounded-stack runs. Semantic mutations reject inconsistent type, NULLability,
identity and provenance; physical mutations reject changed slots and producer
ownership. Cancellation paths and the armed catalog allocation caller include
constants. The old text-projection rejection becomes an unsupported STRING
arithmetic check; nonreserved DATE names remain usable as column aliases.

The [constant tutorial](../docs/query-examples.md#add-constant-labels-and-dates)
runs from fresh databases on both platforms: north/5, north/10 and south/20 each
receive `reported` and `2000-02-29`, with successful completion. An additional
stock-CLI check uses `FROM sales AS f |> EXTEND 'joined' AS tag |> JOIN sales AS r
ON f.amount=r.amount |> ORDER BY f.amount |> SELECT tag,f.amount,r.amount`.
Both platforms return exactly three rows, with `joined` and equal key pairs
5/5, 10/10 and 20/20.

An exploratory debug/parallel library run aborted on a bounded stack and is
excluded. Its exited process's eleven leftover fixtures were removed. Required
release/serial checks pass. A first DATE representation exceeded the retained
parser bound; splitting interval and unit operations repaired it without raising
the bound. A selected diagnostic from the frozen macOS test binary reports
4,452 parser bytes, 198 expression bytes and a 960-byte shared operation array,
below the unchanged 4,500-byte parser limit. The wide declared test's initial
4 MB budget correctly refused its roughly 4.4 MB workspace; its 16 MB fixture
budget admits the full-width case. This changes no engine allowance. Untyped
NULL projections, casts and arbitrary STRING functions remain unsupported.

### Literal-list membership

Commit `245609e` implements the [bounded IN profile](../docs/language.md#literal-list-membership)
through the existing Boolean decisions. Before implementation, the public case rejected `id IN
(1,3,1)` at the comparison parser. The repair lowers candidates to equality
leaves joined by OR and adds an explicit NULL candidate. Independent semantic
validation permits that candidate only with equality; physical validation checks
literals and branch decisions against the bound plan. No new allocation owner
is introduced, and persistent formats remain unchanged.

All five new public tests execute on both platforms. Explicit rows cover
INT64/DOUBLE, NaN, STRING/Unicode, DATE, NULL, duplicates, negation and composed
producers. An independent nullable-set model checks all nine pairs of NULL, zero
and seven across four lists and six Boolean forms. Malformed and mismatched
operands fail preparation even in skipped branches. Demanded-overflow checks
retain exact expression spans; skipped branches avoid that error. SELECT plus
fifteen candidates reaches the existing sixteen-stage limit, and another
candidate is rejected. Cancellation, early drop and repeated execution return
to the resource baseline. The retained ordinary/small-stack tests observe the
same execution charge for membership and the equivalent OR filter.

Disposable mutations on both platforms replace NULL's UNKNOWN with FALSE or
replace the membership OR with AND. The unchanged public row oracle rejects both
with exit 101. The first mutation incorrectly returns IDs zero and two from
negated membership containing NULL; the second loses matching rows. Healthy
callers pass. Semantic mutations reject invalid NULL comparison/control/identity
state; physical mutations reject changed literals, negation, branches and filter
counts. Seven independent stock-CLI cases additionally cover the legacy scan's
numeric, STRING, DATE and NULL paths. The catalog allocation caller now uses
membership in its derived join while preserving its independent count. Both
full campaigns cover every current refusal prefix and healthy control.

The [tutorial](../docs/query-examples.md#filter-by-membership) runs from fresh
databases on both platforms. `examples/membership.sql` returns north/5 followed
by south/20 and completes successfully; its negation completes with zero rows.
An initial empty exact test selection and a composition run whose CLI changed
during a build are excluded. Corrected full selectors execute their tests, and
the composition rerun uses a fixed binary. The full gates above use immutable
stock artifacts and pass. Owned example databases, controls, targets, source exports,
logs and containers are removed. General expression operands, subquery IN,
NOT IN spelling and IN UNNEST remain outside this accepted profile; Windows,
whole-process memory and broader durability qualification remain unfinished.

### STRING reader allocation attribution

Commits `03e4736`, `a03f953` and `71b8b71` extend the maintained
`composed-ownership.rs::reader_shapes` caller and repair two measured allocation
owners. The original twelve fixed-width cases remain. Eight STRING cases cover
one/64 columns, ORDER BY/DISTINCT, NULL, duplicates, empty text, Unicode and
65,536-byte cells. Independent values and multiplicities remain visible beside
the SQL. Parked ownership, admission and final release assertions are unchanged.

The initial macOS short-text 64-column ORDER BY case observes 55,230,368 usable
bytes against a 55,177,616-byte charge. A disposable trace attributes 62,464
rounding bytes to 128 text metadata allocations: each requests 2,072 bytes and
occupies 2,560. `TextColumn` now stays inline in `Column`, with a separate
2,048-byte span allocation. The span allocation replaces the former singleton
owner; the text arena remains separate. Column metadata grows from 64 to 80
bytes, while total requested ownership per text column falls by eight bytes.
The first repaired observation is 55,166,880 usable against 55,176,592 charged.

GNU/Linux then exposes a separate allocator-state-dependent excess. Its traced
524,288-byte payloads occupy 528,368 when mapped, and 4,210,688-byte sorting
buffers occupy 4,214,768. Shared `resources::buffer_capacity` now requests large
buffers ending 32 bytes below a 16-KiB boundary for native headers/alignment.
Admission charges that actual capacity; no allowance or attribution equation is
weakened. Encoded column limits remain 524,288 bytes. STRING payload capacity
increases to 540,640 bytes, an extra 16,352 bytes per column. Fixed-width padded
payloads decrease by 32 bytes. The [resource contract](../docs/resources.md#blocking-buffer-capacity)
owns the geometry and its native qualification limits.

STRING fixtures use 64 MiB memory and 64 MB temporary storage; fixed-width
fixtures retain 64 MB memory and 8 MB temporary storage. Maximum-width STRING
DISTINCT charges 64,586,992 bytes. Focused integrated observations remain below
that charge on both platforms. The complete gates execute 40 reader cases on
macOS and 120 on GNU/Linux. Linux uses default, fixed 128-KiB and fixed 64-MiB
mmap thresholds at both short and 384-byte pathnames. Independent allocation
controls observe 4,080 and eight rounding bytes respectively for a 524,288-byte
request, confirming that the configured regimes exercise different paths.

Disposable wrong-empty-value and wrong-DISTINCT-multiplicity callers fail their
exact equality assertions on both platforms; healthy callers pass. Tooling
controls reject missing allocator observations even when other success markers
are present. Batch checks retain short-reservation preservation, sparse and
replacement writes, allocation reuse and release. The 514 buffer extents and 91
hash layouts remain covered. Catalog allocation discovery now observes 858
calls; both gates exercise every refusal position and the healthy prefix at each
pathname length, rather than retaining the old layout's 863-call census.

Earlier full gates at `03e4736` and `a03f953` fail in Rust tests on four stale
physical-capacity expectations. Their later stages do not count as evidence.
The corrected tests retain the original encoded limits, 65,537-byte/1,025-row
refusals, checksummed corruption and failed-refill invalidation. The final full
gates above pass all retained checks. This qualifies the observed native
allocation profiles, not custom allocators, transient peaks, arbitrary schedules,
Windows, whole-process memory or RSS. Source revisions and maintained fixtures
reconstruct the controls; disposable traces and outputs are removed.

### Concurrent readers during reclamation

The new public test in [snapshots.rs](../tests/catalog_lifecycle/snapshots.rs)
parks two worker-owned results on generations containing 11 and 11/22. The
parent publishes 33, reclaims and resolves receipts. The older worker drops its
unfinished result, rereads its pinned plan and releases that plan. Another
reclamation then runs while the middle reader remains parked. That reader
completes and rereads exactly 11/22. Reexecuting the pinned plans detects unlink
that an already-open file might mask. The test checks resource release,
idempotent final reclamation, append 44, fresh results and all receipts after
close/reopen. Explicit expected rows and bounded channel/step waits keep the
schedule and oracle local. No production code or resource allowance changed.

The stock scenario passes on both platforms. In disposable source copies,
changing `Reachable::open` from current-or-data-pinned roots to current roots only
makes that same test fail while reopening the older query: `inspect namespace
entry` returns NotFound. Each control exits 101 with one failed test; its parked
sibling exits at the 30-second receive deadline during failure teardown. The
controls are reconstructed from `b8f1b8f` with that single mutation and need no
retained binary or database. This qualifies the exercised deterministic schedule
of overlapping lifetimes; it does not establish arbitrary-race detection,
parallel execution schedules, sanitizer coverage or hardware durability.

## Earlier union verification checkpoint

The September 12, 2026 complete gates for `1153e8d` passed all 24 stages on
macOS and GNU arm64 Linux. Both used Rust 1.98.1, release artifacts, offline
locked dependencies, and warnings-denied compilation and documentation. macOS
used arm64 Darwin 25.6.0, Python 3.14.7, and the native Apple toolchain. Linux
used uid/gid 1000, GNU libc 2.36, and native overlay storage; its source export
was mounted read-only. The existing verification image and toolchains remain.

Formatting, maintenance, filesystem ABI, rounding vectors, attempt models,
Clippy, Rust tests, rustdoc, doctests, stock CLI construction, aggregate semantics
and composition, public/CLI allocation, native initialization/synchronization/I/O,
catalog interruption and independent graph inspection all passed. Maintenance
executed 96 tooling tests, reproduced 44 codec fixtures, and checked 507 local
links. Independent aggregate semantics checked 24 cases and composition checked
304 cases against unchanged stock CLI artifacts.

Each platform executed 506 ordinary Rust tests without failures or ignored tests.
macOS executed 371 library and 21 filesystem tests; Linux executed 373 library
and 19 filesystem tests. Shared suites executed 15 CLI, 79 catalog, seven
execution, seven lifecycle and six load tests. The lease test separately ran its
normal child and intentionally killed its early-teardown child before verifying
reopen. All 11 public union tests and the retained bounded-thread scenarios ran
on both platforms. Four example targets compile without test bodies; their
compilation is separate from the example execution evidence below.

The 671 inputs matched before/after and across both gates. The frozen manifest
SHA-256 is `51bab09aff511f76a420220bb2729dfda88db1bae546b7e04d149b45c30732e9`;
`1153e8d` retains those exact inputs. Finalization changes only README's capability
summary and the two notes files. The other 668 manifested inputs retain
fingerprint `ffaadc7314b6fa9e5260e2a74ebae4116ebd6bb3a56479377339e2b25c8138e9`.
This identifies source, not reproducible binaries.

Stage times totaled 1,747.002 seconds on macOS and 1,064.727 seconds on Linux.
The gates overlapped after macOS Rust testing; these are verification costs, not
query benchmarks. Their result-receipt SHA-256 values are respectively
`a07b7305d1701b13e97a94b5d82142989063bf76a064b2c3aaef7c70086784d1` and
`c36613605e127c44412a9aa04ac0833a63e0e4274e7a6d64a125f0896883488e`.
Both receipts have zero finalization errors. Owned targets, composition databases,
logs, exports, tutorial databases and verification containers are removed after
recording the evidence; current source and fixtures reconstruct the checks.

Both platforms pass all 863 catalog allocation-refusal positions and healthy
controls at short and 384-byte pathnames, 76 append interruption cuts, 46 recovery
cuts, 249 independent graph checks during interruption, 43 graph cases with
retained negative controls, and 1,028 native I/O cells. Linux retains the two
Darwin ACL exclusions. The finite testing/tooling cleanup at `a315e21` and all
subsequent payload-capacity, append-growth, grouped-buffer and hash-layout
regressions remain in the gate. Persistent formats and publication algorithms
are unchanged by this milestone.

The initial macOS gate at `c908071` timed out after 900 seconds in public
allocation, after long-path refusal position 848. It did not run remaining
recovery allocation cells or later native stages and is not passing evidence.
`1153e8d` allows 1,200 seconds for that expanded stage while retaining all cells,
the 20-second cell deadline, failure status and descendant cleanup. Final
allocation stages completed in 1,033.072 seconds on macOS and 601.897 seconds on
Linux. Gate timeout controls pass; no engine memory allowance increased.

The declared-table, column-transform and updated union tutorials, three numeric
grouping cases, eight default STRING cases, two bulk STRING cases and both
composed budgets execute successfully on both platforms. They use the unchanged
engine/example inputs from `c908071`; only the gate deadline and plan changed
before the final freeze. The union example produces north total 15/count 3 and
south total 20/count 1 with `status=queried`. On macOS its ALL variant produces
25/count 4 and 20/count 1, and a deliberately wrong expected count 999 is rejected
against actual count 4 by the stock composition checker.

Run `sh tools/check.sh --output /absolute/new-result-directory` with the
[documented prerequisites](../docs/testing.md#complete-local-gate). Windows,
broader durability, physical memory bounds and sanitizer/concurrency qualification
retain their existing limitations. Release gates do not qualify debug small-stack
execution; an exploratory debug selection aborted and its owned outputs were
removed.

## Catalog healing witnesses

`aa5c382` consolidates only the catalog allocation caller's post-reopen queries.
An ordered scan independently checks all three stored columns, exact INT64 and
DOUBLE values, NULLs, Unicode/control text and duplicate multiplicity. It checks
nonempty scratch use and final memory/temp release. Receipt resolution and a
usable writer remain separate. The complete armed sequence, diagnostics,
destruction, same-handle scan/debt checks and outcome classification are textually
unchanged from `1153e8d`. Public aggregate tests retain ordinary close/reopen
semantic checks; no engine, filesystem, vendor or build input changed.

Complete default `tools/check-diagnostic-allocation.py` campaigns pass on macOS
and unprivileged native-storage GNU arm64 Linux. Each executes refusal positions
0–862 and healthy control 863 at both short and 384-byte pathnames. Both observe
the same 38 catalog phases and pass the unchanged required-outcome checks and
remaining lifecycle/load/recovery cells. Linux retains both Darwin ACL exclusions.
Maintenance on each platform passes 96 tooling tests and 44 independent codec
fixtures. Formatting and warnings-denied standalone caller compilation pass.
These focused checks supplement the preceding full-gate checkpoint.

Disposable callers against the same stock library challenge the changed oracle
on each platform. Replacing its DOUBLE 3.5 expectation with 3.75 rejects; reducing
only reopened temporary capacity to one byte rejects. Masking the same-handle
scratch error at allocation prefix 399 rejects the RecoveryRequired assertion.
Matching healthy full-census and prefix-399 controls pass. No modified caller
or generated database is a maintained input.

Observed complete campaign costs are about 712.875 seconds on macOS (log lifetime)
and 351.443 seconds on Linux (monotonic subprocess duration), versus the prior
full-gate allocation stages' 1,033.072 and 601.897 seconds. These single-run costs
include builds and are not a controlled benchmark. Runs overlapped other work;
the direct library builds used empty RUSTFLAGS while the earlier gate denied
warnings. Compiler, engine inputs and optimization profile are unchanged.
The shared exported 671-input manifest SHA-256 was
`91f95f5b6c10056f474769fd063cf5af3c8a2c660548e3099ada6a18f13a8e1d`;
subsequent finalization edits affect the notes. Logs, controls, build targets,
export, databases and both stopped verification containers are removed. The
existing image and toolchains remain. Windows, broader durability and physical
memory qualification are unchanged.

## Positional UNION DISTINCT

`c908071` normalizes each complete argument list to binary union stages followed
by one ordinary DISTINCT stage. The extra stage consumes the existing 16-stage
budget. No alternate frontend, resource account, sorter or scheduler was added.
The [language contract](../docs/language.md#union-distinct) owns the accepted scope.

Retained and expanded tests check nested ALL/DISTINCT boundaries, the additional
stage, positional names and types, complete-field demand and original overflow
spans, NULL/signed-zero/NaN equivalence, original floating representatives, date
bounds and prepared snapshots across appends. Independent semantic and physical
mutations reject corrupt mappings and bypassed duplicate removal. Full-width
small-stack execution, exact admission, cancellation prefixes, sorter corruption,
short/failed I/O, temporary refusal and observed grouping fallback replay pass.
Late cancellation may observe an already finished producer; the prefix sweep
requires complete output in that case and continues through an uncancelled run.

Six new stock CLI cases use the retained independent catalog fixture at an
explicit 4 MB budget. The original 298 cases retain their 2 MB budget. Fixture
assembly now has one owner shared with graph verification; its bytes and
independent inspector are unchanged, and existing directories or links are
refused before writing. The public allocator sequence adds UNION DISTINCT
prepare/execute/step refusal and healed results. Its census rose from 795 to 863;
the 900-allocation campaign ceiling limits work, not engine memory.

## Positional UNION ALL

The pinned GoogleSQL parser and analyzer fixtures are
[`pipe_set_operation.test`](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/googlesql/parser/testdata/pipe_set_operation.test)
and the corresponding
[analyzer fixture](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/googlesql/analyzer/testdata/pipe_set_operation.test).
Their SHA-256 values are `444ef8e948a1f1c9515c29760d63073bd9c9e045b311252bd4c5a605b1401751`
and `4ba1033c2e770b5e7df93fb35cb476fa1b41a94ba36292ba84a9d624073dea3f`.
The same revision's `resolver_query.cc` establishes argument scope, positional
width checks, fresh outputs, first-input names, and removal of input ranges.
GoogleSQL supports common-supertype coercion; PipeSQL's identical-type restriction
is local. This is source/fixture inspection, not an upstream analyzer execution.

The [language contract](../docs/language.md#union-all) records the bounded local
profile. Reduced parser, binding, physical-plan, runtime, and public SQL tests
keep independent expectations for positional names and identities, types and
NULLability, scope, nested branches, transforms, joins, grouping, DISTINCT,
ordering, LIMIT, demanded overflow spans, typed bytes, and pinned snapshots.
Semantic and physical mutation tests reject corrupt mappings independently.

Both full gates execute the 64-source-column and 64-output-column boundaries,
including the small-stack path. Exact preparation and execution admission checks
refuse one byte short and release reservations; execution refusal precedes source
I/O even for LIMIT 0. Forced sorting/grouping spill, temporary-space refusal,
cancellation at each scheduled prefix, and partial/full replay retain their
independent results and release checks. Replay visits only previously initialized
branches, preserving unvisited aggregates when a LIMIT ends a prefix.

At `98de11f`, the public allocation caller composed typed union, sorting, and
COUNT. Both platforms passed all 790 catalog refusal prefixes and the healthy control on short
and 384-byte paths, including union preparation, execution, and stepping
refusals, healed reopen, and retry. That milestone used an 800-allocation census bound. This is
observed allocation coverage, not a whole-process memory guarantee.

The broad development debug run aborted on a small-stack test; it is not passing
evidence. The required release-profile gates pass with unchanged stack ceilings.
No persistent format or publication algorithm changed. Linux's two Darwin ACL
cells, Windows, broader filesystem durability, allocator-usable memory bounds,
and sanitizer coverage retain their existing qualification limits.

## Column transformation semantics

The SET, DROP, and RENAME contract comes from GoogleSQL revision
`0e7d7073ed0360be587a5efa0fa78abeee00f17b`, specifically the analyzer fixtures
[pipe_set.test](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/googlesql/analyzer/testdata/pipe_set.test),
[pipe_drop.test](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/googlesql/analyzer/testdata/pipe_drop.test), and
[pipe_rename.test](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/googlesql/analyzer/testdata/pipe_rename.test),
together with `ResolvePipeSet`, `ResolvePipeDrop`, `ResolvePipeRename`, and the
name-scope implementation. This is source and fixture evidence, not a fresh
upstream analyzer execution. The respective fixture SHA-256 values are:

- `efb519559a8bdff538d59b4302f6674751f6a718735cf8464682283175a665c3`
- `c088818f5c9ceefa5825445083995e78319aa113010c4fa2ce00bd61ac591420`
- `5664b7886de4ffaae069c51cdc0904b427135f2659e0c53d60b931c06b451bde`

Reduced regressions in the frontend and public computation tests preserve
simultaneous assignments, ambiguous and missing targets, duplicate targets,
fresh typed copies, original range members, type/NULLability, exact spans, and
hidden versus demanded failures. The [language manifest](../docs/language.md#current-public-query-manifest)
owns the accepted profile. `check_column_transforms` in the ordinary composition
campaign compares output against independently encoded input rows; its expected
answers do not come from the binder or physical planner.

Two prerequisite defects have durable regressions. Qualified original values
must survive DROP/SET even when absent from ordinary outputs; grouping and numeric
binding must use that scope too. Sorting all 64 visible keys plus one original
value requires 65 intermediate values. Internal owners now admit at most 128
values while native schemas and public rows stay at 64. Larger inline arrays
remain charged to their existing owners; this is an accepted capacity cost,
not a performance improvement. The unchanged small-stack regression exposed
stack growth during preparation. Allocating the admitted plan in a separate
construction frame before binding repaired that failure.

The column-transformation gates at `5a3cb0a` covered all 298 independent
composition cases. Their allocation caller composed EXTEND, SET, DROP, and RENAME
against unchanged expected rows; both platforms passed all 723 catalog refusal
prefixes and the healthy control on short and 384-byte paths. The current
checkpoint above extends that caller and records its larger census.
Cancellation, exact admission refusal, invalid scope/identity controls, typed
copies, and the 65-value sorting regression execute in the Rust suites.

The revised [tutorial](../docs/query-examples.md#transform-columns-while-retaining-the-original-values)
runs from the frozen source on both platforms with fresh native-filesystem
databases. The declared example prints the expected north/south totals. The
transformation query returns `(north, 5, 10, 11)`, `(north, 10, 20, 21)`, and
`(south, 20, 40, 41)`, followed by `row_count=3`, `status=queried`, and exit zero.
The owned example databases and build targets were removed after verification.

## EXTEND projection semantics

EXTEND appends direct references or current numeric expressions while preserving
input columns, identities, range members, and nonanalytic ordering. Each list
binds against its original input; later EXTEND stages can use earlier aliases.
Duplicate names remain visible but ambiguous when referenced. No aggregate,
window, or additional scalar forms are admitted. The
[language manifest](../docs/language.md#current-public-query-manifest) owns the
accepted syntax and demand rules.

The semantic review used GoogleSQL commit
`0e7d7073ed0360be587a5efa0fa78abeee00f17b`. Its
[EXTEND analyzer fixtures](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/googlesql/analyzer/testdata/pipe_extend.test)
provide independent cases for sibling-alias rejection, repeated stages, duplicate
names, and range preservation. `ResolvePipeExtend` and the input-name merge in
`googlesql/analyzer/resolver_query.cc` establish direct-reference identity reuse
and retained scope. `ResolvedProjectScan` in
`googlesql/resolved_ast/gen_resolved_ast.py` propagates input ordering. These are
pinned fixture and source observations, not a fresh upstream analyzer run.
The reviewed source SHA-256 values are:

| Source at that commit | SHA-256 |
| --- | --- |
| `pipe_extend.test` | `51774f9c05fa4f10bed268a5f9fd2e3939f2e030b777e181cb9ec80a4a236249` |
| `resolver_query.cc` | `fc438f439784f0b02e7ba76437f2d0f4fc80ee97d19d701dd3f09755a97e5177` |
| `gen_resolved_ast.py` | `28a2b60b1800d67b32a8bc41f069c294a2c6982d88907bfcc5771dd05cd7435e` |

The implementation records only appended projection entries; repeated EXTEND
stages inherit existing columns without exhausting the syntax-sized pool through
copies. Independent validation checks combined width and definition scope. A
prerequisite parser repair preserves direct STRING references named `aggregate`,
which the existing grammar already allows as an identifier.

The full gates add six Rust regressions and extend existing validator and
cancellation checks. They cover all scalar types, NULL and empty input, retained
ranges and identities, sibling and colliding aliases, 64-column admission,
one-byte-short preparation refusal, hidden versus demanded overflow, exact
source spans, and composition through filters, grouping, joins, ordering, and
derived inputs. Both composition campaigns execute 293 cases. The catalog
allocation campaign includes EXTEND and still passes all 723 injected prefixes
plus healthy completion on ordinary and 384-byte paths. Cleanup and reservation
release remain checked; there are no persistent-format changes.

Replay the focused checks with:

```sh
cargo test --release --offline --locked --lib frontend:: -- --test-threads=1
cargo test --release --offline --locked --test catalog_lifecycle computed:: -- --test-threads=1
```

The [learning example](../docs/query-examples.md#transform-columns-while-retaining-the-original-values)
ran from fresh databases on macOS and Linux. Both returned the same schema and
three expected rows, then `row_count=3`, `status=queried`, and exit zero. Its SQL,
setup program, and expected values are maintained inputs. Successful raw output
and temporary source copies are not replay dependencies. Windows and production
qualification remain open.

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
two-key, and repeated aggregation under 2 MB. At `827cad5`, declared STRING
extrema reserved 65,536 bytes per slot per group regardless of actual length.
The compact hash representation below replaces that cost while retaining fixed
slots for disk reduction. Persistent formats are unchanged.

The extended public allocation caller checks numeric and text extrema alongside
its existing COUNT/SUM/AVG results. At `827cad5`, both platform gates executed 721
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
above. It executed 498 Rust tests, with no ignored tests and one
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
2,700,983 bytes on the short path and 2,701,142 bytes on the long path; GNU/Linux
observed 2,700,952 and 2,701,126 bytes. Both readers completed on both platforms,
and the departing reader restored the held owner's allocation totals.

The current gate's held-group checkpoints also distinguish logical database
charges from the caller's live requested and allocator-usable totals. Every row
below has zero temporary bytes and six observed descriptors. Requested/usable
totals include other allocations visible to the caller's allocator observer;
they are not an isolated grouping allocation or a whole-process memory bound.

| Platform and path | Logical charge | Requested bytes | Usable bytes |
| --- | ---: | ---: | ---: |
| macOS, short | 2,700,983 | 2,653,063 | 2,711,936 |
| macOS, long | 2,701,142 | 2,654,302 | 2,713,328 |
| GNU/Linux, short | 2,700,952 | 2,652,696 | 2,662,280 |
| GNU/Linux, long | 2,701,126 | 2,654,158 | 2,663,864 |

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

### Attribution of composed memory

The [resource equations](../docs/resources.md#interpret-composed-memory-observations)
separate logical charges, live requested bytes, usable extents, inline handles,
path allowances, and caller synchronization. Owners are sampled while readers
are parked; their independently observed changes must sum to the global change.
Cancellation and completion release the affected owner while preserving the
others. Caller barriers and observation mutexes are destroyed and measured before
database-close reconciliation. No constant subtraction hides caller allocations.

The append deficit was reproduced on `b8a7e4a` runtime inputs. Its small write
charged 139,905 bytes and requested 131,241; macOS reported 147,520 usable bytes,
exceeding the charge by 7,615. GNU/Linux reported 131,272 usable bytes. A bounded
caller trace isolated requests of 65,641, 65,536, and 64 bytes, occupying 81,920,
65,536, and 64 usable bytes on macOS. The first request summed 96 metadata bytes,
nine column bytes, and 65,536 later commit-scratch bytes despite disjoint lifetimes.
The disposable trace was removed; `813a049` records its finding.

Repair `1633477` takes the maximum of encoding and commit workspace requirements.
The complete native size census additionally observes rounding up to 16,383 bytes
for workspaces and 16,352 for reference arrays on macOS. The largest GNU/Linux
observations are 3,687 and eight bytes. Each of the three retained allocations
therefore receives its own explicit 16,384-byte ceiling before allocation or
issuance. The caller checks all 460,865 workspace sizes and 4,096 reference counts,
then verifies full-width writes at the maximum encoded-column size with reference
capacities of 1,025 and 4,096. Small/maximum/small writes must grow and reuse the
workspace, publish COUNT/SUM results of `(24, 168)` then `(48, 336)`, and release
heap, logical, and temporary ownership. Internal exact/one-byte-short tests check
admission before effects and old-workspace release before replacement.

Both full gates include these checks, short/384-byte composed ownership, physical
and logical allocation refusal, temporary refusal, cancellation, commit, and final
release. Negative controls reject a missing rounding ceiling, an incorrect complete
row, and a one-byte attribution error at distinct checks. Repair-checkpoint driver SHA-256:

- macOS: `428221297c35af8ffd1c75e99bb55b74f4dc8ca01ba392d40f41e1d944b5298f`
- GNU/Linux: `2fb958e788f79eb8b25a1d8655402c4a5b5a623558cf65c5f2e295aaa37e3012`

Selected final observations are:

| Platform and owner | Logical charge | Requested bytes | Usable bytes |
| --- | ---: | ---: | ---: |
| macOS, small append | 188,952 | 131,136 | 131,136 |
| GNU/Linux, small append | 188,952 | 131,136 | 131,160 |
| macOS, maximum-column append with 1,025 references | 682,552 | 624,736 | 655,360 |
| GNU/Linux, same append | 682,552 | 624,736 | 626,720 |
| macOS, maximum-column append with 4,096 references | 780,824 | 723,008 | 737,280 |
| GNU/Linux, same append | 780,824 | 723,008 | 723,032 |
| macOS, parked ORDER BY or DISTINCT, short path | 546,692 | 533,972 | 538,784 |
| GNU/Linux, same reader | 546,692 | 533,920 | 538,176 |
| Both, terminal reader | 552 | 0 | 0 |

For the small append, usable macOS memory decreases by 16,384 bytes while the
logical charge increases by 49,047 bytes. For encoding demands at least 65,536
bytes, shared workspace drops 65,536 requested bytes and the added rounding
reservation is 49,152 bytes, reducing the retained logical charge by 16,384.
These are capacity/accounting changes, not throughput or RSS measurements.

Repair `ca5f59e` closes the sampled reader deficit by requesting and charging
whole 16-KiB native payload capacities. The 21-allocation trace found the main
increment in the INT64 source buffer: 266,240 requested bytes occupied 278,528
usable bytes on macOS. Source and debug-type attribution of the smaller scan,
run-arena, pipeline, controller, and node allocations is retained in that
revision's plan. No sorter or generic allocation allowance was added; the
independent reader equation is unchanged. A caller linked to the previous library
rejects its usable extent with exit 101 after joining the parked readers.

The stock caller checks one and 64 INT64, DOUBLE, and DATE columns through both
ORDER BY and DISTINCT, complete nullable results, and final release. Internal
checks observe all four payload capacities and exact/one-byte-short admission
before I/O. The first full gates failed two nullable COUNT fixtures whose
1,600,000-byte budget no longer admitted three fixed-width source buffers. Adding
exactly 36,864 bytes to that test budget preserves actual spill, cancellation after
spill, temporary refusal, retry, and release; both final gates execute those cases.

On the 384-byte path, both readers request 534,242 bytes. macOS reports 539,104
usable bytes and GNU/Linux 538,496, within the same 546,692-byte charge. The former
macOS short/long deficits were 4,380/4,700 bytes under a 534,404-byte charge.
The new request leaves macOS usable extents unchanged and increases the observed
GNU/Linux extent by 12,288 bytes. This is an accounting/capacity repair, not a
physical-memory reduction.

The `ca5f59e` full-gate healthy catalog controls exposed a separate grouped-query
owner excess. For short/384-byte paths, macOS reported charges of
3,977,576/3,977,312 bytes, requested heap 3,926,846, and usable extents of
4,089,072/4,089,392: deficits of 111,496/112,080 bytes. GNU/Linux charges
3,977,644/3,977,328 covered requested 3,926,862 and usable 3,927,376/3,927,344.
The sample includes the GROUPED prepared query and result immediately after
execution admission in [catalog-allocation.rs](../tools/fixtures/catalog-allocation.rs).

A disposable caller trace reconciled all 66 retained allocations against the
independent heap delta without instrumenting the library. Result/run/merge/prior-key
requests of 196,709, 364,548, 131,133 (twice), and 65,541 bytes occupied 212,992,
376,832, 147,456 (twice), and 81,920 on macOS. The 2,836-group hash arrays and key
arena also crossed native allocation classes. Reducing the default hash count to
2,048 alone left a 54,148-byte deficit; it was not sufficient qualification.

Repair `495cbdd` also requests and charges whole 16-KiB capacities for large
blocking buffers and run-span arrays. Optional run growth fits actual allocation
steps while preserving the external minimum. Encoded row/frame limits remain
separate; a new regression rejects extra rows and a valid oversized checksummed
frame even when padding would hold them. The [resource contract](../docs/resources.md#blocking-buffer-capacity)
owns the capacity policy and earlier-hash-fallback tradeoff. No allowance was
inflated and the independent full-row and owner equations remain unchanged.

The maintained usable-byte guard rejects the old library on both path lengths
with exit 101 and accepts the repair. Complete final-gate healthy samples are:

| Platform/path | Charge | Requested | Usable |
| --- | ---: | ---: | ---: |
| macOS, short | 3,974,168 | 3,923,438 | 3,933,424 |
| macOS, 384 bytes | 3,974,168 | 3,923,702 | 3,933,744 |
| GNU/Linux, short | 3,974,168 | 3,923,386 | 3,927,992 |
| GNU/Linux, 384 bytes | 3,974,168 | 3,923,702 | 3,928,296 |

Both complete gates preserve 790 catalog refusal prefixes per pathname, full
nullable/extrema rows, exact/short admission, spill/replay, cancellation, and
release. The bounded size census checks 514 aligned byte capacities from 32,768
through 8,437,760 and 91 layouts for this caller's hash states. Darwin large
buffers have zero observed tail; GNU/Linux's largest tail is 4,080 bytes at
147,456. Small hash arrays have maximum tails of 12/20 bytes respectively.
These observations qualify the exercised owner and size families; they do not
cover arbitrary aggregate-state widths, schemas, allocators, or schedules.

Append's ceiling is a qualified premise for the exercised stock allocators and
request-size domains, not arbitrary global allocators or all allocator states.
Parked samples exclude transient peaks, direct foreign allocations, allocator
metadata/retention, and physical stack pages. Windows, broader durability,
arbitrary schedules, and whole-process memory remain unqualified.

### Mixed aggregate allocation history

The fresh GROUPED size census did not qualify reuse after other queries. On
`495cbdd` runtime inputs, the 40-case sequential macOS profile completed every
row and released every owner, but 16 cases at the 16 MB budget exceeded their
prepared/result charge. The largest excess was 2,819,476 bytes for seven integer
minima: charge 15,980,652, requested 15,943,203, usable 18,800,128. A 48-allocation
trace reconciled the complete owner and found its 7,700,480-byte key arena occupied
10,551,296 usable bytes. The same isolated query occupied exactly its requested
arena extent and fit admission. Allocation history was therefore a necessary
reproduction input, not just the request size.

Power-of-two key arenas alone left seven deficits; a further trace identified a
3,670,016-byte state array occupying 4,194,304. Repair `986b673` requests real
power-of-two capacities for large state arrays and key arenas. Logical state
lengths and group/key limits stay separate. Key-slot padding is explicit and
charged; sizing reduces optional group capacity when padded arrays would exceed
the metadata budget. The [resource contract](../docs/resources.md#declared-grouping-admission)
owns that policy and its possible earlier fallback. The actual-capacity regression
rejects charging padding without allocating it.

The first Linux public check caught an unnecessary spill caused by rounding the
maximum encoded-key limit down despite spare capacity. The final sizing rounds
the available budget before clamping that limit and reserves its rounded physical
allocation. Existing 4,096-group hash/spill budgets and assertions remain unchanged.
No query throughput improvement is claimed.

[`grouping-ownership.rs`](../tools/fixtures/grouping-ownership.rs) preserves the
reproduction sequence: 4 MB then 16 MB, one/three/five/seven/nine states, floating
sums, integer sums, integer minima, and mixed layouts. Every cell is checked
against the four literal source rows, with NULL key ordering, real hash execution,
and complete owner release. The new caller linked to the old library rejects all
16 original deficits after completing the 40 row/hash/release cases.

Both complete final gates pass all 40 cases. Minimum usable headroom is 29,348
bytes on macOS and 16,732 on GNU/Linux. The seven-minimum 16 MB case now charges
14,178,412 bytes: macOS requests 14,140,978 and observes 14,147,088 usable; Linux
requests 14,140,926 and observes 14,141,296. The original GROUPED healthy controls
also fit: charge 3,957,784 at both paths, macOS usable 3,917,040/3,917,360 and Linux
3,907,536/3,907,840. The catalog census is now 795 allocations at each pathname
length, and every refusal prefix executes. Other schemas, allocator histories,
transient peaks, Windows, and whole-process/RSS memory remain unqualified.

### Composed execution cost

The maintained [composed example](../examples/composed.rs) now reports successful
query time and public Progress/Rows counts. Its input, ordered NULL/count/sum
oracle, completion, resource release, and separate cancellation exercise remain
unchanged. The caller SHA-256 is
`ececd20274affdcbd25e4a339b1edb919e606788fc1a146da115bc0c48077e12`.
The [walkthrough](../docs/getting-started.md#follow-a-join-through-grouping-and-sorting)
owns the two current commands and timing scope.

The finite September 12 profile runs three repetitions at 2.2 MB and 12 MB per
platform, using fresh databases. Budget order is low/high, high/low, low/high.
Both platforms use the same caller against unchanged `986b673` runtime sources,
Rust 1.98.1 stock release libraries, offline locked dependencies, and
`RUSTFLAGS=-Dwarnings`. Callers link with `rustc --edition=2024 -O -C debuginfo=2
-Dwarnings --extern pipesql=LIBRARY -L dependency=DEPS`. macOS uses arm64 Darwin
25.6.0/Python 3.14.7; GNU arm64 Linux uses Python 3.11.2, the retained image,
UID/GID 1000, read-only sources, and native container storage. Platforms run
sequentially to avoid measurement contention.

Every measured process checks all 4,096 descending groups and successful query
release, then reaches temporary storage in a second execution, cancels it, and
checks release again. Parent monotonic time around `check_process.run` includes
setup, opening/preparation, both executions, close, output, and subprocess
supervision. The query's Instant interval includes admission, every result check,
Finished, and result destruction, excluding the second cancellation execution.
All 12 processes complete within their 120-second deadlines. These are recently
constructed inputs, not cold-cache or sustained service measurements.

Times below are milliseconds, median [minimum, maximum]; the larger first macOS
whole-process sample is retained rather than discarded.

| Platform | Query budget | Whole process ms | Successful query ms |
| --- | ---: | ---: | ---: |
| macOS | 2,200,000 | 478.490 [471.649, 1065.343] | 136.323 [133.263, 138.464] |
| macOS | 12,000,000 | 440.300 [435.057, 457.017] | 114.248 [110.920, 114.277] |
| GNU arm64 Linux | 2,200,000 | 161.925 [161.834, 165.564] | 123.036 [122.913, 124.318] |
| GNU arm64 Linux | 12,000,000 | 140.384 [139.999, 140.953] | 103.205 [102.985, 103.304] |

Every low-budget run returns 848,808 Progress steps and 4,096 Rows steps with
2,336,640 sampled temporary bytes. At 12 MB those figures are 549,765, 4,096,
and 1,263,448 bytes. Each query also returns Finished once. These match the
prior allocation workload's observation. Rows counts final output batches;
intermediate join rows and producer completions can return Progress to the public
caller. The scheduler performs bounded producer/sorter work, so the counts are
neither per-operator CPU attribution nor evidence of wasted work.

This profile establishes a baseline and the cost of the two configured budgets.
It does not identify a specific scheduler optimization: successful query time is
roughly 0.10–0.14 seconds, and the larger whole-process remainder includes several
unseparated operations. Reducing Progress counts or increasing batch size alone
would not establish a useful speedup and could weaken work/cancellation bounds.
No scheduler or engine change is justified by this finite observation; reopen
with an affected workload or profile that attributes a material cost to an owner.

Sampled logical memory peaks are 2,187,429/7,435,103 bytes on macOS and
2,187,348/7,435,022 bytes on Linux for the low/high budgets. All remain within
the configured limits; these counters do not bound allocator-usable bytes or RSS.
The stock-linked caller executable SHA-256 values are
`fb7e3e7fbad6f7847d8e33a3feaf7f993a113a5b3d4f15a45d4c245f183fd565`
(macOS) and
`fcce7b779541ded71b3c928bf0dd339bc4cdd55421237ef4676149bde9c338ab`
(Linux).

Measurement-record SHA-256 values are
`874a9c59b07bb33e1bcec96274cee1b5bc682716be641ecbfdf80ae7ad065983`
(macOS) and
`fe62571571d6435bbebd100f70a32cc111c891147fa887742e6f7d0036d71a36`
(Linux). Both documented Cargo commands pass on both platforms. A macOS caller
with the wrong expected sum fails at the full-row oracle before printing a
successful observation. Formatting, all-target macOS Clippy, Linux example
Clippy, and maintenance pass (95 tooling tests, 44 codec fixtures, and 503 final
local documentation links). These are focused example checks, not a rerun of
the complete engine gate. Owned outputs are removed; image/toolchains and all
qualification limitations are preserved.

### Composed-query allocation boundaries

The September 12 stock public allocation caller now observes the nullable
self-join, grouping, and descending-order workload from
[`examples/composed.rs`](../examples/composed.rs). Two literal rows per key yield
four joined pairs; expected present counts and sums are independently fixed by
the key's NULL class. Both 2.2 MB and 12 MB budgets check all 4,096 groups,
completion, and exact final heap, descriptor, and reservation release.

After execute and each public step (including Finished), requested and usable
Rust allocation increments are compared with the current prepared/result charge.
Caller heap storage stays fixed across the interval. Database reservations must
equal their original baseline plus those charges. This tests the combined owners,
not a decomposition by operator or allocations made and freed inside a step.

| Platform | Query budget | Progress steps | Row steps | Minimum usable headroom | Sampled temporary peak |
| --- | ---: | ---: | ---: | ---: | ---: |
| macOS arm64 | 2,200,000 | 848,808 | 4,096 | 11,800 | 2,336,640 |
| macOS arm64 | 12,000,000 | 549,765 | 4,096 | 11,800 | 1,263,448 |
| GNU arm64 Linux | 2,200,000 | 848,808 | 4,096 | 12,960 | 2,336,640 |
| GNU arm64 Linux | 12,000,000 | 549,765 | 4,096 | 12,960 | 1,263,448 |

No requested- or usable-byte deficit was observed, so no engine allowance or
implementation changed. The first probe's borrowed 200,000-step bound stopped
before output. The maintained check uses the existing 20-second subprocess
deadline, which bounds the complete workload and descendant cleanup. Both
budgets finish within it. A wrong-owner control adds a nonexistent charge-sized
owner to measured usable bytes; after all rows and release at 2.2 MB, the same
usable-byte guard rejects it. The tooling interpretation test rejects missing
joined completion. Both the ownership selection and default campaign discover
the case through their existing `check_ownership` entry point.

`RUSTFLAGS=-Dwarnings python3 -B tools/check-diagnostic-allocation.py --ownership-only`
passes on both platforms, including retained mixed grouping, typed readers,
append shapes, parked readers/writer, timeout cleanup, and negative controls.
The runtime sources are unchanged from `986b673`. Builds use Rust 1.98.1 with
release, offline locked dependencies. macOS uses Darwin 25.6.0/Python 3.14.7;
Linux uses Python 3.11.2, the retained image, UID/GID 1000, read-only sources,
and native container storage. The changed caller sources match across platforms.
The composed fixture SHA-256 is
`a326794b39a233a5e91b05c7b4a67d9dac7c5ad3b1176b412442efb1c022459c`.

| Platform | Stock library SHA-256 | Caller SHA-256 |
| --- | --- | --- |
| macOS | `aff5b12d70bfca44dc11f3e490dadcbe76f31ca443932b4a329579e564e5b5b5` | `8fa309e78d35094c788239200a1dcaaf46b23b797c48238420f110cef5339b7a` |
| GNU/Linux | `cd9955b88fc6bf271e68ed59f98fa85220852ed7ba38a977ab8046b2b91008c6` | `4a352485ff48a5b22089a4766fc3b98c7ea4e583a22af1b15d0376e47af1c10a` |

Formatting and maintenance pass: 94 tooling tests, 44 independent codec
fixtures, and 496 final local documentation links. This is focused public ownership coverage, not another full engine
or allocation-refusal gate. Owned source exports, builds, databases, logs, and
the container are removed; image/toolchains are retained. Arbitrary schemas,
allocation histories, transient peaks, Windows, and whole-process/RSS bounds
remain unqualified.

### Explicit STRING append batches

The September 12 example extension preserves one-row input units by default.
It accepts four-row batches at both widths and 256-row batches for eight-byte
text. Both passes still visit keys in descending order, with the complete low
pass preceding the high pass. Fixed caller arrays bound setup storage; append
admission uses the actual number of batches. Four-group runs with a requested
256-row batch exercise a partial batch and its validity mask. The complete-row
oracle remains independent of the append loop.

The [walkthrough](../docs/getting-started.md#measure-string-grouping-costs) owns
current commands. On each platform, eight default profiles, eight four-row
profiles, and four short-text 256-row profiles pass, using groups 4/256, widths
8/65536, and memory budgets 4 MB/80 MB. Every key, extrema, count, completion,
and final release is checked. Five invalid argument profiles reject zero and
unsupported batch sizes, oversized wide-text batches, extra arguments, and
non-UTF-8 batch input before database creation. A macOS caller with deliberately
wrong expected count rejects the result at the row oracle. Both new walkthrough
commands also execute through Cargo's release example build on both platforms.

The finite timing comparison runs three repetitions per shape, alternating
one-row/bulk, bulk/one-row, then one-row/bulk. Each run creates a fresh database
and checks all results. Groups are fixed at 256; short text uses 4 MB and wide
text uses 16 MB. Both platforms use the unchanged `986b673` runtime sources,
Rust 1.98.1, stock release libraries, and the same direct caller flags described
in the preceding capacity comparison. macOS uses Darwin 25.6.0 and Python
3.14.7; GNU arm64 Linux uses Python 3.11.2, the retained verification image,
UID/GID 1000, and native container storage. Platform measurements run sequentially.
The caller SHA-256 is
`4edd9b61e93f4008ebcf3fd67e1f558d2a40e6914db113e39ccddab970b9c577`.

Times below are milliseconds, median [minimum, maximum]. Whole-process time is
parent monotonic time around `check_process.run`, including process startup,
setup, opening, preparation, validation, and close. The printed query timer
covers execution, complete validation, and result destruction. Each subprocess
has a 120-second timeout; none times out. This is a fresh-process, recently
constructed-input comparison, not a cold-cache or sustained workload study.

| Platform | Text bytes | Batch rows | Whole process ms | Execution/validation ms |
| --- | ---: | ---: | ---: | ---: |
| macOS | 8 | 1 | 2678.989 [2655.385, 2702.689] | 16.206 [14.822, 17.635] |
| macOS | 8 | 256 | 160.809 [153.329, 186.805] | 1.164 [1.133, 1.410] |
| macOS | 65536 | 1 | 3216.896 [3207.142, 3246.272] | 423.752 [423.641, 426.140] |
| macOS | 65536 | 4 | 1335.528 [1315.256, 1363.666] | 440.262 [433.319, 442.144] |
| GNU arm64 Linux | 8 | 1 | 256.461 [254.368, 273.659] | 3.808 [3.808, 3.818] |
| GNU arm64 Linux | 8 | 256 | 20.131 [18.342, 20.482] | 0.813 [0.782, 0.892] |
| GNU arm64 Linux | 65536 | 1 | 1020.624 [984.841, 1030.090] | 551.614 [545.634, 554.263] |
| GNU arm64 Linux | 65536 | 4 | 758.916 [750.234, 776.802] | 546.722 [541.617, 557.392] |

Short-text temporary reservations remain zero. Wide-text runs at 16 MB retain
67,169,320 sampled temporary bytes with either batch shape. The original matrix
also retains its spill distinction: only 256 maximum-width groups at 4 MB use
temporary storage; at 80 MB they fit without it. Reducing input units lowers
setup-inclusive time on both platforms. Short-text query time also falls; wide
query times remain close, with a modest increase on macOS. These are changes to
input layout and I/O, not a hash-runtime speedup. Sampled logical reservations
are not allocator-usable bytes, cumulative I/O, filesystem blocks, or RSS.

Final timing-record SHA-256 values are
`2315e3e0d8dff0b0c0148da152954e5f84d64e12496dd0010409f18d64997cd8`
(macOS) and
`bc35705ff7e0b14a6682854daf925415c19dead358ad2d107010a5a1ac35a6fb`
(Linux). Formatting, all-target macOS Clippy, Linux example Clippy, and maintenance
checks pass (93 tooling tests, 44 independent codec fixtures, and 492 final local
documentation links). These focused example checks do not rerun or extend the complete
engine gate. Owned outputs and the verification container are removed; the
user-owned image and toolchains remain. Windows and broader qualification gaps
remain unchanged.

### Grouping capacity cost

The September 12 comparison measures both hash-capacity repairs: stock libraries
from `ca5f59e` (before) and `986b673` (after). `495cbdd` already contains the first
repair and is not the before baseline. Both libraries use the identical example
source retained at `d491101`. The numeric caller SHA-256 is
`627d26af038c942710d974fec470d49f977e2c3679b8afc0d7735803135a441c`;
the unchanged STRING caller is
`78d09fb7b0b3b76c06b5bc2b1aa6d3f792d52c9a7f31e3d933b3a0fc8dd02514`.

Each platform runs ten cells, three repetitions, and two versions: 60 successful
executions per platform. Numeric inputs have 8,192 rows, 32 or 4,096 groups,
even or skewed second passes, and 1.2 MB or 2 MB query budgets. STRING inputs have
four or 256 groups, two eight-byte values per group, and a 4 MB budget. Each fresh
database is generated by the same caller source and arguments. Every ordered
result cell, completion, and final reservation release is checked independently.
The numeric extension preserves the default invocation and adds the existing
public fixture's even/skewed distributions; no benchmark runner was added.

Rust 1.98.1 builds release libraries with offline locked dependencies and
`RUSTFLAGS='-D warnings'`. Callers are linked with `rustc --edition=2024 -O
-C debuginfo=2 -D warnings --extern pipesql=LIBRARY -L dependency=DEPS`. macOS
uses arm64 Darwin 25.6.0 and Python 3.14.7; GNU arm64 Linux uses Python 3.11.2
and the existing unprivileged native-storage container environment. Platforms
run sequentially. Within each cell, version order is before/after, after/before,
then before/after. These are fresh processes with recently constructed inputs,
not cold-cache or sustained service measurements.

Query time includes execution admission, all steps, row validation, successful
completion, and result destruction. It excludes construction, open, prepare, and
close. Process time additionally includes those operations, process launch,
output collection, and writing its short log. Both use monotonic clocks. The
existing process helper enforces a 120-second case deadline, descendant cleanup,
and nonzero status propagation; no case timed out. Both platforms finished well
within the 30-minute execution budget. Memory/temporary peaks are sampled logical
reservations, not allocation-usable bytes, filesystem blocks, cumulative I/O,
physical stack pages, or RSS.

Values below are milliseconds: query median [minimum, maximum] of three samples,
and process median. Temporary peaks are identical across all three repetitions
and both versions. These are observations, not latency distributions or a
cross-platform performance ranking.

macOS:

| Profile / budget | Before query ms | After query ms | Process median ms, before → after | Temp bytes |
| --- | ---: | ---: | ---: | ---: |
| numeric 32 even / 1.2 MB | 5.761 [5.743, 5.965] | 6.884 [5.222, 7.114] | 340.955 → 335.681 | 0 |
| numeric 32 even / 2 MB | 6.367 [5.962, 6.953] | 5.993 [5.637, 6.092] | 341.947 → 329.447 | 0 |
| numeric 32 skewed / 1.2 MB | 5.984 [5.531, 6.143] | 2.023 [1.874, 5.250] | 330.148 → 297.965 | 0 |
| numeric 32 skewed / 2 MB | 6.377 [5.939, 6.546] | 5.787 [5.535, 6.459] | 329.217 → 336.651 | 0 |
| numeric 4096 even / 1.2 MB | 38.507 [37.307, 39.080] | 38.672 [36.984, 39.690] | 360.154 → 368.230 | 803,016 |
| numeric 4096 even / 2 MB | 14.853 [14.002, 15.321] | 14.058 [13.614, 14.680] | 336.057 → 338.474 | 0 |
| numeric 4096 skewed / 1.2 MB | 38.682 [36.759, 39.148] | 38.027 [37.332, 38.420] | 363.979 → 359.344 | 803,016 |
| numeric 4096 skewed / 2 MB | 13.677 [12.026, 14.530] | 12.306 [7.835, 14.569] | 330.989 → 351.158 | 0 |
| string 4 even / 4 MB | 0.968 [0.888, 1.683] | 1.743 [1.715, 1.770] | 219.959 → 233.173 | 0 |
| string 256 even / 4 MB | 17.211 [16.764, 17.355] | 17.004 [16.298, 17.654] | 2716.482 → 2746.186 | 0 |

GNU/Linux:

| Profile / budget | Before query ms | After query ms | Process median ms, before → after | Temp bytes |
| --- | ---: | ---: | ---: | ---: |
| numeric 32 even / 1.2 MB | 1.769 [1.748, 1.778] | 1.828 [1.797, 1.891] | 34.967 → 35.097 | 0 |
| numeric 32 even / 2 MB | 1.849 [1.835, 2.161] | 1.838 [1.836, 1.941] | 34.742 → 35.080 | 0 |
| numeric 32 skewed / 1.2 MB | 1.767 [1.713, 1.781] | 1.785 [1.784, 1.813] | 38.528 → 35.032 | 0 |
| numeric 32 skewed / 2 MB | 1.890 [1.842, 1.920] | 1.995 [1.955, 2.028] | 37.873 → 37.722 | 0 |
| numeric 4096 even / 1.2 MB | 13.558 [13.497, 14.725] | 13.871 [13.396, 14.041] | 49.406 → 48.444 | 803,016 |
| numeric 4096 even / 2 MB | 4.634 [4.584, 4.786] | 4.762 [4.716, 4.788] | 38.865 → 38.005 | 0 |
| numeric 4096 skewed / 1.2 MB | 12.878 [12.844, 12.935] | 12.877 [12.823, 13.269] | 47.489 → 47.310 | 803,016 |
| numeric 4096 skewed / 2 MB | 4.863 [4.598, 4.946] | 4.737 [4.730, 4.776] | 40.260 → 38.498 | 0 |
| string 4 even / 4 MB | 0.539 [0.538, 0.549] | 0.562 [0.521, 0.582] | 23.817 → 21.971 | 0 |
| string 256 even / 4 MB | 3.801 [3.763, 3.888] | 3.850 [3.749, 3.878] | 273.764 → 284.898 | 0 |

No cell changed its spill path: only 4,096-group numeric queries at 1.2 MB
reserved scratch, always 803,016 bytes. The macOS four-group STRING query median
increased by 0.775 ms; GNU/Linux's corresponding change was 0.023 ms with
overlapping ranges. The short macOS samples vary substantially, including the
apparent decrease in low-budget skewed numeric time. These observations do not
establish a material end-to-end regression attributable to the capacity policy,
nor a general speedup. Retain the bounded allocation repairs and stop this finite
comparison; change runtime behavior only with a stronger representative
counterexample. Larger keys, more groups, other budgets and allocator histories,
cold I/O, sustained concurrency, and Windows remain outside the timing profile.

The measured library SHA-256 identities are:

| Platform | Before | After |
| --- | --- | --- |
| macOS | `04f4f60c6489600568bba6f5a02f61abc73181f9b9c0c51f3ee9987fd3e1bd63` | `c5a5a86bd34f20cabb528e8bc7c75f54802c648daa64b7b7f77171d4b7a0f6c8` |
| GNU/Linux | `f4375836908d1c684a0e797d59ccde6b19248c1bb4c5a715868be8a9666aaac3` | `f82b17ca69c4e2834a2290ff353e79c35aa7d40646d967a1be1358dd1d0153d8` |

The measurement-record SHA-256 values, including all samples and caller artifact
identities, are `bd47d1959c29cdcdb45617676e1405c021c6874305e30198a27c3e95e6880381`
and `40de51d5833b948992835b2b4172bf48f2891c47f77cd15f9104e855255a9767`.
Source identities and commands reconstruct the experiment; hashes do not restore
removed logs or promise bit-reproducible debug artifacts.

To reproduce, use clean source trees at the two library revisions and the two
example files from `d491101`. Build each library from its own tree, with a fresh
absolute target directory, then link the same caller file against each library:

```sh
RUSTFLAGS='-D warnings' cargo build --release --offline --locked --lib --target-dir "$target"
rustc --edition=2024 -O -C debuginfo=2 -D warnings \
  --extern "pipesql=$target/release/libpipesql.rlib" \
  -L "dependency=$target/release/deps" "$caller" -o "$binary"
```

Here `target`, `caller`, and `binary` are absolute paths; `caller` selects
`grouping.rs` or `string_grouping.rs`. Run each table cell three times in the
version order above, with a fresh absolute database path for every invocation:

```sh
"$numeric_binary" "$new_database" "$memory_bytes" "$groups" "$distribution"
"$string_binary" "$new_database" "$groups" 8 4000000
```

For numeric cells, distribution is `even` or `skewed`. Collect each process's
output through `tools/check_process.py` with `cwd` set to the absolute source
root, `capture_output=True`, `text=True`, and `timeout=120`. Measure
`time.monotonic()` immediately before that call and after writing stdout/stderr
to a fresh log and checking its return code. The caller prints the independent
query timer and both logical peaks. Keep failed-case context; remove each
successful owned database after checking its complete output. On GNU/Linux,
source may be read-only mounted, but targets and databases must use native
container storage under the unprivileged user. Do not compare the two platforms
as if their filesystem and synchronization costs were identical.

Focused qualification for `d491101` also runs the three documented Cargo
invocations on both platforms: default high/low budgets and 32 skewed groups.
They retain the expected zero/803,016/zero temporary peaks. macOS command
controls reject unsupported group counts, distribution names, extra arguments,
and non-UTF-8 distribution input before database creation. A disposable wrong-sum
expectation fails the complete-row oracle. Formatting and Clippy pass. Maintenance
passes 93 tooling tests, 44 independent codec fixtures, and 489 local links.
Engine source is unchanged from `986b673`; its full gate remains
separate evidence, not a claim that the changed example reran the entire gate.
Owned measurement targets, databases, logs, source exports, and containers are
removed after finalization. The verification image and toolchains are retained.

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

DuckDB is not a universal differential oracle: its documented
[floating-point ordering](https://duckdb.org/docs/current/sql/data_types/numeric#floating-point-types)
places NaN above other numbers, while PipeSQL's MIN/MAX propagate NaN in both
directions. NULL, overflow, floating-point, collation and ordering contracts must
agree before results can be compared. Retain independent local expectations for
incompatible behavior. The implementation's admitted text spans, replay
validation and competing-owner checks are specified in the
[resource contract](../docs/resources.md#declared-grouping-admission).

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

### STRING grouping costs

The September 12 study uses [examples/string_grouping.rs](../examples/string_grouping.rs)
with unchanged engine inputs from `5ec3589`. The [tutorial](../docs/getting-started.md#measure-string-grouping-costs)
reconstructs all eight inputs and commands. Each key has two rows, containing
all-`a` and all-`z` strings of the selected width. Every run checks every ordered
key, MIN/MAX, count 2, completion, and release. One row per input unit keeps the
batch shape fixed, but does not represent typical bulk-ingestion performance.

Stock release runs used the macOS and unprivileged GNU arm64 Linux environments
above, with no concurrent verification campaign. Setup, opening, and preparation
are outside the timer; execution, full result validation, and result destruction
are inside it. These are single-run observations with warm input from setup,
not latency distributions or a cross-platform benchmark ranking. Memory counts
include the database and prepared query; small path-length differences affect
them. Temporary counts are reserved extents, not cumulative I/O or filesystem
block usage.

| Groups | String bytes | Memory budget | Sampled logical memory, macOS / Linux | Temporary bytes, both | Seconds, macOS / Linux |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 4 | 8 | 4,000,000 | 2,898,510 / 2,898,425 | 0 | 0.002216 / 0.000996 |
| 4 | 8 | 80,000,000 | 40,928,793 / 40,928,708 | 0 | 0.009260 / 0.002389 |
| 4 | 65,536 | 4,000,000 | 2,898,514 / 2,898,429 | 0 | 0.001314 / 0.001876 |
| 4 | 65,536 | 80,000,000 | 40,928,797 / 40,928,712 | 0 | 0.009601 / 0.004000 |
| 256 | 8 | 4,000,000 | 2,898,512 / 2,898,427 | 48,680 | 0.021542 / 0.005141 |
| 256 | 8 | 80,000,000 | 40,928,795 / 40,928,710 | 0 | 0.027226 / 0.020152 |
| 256 | 65,536 | 4,000,000 | 2,898,516 / 2,898,431 | 67,169,320 | 0.434948 / 0.552074 |
| 256 | 65,536 | 80,000,000 | 40,928,799 / 40,928,714 | 0 | 0.062368 / 0.077310 |

The source SHA-256 is
`78d09fb7b0b3b76c06b5bc2b1aa6d3f792d52c9a7f31e3d933b3a0fc8dd02514`.
Stock executable hashes were
`60ec5d453156a3c02d0f9c5b909e69e44f407251b668f33a678c9373ed6ab372`
on macOS and
`96e8a760851efe30a16b922177ad975013e9678bb8c85f7b1a7641bc5878a727`
on Linux. A separate macOS caller replaced only `MIN(word) AS lo` with
`MAX(word) AS lo`. At 4 groups, 8-byte strings, and 4 MB, the unchanged checker
rejected the wrong extremum with exit 1 and no `verified` output.

#### Allocator observation

A separate macOS diagnostic reused the System-forwarding observer from
[diagnostic-allocation.rs](../tools/fixtures/diagnostic-allocation.rs) around the
same query. Its timer values are excluded from the stock table. The following
peaks are increases above live counters sampled immediately before execution.
They include observed Rust allocations, not foreign allocations, allocator
metadata, thread stacks, or RSS. Both requested and usable live counters returned
exactly to their starting values after dropping the result.

| Groups | String bytes | Budget | Peak requested increase | Peak usable increase | Allocation calls |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 4 | 8 | 4,000,000 | 2,860,808 | 2,941,024 | 57 |
| 4 | 8 | 80,000,000 | 40,891,093 | 40,972,256 | 57 |
| 256 | 8 | 4,000,000 | 2,860,814 | 2,941,056 | 571 |
| 256 | 8 | 80,000,000 | 40,891,099 | 40,972,256 | 561 |
| 256 | 65,536 | 4,000,000 | 2,860,826 | 2,941,056 | 571 |
| 256 | 65,536 | 80,000,000 | 40,891,111 | 40,972,256 | 561 |

To reconstruct this disposable diagnostic, save the following as `observe.py`
in a fresh directory outside the repository and run it from the repository root.
It copies the existing observer, adds only measurement boundaries, and leaves
engine code unchanged. The generated source SHA-256 for this study was
`a81be2005de7a2c46eca995e5d586ed8999ef6bbaf1bee5daf1f429488137c16`.

```python
from pathlib import Path

root = Path(__file__).resolve().parent
example = Path('examples/string_grouping.rs').read_text()
allocator = Path('tools/fixtures/diagnostic-allocation.rs').read_text().split('#[path = "catalog-allocation.rs"]', 1)[0]
observer = '''
pub fn begin() -> (usize, usize) {
    let before = (LIVE_REQUESTED.load(Ordering::Relaxed), LIVE_USABLE.load(Ordering::Relaxed));
    PEAK_REQUESTED.store(before.0, Ordering::Relaxed);
    PEAK_USABLE.store(before.1, Ordering::Relaxed);
    CALLS.store(0, Ordering::Relaxed);
    TRACK.store(true, Ordering::Relaxed);
    before
}
pub fn finish(before: (usize, usize)) {
    TRACK.store(false, Ordering::Relaxed);
    assert_eq!(LIVE_REQUESTED.load(Ordering::Relaxed), before.0);
    assert_eq!(LIVE_USABLE.load(Ordering::Relaxed), before.1);
    println!("allocator baseline_requested={} baseline_usable={} peak_requested={} peak_usable={} calls={}", before.0, before.1, PEAK_REQUESTED.load(Ordering::Relaxed), PEAK_USABLE.load(Ordering::Relaxed), CALLS.load(Ordering::Relaxed));
}
'''
assert example.count('let start = Instant::now();') == 1
assert example.count('let elapsed = start.elapsed();') == 1
example = example.replace('let start = Instant::now();', 'let before = observer::begin(); let start = Instant::now();')
example = example.replace('let elapsed = start.elapsed();', 'let elapsed = start.elapsed(); observer::finish(before);')
combined = '#[allow(dead_code, unused_imports)] mod observer {\n' + allocator + observer + '\n}\n' + example.replace('//!', '//')
(root / 'observed.rs').write_text(combined)
```

Build the ordinary release library with
`cargo build --release --offline --locked --lib`. Compile `observed.rs` using
`rustc --edition 2024 -O`, `--extern pipesql=target/release/libpipesql.rlib`, and
`-L dependency=target/release/deps`, selecting an output beside the generated
source. Run that caller with the same arguments as the tutorial. Its assertions
check return to the pre-execution allocator baseline. Remove the disposable
caller, generated source, and databases afterward. The recorded observer binary
hash was `feb3a13c1825b29c2e9110d0707e59d475eaa05841811a0c8d3ced194cda2a47`;
this recipe does not claim reproducible binaries.

#### Decision

The study selected compact text storage for optional hash grouping. Four
short-string groups needed only 64 bytes of extrema but admitted about 41 MB;
256 groups spilled at 4 MB despite retaining only 4,096 useful text bytes.
The implementation and accepted growth costs follow. The original workload,
source identity, and measurements remain the comparison baseline.

### Compact hash text storage

Commit `8aceaee` implements explicit text spans, geometric region capacities,
and separately admitted arena growth. The [resource contract](../docs/resources.md#declared-grouping-admission)
owns the space bound, copying quantum, admission order, and fallback behavior.
Fixed disk reduction and serialized records remain unchanged. STRING hash
metadata admits at most 4,096 groups, subject to available memory; numeric-only
sizing is unchanged. This policy avoids replacing unused maximum-width text
with another large reservation for empty slots.

The unchanged [STRING example](../examples/string_grouping.rs) ran all eight
combinations on the macOS and GNU arm64 Linux environments above. Each checked
all ordered keys, both extrema, count 2, successful completion, and reservation
release. Stock timings exclude setup/open/prepare and include validation and
result destruction. These single observations ran without a concurrent gate;
they do not establish latency distributions or a speedup. Paths account for
small differences in logical reservations.

| Groups | String bytes | Budget | Sampled logical memory, macOS / Linux | Temporary bytes, both | Seconds, macOS / Linux |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 4 | 8 | 4,000,000 | 2,492,857 / 2,492,775 | 0 | 0.001834 / 0.000526 |
| 4 | 8 | 80,000,000 | 2,492,858 / 2,492,776 | 0 | 0.001721 / 0.000523 |
| 4 | 65,536 | 4,000,000 | 3,279,197 / 3,279,115 | 0 | 0.002055 / 0.002064 |
| 4 | 65,536 | 80,000,000 | 3,279,198 / 3,279,116 | 0 | 0.003358 / 0.002047 |
| 256 | 8 | 4,000,000 | 2,498,907 / 2,498,825 | 0 | 0.016103 / 0.005729 |
| 256 | 8 | 80,000,000 | 2,498,908 / 2,498,826 | 0 | 0.016194 / 0.003789 |
| 256 | 65,536 | 4,000,000 | 3,279,199 / 3,279,117 | 67,169,320 | 0.422369 / 0.551240 |
| 256 | 65,536 | 80,000,000 | 52,824,416 / 52,824,334 | 0 | 0.072121 / 0.091444 |

The four-group, eight-byte case falls from about 40.93 MB to 2.49 MB at an
80 MB budget. The 256-group short-string case no longer spills at 4 MB, and a
public regression checks that property with complete results. Maximum-width
values still spill at 4 MB and remain in memory at 80 MB. Growth temporarily
owns old and new buffers: the large-budget wide-value peak rises from 40.93 MB
to 52.82 MB. This is an accepted reservation cost of bounded copying, not a
whole-process-memory guarantee. Shorter replacements reuse capacity; historical
large values can therefore retain more space than their current lengths.

Stock executable SHA-256 values were
`2e26e04326ba1241b71cae125b735c276fc71e5a7803e44b280145c3f82379f1` on macOS and
`753f0a783a235c7fb928bbadd59ca4ad08d8bf93aaa8a49a4cf90aa6de9cf3d3` on Linux.
The example source and reconstruction commands are unchanged from the baseline.

The existing disposable allocator recipe above ran six macOS cases against the
new ordinary release library. All returned requested and usable live counters
exactly to baseline after result destruction. The following are peak increases
over the pre-execution baseline. Diagnostic timings are excluded: the full gates
ran concurrently with these allocation observations.

| Groups | String bytes | Budget | Peak requested increase | Peak usable increase | Allocation calls |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 4 | 8 | 4,000,000 | 2,455,070 | 2,547,552 | 60 |
| 4 | 8 | 80,000,000 | 2,455,073 | 2,547,552 | 60 |
| 256 | 8 | 4,000,000 | 2,455,076 | 2,547,552 | 570 |
| 256 | 8 | 80,000,000 | 2,455,079 | 2,547,552 | 570 |
| 256 | 65,536 | 4,000,000 | 3,232,990 | 3,323,456 | 572 |
| 256 | 65,536 | 80,000,000 | 52,778,207 | 52,868,672 | 570 |

The observer executable hash was
`9db6f5703b08475672f8169de6ce7b51696542d336fa11653ada2ca029041924`.
Requested/usable peaks and sampled logical reservations have different scopes;
they are not interchangeable with RSS, foreign allocations, or filesystem usage.

The full gates above passed all maintained semantic, allocation, native,
interruption, and graph campaigns. Both public allocation sweeps covered 723
catalog refusal prefixes plus the healthy control on short and 384-byte paths.
New internal tests check region reuse, invalid extents, competing reservation
refusal, cancellation after one copying quantum with both buffers live, and
release back to baseline. Existing tests retain NULL/empty/Unicode behavior,
full-width replay, independent decoding, demanded errors, and cleanup checks.
An earlier debug-profile aggregation selection overflowed the bounded-stack
test; the required release-profile test passed. Debug stack qualification is
not claimed. The first new growth fixture incorrectly selected legacy one-byte
text storage; selecting the declared UTF-8 layout repaired that fixture.

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
guard residency, or a whole-process cap. The full gates for this change passed; the current checkpoint also retains
these regression controls.

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

The verifier, fixtures, and contracts are committed in `97a253c`. The 90 tooling
tests, caller formatting, 39 codec fixtures, and final local documentation links
pass. Only notes changed after the frozen runtime checks; all other manifested
inputs match that commit, with SHA-256 `716b5fc787cfbf70f8d263ac9f421c2600e21efb538d0db1aeb17fa018e321f5`.
The prebuilt nightly standard-library archive hashes are:

- macOS: `1d648294ae1483fce796cb48c40ee6898c3b653d1ce3e7d7aabadbc9f241b7b3`
- GNU/Linux: `cad6d0959670f358aab642758084db7f4c830afcd788ab8927ad052a03d17d9e`

The maintained [AddressSanitizer diagnostic](../docs/testing.md#qualify-native-sanitizer-observations)
passes on arm64 macOS and GNU arm64 Linux for the native mutex boundary. Both
use diagnostic rustc `f248f4038796913873f11ca65b1b901e311c8dae`
(1.100.0-nightly, September 5, 2026; LLVM 23.1.1), compared with the pinned
1.98.1 compiler and the same nightly without instrumentation. Each configuration
executes the four existing tests for stationary storage/moves, threaded updates
and single destruction, poisoning, and forgotten-guard teardown. No test is
ignored, and a selection missing any required case fails the verifier.

The clean control completes; the isolated heap-bounds fault emits the expected
AddressSanitizer report and exits 86. Runtime options are
`halt_on_error=1:abort_on_error=0:exitcode=86:detect_leaks=1`. Both complete
verifier runs report unchanged source manifests and successful owned-build
cleanup. The frozen input fingerprint is
`ce25bbda54dab6eaa862785ef01fcd65eb9ff672602d81b809658e47a99f0cce`.

| Target | AddressSanitizer runtime SHA-256 | Instrumented test executable SHA-256 |
| --- | --- | --- |
| macOS arm64 | `f2154d27ed44e47a2de5409b19136d92d9c4b6c22b7636548c4bd6b2b823e76c` | `ef4e46be6f9796a2fe133046af0a0e2749999855aea49af5ee7683631323da84` |
| GNU/Linux arm64 | `99d061c74157daffad9c90ac4ff6b87a6cb87d82ce9cc37cb3479a41e924fde9` | `2c029b8a0737b18edfe06628fb45fda8d90edfdff4a677dd44f016989ea12aff` |

The diagnostic uses prebuilt standard-library archives, not rebuilt instrumented
standard libraries. macOS links the nightly ASan dylib, libiconv, and libSystem
1359.0.0. Linux embeds the supplied ASan archive and links glibc 2.36, libm,
libgcc_s, and the native loader. System-library internal accesses are outside the
instrumented Rust boundary. Leak detection is enabled, but passing these cases
does not qualify every native allocation, access, schedule, or teardown path.
The [verification contract](../docs/verification.md#native-sanitizer-observation)
owns the permitted claims.

Verifier tests independently reject missing/duplicate results, wrong diagnostics
or exit codes, ambiguous artifacts, and instrumentation-altering environment
settings. Failed commands and timeouts retain context and remove build outputs.
The ordinary engine and filesystem implementation were unchanged by this work.
These are focused diagnostic results, not a new full-engine gate.

This comparison produced no report in the four mutex tests. It does not resolve
the earlier disagreement among retired compiler/standard-library controls;
no current engine defect, general race freedom, or whole-engine memory-safety
claim follows. ThreadSanitizer, instrumented standard libraries, other native
boundaries, and Windows remain separate qualification work.

### Pathname sanitizer qualification

The explicit `--scope pathname` selection extends the maintained diagnostic to
existing native traversal, metadata, directory-cursor, and record-decoder tests.
Required sets are explicit: 16 macOS tests and 14 GNU/Linux tests. Discovery and
successful completion must match the selected names exactly; the receipt retains
those names. The default remains the four-test mutex scope. Test temporary
directories are owned by the diagnostic, including after an aborted subprocess.
No native implementation or fixture changed.

The final macOS pathname and default mutex runs pass under Rust 1.98.1,
uninstrumented diagnostic nightly, and AddressSanitizer nightly. The diagnostic
compiler is `f248f4038796913873f11ca65b1b901e311c8dae` (September 5, 2026;
LLVM 23.1.1). Both clean controls complete, and isolated heap-bounds controls emit
the required diagnostic and exit 86. Each run records unchanged inputs and no
finalization errors. The shared input-manifest SHA-256 is
`53254b5917b37f2998687ffb3f8017d55085ad8f7ff658a0a1deb6bff1478d50`.
The pathname ASan test executable SHA-256 is
`113dee537aeb7a17384a33337cb22583012518651555d0dadbdacbbf7f786251`.
The runtime and prebuilt standard-library hashes match the preceding diagnostic.

macOS uses arm64 Darwin 25.6.0, Python 3.14.7, libiconv, and libSystem 1359.0.0.
The pathname scope executes all 16 tests in each configuration; mutex executes
all four. The final pathname/mutex receipt SHA-256 values are
`2b2da3b1167646ccdeb5e01f4dbf788215d91bb607b1c48c9e4f466e7f9486b1` and
`9966c51ef7b509737d22116921c60cd96acd1b947e690507bbbfa9f91a29f010`.

GNU arm64 Linux also passes all 14 pathname tests and all four default mutex
tests in each of the three configurations. It uses the same diagnostic compiler,
Python 3.11.2, glibc 2.36, libm, libgcc_s, and native container storage under
UID/GID 1000. Its ASan runtime and prebuilt standard-library hashes match the
preceding diagnostic. The pathname ASan test executable SHA-256 is
`529dacc681b8db6ee0f2a4ed01a0b269551426f8b250fffbb659b08e06cb7497`.
The pathname/mutex receipt SHA-256 values are
`40e160d6ca970542641de646ffd097ff3e7665a3aa235a374fe351f40414298a` and
`7b92d951c9c87d5dfb40039d24e9cb62859269363b84491445d6b247e12d3619`.
Both receipts pass clean/fault controls, unchanged inputs, and final cleanup.
All four runs share the input manifest recorded above. Only documentation and
plan/evidence prose changed afterward.

Linux's dated nightly was provisioned in an owned container directory because
the retained image had only the pinned compiler. At the 20-minute reassessment,
58 of 66 MiB of the final compiler archive was downloaded; the existing download
completed within the added ten-minute allowance. The optional tool-manager
self-update was stopped and disabled after compiler installation. Qualification
then ran from fresh outputs using the installed compiler; that setup interruption
is not a test result. Existing image and host toolchains are preserved.

| Scope/platform | Stock test seconds | Nightly test seconds | ASan test seconds |
| --- | ---: | ---: | ---: |
| Pathname/macOS | 0.664 | 0.671 | 0.873 |
| Pathname/GNU Linux | 0.051 | 0.048 | 0.067 |
| Mutex/macOS | 0.006 | 0.010 | 0.215 |
| Mutex/GNU Linux | 0.001 | 0.002 | 0.008 |

These are single diagnostic process observations, not performance comparisons.
Verifier tests and maintenance pass: 95 tooling tests, 44 independent codec
fixtures, and 498 final local documentation links. Negative selections cover missing, duplicate, ignored, and
wrong-platform cases. Owned results, exported sources, temporary toolchain,
and container are removed. This is focused Rust wrapper/test instrumentation;
standard-library, system-library, and kernel internals remain uninstrumented.
It does not establish general race freedom, thread-stack bounds, whole-engine
memory safety, Windows support, or durability. The full engine gate was not
rerun because no native or engine implementation changed.

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

## Testing and tooling cleanup

The finite review implemented in `a315e21` followed Cargo module inclusion and
actual Python callers, fixture construction, assertions, costs and cleanup paths.
The [test map](../tests/README.md) and [tool map](../tools/README.md) own current
navigation. The retained review dispositions are:

| Reviewed area | Protected contract and disposition |
| --- | --- |
| Public suites and child modules | Literal SQL/results and independent nullable-row/Boolean models retained lifecycle, composition, snapshots, spans, demand, spill, cancellation and release. Ordinary/bounded-thread pairs remained because only the latter observes native stack extent. The empty lease-child entry was folded into its parent, retaining normal and early-teardown subprocess checks. |
| Frontend and physical planning | Limits, identities, scope, spans and independent malformed-plan mutations remained beside their owners. Two historical test-name prefixes were removed without changing bodies; structural assertions still supplement public result oracles. |
| Execution and resources | Scan, batch/scalar, aggregates, hash/reduction/replay, sorting/join/order, LIMIT/union and authority tests retained exact/short admission, actual capacities, row/byte bounds, demanded failures, phase-specific cancellation and release. Independent row and rational-rounding expectations remained unchanged. |
| Storage, database and catalog | Independent encoded vectors, matching-checksum corruption, publication outcomes, pins/receipts, recovery/reclamation, short I/O and interruption schedules remained. Shared test-only directory cleanup replaced ignored errors and panic-on-unwind cleanup. The duplicate cleanup regression was removed; its retained owner checks normal, missing-directory, real-error and unwind paths. |
| CLI and filesystem | Source/sink/diagnostic ownership and native metadata, paths, extents, locking, mutex, thread and stack checks remained. Iterative deep-path teardown and platform guards retained their distinct OS/depth premises. |
| Fixtures and reference models | Five catalog encoders and the duplicated semantic snapshot writer were consolidated. Six superseded scripts and unused digest walks had no remaining consumers. Persisted bytes and separate expected query results were preserved; five before/after snapshot comparisons covered empty input and a DOUBLE block crossing. Five catalog/schema vectors were added to ordinary independent fixture comparison. Old-format encoders/decoders, Q1 comparison, identity/publication/reclamation models and their negative controls remained. |
| Python commands and tests | Maintenance retained discovery of every `test-*.py`, entry-point guards, syntax/docs/manifests, fixture comparison, oracle rejection, build/loader selection, sanitizer interpretation, receipts and live-descendant cleanup. Maintained native observers, caller fixtures, identity diagnostics and sanitizer controls had active consumers. |
| Campaigns and gate | Full allocation prefixes and healthy controls, native operation/partial-transfer cases, graph mutations and interruption cuts remained. Two semantic campaigns had redundantly built the stock CLI; a fresh macOS build cost 8.54 seconds. The gate now builds once and supplies the immutable artifact sequentially, then removes its target and case databases. Native callers retain isolated targets. |
| Documentation | Maps, fixture generation and artifact ownership were corrected. The documented generator ran outside the repository into fresh output. Ordinary shell/Cargo/Python entry points remained; no workflow framework was added. |

Final discovery reconciled two removed tests, one moved cleanup test and two
renamed tests. No SQL result, fault schedule, persisted fixture byte or platform
exclusion was removed. New controls rejected damaged schema vectors and reused
output names; gate controls checked artifact ordering and cleanup. Both complete
gates passed on `a315e21`: 496 ordinary Rust tests per platform, separate lease
subprocesses, 93 tooling tests, 44 independent codec fixtures, 24 semantic cases
and 298 composition cases. Removed names had no active consumers. Later changes
have their own evidence and counts above.

The working-plan consolidation removes completed narratives whose contracts,
measurements and limitations are already retained here. `2280ae5:notes/plan.md`
retains the original section dispositions and historical detail; no archive copy
is required. The unresolved qualification and publication obligations remain in
[the work plan](plan.md). Documentation verification passes 496 local links.
Only the two notes files differ from the qualified runtime inputs; unchanged
full gates were not repeated.

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
