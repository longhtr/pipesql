# Engineering guide

Build a database whose answers, commit outcomes and resource limits remain
trustworthy under composition and failure, and whose code engineers can understand
and change. This guide owns working method, code review and evidence maintenance
for the DBMS, simulator, tests and tools. README owns product constraints and
decision order; the subsystem guides own their technical contracts.

The repository must also help readers learn how a DBMS works. Make the main
operations traceable through real code. Introduce necessary database concepts
and explain consequential invariants beside their owners. Connect walkthroughs
to working examples and implementation entry points; keep detailed language and
storage rules in their contract documents.

## Choose and finish work

Resume from the working tree, [current plan](../notes/plan.md) and relevant
evidence. Confirm facts that can change the next decision and finish useful work
already underway. Choose coherent changes by README's decision order and their
contribution to the intended analytical workload and release readiness.

Count runtime resources, code and state complexity, verification, maintenance,
engineering time, compute and displaced work. A smaller diff, more tests or a
local speedup does not by itself lower total cost. Research and process must
support a concrete capability, consequential uncertainty or recurring cost.

Before substantial work, identify the expected benefit, affected contracts,
cheapest useful falsifier and point at which to reassess. Use the existing plan
or result record when this needs to survive interruption. Set an explicit effort
budget for expensive or uncertain investigations; routine changes need neither a
timer nor a separate report. Reassess after a consequential result or an exhausted
budget. Stop optional exploration when another step cannot change the decision.
Complete the correctness and verification obligations of retained changes.

Every pre-release design can be reconsidered, including architecture, interfaces,
language, implementation and test machinery. Use source inspection, resource
arguments, independent models or representative experiments according to the cost
of being wrong. Repair a failed premise across its consumers instead of adding
exceptions. After making the design correct, look for a smaller state space.

A finished change includes implementation, applicable verification, current
contracts and useful retained evidence. A passing component test or partial
refactor is a checkpoint. Review the final diff and commit coherent verified work;
keep related repairs together when splitting them would leave invalid contracts.
The review description explains the problem, resulting behavior, consequential
tradeoffs, checks and unresolved limits. Feedback identifies a defect, risk or
cost and distinguishes it from a style preference. Resolve the concern or explain
the evidence for the alternative; agreement does not replace verification.

Keep active source in the primary checkout or durable project worktrees. Review
the destination and existing authorization before publication; do not overwrite
remote history to make a checkpoint fit.

## Find the governing contract

[README's repository map](../README.md#find-your-way-around) identifies the contract
owners; the [ownership map](source-map.md) locates implementation and
tests. Read the relevant owners before changing a subsystem. Distinguish current
behavior, target requirements and historical observations. Notes and experiments
cannot silently amend a contract.

If contracts conflict, stop the affected design, identify the failed premise and
retain a concrete counterexample. Examine neighboring dependencies and repair the
affected owners together. Preserve product constraints and explicit user limits;
never weaken a requirement or test to excuse a defect or claim completion.

## Review foundations and code

At each consequential boundary, establish the following facts and check that they
hold when composed:

- canonical representation, semantic units and valid identities;
- ownership, lifetime, mutation and effect authority;
- capacities, accounting, admission and release;
- states, transitions and a finite progress measure;
- success, expected failure, cancellation and cleanup;
- concurrency, locking, callbacks and blocking; and
- independent checks by the producer and consumer.

For PipeSQL, this means tracing column identity and demanded evaluation through
the binder, physical plan and every producer; admitting blocking minima before
optional growth; and following publication, recovery and reclamation through the
same commit and snapshot authorities. A local guarantee is insufficient if its
consumer can invalidate it. Resolve known violations and consequential unsupported
premises before extending the affected design.

Use direct Rust functions and concrete types. Keep call targets, mutable authority,
state transitions and lifetimes visible. Validating constructors and private
fields should exclude invalid states. Distinguish semantic identities from names
and positions, and distinguish counts, lengths, offsets and generations where
confusion can change meaning or ownership.

Split functions and modules by invariant, phase or effect authority. Large files
warrant inspection of those boundaries, not automatic splitting. Add a crate,
trait, callback, macro, runtime, generator or other abstraction only when a
concrete need repays its ownership and verification cost. Unsafe containment or
independent verification can justify a crate. Remove dead paths, placeholders,
speculative interfaces and duplicate semantic or durability implementations.

Avoid input-controlled recursion, including teardown. Each loop traverses a
validated finite range, respects a named capacity or advances a finite decreasing
measure. Bound bytes as well as rows. Check arithmetic before allocation,
narrowing, indexing, seeking or mutation. Avoid silent saturation, lossy fallback,
ambient mutable globals and hidden allocation. Reuse bounded batches and avoid
allocation in row hot loops. [Resources](resources.md) owns the reservation,
fallible allocation, ownership transfer and physical-release protocol.

Return typed outcomes for invalid input, exhaustion, contention, cancellation,
I/O and corruption. Preserve the distinction between definite abort, ambiguous
commit and failed cleanup through every caller. Handle each fallible result or
justify why it is irrelevant. Assertions protect programmer invariants rather
than reject expected external failures. Keep them side-effect-free, near the
transition and active where release safety requires them. Contain unexpected
internal failure according to every mutable owner that may have been touched.

Comments explain invariants, units, ownership and non-obvious reasons. Avoid
narrating syntax or retaining a development diary in code. Public documentation
explains behavior, limits and failure outcomes.

Separate type definitions and implementation blocks with a blank line. Keep
attributes and documentation attached to the declaration they describe.

Review readability separately from correctness. Follow one normal operation and
one consequential failure in the final code. A reader should be able to explain
the decisions and ownership without reconstructing a web of unrelated helpers.
Extract functions around meaningful operations; grouping locals into a context
is useful only when it clarifies a coherent owner. Avoid replacing a large
function with many wrappers or a context that grants every helper every authority.

Useful primary references are Google's
[code review guidance](https://google.github.io/eng-practices/review/reviewer/looking-for.html)
and the Rust API Guidelines on
[type safety](https://rust-lang.github.io/api-guidelines/type-safety.html) and
[validation](https://rust-lang.github.io/api-guidelines/dependability.html).
Use their reasoning to assess concrete choices; another project's architecture
or style is not a substitute for this project's contracts.

## Rust and dependencies

Use the pinned stable Rust toolchain and ordinary formatting, warnings and Clippy
gates. Prefer standard-library mechanisms when they are the safer complete choice.
A dependency must lower total semantic, security, allocation, panic, I/O, build,
supply-chain and maintenance risk. Preserve offline builds from pinned inputs,
licenses and attribution. Generated behavior retains its generator and immutable
inputs and reproduces byte-for-byte.

Avoid unsafe code. Confine required OS, SIMD or FFI operations behind small safe
interfaces with documented preconditions and provenance. Apply Miri, sanitizers
and target-specific checks where they exercise that boundary. Rust layout,
allocator values, panics and implicit pointer lifetimes are not a stable ABI.
[Architecture](architecture.md#filesystem-effect-boundary) owns the current
unsafe boundary; [verification](verification.md) owns platform qualification.

## Verify and measure

[Verification](verification.md) owns required evidence and permitted claims.
Exercise observable behavior through public queries and APIs using the production
path. Use component checks for internal invariants, fault cuts and transitions.
A bug fix needs a durable regression that exposes the defect. Include the relevant
composition, resource pressure, cancellation, failure and cleanup behavior. Check
that intended tests actually ran; keep known regressions in the ordinary gate.

Independent evidence needs a different method or provenance, not merely another
file implementing the same premise. Apply code review to generators, oracles,
simulators and result parsers. Use negative controls to show that they detect
wrong answers, omitted states and failed runs. Simulation controls effects around
production transitions; abstract models challenge representations and assumptions.
Neither substitutes for the other's coverage or for stock platform evidence.

Run focused checks during development and the complete verification required for
the retained change. Repeat or broaden checks after changed inputs, failures or
unresolved risks; unchanged green runs add no coverage. Documentation-only changes
need factual accuracy, contract preservation, links and affected examples checked.
Execute changed commands or examples. Compare exact build/gate inputs before
reusing runtime evidence; editorial changes alone need no artificial runtime tests.

Use the [ordinary build and gate](../README.md#build-test-and-use) before adding a
runner. Freeze build/gate inputs during a complete run and compare source
manifests before and after. Do not rebuild a shared target while a runner can
still start its executables. Independent runs need isolated artifacts and owned
processes, outputs and cleanup. Preserve failure status and enough case context
to diagnose a failure. The sequential gate's cost and current limitations belong
to the verification contract and its evidence, not another workflow framework.

Before performance work, sketch bytes, allocations, operations and the expected
bottleneck. Measure optimized stock artifacts end to end on representative,
identical data and hardware, checking results in the same run. Include forced
spill and concurrent ownership where relevant. Microbenchmarks explain a cost;
they do not establish a workload improvement. Process timing and peak RSS do not
bound a process tree or reconcile logical memory with physical allocation.
Retain regressions and their disposition. Stop optional tuning without a useful
hypothesis; deferral does not qualify performance.

## Maintain documentation and evidence

Give each rule one authoritative owner and link to it instead of copying it.
Keep contracts, usage, evidence and current work distinct. Put important reasons
where maintainers need them to change the system safely. Consolidate continuously;
the [production audit](verification.md#production-consolidation-gate) checks the
result rather than postponing cleanup until release.

Keep the work plan to the current outcome, completion criteria, decisions, gaps
and next actions. Replace settled entries. If interrupted, record exact partial
state, live work and the next command or action. Reports distinguish verified
outcomes from incomplete implementation and untested claims.

Retain artifacts for a named consumer or decision:

- Keep maintained models, generators, fixtures and regressions under test/tool
  ownership, including necessary licenses and input provenance.
- Keep the minimum usable evidence for current claims, consequential decisions
  and unresolved failures. Record immutable source, inputs, environment, commands,
  outcomes and limitations as required by verification. Consolidate repeated logs
  without losing case identity, failure status or the meaning of empty output.
- Prefer an available Git revision to a duplicate source archive when it restores
  the exact tested state. Required uncommitted variants need a color-free binary
  patch, including new files, verified against its base and expected inputs.
  Hashes identify missing bytes but cannot reconstruct them.
- Replay commands must identify their inputs and write to a fresh output directory
  without overwriting retained observations. Document generated inputs and their
  recreation cost. An ignored or temporary file is not durable evidence.
- Before moving or deleting material, check callers, links, licenses, manifests
  and reproduction dependencies. Preserve still-required facts and falsifiers,
  then verify affected consumers. Group related dispositions; do not build a new
  administrative record for every obsolete log.
- Retire obsolete experiments and duplicates once their useful conclusions have
  a surviving owner. Repair unusable replay only when a current claim, decision
  or regression needs it. Otherwise withdraw the replay claim and retire the
  package; reconstructing every historical experiment is not a release obligation.
  Preserve historical observations as historical, including their limitations.

Use descriptive work names and remove redundant branches after integration.
Repository content, ignore rules and commit messages remain free of private
instructions, contributor-specific tooling and development telemetry. These rules
do not authorize rewriting published history. Repair the existing owner rather
than adding a policy document for an isolated mistake.
