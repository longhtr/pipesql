# Verification evidence and open limitations

This record identifies checked behavior and consequential limitations. Product
and implementation contracts live in [docs](../docs/README.md); current work
lives in [the plan](plan.md). Maintained fixtures and callers provide replay inputs.
No build, test, or investigation below requires a retired project checkout.

## Full verification checkpoint

The September 12, 2026 complete gates for `ca5f59e` passed all 24 stages on
macOS and GNU arm64 Linux. Both used Rust 1.98.1, release artifacts, offline
locked dependencies, and warnings-denied compilation and documentation. macOS
used arm64 Darwin 25.6.0, Python 3.14.7, and the native Apple toolchain. Linux
used the unprivileged native-storage environment described below.

The gates covered formatting, maintenance, filesystem ABI, rounding vectors,
attempt models, Clippy, Rust tests, rustdoc, doctests, stock CLI construction,
aggregate semantics and composition, public/CLI allocation, native
initialization/synchronization/byte I/O, catalog interruption, and independent
graph inspection. Maintenance passed 93 tooling tests, 44 independent codec
fixtures, and 480 local documentation links. Independent semantics checked 24
cases; composition checked 298 cases. Both semantic campaigns used the same
unchanged CLI on each platform. Native callers retained isolated build targets.

Each platform executed 499 ordinary Rust tests without failures or ignored tests.
macOS executed 367 library and 21 filesystem tests; Linux executed 369 library
and 19 filesystem tests. Shared suites executed 15 CLI, 76 catalog, seven
execution, seven lifecycle, and six load tests. The lease test separately ran its
normal child and intentionally killed its early-teardown child, then verified
reopening. The native payload-capacity test and both append admission/growth tests
execute on each platform. Four example targets compiled without test bodies;
that compilation is not runtime example evidence.
All eight public union tests executed on both platforms.

All 24 stage statuses were zero. The 670 manifested inputs matched before/after
and across both runs; Linux used a read-only export. The frozen manifest SHA-256
is `3c71d1c6d2a621db0cada3182a79b8d129e51299a358501d005758e1c1eefddb`.
Commit `ca5f59e` retains those exact inputs. Only the two notes files were
finalized afterward; final documentation verification checks 483 local links and covers those prose-only
changes. No runtime source, fixture, or tool changed afterward. This identifies
source, not reproducible binaries.

The stages took 1,586.223 seconds on macOS and 819.286 seconds on Linux; these
are verification costs, not query benchmarks. The respective result-receipt
SHA-256 values are `79b280863bef5f83c0a7cd4c95976ff2e26067133682fda42ed2788c1fd46cef`
and `1503fe9f47ced0dbdfa20aadc58d96e5d985f5cb71854547e04da398ea0c3fbd`.
Finalization reported no errors and removed owned targets and composition
databases. Successful logs, exports, the verification container, and remaining
scratch outputs were removed; the user-owned image and toolchains remain.
Current callers and fixtures reconstruct cases; hashes do not restore removed
logs.

The [finite testing/tooling review](plan.md#completed-testing-and-tooling-cleanup)
remains complete at `a315e21`; its fixture, oracle, and failure controls are retained.
The reader repair adds actual payload-capacity admission and 12 typed public
ORDER BY/DISTINCT allocation checks. The exhaustive append allocation-size and
full-width growth/reuse/publication checks remain. Both platforms retain 790
catalog allocation-refusal prefixes plus healthy controls at both pathname lengths,
76 append interruption cuts, 46 recovery cuts, 249 independent graph checks during
interruption, and 43 graph cases with their oracle controls. Native I/O exercises
1,028 cells. Platform exclusions and persistent-byte compatibility remain intact.

The declared-table and union tutorials last ran on both platforms at `98de11f`
and are unchanged. The union query produced north's total 25 across four rows and
south's total 20 across one row, then reported `status=queried` and exited
successfully. The earlier frontend walkthrough produced INT64 result 38 on macOS.
These are retained tutorial observations, not reruns during the tooling cleanup.

Run `sh tools/check.sh --output /absolute/new-result-directory` with the
[documented prerequisites](../docs/testing.md#complete-local-gate). The gate keeps
stage logs and a JSON receipt, checks before/after source manifests, and removes
its owned build target and composition databases. Preserve failure context before
disposing of a run. Windows, broader durability, physical memory bounds, and
sanitizer/concurrency qualification retain their existing limitations.

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

The public allocation caller composes typed union, sorting, and COUNT. Both
platforms pass all 790 catalog refusal prefixes and the healthy control on short
and 384-byte paths, including union preparation, execution, and stepping
refusals, healed reopen, and retry. The unchanged census bound is 800. This is
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
prefixes and the healthy control on short and 384-byte paths. The current union
checkpoint above extends that caller and records its larger census.
Cancellation, exact admission refusal, invalid scope/identity controls, typed
copies, and the 65-value sorting regression execute in the Rust suites.

The revised [tutorial](../docs/getting-started.md#transform-columns-while-retaining-the-original-values)
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

The [learning example](../docs/getting-started.md#transform-columns-while-retaining-the-original-values)
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
row, and a one-byte attribution error at distinct checks. Current driver SHA-256:

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

The same full-gate healthy catalog controls expose a separate grouped-query
owner excess. For short/384-byte paths, macOS reports charges of
3,977,576/3,977,312 bytes, requested heap 3,926,846, and usable extents of
4,089,072/4,089,392: deficits of 111,496/112,080 bytes. GNU/Linux charges
3,977,644/3,977,328 cover requested 3,926,862 and usable 3,927,376/3,927,344.
This sample includes the GROUPED prepared query and result immediately after
execution admission in [catalog-allocation.rs](../tools/fixtures/catalog-allocation.rs).
Its requested-byte assertion passes; it does not yet enforce a usable-byte bound.
The [next repair](plan.md#next-grouped-query-allocation-bounds) must first trace
those actual owners and preserve its independent full-row oracle.

Append's ceiling is a qualified premise for the exercised stock allocators and
request-size domains, not arbitrary global allocators or all allocator states.
Parked samples exclude transient peaks, direct foreign allocations, allocator
metadata/retention, and physical stack pages. Windows, broader durability,
arbitrary schedules, and whole-process memory remain unqualified.

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
