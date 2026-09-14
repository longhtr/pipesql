# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan owns the
unfinished queue and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The verified implementation checkpoint is `464fbd6`. Its
[calendar-year record](evidence.md#calendar-year-projections) identifies matching
macOS/GNU arm64 Linux core gates: 680 ordinary Rust tests, 100 tooling tests,
44 independent codec fixtures, expanded ownership checks, 24 semantic cases and
326 composition records. Fresh calendar, declared-table, logical-plan and STRING
measurement examples pass. The last complete 24-stage native/persistence
[checkpoint](evidence.md#full-verification-checkpoint) remains attached to
`1e5cfdc`; newer scoped checks do not update that full claim.

The engine already has typed appends, snapshot readers, nullable projections,
joins, grouped aggregation, sorting, spill/replay, cancellation and recovery.
The [language manifest](../docs/language.md#current-public-query-manifest),
[public interfaces](../docs/interfaces.md), [test map](../tests/README.md) and
[tool map](../tools/README.md) locate their exact scope. Existing
[preparation/execution](evidence.md#query-execution-walkthrough),
[snapshot](evidence.md#snapshot-pins-and-retained-outcomes),
[equality](evidence.md#sql-equality-and-stored-double-bits) and
[partial-result](evidence.md#partial-query-results-and-terminal-ownership)
lessons remain complete. The testing/tooling overhaul and earlier language,
append, reader and grouped-allocation repairs remain closed unless a concrete
defect or new workload changes their scope. No historical archive is required.

## Internal analytical learning checkpoint

This is the finite internal checkpoint informally called “0.0.1 level.” It does
not change the package version, activate publication, stabilize an API or format,
or certify the [production release criteria](../docs/verification.md#release-criteria).
It covers macOS and GNU arm64 Linux on the qualified local/native database paths.
Windows work is outside this queue and remains an unfinished product obligation.

The checkpoint follows one complete analytical task: append events and a small
dimension table, reopen, prepare a report, publish more events while an older
report remains pinned, compare old/new answers, reclaim safely and recover an
interrupted append. Events contain an INT64 identity, a nullable dimension key,
a nullable DATE, a nullable INT64 amount and a DOUBLE measurement. Dimensions
contain an INT64 key and a STRING label. A LEFT JOIN retains missing dimensions;
year extraction and grouping produce per-year/per-label counts and totals.
A separate projection exercises the DOUBLE values without making approximate
arithmetic the oracle for exact integer totals.

Use a readable sixteen-event fixture first, with literal expected rows. Add a
bounded 131,072-event profile with independently varied NULLs, missing keys,
duplicate join keys and skew. Keep tables, batches and native units within the
existing [ingestion limits](../docs/interfaces.md#streaming-ingestion). Choose
high/low query budgets from measured working storage: the low budget must admit
the operation and demonstrably spill. A large input file alone is not spill
evidence. Reuse caller buffers and existing test/campaign owners; no new server,
scheduler, general expression framework or simulator is needed.

### Completion criteria

| Criterion | Evidence required before this checkpoint is complete |
| --- | --- |
| Correct, useful answers | Exact schema and complete rows agree with literal small-case expectations and an independent row model at scale. Cover NULL, duplicates, missing dimensions, skew, empty input and demand-sensitive errors through stock public interfaces. |
| Bounded ownership and failure | High/low budgets agree; actual spill/replay is observed. Requested allocation ownership, memory/temp refusal, cancellation, partial-result failure and terminal release reconcile in the composed workload. Preserve every existing strict negative control and demanded error span. |
| Snapshot and publication truth | Reader/writer/reclamation schedules preserve pinned answers and receipts. Process cuts and failed recovery produce only independently permitted old/new/unresolved/refused states, followed by a successful healthy continuation when the contract allows it. |
| Understandable architecture and interfaces | Follow one result and one consequential failure from public call to semantic identity, producer, allocation/effect owner and cleanup. Repair obstacles discovered during these changes beside their owners; do not require speculative rewrites or a separate cosmetic audit. |
| Usable learning path | One fresh setup-to-cleanup path connects the report to existing preparation, execution, snapshot and outcome explanations. Commands check completion, expected failures and release. Readers need no development chronology or removed outputs. |
| Reproducible final evidence | Review the exact final tree and its API/format claims; run complete sequential macOS/Linux gates and the composed workload on matching frozen inputs, followed by fresh examples. Record discovery, negative controls, source/artifact identities and exclusions; clean owned outputs and commit. |

An unexplained semantic, requested-ownership, publication or cleanup failure in
this workload blocks completion. An existing passing component test cannot
stand in for the missing composed observation. Every implementation goal includes
its affected documentation, readable ownership flow and sufficient verification;
quality work is not deferred to the final gate.

### Ordered remaining goals

The active goal is **1. Establish the composed report**.

These are nine bounded outcomes, not nine already activated goals. Activate one
through the goal tool at a time. Prerequisites below name queue positions; the
first goal has no unfinished prerequisite. Existing narrower scenarios are inputs
to these goals, not work to repeat for its own sake.

| Order and outcome | Prerequisites | Cheapest useful falsifier | Completion and verification boundary |
| --- | --- | --- | --- |
| 1. Establish the composed report | Current baseline | Run the sixteen-event public query against literal rows; deliberately alter one expected group and require rejection. | Add reusable, bounded fixture construction and an independent reference answer. Check all four types, LEFT JOIN multiplicity, nullable year groups, two append generations, schema, Finished and release. Explain the fixture and query beside their owners. Reuse [composed](../examples/composed.rs), [calendar](../examples/calendar_year.rs) and [snapshot](../tests/catalog_lifecycle/snapshots.rs) patterns. |
| 2. Exercise scaled spill and refusal | 1 | Compare one high-budget run with one admitted low-budget run and assert observed temporary storage. | Add the scaled profile, skew/NULL/empty variants and exact answers. Exercise memory and temp refusal, spill cancellation and replay using existing [grouping tests](../src/execution/aggregation/grouping/tests/replay.rs) and [resource contracts](../docs/resources.md). Repair any affected producer/admission defect; do not lower expectations to avoid the failing path. |
| 3. Reconcile transient allocation histories | 2 | Arm the existing observer around one preparation/execution/drop history for the report, then challenge it with a known wrong attribution. | Extend the [ownership caller](../tools/fixtures/composed-ownership.rs) for cold, reused and refused construction histories at both pathname lengths/platforms. Observe transient requested/usable quantities, live errors and release separately. Resolve any newly exposed engine ownership gap. Compare with the retained native-reuse limitation below without padding an allowance or claiming a general physical cap. |
| 4. Exercise overlapping report lifetimes | 1–3 | Park two report readers at different generations, cancel one while a writer/reclaimer advances, then rerun both pinned plans with fresh tokens. | Extend the existing [real-thread snapshot scenario](../tests/catalog_lifecycle/snapshots.rs) with blocking report state, shared-budget refusal and bounded alternative publication/cancellation/completion orders. Require exact old/new rows, receipts, clean terminal ownership and healthy reuse. Retain bounded waits and a control that breaks pin protection. This is enumerated schedule evidence, not arbitrary-race freedom. |
| 5. Recover the mixed append history | 1 and 4 | Interrupt one report-data append between an issuance/publication boundary and verify the public reopen outcome against independent history. | Extend the existing [interruption campaign](../tools/check-catalog-interruption.py) with the composed data and retained reader/receipt history where process lifetime permits. Cover relevant publication cuts and recovery-after-failure, inspect the persisted graph independently, and require a healthy subsequent append/report. Reuse process ownership and cut machinery; do not implement a second publisher. |
| 6. Challenge query composition | 1–2 | Apply a harmless rename or legal projection boundary to the report and compare complete typed answers; a wrong-row oracle must fail. | Extend the [composition corpus](../tools/check-composable-aggregates.py) with bounded variants of identities, NULL predicates, source/derived inputs and numeric consumers. Include rejected near-misses and demanded spans. Use only equivalences guaranteed by the language; record covered dimensions and retain replayable failing inputs. No general fuzz framework is required. |
| 7. Challenge corruption and failed recovery | 5 | Corrupt one referenced typed payload and one authoritative metadata field in fresh quiescent fixtures; demand must fail without silently choosing an older valid state. | Add composed-workload cases to existing [graph and corruption checks](../docs/verification.md#independent-catalog-inspection). Preserve skipped payload demand, bounded validation, failed-recovery outcomes and cleanup. Independently distinguish structural validity from the report's semantic result. Keep source and scratch corruption expectations with their actual owners. |
| 8. Measure and teach the complete workflow | 2–7 | Run the same verified report at both budgets and compare complete-query costs; stop an optimization proposal if it cannot change that result. | Measure ingestion, prepare, execute, reopen and reclamation phases with CPU, I/O, logical/temp and process-memory observations kept distinct. Document a bounded baseline and explain material differences. Consolidate the fresh report lesson with existing owner explanations, including one failure and recovery path. Make performance changes only for a demonstrated workload cost and verify them fully. |
| 9. Verify the frozen internal checkpoint | 1–8 | Run the documented workflow from a clean source export; any undocumented setup dependency or false capability claim fails the audit. | Review current interfaces, dependency/unsafe boundaries, documentation and actual change locality using the work above. Remove only demonstrated stale/duplicate material. Run the complete [gate](../docs/testing.md), all added campaigns and sequential fresh examples on matching inputs. Reconcile records, preserve exclusions, record compact evidence, clean outputs and synchronize coherent commits. |

### Estimate and reassessment

Nine planned goals remain after this planning audit. Budget **12–18 additional
goals**, allowing three to nine bounded prerequisite repairs or necessary splits.
The earlier 20–35 estimate was not a mapped backlog and counted broad areas that
already have substantial implementation and evidence. It is superseded for this
explicit internal scope; it was not a reliable production-readiness estimate.

This is a planning allowance, not a completion promise or a calendar estimate.
An allocation-history explanation, concurrency defect or recovery repair can
outweigh several feature goals. After goals 1–3, reassess using observed costs
and failures; later discoveries update this same queue with their concrete reason.
Do not manufacture goals to use the allowance. Retire a planned item only when
its full outcome already has applicable evidence. Additional SQL functions,
operator families, optimizer rewrites and platform expansion enter only if this
workload demonstrates a prerequisite; they are not part of the countdown.

### Qualifications that remain outside this internal claim

The [combined allocation history](evidence.md#native-allocation-reuse) still
exposes a macOS usable-heap deficit while requested ownership is covered. The
independent native reduction demonstrates oversized block reuse but does not
identify the original query block or explain its entire deficit. The strict
combined-query diagnostic stays available and must not be described as passing.
This checkpoint does not claim arbitrary allocator-history, usable-heap or
whole-process/RSS bounds. Any newly uncovered violation of an advertised resource
contract still requires repair; naming a qualification gap is not a waiver.

Deterministic overlapping-reader schedules, effect injection and process cuts
already have evidence. General concurrency/race detection, ThreadSanitizer,
whole-system scheduling/fault simulation and uninstrumented native/runtime
internals remain unfinished. The [sanitizer record](evidence.md#platform-and-sanitizer-limitations)
identifies the exercised mutex/pathname scopes; no broader claim follows.

The [durability contract](../docs/transactions.md#platform-contract) remains
conditional on exact OS/filesystem/device premises. Broader Linux durability,
power-loss/device certification, additional filesystems and stable API/format
upgrade compatibility are not established by this queue. The tested host-shared
Linux database mount remains excluded; use native database storage and preserve
fail-closed identity checks. Reopen that sharing-layer investigation only when
new evidence can change its disposition. Windows implementation/runtime work
remains outside this checkpoint. These exclusions make this an internal learning
checkpoint, not fulfillment of README's complete production standard.

Partitioned hash aggregation, a shared buffer manager and parallel execution
remain alternatives requiring a demonstrated workload need, independent results,
resource costs and end-to-end measurements.

## Publication

Repository synchronization is permitted for reviewed, verified checkpoints.
Check the exact committed tree, destination and current remote state before
pushing. Use a fast-forward update when the remote history is an ancestor;
reconcile divergent work before deciding whether history replacement is needed.
Unfinished working-tree changes remain outside published checkpoints. Repository
synchronization does not establish release readiness or authorize a package release.
