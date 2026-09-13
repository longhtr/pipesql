# Verification evidence and open limitations

This record identifies checked behavior and consequential limitations. Product
and implementation contracts live in [docs](../docs/README.md); current work
lives in [the plan](plan.md). Maintained fixtures and callers provide replay inputs.
No build, test, or investigation below requires a retired project checkout.

## Full verification checkpoint

Both complete 24-stage gates verify the 690 frozen inputs retained in `2d41739`
on macOS arm64 Darwin 25.6.0 and GNU arm64 Linux 7.0.12-linuxkit. Both use Rust
1.98.1, release artifacts, locked offline builds and warnings-denied compilation
and documentation. Linux uses uid/gid 1000, glibc 2.36 and native overlay storage
with read-only source. Input manifests match before/after and across gates:
`5ed9054ba12166782a615e93c2d88a2fe6191e965e7d6d24534027ae1a9220c6`.
Only the two notes files change during finalization. The other 688 inputs retain
fingerprint `963a85a7d3a4d6e4eb855428296717a95856bc9cd3da805c365c0b67370b93bc`;
all inputs remain tracked. Final documentation verification passes 557 local links.

Each platform executes 583 ordinary Rust tests, including all 113 public catalog
tests and all six new COALESCE tests, plus the separate lease subprocess.
No ordinary test is ignored or filtered; the selected lease child reports six
filtered siblings. Maintenance passes 96 tooling tests, 44 independent codec
fixtures and 554 local links. Independent aggregate semantics pass 24 cases and
composition passes 311 cases. The Rust join corpus separately checks 648 cases.
Both allocation campaigns retain positions 0–984 and healthy control 985 at each
pathname length; the ordered lists were reconciled explicitly. The caller ceiling
remains 1,000. Native initialization passes 30 macOS and 80 GNU/Linux cells;
synchronization passes 241 cells and I/O passes 1,196 cells per platform.
Interruption checks retain 76 append cuts, 46 recovery cuts and 249 independent
graph checks. All 43 graph cases, two oracle controls, three CLI limits, genesis,
lease contention and independent column order pass. Linux retains the two Darwin
ACL exclusions.

Both receipts have zero finalization errors. Stage times total 1,967.956 seconds
on macOS and 1,339.058 seconds on Linux. Receipt SHA-256 values are respectively
`3eab12523340abcfeacb40649576c88a31f04c3819c7e505bb849edcc477c282` and
`776593688463c0186df4e95f2481798dae78ed2a69e36e823b8fb228cf08ffbc`.
Overlapping verification runs are not performance benchmarks. Resource sampling
observed normal/warning host memory pressure and 1,650–2,093 MiB of swap use.
A compiler burst used about nine container CPU cores and 1.76 GiB; the container
was subsequently capped at two CPUs. Container network traffic remained about
2 kB. These host observations do not qualify engine physical-memory bounds.
Owned gate/control outputs, source exports, logs, example databases and containers
are removed. The existing verification image and toolchains remain. Windows,
broader durability, physical-memory and sanitizer qualification remain unfinished.

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
runs supplies complete-gate evidence; the matching full passes above supersede
them. An initial native fixture used a function on WHERE's unsupported left side;
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

Maintenance passes 96 tooling tests and 44 independent codec fixtures. Final
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

The [constant tutorial](../docs/getting-started.md#add-constant-labels-and-dates)
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

The [tutorial](../docs/getting-started.md#filter-by-membership) runs from fresh
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
