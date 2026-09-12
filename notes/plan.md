# Work plan

[README](../README.md) owns product requirements; [Engineering](../docs/engineering.md)
owns working method. This plan identifies unfinished product work.
[Evidence](evidence.md) records verification and consequential limitations.

## Current baseline

Use the [reading path](../docs/README.md#learn-the-implementation),
[source map](../docs/source-map.md), [test guide](../tests/README.md), and
[tool guide](../tools/README.md) to navigate the implementation and its checks.
Maintained builds, tests, and examples require no historical checkout or archive.

The complete 24-stage gates for `986b673` pass on macOS and GNU arm64 Linux
on matching frozen inputs. Each platform executes 501 Rust tests and 298
composition cases, plus its applicable native and allocation campaigns. Bounded
thread scenarios and their ordinary-thread counterparts execute on both platforms,
including full-width union preparation and execution.
Target-specific ceilings remain explicit.
Two Darwin ACL-specific allocation cells remain excluded on Linux. Windows remains
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

## Completed: column transformations

SELECT, EXTEND, SET, DROP, RENAME, and AS form the current bounded
column-transformation profile. The [language manifest](../docs/language.md#current-public-query-manifest)
owns its syntax and semantics; [evidence](evidence.md#column-transformation-semantics)
records independent cases, retained qualified inputs, typed-copy identity,
resource/failure coverage, and the verified cross-platform tutorial. Both complete
gates pass on frozen source `5a3cb0a`. The 64-column public bound remains intact;
internal payloads admit up to 128 visible and retained values. Larger inline
owners retain their existing resource charges. Compact hash storage and EXTEND
remain complete. Reopen these only for a concrete defect or limitation.

## Completed: composed memory attribution

The maintained ownership caller reconciles prepared plans, two parked readers,
and an append against requested and allocator-usable bytes on macOS and GNU/Linux.
The [resource equations](../docs/resources.md#interpret-composed-memory-observations)
explain inline handles, allocation allowances, and unused pathname capacity.
Independent wrong-row and wrong-attribution controls fail at their intended
checks. Physical allocation refusal, logical admission refusal, temporary refusal,
cancellation, completion, and final release preserve the other owners.

Caller synchronization storage is separately destroyed and observed. macOS
allocator rounding can exceed an engine owner's logical charge, so physical
memory remains a release obligation. The [evidence](evidence.md#attribution-of-composed-memory)
records the measured scope; it does not establish a whole-process cap.

## Completed: native mutex sanitizer baseline

The maintained [sanitizer command](../docs/testing.md#qualify-native-sanitizer-observations)
passes on macOS and GNU/Linux. Clean/fault controls establish AddressSanitizer
heap-bounds detection and its expected failure exit. The pinned compiler,
uninstrumented nightly, and instrumented nightly each execute all four mutex
tests. Receipts record unchanged inputs, compiler/runtime/artifact identities,
linked native libraries, and cleanup. Verifier negative controls reject empty
selections, wrong failures, and ambiguous artifacts.

This qualifies the exercised Rust wrapper paths. Prebuilt standard libraries
and native pthread implementations remain uninstrumented; ThreadSanitizer,
other native boundaries, Windows, and general race freedom remain unfinished.
The [evidence](evidence.md#platform-and-sanitizer-limitations) records the limits.
Do not repeat this investigation without a concrete new report or affected change.

## Completed: positional UNION ALL

The [language contract](../docs/language.md#union-all) owns positional syntax,
names, types, scope, ordering, and demand. The runtime streams admitted branches
through one output batch using the existing scheduler. Independent semantic and
physical validators check mappings and graph edges. No whole-result union spool
or second query pipeline was added.

The complete macOS and GNU/Linux gates pass on frozen implementation `98de11f`.
All eight public union tests execute on both platforms, including forced spill,
refusal, snapshots, cancellation, and full-width small-stack execution. The
[tutorial](../docs/getting-started.md#combine-pipeline-results) runs on both;
[evidence](evidence.md#positional-union-all) records provenance, checks, and limits.
Do not reopen this milestone without a concrete defect or missing contract.

## Completed: testing and tooling cleanup

The finite review follows Cargo module inclusion and each Python command's actual
callers, fixture construction, assertions, and effect/cleanup paths. The reviewed
areas and retained contracts are below. The test/tool maps own navigation.

| Reviewed area | Coverage, independence, and disposition |
| --- | --- |
| Four public suites and all child modules | Literal SQL/results and independent nullable-row/Boolean models cover lifecycle, scalar/aggregate composition, snapshots, spans, demand, spill, cancellation, and release. Keep ordinary/bounded-thread pairs: only the latter observes native stack extent. Share directory mechanics; fold the vacuous lease-child entry into its parent and verify early teardown. |
| Frontend and physical planning | Lexer/parser/binding limits, identity/scope/span cases, and independent malformed-plan mutations remain beside their owners. Shape assertions support validator boundaries; they do not replace public result oracles. Remove two review-era test-name prefixes without changing bodies. |
| Execution and resource tests | Scan, batch/scalar, accumulation/arguments, grouping/hash/reduction/replay, sorting/join/order, LIMIT/union, and authorities retain exact/short admission, measured capacities, row/byte bounds, demanded failures, cancellation at observed phases, and release. Generated row expectations and rational rounding vectors remain independent. Keep internal effects and corruption cases that public SQL cannot inject. |
| Storage, database, and catalog tests | Schema/unit/index/history bytes, declaration/append/publication, pins/receipts, recovery/reclaim/scratch, and legacy loading retain independently encoded vectors, matching-checksum corruption, effect cuts, short I/O, abort/ambiguous outcomes, and healed retries. Replace ignored or panic-on-unwind directory cleanup across fixture owners; remove the now-duplicate cleanup regression. |
| CLI and filesystem tests | Argument/source/sink/diagnostic ownership, native metadata/path/name/extent/locking/mutex behavior, real threads, and native stack controls remain. Keep the iterative deep-path teardown and platform guards; their OS/depth premises differ from ordinary directory mechanics. |
| Fixtures and reference models | All persisted/SQL/numeric fixtures have maintained consumers or rejected-format provenance checks. Keep old-format independent encoders/decoders, the Q1 comparator, identity/publication/reclamation models, and their negative controls. Consolidate five catalog encoders and the duplicated semantic snapshot writer; remove six superseded script files and unused digest walks, preserving bytes and separate query expectations. Add the five catalog/schema vectors missing from the ordinary fixture check. |
| Python commands and their tests | Maintenance discovers every `test-*.py`; syntax, docs, manifests, fixture comparison, entry-point guards, oracle rejection, build/loader selection, sanitizer interpretation, receipts, and live-process cleanup retain distinct controls. No maintained caller fixture, native observer, standalone identity diagnostic, or sanitizer control is dead. |
| Campaigns and gate | Keep full allocation prefixes, healthy controls, native operation censuses/short-transfer crossings, graph mutations, and append/recovery interruption cuts. Independent graph/semantic checks remain outside producer code. A fresh Mac stock CLI build took 8.54 s; two semantic campaigns redundantly built it. The gate now builds it once after Cargo checks and supplies the immutable artifact sequentially, removing its target and case databases afterward. Native callers retain isolated targets. |
| Documentation | Update test/tool maps, fixture generation, and gate artifact ownership. Ordinary shell/Cargo/Python entry points remain; no new runner or workflow framework. |

Implemented worklist: shared test-only cleanup; lease-child discovery and bounded
teardown; complete catalog fixture comparison and consolidated encoders; one stock
CLI for both semantic campaigns; removal of stale naming and command references.
The public cleanup test retains normal, missing-directory, real-error, and unwind
controls. Rust discovery must reconcile two removed tests (the empty child entry
and duplicate cleanup check), one moved cleanup test, and two renamed tests.
No SQL result, fault schedule, persisted fixture byte, or platform exclusion was
removed. Five before/after snapshot-writer comparisons match, including empty
input and a DOUBLE block crossing. New tooling controls reject damaged schema
vectors and reused output names; gate controls check artifact ordering/cleanup.

The final navigation pass and both complete gates pass on frozen implementation
`a315e21`. Discovery reconciles 496 ordinary Rust tests per platform, with no
ignored tests; the normal lease child and intentional early teardown run
separately. Maintenance executes 93 tooling tests and compares 44 independent
codec fixtures. Both platforms retain 24 semantic and 298 composition cases and
all applicable native, allocation, interruption, and graph campaigns. Removed
script/test names have no active consumers. Fixture bytes and oracle/model
sources are unchanged. The documented generator also runs from outside the
repository into a fresh output. Final documentation checks cover the corrected
five-public-stack-scenario map. [Evidence](evidence.md#full-verification-checkpoint)
records the frozen inputs and costs. Owned outputs are removed; changes are
committed locally without publication. The append repair below is also complete;
the sorting-reader and grouped-query repairs below are also complete.

## Completed: append allocation bounds

The reproduced 7,615-byte macOS deficit came from summing encoding and later
commit scratch in one allocation. Commit `1633477` sizes that shared workspace by
the maximum of the two phases. Larger reference arrays and workspaces also cross
native size classes; admission now reserves a qualified 16,384-byte rounding
ceiling for each of its three retained allocations before allocation/issuance.
The [resource contract](../docs/resources.md#streaming-append) owns the equations,
request-size ranges, growth/release order, and allocator premises.

Both complete gates pass on matching frozen inputs. The independent caller checks
460,865 workspace sizes, 4,096 reference counts, and a missing-ceiling negative
control. Full-width small/maximum/small writes exercise growth, reuse, 1,025 and
4,096 retained reference capacities, independent COUNT/SUM results, publication,
and release. Exact/short admission, old-buffer release before growth, allocation
refusal, cancellation, recovery, and all retained interruption schedules pass.
The [evidence](evidence.md#attribution-of-composed-memory) records the smaller
physical workspace and larger small-append reservation. Owned outputs are removed
and changes are committed locally. Custom allocators and process/RSS bounds remain
separate qualifications.

## Completed: sorting-reader allocation bounds

Commit `ca5f59e` repairs the reproduced macOS ORDER BY/DISTINCT deficits of
4,380/4,700 bytes. A 21-allocation trace located the main size-class increment in
the native INT64 payload. Admission now requests and charges whole 16-KiB
payload capacities before allocation or I/O. The [resource contract](../docs/resources.md#declared-scan-admission)
owns the type capacities, unchanged encoded limits, and physical-memory tradeoff.
No sorter allowance, allocator framework, or query feature was added.

Both complete 24-stage gates pass on identical frozen inputs, with 499 ordinary
Rust tests per platform. The independent reader equation, complete-row oracle,
12 one-/64-column typed cases, exact/short admission, spill/replay, cancellation,
refusal, and final release pass. Linking the observer to the previous library
rejects the original deficit after joining its barrier participants. Two grouping
fixtures required exactly three payload increments of additional test budget;
their actual spill, temporary refusal, cancellation, and release assertions remain.
[Evidence](evidence.md#attribution-of-composed-memory) records the observations,
initial gate failures, and qualification limits. Owned outputs are removed and
changes are committed locally without publication.

## Completed: grouped-query allocation bounds

Commit `495cbdd` repairs the reproduced 111,496/112,080-byte macOS GROUPED
prepared/result deficits. The 66-allocation trace located large buffer tails and
hash-array capacity rounding. Large blocking buffers now own charged 16-KiB
units, optional run growth fits those capacities, and hash group counts use powers
of two. Encoded row/frame limits and the external fallback minimum remain intact.
The [resource contract](../docs/resources.md#blocking-buffer-capacity) owns the
policy; [evidence](evidence.md#attribution-of-composed-memory) records the trace,
old-library rejection, complete owner measurements, and qualification limits.

Both complete 24-stage gates pass on identical frozen inputs with 500 ordinary
Rust tests per platform. The new padding regression, independent full-row oracle,
exact/short admission, growth/fallback, spill/replay, refusal, cancellation, and
release pass. The public caller now checks allocator-usable bytes against the
complete grouped charge at both pathname lengths. The 514-buffer/91-hash-layout
census qualifies the exercised stock allocator families, not arbitrary schemas
or process/RSS memory. Owned outputs are removed and changes committed locally;
no publication occurred.

## Completed: mixed aggregate allocation qualification

Commit `986b673` repairs the allocation-history deficits exposed by the finite
40-case public profile. Traces located retained native excess in arbitrary key
arena and state-array extents. Large arrays now request charged power-of-two
capacities while retaining exact logical state lengths. Key-slot padding is
explicit, and metadata sizing reduces optional group capacity when necessary.
The encoded-key limit remains complete when its rounded allocation fits.

Both complete 24-stage gates pass on identical frozen inputs with 501 ordinary
Rust tests per platform. All 40 sequential cases check complete independent rows,
real hash execution, requested/usable ownership, and release; the old library
fails the same guard after those checks. The actual-capacity unit test rejects
charge-only padding. Existing 4,096-group hash/spill budgets, fallback/replay,
refusal, cancellation, and release tests remain intact. The current catalog
campaign exercises all 795 allocation prefixes at each pathname length.
[Evidence](evidence.md#mixed-aggregate-allocation-history) records the causal
sequence, rejected prototype, final measurements, and qualified scope. Owned
outputs are removed and changes committed locally without publication. Arbitrary
histories, transient peaks, and process/RSS memory remain separate obligations.

## Current: grouping capacity cost

The allocation repairs preserve correctness and bounded fallback, but their
possible earlier spill is not yet a measured performance claim. Compare retained
`495cbdd` with `986b673` using the existing grouping examples and public fixtures.
The numeric example currently fixes 4,096 evenly distributed groups and has no
execution timer; the STRING example already separates input construction from
execution/validation timing and checks complete rows and release.

Finite worklist: extend the numeric learning example to retain its existing
invocation while allowing the existing 32-/4,096-group even/skewed fixtures and
reporting execution/validation time. Use the same caller source against both
stock libraries. Measure those four distributions at 1.2 MB and 2 MB, plus the
existing four-/256-group STRING cases at 8-byte width and 4 MB. Use three
repetitions per cell on macOS and native-storage GNU arm64 Linux, checking every
result and sampled temporary peak. Limit initial measurement and triage to
30 minutes of execution per platform; preserve failures and reassess material
regressions before extending the profile. No new benchmark runner or framework.

Keep source/input identity, timing scope, and physical-memory exclusions explicit.
Fix a runtime issue only after attributing it to the measured capacity change;
otherwise retain the example improvement and concise cost evidence. Verify the
changed example and documentation, remove owned outputs, and commit locally.
No measurement process or scratch output is currently active for this milestone.

## Applying DuckDB lessons

DuckDB's published designs inform the following work. These are PipeSQL design
choices and acceptance checks, not claims that DuckDB's results transfer here.
The [MIN/MAX evidence](evidence.md#typed-min-and-max) and
[grouping workload](evidence.md#grouping-learning-workload) record the delivered
implementation, examples, checks, and accepted costs.

| Priority | Lesson and source | PipeSQL action and completion check |
| --- | --- | --- |
| Applied in MIN/MAX | [External aggregation](https://duckdb.org/2024/03/29/external-aggregation) makes variable-width ownership and relocation part of the spill design. | Capture owned STRING bytes before releasing source batches. Independently validate replay lengths, UTF-8, NULLs, and type identity. Exercise maximum-length values, growing/shrinking replacements, and replay after producer release. Keep checked serialized records; pointer relocation would add machinery without an established need. |
| Applied in compact hash storage | The same external-aggregation study varies group cardinality and measures operation beyond available memory. | Existing workloads check cardinality, budget pressure, and full-width text replay; exact admission checks cover the fallback minimum. The completed STRING study establishes unused capacity and short-string spill costs. Compact spans and admitted arena growth remove short-string spill in the maintained 4 MB workload. Retain the recorded transient growth cost for wide values; no speedup is claimed. |
| Applied in MIN/MAX | [Memory management](https://duckdb.org/2024/07/09/memory-management) treats blocking intermediates and their competing owners explicitly. | Extend existing composed-query checks to extrema. Exercise memory refusal, exhausted temporary capacity, cancellation, and release. Attribute spill to the relevant operator or a controlled query; total temporary bytes alone do not prove aggregation spilled. |
| Current tests | DuckDB's [SQL test guidance](https://duckdb.org/docs/lts/dev/sqllogictest/writing_tests) favors exercising behavior through SQL. | Keep queries and independent expected results visible in existing public tests. Retain internal corruption and allocation controls where SQL cannot establish the invariant. No additional test framework is needed. |
| Applied in ownership attribution; physical cap remains open | DuckDB's memory-management article distinguishes component memory and temporary storage measurements. | The maintained caller attributes prepared/query/append charges and caller synchronization on macOS and GNU/Linux. Allocator rounding can exceed an owner's charge; this does not establish a usable-heap or process-memory cap. |

DuckDB is not a universal differential oracle. Its documented
[floating-point ordering](https://duckdb.org/docs/current/sql/data_types/numeric#floating-point-types)
places NaN above other numbers. Our GoogleSQL-derived MIN/MAX contract propagates
NaN in both directions. Establish compatible NULL, overflow, floating-point,
collation, and ordering semantics before comparing results. Keep explicit local
expectations for incompatible cases; never change them to match DuckDB.

The qualification gaps below remain separate obligations. Partitioned hash
aggregation, a shared buffer manager, and parallel execution remain possible
alternatives, not scheduled rewrites. Investigate one only when a representative
workload exposes a concrete limitation in the current owner. Require an independent correctness oracle,
resource costs, and an end-to-end measurement that could reject the proposal.
Keep adopted invariants in their existing contracts and replace settled plan
entries instead of accumulating a research archive.

## Remaining qualification

| Concern | Required outcome |
| --- | --- |
| Windows | Implement native paths, handles, traversal, locking, synchronization, CLI startup, process ownership, and target-specific verification. |
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
