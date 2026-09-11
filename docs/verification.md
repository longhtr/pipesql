# PipeSQL verification

This document owns what tests and measurements are required and what claims they
permit. [Build and test](testing.md) owns commands; [retained
evidence](../notes/evidence.md) records completed runs and their limits. This
guide separates current regressions from requirements still needed for
production release.

## Choose the evidence for a claim

| Question | Start here |
| --- | --- |
| Which checks do I run for a change? | [Build and test](testing.md#focused-verification), then the relevant requirements below. |
| What does the maintained gate check? | [Local regression gate](#local-regression-gate). |
| What has actually passed? | [Completed checkpoint](../notes/evidence.md#latest-completed-gate-checkpoint), including its exact source and exclusions. |
| Which operating systems have been exercised? | [Platform status](testing.md#platform-status). |
| What remains necessary for production? | [Release criteria](#release-criteria) and the capability-specific requirements below. |

The requirements below are obligations, not a record that every listed check
exists or has passed. Requirements for an absent optimizer, worker scheduler,
foreign interface, or stable format apply when that capability is introduced.
Existing capabilities still need their stated release qualification. A complete
local gate preserves maintained regressions; it does not certify production
readiness.

## Principles

1. Verify user-visible behavior through the same production path that ships.
2. Use independent methods for important semantics and persistent state.
3. Test invalid, boundary, refusal, cancellation, and cleanup behavior as
   deliberately as successful behavior.
4. Separate safety evidence from liveness evidence.
5. Treat deterministic simulation as one instrument, not reality.
6. Measure generator and checker effectiveness, not just generated case counts.
7. Run stock optimized artifacts on real operating systems before making a
   release or performance claim.
8. Scope every conclusion to the exact feature, configuration, platform, and
   failure model exercised.

A test written from the same mistaken representation as production can agree
perfectly and still be wrong. “Independent” therefore describes provenance and
method, not merely a separate crate or file.

## Claim vocabulary

- **Experimental:** a disposable prototype answered a named design question.
  Its interface and data are not compatibility promises.
- **Implemented:** the production path exists and passes its local positive and
  negative tests. Adjacent resource, crash, concurrency, or release behavior is
  not implied.
- **Release-ready:** the complete user-visible capability passes every relevant
  semantic, resource, failure, cancellation, public-artifact, and platform gate
  in this document.
- **Stable:** a released compatibility surface has retained old-version
  fixtures and an upgrade policy. Pre-release syntax, API, and files are not stable.

Do not promote a utility, type, manifest, model, or internal boundary to a
product capability. A claim names what remains outside its evidence.

## Evidence record

Every consequential experiment or gate records:

- exact source revision and toolchain;
- artifact type and build options;
- operating system, architecture, and relevant hardware/filesystem;
- deterministic inputs or retained data generator;
- workload and limits;
- expected result or independent oracle;
- faults and schedules explored;
- observed coverage and measurements;
- failures, exclusions, and unresolved risks; and
- the decision the result supports or reopens.

Evidence supporting a current claim must be reproducible from identified
retained inputs using a clean checkout. Historical observations with missing
inputs cannot support that claim; use the engineering guide's retention policy
to repair or retire them. Timing reports include enough environment data to
compare them honestly; semantic reports exclude timestamps and ambient paths
where byte-for-byte reproduction is expected.

Python assertions must remain enabled. Record immutable source and artifact
identity for consequential runs. Uncommitted variants need a parent revision
plus complete input content, not a parent hash alone. Runtime data and
observation drivers need separate records. Hashes identify inputs; they do not
prove a build or reconstruct missing bytes.

## Local regression gate

[Build and test](testing.md) owns prerequisites, commands, focused checks,
source freezing, and failure handling. The [tool inventory](../tools/README.md)
and [test map](../tests/README.md) locate maintained checks. The complete gate
retains known regressions and runs sequentially; a filtered or interrupted run
cannot establish a passing full gate.

The local gate checks the following boundaries through production code and
independent models or fixtures. Native and persistence campaigns are described
in their dedicated sections below:

| Boundary | Required checks |
| --- | --- |
| Build inputs | Formatting, warning-denied release workspace Clippy, all Rust test targets, native SDK agreement, and source-manifest inclusion checks. Literal Rust code/data includes and explicit module paths must be recorded; generated or nonliteral inputs need an explicit generator contract. |
| Persistent fixtures | Independent byte-for-byte reproduction of rejected formats 1/2/3, current legacy format 4, retired format 5, and namespace-7/catalog-object-6 vectors. Fixture agreement establishes provenance consistency, not recovery or stable compatibility. |
| Numeric semantics | Rational rounding vectors, the full admitted aggregate row-count bound, exceptional values, exact integers, scalar errors, final SUM overflow, later cancellation/nonfinite values, and finite AVG after intermediate sum overflow. Floating accumulation need not be exact or order independent. |
| Composition | Projections, aliases, source and post-aggregate filters, grouping/order, demand, dates, numeric boundaries, empty input, and rejected forms through the ordinary public parser and executor. |
| Results | Progress, Rows, Finished, Failed, borrowed lifetimes, cancellation, drop, and sink failures. Successful execute or partial rows cannot establish completion. |
| Admission | Exact and one-byte-short minima, typed array/scratch capacities, omitted-charge negative controls, refusal before effects, and complete ownership reconciliation. Account equality is not a whole-process memory bound. |
| Construction | Legacy staging row/byte allowances at empty, exact, and next-row boundaries; next append batch/encoded-extent refusal before file effects; abort-only state after failed writes. Final source validation remains required. |

## Language and semantic evidence

The accepted language profile is a versioned manifest derived from the exact
GoogleSQL source named in `language.md`. For each accepted grammar alternative,
type rule, function, and pipe operator, the corpus includes:

- representative positive cases;
- malformed and explicitly unsupported near-misses;
- minimum, maximum, empty, NULL, duplicate-name, and ambiguity boundaries;
- input and output relation state, including stable identities and order;
- expected error category and source span where applicable; and
- a reduced upstream observation or documented PipeSQL divergence.

The GoogleSQL analyzer is an external conformance oracle, not production code
and not the only oracle. A small row-oriented semantic model is authored from
the language contract and reduced source observations. It must not import the
production parser, binder, plan nodes, vector kernels, hash/equality helpers, or
optimizer rules. Where the model and pinned analyzer disagree, minimize the case
and leave the feature unsupported until the language contract adjudicates it.

Property and metamorphic tests cover transformations whose equivalence is
actually guaranteed: harmless renaming, legal pipe-prefix composition,
partitioning and recombination, batch-capacity changes, optimizer on/off, and
spill versus in-memory execution. Metamorphic relations never assume order,
error discovery, floating behavior, or expression demand that the language
leaves unspecified.

### Required composition regressions

The following composition details are required, not implied by a related passing
case:

- STRING predicates cover six comparisons, NULL, empty text, Unicode, escapes,
  malformed input, source/decoded bounds, and literal ownership after source text
  is released. Include type mismatches, long values, every admitted producer,
  allocation/I/O refusal, and cancellation. Legacy one-byte keys do not establish
  arbitrary UTF-8 behavior.
- NULL predicates distinguish NULL from empty text, zero, and NaN for every type.
  Include computed/aggregate/derived values, joins, order, limits, and earlier
  filters. Nonnullable predictions cannot hide demanded arithmetic or corruption.
  An oracle transport that maps NaN to NULL does not prove NaN semantics.
- Boolean checks cover the full three-valued table, precedence, parentheses, NOT,
  NULL/NaN, lazy branch demand, and agreement between row and batch consumers.
  Independent value models and conditional-error witnesses remain separate.
  Include parser/token/stage bounds, invalid branches, small stacks, optional
  scratch, cancellation, and allocation/I/O failures across producer types.
- Derived inputs cover sibling isolation, alias hiding, duplicate names, nested
  joins, shared syntax bounds, and small stacks. Child ORDER BY must still select
  LIMIT rows without promising outer order. Retain demanded failures. Legacy CLI
  fixtures do not qualify declared joins or standalone ordering.
- Preparation checks distinguish catalog-loading peak from retained-plan plus
  temporary-scope peak. Exercise each at its exact limit and one byte below,
  with released owners and healed allocation/I/O/cancellation cases.
- Repeated aggregates cover grouped/global chains and intervening computations,
  empty legacy chains through ten outputs, and refusal of the next identity.
  Force downstream hash fallback through the production scheduler; injected
  replay alone is insufficient. Exercise memory/disk output after prefixes and
  completion, reread corruption, cancellation, combined minima, allocation/I/O
  refusal, and healed grouped/grouped/global compositions.
- Arithmetic diagnostics retain complete UTF-8 spans after source, query, and
  result drop. Distinguish demanded AVG from an earlier dropped shared SUM and
  final SUM overflow from shared AVG state. Check constants including LIMIT 0,
  invalid spans, allocation-free fixed-buffer formatting, and cause capture.
- Parsed numeric pools preserve individual operation bounds and source extents
  with four maximal programs in one query. Small-stack public queries combine
  numeric/DATE filters, aggregation, and LIMIT. Computations cover named/anonymous
  outputs, identities above 127, exact admission, cancellation, independent
  models, and forced replay through scan/sort/join/LIMIT. Static parser size is
  not a whole-call stack bound; pinned naming source is not executed conformance.
- Legacy key roundtrips enumerate all 8,836 admitted pairs and exhaustive byte
  constructor/index behavior, including delimiter rejection. Cardinality alone
  cannot establish the correct admitted byte set.

### Demanded evaluation, ordering, and duplicate removal

Computed SELECT evidence must cover dependency demand through each producer. The
[demand study](../notes/evidence.md#query-semantics-and-accepted-costs) retains
a stock aggregate counterexample: a rejecting COUNT predicate avoids final SUM
overflow through a direct projection. An INT64 `s + 0` projection must preserve
that outcome. Cover earlier predicates that do need the computation, dropped
columns, hidden ordering keys, input demand at joins/aggregates/LIMIT, empty and
nullable input, and preparation-time versus runtime constant failures.
Distinguish permitted LIMIT prefetch observations from guaranteed suppression on
rows rejected within the SELECT/WHERE/AS sequence. The integer model is
independent of production code but supplies no parser, DOUBLE, resource,
cancellation or spill qualification.

Order evidence must distinguish PipeSQL's explicit WHERE-preservation guarantee
from the pinned analyzer's weaker FilterScan property. Check a mixed-direction,
nullable order through projection of hidden keys and subsequent filtering; a
later alias must not rebind the hidden identity. Equal-key ties remain unordered
unless another declared key distinguishes them. The [ordering
study](../notes/evidence.md#query-semantics-and-accepted-costs) retains the
pinned counterexample and the scopes of model, sorter and public integration
evidence. The language manifest owns current accepted syntax. Ordering checks
also cover hidden aggregate demand, independent ordinal/identity and
physical-position mutations, actual allocation capacities at the exact minimum
and one byte short, cancellation in every producer phase, damaged and truncated
runs, short/failed I/O, temporary refusal and grouping fallback replay. Shared
sorter evidence does not replace these producer and composition checks. The
public allocator fixture also checks ordered text and join/order/count under
every allocation prefix on short and long paths, including typed refusal and
exact results after healed reopen. Logical memory admission alone cannot
establish physical allocator-failure cleanup.

DISTINCT verification covers all ordinary-output identities, duplicate-output
sharing, fresh identity ranges, preserved visible ranges and cleared semantic
order. Include all 64 columns and maximum byte widths, repeated stages, empty
input, NULL/NaN/signed-zero equivalence, and demand before and after duplicate
removal. Exercise independent child inputs, joins, pinned snapshots and grouping
fallback replay. The [DISTINCT
record](../notes/evidence.md#query-semantics-and-accepted-costs) retains public
and malformed-plan witnesses, sorter refusal/cancellation coverage, stock
allocation checks and below-working-set measurements. The allocator caller
checks the original handle before reopening: scratch-free reads remain usable
when scratch construction reports recovery debt. A healed constructor and writer
must work on the same handle when no such debt was reported. These observations
do not turn logical reservations into a whole-process memory bound.

TPC-H pipe examples are required representative inputs, not a complete semantic
suite. Results are checked against retained expected answers and, where useful,
a separately expressed query in an established engine. Dialect translation is
reviewed and cannot become the source of expected semantics.

### Declared-table global aggregate evidence

Exercise COUNT(*), SUM and AVG through public declarations, appends, prepare,
execute and terminal result consumption. Cases include empty/all-NULL input,
independent NULL patterns, exact integers beyond 2^53, final integer overflow,
AVG-only demand, later cancellation/nonfinite DOUBLE values, aliases, source and
post-aggregate filters, retained snapshots and reopen. Independent
integer/rational expectations constrain results. Test the exact source-row bound
separately from the physical cost of generating that many rows.

Force one expression lane and a one-byte-short admission refusal before query
I/O. Include output batches and transient catalog scratch in the peak. Sweep
allocation and read failures through this same aggregate path; observe owner
release, terminal errors and successful replay. The public allocator caller
checks nullable DOUBLE SUM/AVG and INT64 AVG after both normal and healed runs.
Keep the Q1 memory cap and compare the stock Q1/Q6 artifacts on identical SF1
data. General grouping and external-memory query operators require their own
evidence.

### Declared grouping evidence

Exercise the ordinary binder, physical validator and QueryResult with typed,
nullable keys, the nine-key stage boundary, aliases, projections, hidden keys,
ordered and unordered results, independent NULL patterns, NaNs, signed zeros,
dates and large UTF-8 values. Compare memory execution with forced initial disk
execution and late hash refusal followed by one pinned replay. Check ordered
predicate demand and prevent every aggregate row from escaping on late overflow.

Admit the exact complete minimum and refuse one byte below it before query I/O.
Occupy released memory with competing reservations. Check retained field sizes
and allocation capacities independently of account reconciliation. Sweep
allocation, I/O, cancellation and temporary-space refusal through the public
owner; retain the scratch bootstrap process-death and recovery checks. Corrupt
and truncate both argument and result records. Measure representative stock
queries at multiple memory caps, including large cardinality, few groups and
wide keys. Passing component tests does not complete these public obligations.

### Declared equality join evidence

Exercise the public parser, binder, planner and runtime with an independent
row-pair oracle. The ordinary public catalog suite includes a deterministic
nullable-integer corpus that crosses key distribution, key NULLs and value NULLs
in both source directions. It compares exact result multisets under projection,
range renaming, reversed equality operands, filters and aggregation on either
side of the join. Its row model uses no production semantic helpers. This small
corpus supplements the fixed typed and resource witnesses; it does not qualify
all grammar alternatives, large workloads or forced grouping fallback.

Public demand witnesses also check SUM and shared AVG across the join boundary,
unused argument elimination, join-key demand, filter order, duplicate-induced
integer overflow and cancellation before final narrowing. Expected arithmetic
failures precede output, remain terminal and release query resources on drop.

Check duplicate multiplicity and typed payloads across self-join, projection,
prior aggregation, later aggregation and another join. Equality must separate
NULL/NaN matching from sort equivalence and preserve signed-zero values. Retain
exact memory admission and physical-capacity reconciliation, duplicate rewind
corruption, cancellation and cleanup after the second input fails. Pinned
queries must keep both sources on one catalog generation across later appends.

The [composition
record](../notes/evidence.md#query-semantics-and-accepted-costs) separates the
full-gate artifact, subsequent scoped corrections, stock observations and
missing release campaigns. Shared sorter and scratch evidence applies to those
unchanged mechanisms; it cannot certify an untested join transition or a new
concurrency, workload, resource or platform claim.

### Wide declared relations

Query admission must cover all 64 stored columns, including late positions and
all admitted scalar types with NULLs. Check direct and reordered projections,
predicates, numeric programs using more than ten source columns and aggregates
over the last columns. Keep the ten-column aggregate-stage bound separate from
the 64-column result bound, including repeated post-aggregate outputs. Reject
the next column before a partial plan or result escapes.

Exercise the complete public create/append/reopen/query lifecycle on the
existing reported small-stack limit. A narrow projection must work when
unrelated payload buffers would exceed memory. Wide refusal and cancellation
must release query owners without changing authoritative state. Preserve the
original ten-output maximum-text memory/disk witness at its original limits;
separately check 64 maximum-size text outputs through public preparation and
execution under forced external-memory admission. Do not let a shared constant
silently change a retained workload without updating its resource sketch and
keeping the original case.

The public scan-memory upper bound must admit a 64-STRING source with exactly
that allowance available after database and prepared ownership. Include
transient catalog scratch, check execution and cleanup, and keep the older
full-INT64 counterexample whose actual admission exceeded the former advertised
bound.

Query identities must remain distinct when two semantic inputs originate from
the same catalog column, including when their batch positions are reversed.
Catalog binding must preserve nonsequential stored IDs independently of query
IDs. Reject missing/duplicate/tail origin entries during semantic validation and
a well-shaped false catalog ID during pinned-schema validation; reconcile memory
after refusal. Keep the public full-width small-stack case in the ordinary gate.

## Planner and optimizer evidence

Every physical plan is validated independently of the rule that produced it.
Validation checks graph shape, schema and column identity, physical property
requirements, effect legality, resource minima, and existence of the declared
external path.

The current implementation has no cost optimizer. Its lowering and independent
validation still require malformed-plan and semantic-composition checks. When
optimizer rules are introduced, for small data execute both optimized and
deliberately simple plans and compare according to semantic ordering.
Rule-specific tests preserve counterexamples for NULLs, outer joins, duplicate
names, volatile/erroring expressions, empty inputs, overflow, and
order-sensitive operators. Random plan generation records which node, property,
and rewrite combinations were reached.

Assigned mutations must demonstrate that tests detect, at minimum:

- dropped or duplicated rows and columns;
- wrong NULL and not-distinct behavior;
- lost identity or alias scope;
- invented or discarded semantic order;
- moved demanded errors or effects;
- invalid join sides or predicates;
- arithmetic overflow in estimates; and
- omitted external-path or resource requirements.

Mutation score alone is not a release target. Surviving meaningful mutations are
either covered or documented as evidence that a test claim was too broad.

## Execution and resource evidence

Each operator is tested at multiple batch capacities and value widths, including
empty batches, all-NULL data, selected/dictionary input where admitted, values
crossing internal boundaries, and output backpressure. A state-transition test
covers every source, transform, breaker, sink, blocked, resumed, cancelled,
finished, and failed edge that the operator exposes.

Memory tests reconcile the database authority with all live owners after each
transition. They deny reservations at successive growth points and check that
state remains valid, no uncharged value becomes visible, and all reservations
return after success, refusal, cancellation, panic containment, and teardown.
Allocator overhead, stacks, adapter memory, retained foreign buffers, resident
memory, and kernel page cache are measured separately from logical reservations.
Large unexplained differences fail the accounting claim. A small-stack
regression must observe the native-reported extent inside each tested worker and
enforce its ceiling; a thread builder's requested size alone is insufficient.
Report requested size, native extent, VM mapping and residency separately. A
successful bounded stack test does not establish whole-process memory or every
call-path bound.

Every supported blocking operator runs with memory caps well below its natural
working set. Tests force initial spill, repeated spill and merge, maximum
allowed fan-out/depth, skew fallback, disk-full before and during spill, corrupt
and truncated temporary data, cancellation at each phase, and cleanup after
abrupt process exit. The result must match the in-memory oracle. A spill
implementation that only works for favorable partitions is incomplete.

Work-quantum tests use wide and nested values so row counts cannot hide
excessive bytes or CPU. Cancellation latency, retained memory, occupied workers,
and cleanup debt are bounded by named units. Backpressure tests fill every queue
and prove that producers neither spin nor retain uncharged output.

### Allocation refusal and composed ownership

Public allocator callers must retain the complete observed refusal prefix for
construction, diagnostics, short/383/384-byte path boundaries, legacy
load/publication/query consumption, and declared creation/append/query/recovery/
resolution/close. Include actual permission failure and demanded corruption,
LIMIT within ordered-text and join/order/COUNT workloads, and native mutex
contention with thread startup/teardown outside allocation denial. Healed checks
compare exact typed unordered multisets, permit a new writer, and resolve
exposed tokens. Distinguish clean abort, cleanup/recovery debt, and ambiguous
commit. Retain the full temporary reservation until confirmed cleanup or
publication; close alone does not remove persistent debt.

Observe requested and usable Rust allocations, descriptors, retained results,
and return to the pre-call owner baseline, including live errors. Park two
reader threads around writer staging/publication with separate ORDER BY and
DISTINCT scratch. Check query charges plus writer deltas, allocation and temp
refusal, one-reader cancellation, writer reuse, old/new snapshots, and
competing-query refusal under held hash grouping. A deliberately false
complete-row expectation must fail after query completion. Caller
thread/barrier/buffer owners remain separate. These schedules do not prove
arbitrary races, every overlapping cut, C-runtime memory, allocator-retained
pages, or RSS; see the [ownership
record](../notes/evidence.md#resource-ownership-and-admission).

Recovery allocation checks cover short/384-byte paths, empty/data missing-peer
repair, corrupt roots, and Darwin repair-rename permission failure. The
supervisor must remove its owned ACL after child failure or timeout. Check exact
authoritative bytes, returned heap/descriptors, healed receipts/results,
repeated open, and a new declaration. After failed rename, damage to the
surviving root cannot promote a private replacement to authority. Other
platforms cannot claim Darwin ACL coverage.

### CLI ownership and outcomes

CLI checks use unchanged CLI source for native capture, allocation-free parsing
and diagnostics, loaded output, and descriptor cleanup. Resolve durable,
aborted, and unknown tokens under allocation refusal and repairing open. Obtain
ambiguous tokens from real CLI diagnostics and challenge both data-root rename
positions, including an abort followed by a successful load. Unknown is never
aborted. Closed-sink tests distinguish runtime sanitization from closure after
bootstrap.

### Linked native effects

Native callers challenge actual linked transitions independently of engine
effects:

| Caller | Required boundary and exclusions |
| --- | --- |
| Initialization | Root-stat refusal, overlapping initialization, subsequent attempts, explicit data-mount and 33-symlink names; finish owned threads before checking outcomes. This does not prove every pathname/backend or libc race freedom. |
| Synchronization | One attempt, original interruption/unsupported errors, no weaker flush; every observed create/load/repairing-open position, healed reopen/resolution/retry, and no query flushes. Keep the standard-library retry negative control. Recheck after toolchain/native dependency changes. No kernel-latency or power-loss claim follows. |
| Byte I/O | Observed read/write/pread/pwrite positions, EINTR/EIO, positive short transfers, partial progress followed by refusal, and healed commit identity. Keep standard convenience-method controls. Narrow single-threaded local-file cases do not qualify every backend, concurrent native state, latency, or process memory. |

On macOS, the observers use dyld interposition around the reviewed symbols;
`F_FULLFSYNC` is the required synchronization call, and `fsync` or
`F_BARRIERFSYNC` is a weaker fallback. On GNU/Linux, ELF interposition resolves
libc forwarding targets with `dlsym(RTLD_NEXT, ...)` before fault activation.
Missing targets terminate the caller. `fsync` is the required call; `fdatasync`
is counted as a weaker fallback. Regular and large-file positional-I/O symbol
spellings share an explicitly checked 64-bit offset ABI. Convenience-method
controls and nonempty per-operation censuses must demonstrate that observation
is active before failure coverage can pass.

Linux [`fsync`](https://man7.org/linux/man-pages/man2/fsync.2.html) requires a
separate directory synchronization for directory entries; device and filesystem
premises remain part of durability qualification. Neither the
[dynamic loader](https://man7.org/linux/man-pages/man8/ld.so.8.html) nor
[symbol forwarding](https://man7.org/linux/man-pages/man3/dlsym.3.html) instruments
kernel internals or qualifies another libc/static-linking configuration.

## Persistence and recovery evidence

`tools/check-catalog-interruption.py` exercises the stock macOS or Linux library with abrupt termination before and after observed regular-file
write/pwrite, native synchronization, rename and unlink calls. It covers a two-batch catalog
append and interrupted recovery of both unpublished and published appends. Fresh
copies isolate each cut; reached traces must match the control prefix. Public
reopen checks fixed expected rows, generations and complete transaction history,
then a later successful append and another reopen. Negative controls reject
wrong rows, generations and receipts. The [retained
campaign](../notes/evidence.md#persisted-graphs-and-interruption) records its
exact scope and replay command. This is process-termination evidence with
host-visible writes preserved, not modeled write loss or power-loss testing.
Open/close cuts, arbitrary concurrent mutations and Windows remain
outside this campaign; the broader requirements below still apply.

### Independent catalog inspection

`tools/catalog_graph.py` independently reads namespace-format-7 roots, selects
an allowed published snapshot, and walks its catalog, schema, index, native-unit
and success-history references. It imports no engine decoder or fixture encoder.
Lengths, counts, identities, canonical coverage, reserved bytes and CRC32C are
checked before dependent data is used. Every column payload is validated,
including columns an ordinary projected scan might not demand. The walk rejects
aliases and cycles and retains column identity independently of physical order.

The inspector accepts a closed, offline database or a quiescent copy. [Inspect a
persisted catalog](testing.md#inspect-a-persisted-catalog) gives the command and
output-ownership instructions. The checker takes the cooperating LOCK lease
without waiting, never repairs files and prints JSON only after validation.
Noncooperating external mutation is outside this diagnostic's contract. It
checks namespace 7 and object codecs 6; it does not implement legacy-format
inspection, backup, repair, or the production resolver's complete error
taxonomy. In particular, overlong root files are rejected rather than
interpreted as repair candidates.

The result contains table/column identities, names, types and complete rows.
Type tags 1/2/3/4 mean Int64/Double/String/Date. JSON null preserves SQL NULL;
DOUBLE is a `double_bits` hexadecimal string preserving every IEEE-754 bit, and
DATE is a signed day offset from 1970-01-01. The `successes` list gives the
attempt at each successive committed generation. Other nonzero attempts through
`issued` are aborted in the selected recovered view; later or foreign tokens are
unknown. The checker never expands an arbitrary u64 issuance prefix into
individual gaps.

`roots_settled` means both roots and fence encode the selected snapshot, not
that power-loss durability was established. `cleanup_names` lists surviving
recognized scratch/root-next names. `reachable` and `unreferenced` describe the
selected graph and their byte totals are file extents, not physical allocated
blocks. Unreferenced objects can belong to older or interrupted work; the report
is not deletion permission. An interrupted namespace can yield a valid selected
graph while its roots or cleanup remain unsettled.

Default diagnostic limits are 4,096 object names, 64 MiB of cumulative file
reads and 1,000,000 decoded values. `--max-objects`, `--max-read-bytes` and
`--max-values` can raise these to at most 1,048,576, 1 GiB and 10,000,000
respectively. File and format capacities impose additional bounds before reads.
Exceeding a diagnostic limit exits 3; invalid or unavailable input exits 2;
validated output exits 0. These bound input-dependent work and retained data,
not Python allocator/RSS usage or kernel latency. A valid database can exceed a
chosen diagnostic budget.

The ordinary gate runs `tools/check-catalog-graph.py`: a stock multi-append
graph, all four persisted types, empty tables, aborted gaps, independent
fixtures with nonordinal/physically reordered columns, checksum-valid structural
mutations, payload corruption, resource boundaries and live engine lease
contention. Wrong row/history expectations challenge the consumer. Metadata
mutations also challenge public open; payload mutations challenge complete
public queries. The append/recovery interruption campaign independently inspects
its raw cut states and healed views with this same checker, comparing against
fixed row and transaction histories. See the [retained validation
record](../notes/evidence.md#persisted-graphs-and-interruption) for reached
cases and replay. These are scoped checks, not complete corruption, concurrency,
compatibility or platform qualification.

### Remaining persistence qualification

The existing publisher and recovery path must be tested through their production
write, flush, replacement, reopen, and cleanup transitions. Effect injection
surrounds those transitions. Independent abstract models constrain allowed
states without replacing production behavior. Any replacement protocol must meet
the same fault model and outcome requirements.

For every persistent transition, test every I/O cut point and every outcome the
advertised filesystem model allows:

- refusal before submission;
- short or failed write;
- completion reported before or after interruption;
- reordered or lost unflushed writes;
- failed or ambiguous flush;
- torn or stale sectors where claimed;
- process death between effects; and
- failure while recovering from an earlier failure.

After each case, reopen through the public path and compare against an
independent abstract history. Recovery must produce exactly one documented
result: old transaction absent, new transaction durable, explicitly unresolved
commit, or fail-closed corruption/unsupported-format error. It may not return a
well-formed but semantically impossible state.

An independent structural checker starts from persisted roots rather than
production in-memory indexes. It validates bounds before reads or allocations,
checksums every protected unit, detects overlap and cycles, computes reachable
and leaked extents, and reconstructs logical catalog/table state. Mutations of
checksums, lengths, offsets, generations, references, free-space ownership, and
publication order must be detected.

Simulated I/O proves behavior only within its model. A platform durability claim
may rely on the advertised OS, filesystem, and device honoring a documented
synchronization contract. The exact runtime-to-OS primitive mapping, ordering
premises, mount/cache assumptions, and authoritative platform documentation are
part of the claim. A successful full-flush call is not evidence about lying
firmware, defective hardware, or behavior outside that contract, and release
language must not imply otherwise.

Candidate platforms also run:

- abrupt process-kill loops against stock files;
- concurrent positional I/O and reopen tests;
- filesystem-full and quota tests;
- lock and alias tests; and
- actual sync/directory-publication experiments.

Controlled power interruption is additional independent evidence. It is required
before describing a platform/device cell as project power-loss tested or
certified, and whenever correctness relies on behavior not guaranteed by the
documented platform contract. Process kill, VM termination, forced detach, and
injected I/O are never labeled controlled-power evidence.

The platform matrix records filesystem, mount and cache settings, device class,
OS, documented atomicity/durability premises, and observed behavior. Missing
required documentation, an unavailable synchronization primitive, or a
contradictory stock result removes the cell or redesigns the protocol; it is not
waived because another platform passed. Every applicable platform campaign is
repeated against the exact production artifact before release.

Compatibility tests retain byte fixtures from every stable release. Current
writers, readers, inspectors, backup/copy paths, and upgrades run against them.
Unknown authoritative features and newer incompatible formats fail closed.
Downgrade behavior is explicit and cannot select stale data to appear
successful.

### Catalog reclamation evidence

Reclamation qualification must use `Database::reclaim`, engine-owned scratch,
actual unlink and directory synchronization. Check old and current query
results, all retained receipts, all snapshot slots pinned, cancellation after
unlink, resource and allocator refusal, and another append after cleanup/reopen.
Sweep scratch creation, scan, merge, unlink and synchronization failures. Cross
failed unlink with failed synchronization and scratch debris with authoritative
graph corruption. Process-death and real-thread checks supplement deterministic
effects; none establishes hardware power-loss behavior.

The external inventory's synthetic maximum-input and checksum tests establish
component bounds only. They exclude complete graph admission, namespace I/O,
engine scratch creation and durable cleanup. Current results and any remaining
gate failures belong in the current work plan until resolved.

### Shared scratch lifecycle evidence

Exercise the shared constructor through ordinary reclamation and directly at its
internal ownership boundary. Retain a public query while a writer publishes with
a scratch name present. Use real threads to show bootstrap contention does not
hold writer authority. Two unlinked owners must share temporary admission
exactly; failed writes keep charges and one byte beyond the combined limit fails
before I/O. Sweep constructor effects, cancellation before and after durable
removal, and process death at creation, unlink, the directory barrier and
admission. Reopen must heal recognized empty debris while refusing corrupt
authoritative state, unknown names, nonempty named files and aliases. Check
checksum-valid old catalog namespace records are rejected and independently
reproduce new records; child object codecs and legacy format-4 behavior retain
their existing checks. This evidence does not qualify a grouped or
external-memory query operator.

## Concurrency and public-interface evidence

Deterministic schedules enumerate small ownership and state-machine races, but
release tests also use real optimized threads. Campaigns cover open/close,
reader/writer publication, snapshot retention, cancellation versus completion,
queue saturation, worker failure, and teardown with outstanding results. Dynamic
race and memory tools are used on supported targets; absence of a report is not
a proof when the relevant transition was unreachable.

Public API tests use only exported interfaces and stock artifacts. They verify
value and buffer lifetimes, close ownership, stale handles, concurrent permitted
and forbidden calls, cancellation identity, partial result failure and
diagnostic allocation failure. Compile-time rejection is evidence where the Rust
API excludes an invalid operation, such as reuse after consuming close.
Repeated-close and callback-reentry tests apply to interfaces that expose those
operations. Foreign interfaces, if admitted, are tested from each supported
language toolchain. No test-only constructor, global last-error state, hidden
thread affinity, unwind, leak, double release, or output mutation on failure is
accepted.

## Fuzzing, simulation, and replay

The current implementation injects failures and short I/O around production
`Effects` transitions and exercises native calls through separate test
interposers. The gate also runs independent attempt-identity and publication
models. Those models assume validated roots and stated persistence rules; they
do not execute the DBMS or establish its crash behavior. The composed scheduler,
time, allocation and I/O simulation described below is a qualification target,
not a claim that a whole-system deterministic simulator already exists. Add
coverage around the production owners instead of duplicating their semantics in
a simulator.

Use several generators with different representations and blind spots:

- lexer/parser bytes and token structures;
- binder expressions, names, scopes, and row shapes;
- logical plans and optimizer rewrites;
- individual vector, hash, sort, join, and spill states;
- storage bytes and publication histories;
- API lifecycle and concurrency operations; and
- whole-database mixed workloads once the real state machines compose.

Each generator reports structural coverage, state/transition coverage, and the
frequency of important semantic dimensions. Inputs such as duplicates, NULLs,
skew, width, cardinality, order, and failure position vary independently unless
the campaign intentionally couples them. A generator that cannot reach a claimed
state invalidates the corresponding fuzz claim.

Deterministic simulation controls scheduling, time, allocation admission, and
I/O around production transitions. A safety phase may continue injecting bounded
faults while checking that no invalid state is published. A separate liveness
phase stops new faults, establishes explicit fair scheduling and usable
resources, and requires pending work either to complete or reach its documented
terminal refusal. Permanent arbitrary faults do not support a progress claim.

A random failure first records its source revision, seed, configuration, input,
choice/effect trace, and oracle version for immediate replay. The durable
regression is a minimized structured fixture. Replaying a seed under a changed
generator is not required to preserve meaning; replaying the retained fixture
is.

Checker and generator mutations are as important as production mutations.
Deliberately remove fault choices, collapse data dimensions, weaken oracle
comparisons, and corrupt replay data to prove the harness notices its own blind
spots.

## Performance evidence

Performance runs use optimized stock artifacts and verify query results in the
same run. Record source revision, compiler, CPU, memory, storage, filesystem,
data scale and distribution, cold/warm state, thread and memory limits, elapsed
and CPU time, allocations, peak logical and resident memory, bytes read/written,
spill, pruning, and relevant tail latency.

Representative benchmarks cover each admitted capability, including:

- complete admitted TPC-H queries at multiple scales;
- scan, decode, filter, expression, string, hash, join, aggregate, sort, top-N,
  and window explanations;
- ingest, commit, reopen, checkpoint/maintenance, and backup/copy behavior;
- forced spill at several fractions of the natural working set;
- narrow, wide, NULL-heavy, duplicate-heavy, correlated, and skewed data; and
- mixed readers and one writer under a shared memory limit.

DuckDB is a comparison point on identical data and hardware, not the semantic
oracle and not an excuse to copy its scope. Microbenchmarks explain a profile;
they cannot override an end-to-end regression. A faster wrong, unbounded, or
non-durable result fails.

Each stabilized capability receives a regression envelope based on retained
measurements. A regression outside it is investigated and either fixed or
accepted with explicit product-level evidence; changing the workload to hide it
is not acceptance.

## Production consolidation gate

Consolidate continuously using [the engineering
guide](engineering.md#maintain-documentation-and-evidence). Do not defer removal
of verified redundancy, stale guidance or obsolete paths until release. Before
retiring an artifact, transfer its required facts and falsifiers and verify the
affected consumers. Structural changes require their applicable checks; earlier
passing evidence does not certify changed behavior.

The final audit applies to the exact shipping tree and artifacts:

1. The implementation passes all applicable semantic, durability, resource,
   failure, concurrency, platform and stock-performance gates after the final
   changes. The release criteria below remain binding.
2. Remove obsolete adapters, migration-only paths, dead code and speculative
   abstractions. Review the shipping source, tests, dependencies, unsafe code,
   generated behavior, public surface and persistent fixtures. Growth must justify
   its runtime and maintenance cost even below the production source ceiling.
3. Transfer still-required contracts, reasons, limits and falsifiers to maintained
   owners. Retain usable evidence for supported claims, consequential decisions
   and unresolved failures. Retire obsolete packages by purpose; reconstructing
   every historical experiment is not required. Missing evidence blocks its claim,
   not unrelated work with sufficient verification.
4. README and the distinct guides must explain the system's current behavior and
   design constraints without requiring reconstruction of development chronology.
   Consolidate repeated rules, stale status and executable details that have a
   clearer owner. Preserve reasons needed to change invariants safely.
5. Implementation and documentation names describe capabilities. P/Q names remain
   only as external TPC-H provenance in the workload matrix; numeric format
   revisions remain internal decode/compatibility discriminators. Historical notes
   are not a product interface or required runtime input.
6. A clean checkout has valid links, clear contract ownership and complete build
   and reproduction instructions. No supported claim depends on an undocumented
   machine-specific artifact or external experiment archive.
7. Verify maintainability through real engineering work: trace an observable
   result or failure to its semantic and effect owners, explain the invariant,
   make a bounded correction, and demonstrate that the regression detects it.
   Review whether ownership, control flow and diagnostics make that reasoning
   understandable and the change local to its responsibility. Repair discovered
   code, interface, documentation or tool obstacles. Use evidence from ordinary
   reviews and defect repairs; a separate newcomer exercise is not required.

The audit checks the ability to understand and safely change the software.
Document inventories, automated link checks and test counts alone cannot
establish that property. Evidence for item 7 does not substitute for any runtime
gate.

Stable contracts have explicit compatibility rules and change deliberately;
development continues. PipeSQL has no public numeric product-version epochs.
Immutable build provenance identifies releases, and internal format revisions
make incompatible bytes fail closed.

## Release criteria

A release candidate may be called production-ready only when all of the
following hold for the exact released artifacts:

1. Every advertised syntax, operator, type, function, API, and format version is
   listed; every accepted surface passes its semantic and negative corpus.
2. All representative and boundary results agree with independent evidence.
3. Every blocking operator passes forced-memory, spill, skew, cancellation,
   temp-failure, and cleanup campaigns.
4. Memory and temporary-space limits reconcile under concurrent workloads.
5. Every persistent transition passes abstract-model, corruption, crash-cut,
   process-death, and documented-platform-contract tests with truthful outcomes;
   any advertised power-loss-tested certification also has controlled-power
   evidence for the exact released artifact and device cell.
6. Public lifecycle and real-thread campaigns pass without leaks, races,
   deadlocks, double ownership, or unwinding across a foreign boundary.
7. Targeted fuzzers, whole-system campaigns, mutations, and durable replay show
   that the claimed state space and checkers are actually exercised.
8. Clean-checkout builds and tests reproduce with pinned tools and inputs;
   shipped dependencies, generated behavior, licenses, and unsafe code are
   reviewed.
9. Shipped first-party production code remains below the 500,000-line ceiling.
10. Representative performance meets the published release envelopes with no
    unexplained correctness or resource exception.
11. Stable files and APIs pass retained backward/upgrade fixtures.
12. No known release-blocking defect or untested advertised platform remains.

Failure of one item blocks the corresponding claim. The response is to narrow
the advertised surface or repair the design—not to weaken the test, rename the
failure, or average it away.
