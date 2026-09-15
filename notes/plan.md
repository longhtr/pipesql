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
records and 18 fresh scenarios per platform. Subsequent work adds timing diagnostics and full-synchronization Linux
verification. The strict-creation change below is undergoing platform verification.

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

## Full-synchronization Linux checkpoint

The [full-synchronization Linux gate](evidence.md#full-synchronization-linux-verification)
passed all 24 stages and four fresh storage workflows. The runner verifies the
host disk policy, native database placement, unprivileged identity, source
integrity, command failures and owned cleanup. Host interruption also passed its
cleanup control. The stronger storage policy required longer finite deadlines
for three storage-heavy stages; engine code and fault coverage are unchanged.

[Testing](../docs/testing.md#linux-verification-with-full-synchronization) owns the
reproduction command. Routine Docker checks remain available for development;
new persistence checkpoints need this full-storage path or an equivalent
established premise. Broader power-loss/device and platform qualifications below
remain unresolved. No additional verification framework is planned.

## Synchronization comparison

The [root-cause investigation](evidence.md#virtual-disk-synchronization-root-cause)
is complete. The observed Docker disk uses a weaker host synchronization policy
than native macOS PipeSQL. Changing only that policy in a controlled Linux VM
reproduced the slowdown with both raw writes and the stock catalog caller. The
[retained experiment](../tools/README.md#compare-virtual-disk-synchronization-guarantees)
and qualification contract now make that distinction explicit. That investigation
changed no engine or gate behavior; the completed analytical checkpoint stays
closed.

## Active strict creation validation

Separate validation of a fresh namespace from recovery of an existing database.
Creation already writes and synchronizes its initial files and child directories,
and now validates the exact initial state, database identity and held lease
without entering recovery or making its four additional synchronization calls.
The implementation is committed as `0f1e47b`. Keep the final database/parent barriers
and every repairing-reopen rule until independent evidence justifies otherwise.

Trace the creation and namespace owners, then challenge initial roots, fence,
control, directory contents and lease identity. Preserve independent decoders,
refusal order, cleanup under the lease and observable failures. Measure stock
creation before and after; reassess the design after 45 minutes of implementation
and focused verification. A local timing improvement cannot justify weaker
validation or durability.

A focused replacement-directory test exposed and now guards a cleanup defect.
Cleanup rechecks the held lease identity before deleting files. An identity
mismatch or inspection failure returns cleanup debt and preserves the files.
macOS verification covers all 24 stages across the initial run and a corrected
native-I/O continuation. Linux verification is running; fresh examples remain pending.

Use focused regressions and reusable build outputs during implementation. Keep
measurements small and tied to the proposed benefit. Run the required sequential
macOS and full-synchronization GNU arm64 Linux gates and affected fresh examples
once the persistence change is ready; repeat only for changed inputs or a
concrete unresolved failure. Record exact inputs and remaining platform
limits, remove owned artifacts, review and commit the verified change, and
synchronize by fast-forward. This is a creation-path improvement, not a new
explanation of the earlier platform timing discrepancy.

## Path to the internal 0.1.0 checkpoint

Finish strict creation validation first. The next checkpoint is a useful local
analytics workflow that can be followed without writing a Rust program: declare
tables, import typed data, query a report, understand its execution, export a
complete result and resolve an interrupted import. Extend the existing event
report instead of building another demonstration database. Code and explanations
must make the frontend, execution, resource and publication boundaries traceable.

“0.1.0” is an internal learning target. It does not change Cargo versions, promise
API or format stability, authorize package publication or establish production
readiness. This queue covers macOS and the qualified GNU arm64 Linux environment;
Windows and broader device/power-loss qualification remain outside it. README's
product requirements continue to govern every retained capability.

The planned order is below. Counts are estimates for bounded implementation goals,
not tasks to activate together. Scope each goal from the actual preceding result.

| Order | Outcome | Estimated goals | Completion evidence |
| --- | --- | --- | --- |
| 1 | Consolidate campaign build ownership and shorten test navigation. | 1 | The sequential gate builds one dedicated stock CLI/library set for compatible campaigns, while standalone runs retain fresh isolated builds. Keep observer builds, case expectations and failure interpretation local. Prove artifact/source checks, failure propagation and cleanup; compare coverage and total runtime. Reduce the test map to navigation and remove repeated contract prose. |
| 2 | Expose declared database creation, table declaration, schema inspection and logical plans through the stock CLI. | 1 | A fresh event-report schema can be created and inspected through existing library owners; invalid declarations retain typed failures and cause no publication. Reuse `PreparedQuery::logical_plan`; it reports logical structure, not runtime costs. Choose one explicit schema input format, with no parallel SQL frontend. |
| 3 | Import a documented CSV profile into a declared table with bounded streaming buffers. | 2 | One goal owns decoding, types, NULLs, quoting, limits and byte-offset errors; one integrates append, cancellation, transaction tokens, reopen and ambiguous-outcome resolution. Malformed input cannot become reported success. Independent fixtures and refusal cuts cover both layers. |
| 4 | Export typed query results in a documented machine-readable form. | 1 | A fresh report round-trips supported values, preserves NULL distinctions and requires successful query completion. File output has explicit completion/publication rules; stdout and sink failures cannot imply a complete file. Reuse the query cursor. |
| 5 | Add conditional report expressions required by the event workflow. | 1 | A bounded searched CASE profile classifies events with independently checked NULL, type and demanded-error semantics. Reuse the expression demand machinery and show skipped failing branches. Defer unrelated scalar functions. |
| 6 | Support a bounded partitioned, ordered reporting window. | 2 | First specify and implement partition/order/frame ownership over the shared sorting path; then complete the selected running-total use case with forced spill, peers, NULLs, numeric errors, cancellation and independent results. Do not claim the full window language. |
| 7 | Exercise the complete import/report/export workflow above memory limits with concurrent snapshot ownership. | 1 | Fresh small literal and scaled model results agree; append/reopen/reclaim and interrupted import remain coherent. Measure end-to-end work and repair only demonstrated bottlenecks or contract violations. |
| 8 | Consolidate the integrated capability and its learning path. | 1 | Both full platform gates and fresh workflows pass on identified inputs; interfaces, failure explanations and source maps match the implementation. Remove dead paths and duplicate prose found along those flows. Record remaining limits and stop this queue. |

This is **10 planned goals after the current goal**, with a working estimate of
**10–14** if the importer or window exposes prerequisite repairs. This is not a
calendar estimate. The range is uncertainty, not permission to invent extra
milestones. Reassess after import integration and after the first window prototype;
split a goal when ownership or failure boundaries warrant it. Remove a proposed
feature if the workflow no longer needs it, and explain the changed target.

The first goal after creation removes a concrete recurring cost: several native
campaigns rebuild the same stock library through `check_support.build_library`.
Use one dedicated immutable stock build within the sequential gate; keep test
binaries with different feature sets separate. Keep the existing gate and campaigns. Also run long checks from a frozen source
export so ordinary edits do not invalidate their inputs. Then design the smallest declared-schema CLI
interface around the event-report schema.
No buffer manager, parallel executor, format migration, distributed component or
general optimizer is scheduled without a measured need in this workflow.

Use the [checkpoint policy](../docs/verification.md#verification-checkpoints):
focused checks and warm targets during each goal; broad runs at integrated
capability boundaries. Import publication and any changed native/resource owner
still require both platforms' relevant full campaigns. Batch the portable report
features into one integration checkpoint when their dependencies permit it.
Each goal includes code, its explanation beside the owner and a working example.
Consolidate the affected area before adding another layer: one clear owner per
invariant, shared setup where it removes repetition, and independent expectations
where agreement matters. Remove dead branches, forwarding-only helpers and stale
instructions. A lower file count is useful only if readers can follow the flow.

Documentation must help a reader act, understand a design decision or find its
owner. Delete repeated claims and obvious code narration. Prefer one worked
example to a long list of assurances; keep exact contracts in their owning guide.
Test maps identify where to change a case instead of restating every test. The
final checkpoint reviews these properties across the completed workflow; cleanup
is part of each goal, not work deferred until the end.

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
