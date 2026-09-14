# Work plan

[README](../README.md) owns product requirements and decision order;
[Engineering](../docs/engineering.md) owns working method. This plan owns the
unfinished queue and publication restrictions. [Evidence](evidence.md) owns
completed investigations, verification results and consequential limitations.

## Current baseline

The finite internal analytical learning checkpoint is complete. All nine planned
outcomes are verified; no checkpoint goal remains. The final tested source is
`081b1f8`. Its [complete verification record](evidence.md#final-internal-analytical-checkpoint)
covers matching 24-stage macOS/GNU arm64 Linux gates, 687 ordinary Rust tests per
platform, 103 tooling tests, 44 codec fixtures, 24 semantic cases, 350 composition
records and 18 fresh scenarios per platform. Engine sources remain at that
checkpoint; subsequent work adds timing diagnostics and clarifies qualification.

The [event-report lesson](../docs/event-report.md) is the entry point: build typed
events and dimensions, append, reopen, report, retain an older snapshot, compare
answers, reclaim and understand interrupted publication. The small literal
fixture and scaled independent model connect query semantics to ownership and
persistent effects. Allocation histories exposed one constructor-failure defect;
[keeping the reservation owner intact](evidence.md#composed-report-allocation-histories)
through fallible allocation repaired it without changing an allowance. The other
checkpoint outcomes required no engine repair.

The [language manifest](../docs/language.md#current-public-query-manifest),
[public interfaces](../docs/interfaces.md), [test map](../tests/README.md) and
[tool map](../tools/README.md) own exact capabilities and verification entry points.
The completed queue and its acceptance criteria remain in Git revision `081b1f8`;
[evidence](evidence.md#final-internal-analytical-checkpoint) links the outcomes.
Earlier language, tooling, append, reader and grouped-allocation work remains
closed unless a concrete defect or new workload changes its scope. No historical
archive is required.

## Synchronization comparison

The [root-cause investigation](evidence.md#virtual-disk-synchronization-root-cause)
is complete. The observed Docker disk uses a weaker host synchronization policy
than native macOS PipeSQL. Changing only that policy in a controlled Linux VM
reproduced the slowdown with both raw writes and the stock catalog caller. The
[retained experiment](../tools/README.md#compare-virtual-disk-synchronization-guarantees)
and qualification contract now make that distinction explicit. No engine or full
gate behavior changed; the completed analytical checkpoint stays closed.

Fresh creation's four recovery calls remain an optimization candidate, not an
explanation of the platform gap or an activated implementation milestone. Any
future change needs strict initial-state and lease validation, an end-to-end
benefit, and persistent-boundary qualification. There is no remaining task in the
synchronization investigation. Broader qualifications below remain unresolved.

## Internal analytical learning checkpoint

This is the internal checkpoint informally called “0.0.1 level.” It does not
change the package version, activate publication, stabilize an API or format,
or certify the [production release criteria](../docs/verification.md#release-criteria).
It covers macOS and GNU arm64 Linux on the exercised native database paths.
Windows remains an unfinished product obligation outside this checkpoint.

The finite queue is finished. Earlier goal estimates and contingency allowances
are retired; they are not additional work to manufacture. A future milestone
requires a concrete learning outcome or workload need, selected using README's
decision order and the limitations below. Extra SQL functions, optimizer rewrites
and platform expansion are not outstanding requirements of this checkpoint.

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
