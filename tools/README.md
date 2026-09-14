# Development tools

These are maintained development commands, not additional product interfaces.
[Build and test](../docs/testing.md) owns setup and workflow.
[Verification](../docs/verification.md) defines their evidence boundaries.
Python tools use the standard library, require Python 3.11 or newer, and need
assertions enabled.

## Entry points

| Command | Inputs and output | Executes engine code? |
| --- | --- | --- |
| `python3 tools/check-maintenance.py` | Current guides, Python sources, and tooling unit fixtures; reports failures. | No |
| `sh tools/check.sh` | Sequential gate with fresh outputs, source comparison, stage logs, and a result receipt. Accepts `--output` and `--scope`. | Yes |
| `python3 tools/check-docs.py` | Current guides; checks relative links and Markdown anchors. | No |
| `python3 tools/source-manifest.py` | Build/gate source trees; prints SHA-256 identities and checks Rust file inclusions. | No |
| `python3 tools/check-fixtures.py` | Independent encoders and retained bytes; refuses disagreement. Does not update fixtures. | No |
| `python3 tools/catalog_graph.py --help` | Usage for the bounded, read-only persisted graph inspector. | No |

The graph inspector reads a quiescent copied database. Its budgets, outputs,
refusals, and limitations are specified in
[Independent catalog inspection](../docs/verification.md#independent-catalog-inspection).
It is not a backup, repair, or online-consistency interface. Follow
[Inspect a persisted catalog](../docs/testing.md#inspect-a-persisted-catalog) for
the command and output ownership.

## Models and semantic oracles

| Owner | Responsibility |
| --- | --- |
| [models/attempt_identity.py](models/attempt_identity.py) | Bounded issuance-prefix/success-index model against explicit receipts. |
| [models/attempt_publication.py](models/attempt_publication.py) | Root/fence representation model under stated persistence premises. |
| `aggregate-rounding-vectors.py` | Independent rational rounding vectors; `--check` compares retained inputs. |
| `check-aggregate-semantics.py` | Public stock CLI aggregate boundary campaign. |
| `check-composable-aggregates.py` | Independent public composition corpus through the stock CLI: legacy aggregates and positional unions over the retained catalog fixture. |
| `test-q1-compare.py` | Regression tests for strict typed output parsing and comparison. |

The models do not execute the engine or prove filesystem persistence. Their
premises remain in their module documentation. Keep models and independent
oracles separate from production semantic helpers.

## Change a campaign case

Keep each case's input, expected outcome, and failure context together. The main
campaign functions show execution order; shared subprocess helpers do not decide
whether a database result is correct.

| Campaign | Where to change a case |
| --- | --- |
| Composition | The `check_*` families separate grouping, expressions, dates, numeric failures, derived queries, predicates, repeated aggregation, demand, and corrupt storage. `expected_groups` calculates independent expectations; `QueryChecks` executes and records results. |
| Library allocation | `allocation_cells` selects operations and path bounds; `required_outcomes` lists required observations. `check_allocation_prefixes` runs the healthy prefix before refusal positions. `check_ownership` owns composed-reader/writer controls. In `catalog-allocation.rs`, `check_healed_rows` verifies all persisted fields and reusable scratch after reopen; the armed query sequence (including typed projection constants) and same-handle debt checks precede it. |
| CLI allocation | `parser_cases`, `check_native_capture`, `check_publication`, `check_operations`, and `check_output_sinks` own distinct scenarios. `DatabaseFixtures` names the input and baseline databases; `heal_ambiguous` verifies retained tokens and retry outcomes. |
| Catalog graph | `reference_mutations` lists re-anchored corruptions; `check_namespace_and_authority` covers roots and filesystem ownership. `GraphChecks.case` copies the seed, applies a mutation, checks independent inspection, and optionally confirms public rejection. |

The graph caller separates fixture creation, receipt checks, and typed-row checks.
Keep its expected values independent of fixture construction. Native observers
must retain allocation, fault-arm, syscall, and cleanup order when edited.
`test-campaign-oracles.py` challenges extracted interpretation with wrong rows,
incomplete output, failed processes, and missing allocation coverage. Run it
through the maintenance check, then run the affected stock campaigns.

## Fixture encoders

`check-fixtures.py` is the supported comparison entry point. It calls the current
snapshot and catalog encoders and retained old-format oracles. Comparison does
not overwrite fixtures.

| Encoders | Bytes owned |
| --- | --- |
| `snapshot-fixtures.py` | Current legacy vectors and the common snapshot writer for both semantic campaigns. Expected query results stay in each campaign. |
| `candidate-fixtures.py` | Retired/rejected multi-table vectors. |
| `catalog_fixtures.py` | Schemas, catalogs, typed units, table indexes, and namespace roots. All 13 vectors are compared by the fixture gate. |

To generate catalog vectors for inspection, run `python3 -B
tools/catalog_fixtures.py /absolute/new-vector-directory`. The destination must
not exist; the command does not overwrite committed fixtures. Remove that owned
directory after comparison. `test-fixtures.py` checks exact generation, output
refusal, and that damage to each catalog/schema vector fails the ordinary check.

The independent legacy encoders and Q1 comparator live in
[`oracles/`](oracles/README.md). Historical manifests must be replayed at their
recorded revision after a path move.

## Native and allocation callers

| Runner | Owned boundary |
| --- | --- |
| `check-filesystem-abi.py` | Native SDK/decoder agreement and independent pthread extent/minimum controls. |
| `check-diagnostic-allocation.py` | Allocation-capacity preflight and public library construction, errors, queries, catalog recovery, and composed ownership under allocator refusal. |
| `check-cli-allocation.py` | CLI startup, parsing, output, publication tokens, and allocation refusal. |
| `check-native-initialization.py` | Darwin root-stat and Linux lstat/readlink observation, refusal, and overlapping callers. |
| `check-native-sync.py` | Linked synchronization calls, refusal, and healed outcomes. |
| `check-native-io.py` | Linked byte-I/O calls, partial progress, refusal, and healed outcomes. |
| `check-native-sanitizer.py` | Explicit nightly AddressSanitizer controls and mutex/default or pathname tests, compared with stock and uninstrumented nightly builds. |
| `check-catalog-interruption.py` | Stock catalog append/recovery process-termination cuts. |
| `check-catalog-graph.py` | Independent persisted-graph checks and negative controls. |
| [Native allocation reuse](#isolate-native-allocation-reuse) | Optional C control separating allocator reuse from engine accounting. |

These runners compile and execute code. C/Rust callers live in
`fixtures/`; they are development scaffolding with their own unsafe and process
ownership, not shipped adapters.

Every diagnostic-allocation selection first runs
[`allocation-capacity.rs`](fixtures/allocation-capacity.rs). It includes the
production resource helper directly, using the same allocator observer as the
public rlib probes. Three oversized requests must refuse before any allocation
attempt, including requested-byte overflow. Empty, exact/spare-capacity and actual
allocator-refusal controls distinguish preflight refusal from a disabled observer
or unconditional rejection. Reservations remain live until returned vectors are
dropped. This checks an internal capacity boundary; public query probes continue
to use the stock rlib.

Use `python3 -B tools/check-diagnostic-allocation.py --ownership-only` to
reconcile prepared queries, parked readers, and an append. The
[resource equations](../docs/resources.md#interpret-composed-memory-observations)
explain logical charges, requested/usable bytes, caller storage, and observer
limits. It checks reader usable extents against admission, including one- and
64-column INT64, DOUBLE, DATE and STRING ORDER BY/DISTINCT cases at short and
384-byte database pathnames. STRING covers empty/short Unicode and 65,536-byte
values. Each case checks complete nullable results, duplicate counts and final
release. GNU/Linux additionally runs fresh callers with fixed 128-KiB and 64-MiB
mmap thresholds, with observed allocator controls before the reader cases.
Legacy lineitem cases check one/64-column fixed keys, empty and 32-byte UTF-8
constants over two full batches, and global/grouped text extrema. They reconcile
requested allocations with independently derived scan and aggregate charges at
admission and output, then check complete release. A wrong attribution term must
fail. Both pathname lengths run through the same selection.
Prepared aggregate cases cover widths 1–10 and four partitions of the ten-entry
budget across repeated stages. Widths 11–64 must reject and release preparation
ownership. The caller checks each retained-vector allowance independently,
executes COUNT results over 512 source rows, and rejects a wrong allowance.
Analytic count cases cover empty input, one/nineteen counts, typed rows, 64 output
columns, consecutive analytic stages and grouped composition. Twenty repeated
calls must reject at the token bound without retaining preparation ownership.
The caller samples requested/usable admission after every step and independently
reconciles nonheap allowances at admission, first spill, emission and completion.
Count-only cases require zero temporary consumption; typed input and the second
stage of the consecutive-count case retain spill coverage. Both pathname lengths
run all eleven cases; a one-byte attribution error must fail.
Composed-reader bound failures are reported after the
barrier participants join. This selection also checks the complete append
allocation-size ranges and full-width maximum-column growth, reuse, publication,
and release. Its controls
reject a missing rounding ceiling, a wrong result, and wrong attribution.
The selection also observes 514 large blocking-buffer capacities and 91
power-of-two hash layouts for the GROUPED caller; usable extents and final
release are measured independently of the engine's sizing functions.
[`grouping-ownership.rs`](fixtures/grouping-ownership.rs) adds 40 sequential public
cases at 4 MB and 16 MB: one/three/five/seven/nine states for floating sums,
integer sums, integer minima, and mixed layouts. It retains allocator reuse
between queries, checks every row against the literal input values, compares the
complete prepared/result owner with requested and usable extents, and checks
release. A fresh size census alone does not establish these history-dependent
observations.

The catalog allocation campaign includes analytic count followed by ordering and
aggregation, with an independent total of 16 for its four-row input. A separate
count-only query retains the same total and exercises the counter's allocations. The native
I/O composition campaign also consumes analytic count after a LEFT JOIN. Its
right input retains only key 1, so two left groups survive but only the matched
group contributes to AVG; the expected result is 60. The catalog allocation
campaign's derived LEFT JOIN retains four matching pairs and one unmatched row,
with an expected count of five. The separate inner-join ordering query retains
its eight-pair check. Its negated NOT IN list retains the same NULL-aware
membership result. Native I/O selects the right key with NOT BETWEEN, retaining
the expected mean of 60. Native I/O checks a total of three after SIGN of each
positive DIV/MOD quotient over three count-only rows. Its ABS/division aggregate
remains 4.5; a separate FLOOR/CEIL/ROUND aggregate returns nine across the same three
count-only rows after SQRT(n*n) recovers each count of three.
EXP(LN(n/n))-1+LOG10(n/n) contributes zero. Catalog allocation phases
separately prepare, execute and consume a nullable division/filter query with
count two after ABS of the negated ratio and SQRT of ROUND/CEIL/SIGN of an exact
oddness check on INT64 amounts above 2^53. Each unary result remains one. The filter retains the exact ratio check.
DIV by one must preserve the first amount exactly before filtering. COALESCE must select that exact value without evaluating its failing
fallback. A demanded SAFE_DIVIDE result must be NULL without losing its row;
a second COALESCE evaluates NULLIF of the original ratio and the NULL result.
NULLIF retains that ratio; FLOOR rounds 1.75 down to one, LN returns zero,
EXP restores one and LOG10(1) contributes zero.
IS NOT DISTINCT FROM NULL keeps the missing-value rows in the catalog query. Native I/O uses IS NOT DISTINCT FROM zero to select
the defaults, then checks that NULLIF converts three
COALESCE defaults back to NULL, so COUNT returns zero; an outer COALESCE skips
a failing fallback after the count. These queries retain the existing allocation
and I/O schedules while checking both NULLIF decisions.
Fixed-buffer diagnostic controls render division-by-zero, square-root, natural-logarithm
and base-ten-logarithm domain errors, exponential overflow and captured causes under
allocation denial. These retain refusal, recovery and
healthy-reuse checks around the full sequence.

INTERSECT uses the same descriptor allocation, two sorted-input constructors,
scratch files and read/write owners as EXCEPT. The existing allocation-prefix
and native-I/O campaigns continue to exercise those owners. Both operations run
the internal exact/short admission, cancellation, reader corruption and replay
schedules. The independent analytic ownership campaign adds INTERSECT with
256 shared rows and checks actual heap attribution and release at every step;
its expected rows do not call production set comparison. The ALL controls use
unequal duplicate counts and require 768 rows apiece after difference or
intersection, including per-step ownership and terminal release. Both ALL forms
also run the common internal failure schedules. They allocate no new merge
storage; the public prefix and native-I/O sweeps retain the shared constructors
and file effects rather than duplicating the same schedules per quantifier.

EXCEPT coverage compares complete rows in both native-I/O and allocation
campaigns. The native input retains only key 2, whose amount is 90. Allocation
refusal removes the NULL-note amounts and retains one distinct named amount.
The analytic ownership campaign also consumes a typed EXCEPT result through
window count, checking all 256 surviving rows and requested/usable charges at
every returned step. The catalog work ceiling is 1,100 allocation prefixes;
the EXCEPT healthy census observed 1,056.
This ceiling bounds campaign work and does not change engine admission.

The ownership selection also runs `wide_set_shapes` in
[`composed-ownership.rs`](fixtures/composed-ownership.rs) at short and 384-byte
paths. A two-column left source repeats one nullable STRING across 61 positions;
a declared 62-column right source supplies each position separately. Together
they reach the 64-source-column bound without exceeding the query token bound.
All six UNION/EXCEPT/INTERSECT forms check literal row-id sequences, every STRING
position and final release. NULL, empty, embedded-NUL UTF-8 and 65,536-byte cells
exercise different record and output extents. The caller samples requested and
usable allocations against prepared/result charges after execute and every step.
UNION ALL must use no temporary bytes; the sorted forms must use external storage.
A nonexistent measured owner must fail the usable-byte attribution guard.

`wide_left_join_shape` uses the same caller and both pathname lengths. Three
left fields and 61 right fields produce 64 columns. Six rows per side include
unequal duplicate groups, unmatched left keys and NULL keys. A literal 11-pair
oracle checks every field without assuming equal-key order, including nullable,
empty, embedded-NUL UTF-8 and 65,536-byte STRING values. The caller requires
external storage and checks requested/usable charges through execute, every step,
Finished and release. The existing false-attribution mechanism must reject a
nonexistent owner after the complete rows and release have been checked.

[`transient-ownership.rs`](fixtures/transient-ownership.rs) additionally arms a
borrowed, thread-local observer inside this workload's preparation, execute,
step and release calls.
The allocator samples live requested/usable increments after allocation and
before physical free against the current database charge. Caller setup, row
checks and reporting run outside the scope. The caller prints event counts and
minimum requested/usable headroom, requires both event types and nonnegative
headroom, and preserves the independent checkpoint equations. Calibration
detects an uncharged 65,536-byte allocation created and freed within one call
despite unchanged entry/exit counters; a second case observes only its free and
must still detect the live owner. `wide-left-join-observer-negative` disables
calibration observation and must fail calibration. Separate phase samples require
preparation allocation/free events and prepared-plan free events. Two repetitions
abandon the result immediately after execute and after Progress with live
temporary storage; each requires observed frees and full heap, descriptor and
reservation restoration. A finished result has already released its heap inside
step, so its drop is checked for charge release without requiring heap events.
`wide-left-join-lifecycle-negative` leaves preparation unobserved and must fail
phase coverage; the supervisor also rejects missing lifecycle completion output.
These checks cover the exercised single-threaded Rust allocation events, excluding
foreign allocations, allocator metadata/retained pages, other process mappings
and RSS.

Before that complete join, `check_failed_join_preparation` reuses the same
allocator harness to census preparation alone and sweep every refused prefix,
including zero and a healthy full-prefix control. A 32-allocation caller work
ceiling bounds the sweep; it is not an engine admission allowance. Heap and
reservation counters must return to baseline while refusal remains armed. Faults
and their census are then suspended for caller descriptor enumeration and
reporting, without another engine operation. Each returned
error stays live while heap, descriptors and reservations are checked, followed
by error release. A late constant EXP error checks partial-plan cleanup and its
owned span after the caller's query text is freed. Prefix zero reports no sample;
other failed calls must observe all successful allocations and their frees.
`wide-left-join-failure-negative` suppresses one failed-call observation and must
fail event coverage. The supervisor independently rejects incomplete, duplicated
or reordered prefix traces and requires the final healthy control.

`check_failed_join_construction` runs in a separate fresh
`wide-left-join-construction` caller at both pathname lengths. The prepared query
is fixed and excluded from the observer's baseline. A healthy execute-only census
sets the complete prefix sweep, bounded by 512 allocations of campaign work.
Refusal stays armed through heap/charge reconciliation and fixed-buffer error
formatting. Descriptor checks follow with the error live. Every successful
allocation in a refused prefix must have an observed free and nonnegative
headroom. The full-prefix result reconciles its public charge, and the complete
11-pair join and lifecycle checks follow the sweep. The
`wide-left-join-construction-negative` mode omits prefix-one observation; the
supervisor must reject it and incomplete or invalid prefix records.

The raw caller also retains `wide-left-join-construction-sequence`, which follows
the complete sweep with the changed-width demanded-expression histories. This is
an unresolved native usable-heap diagnostic, excluded from passing fresh-cell
qualification. It uses the same strict guards; no deficit is suppressed. To replay
it, build the ordinary disposable caller with the supervisor's existing helper:

```sh
python3 -B - <<'PY'
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile

sys.path.insert(0, "tools")
allocation = runpy.run_path("tools/check-diagnostic-allocation.py")
with tempfile.TemporaryDirectory(prefix="pipesql-construction-sequence-") as name:
    work = Path(name).resolve()
    allocation["build_driver"](work)
    subprocess.run([str(work / "driver"), str(work / "history"),
                    "wide-left-join-construction-sequence"], check=True, timeout=20)
PY
```

A nonzero result is a diagnostic failure, not a passing gate. The
[evidence](../notes/evidence.md#failed-wide-join-construction) records the observed
Darwin extent and scope of the repair.

`check_failed_join_execution` replaces the unique `text60` output with a
numeric expression, retaining 64 columns and the other wide nullable STRINGs.
Three expressions demand the same late LOG10 domain error directly, through a
SAFE_DIVIDE argument and through a COALESCE fallback. Query text is freed before
execution. A fresh observer for each step prevents construction events from
standing in for failure coverage. Each error must follow live external work and
free runtime owners in the failing call with nonnegative requested/usable headroom.
Heap and descriptors already match the prepared baseline while the failed result
remains live; its charge equals its inline handle size. Repeated failure has no
heap events. Error extraction releases the handle charge, and prepared-query
release restores the original baseline while the error remains live.
`wide-left-join-execution-negative` observes construction but omits step
observations; the missing failure frees must reject it. The supervisor requires
all three complete histories, positive external storage and observed frees, valid
step counts and nonnegative headroom. The complete literal healthy join follows
these errors. This is selected demanded-error coverage, not an exhaustive
execution-allocation sweep.

The same selection runs the nullable self-join, aggregation, and ordering workload
at 2.2 MB and 12 MB. `joined_shapes` in
[`composed-ownership.rs`](fixtures/composed-ownership.rs) checks all 4,096 descending
groups, NULL counts and sums, and final heap/descriptor/reservation release.
It samples requested and usable allocations after execute and every public step,
including Finished, against the current prepared/result charge. A nonexistent
owner in the negative control must fail the same usable-byte guard after full
rows and release. The subprocess deadline bounds completion; the sample count
is not capped to the smaller grouping workload's step count. These stable
boundaries do not measure allocations made and freed inside a step or qualify
other schemas, allocator histories, or RSS.

Catalog controls also check the combined GROUPED prepared-query/result owner
against requested and allocator-usable bytes while preserving complete nullable
and extrema results. `--catalog-only --controls-only` reproduces those healthy
observations; it does not execute the allocation-refusal prefixes. The default
gate retains both pathname lengths and the complete refusal campaign.

On GNU/Linux, `check-diagnostic-allocation.py --pathname-only` selects expanded-path
create/open allocation refusals, released-storage checks, successful retries,
and the common mutex control. It does not qualify the other allocation cells.

The [sanitizer command](../docs/testing.md#qualify-native-sanitizer-observations)
requires an installed nightly and a new absolute output directory. It is separate
from the pinned-toolchain gate. It retains source manifests, runtime/artifact
identities, logs, and a result receipt while removing owned build outputs.

The graph runner's `--seed-only --output /absolute/new-directory` builds the stock
caller and independently inspects its seed, then stops before corruption cases.
It reports source, caller, and library hashes and retains the caller for replay.
This option does not establish a passing graph campaign.
[Filesystem identity diagnosis](../docs/testing.md#diagnose-filesystem-identity)
uses that caller and the GNU/Linux `filesystem-identity.c` observer. Its separate
control checks stable and deliberately replaced files; the observer adds metadata
queries and is not a concurrency oracle or a production adapter.

Campaign work starts at the command entry point. Importing a checker does not
start builds or create campaign output. `--help` works before platform checks or
campaign setup, including when Unix-only Python modules are unavailable. Native
observers refuse unsupported platforms with a failing status; skipped observation
is not success. Assertion-based campaigns also refuse optimized Python.
Allocation and native-I/O focus options are mutually exclusive; `--controls-only`
selects a census and cannot establish a passing refusal sweep. Entry-point tests
exercise these boundaries with process launches and output creation forbidden.

`check_support.py` owns isolated locked/offline stock builds, exact dependency
selection, and explicit C/Rust linking. It reserves fresh target/output paths and
rejects missing, empty, ambiguous, or unusable artifacts. Existing outputs,
including dangling symlinks, are never silently overwritten. Callers retain their
case generation, fault schedules, oracles, cleanup, and claim boundaries.
`native_library` and `observer_environment` select the native library format and
loader variable. `fixtures/native-interpose.h` owns dyld registration or ELF
symbol forwarding; each observer retains its operation selection and fault state. Unit tests inspect command construction
and failure propagation without running the engine.

Gate and graph/interruption receipts accept frozen source exports without Git
metadata. `source_revision` records an available parent commit or JSON `null`;
the checked source manifest remains the input identity. Revision lookup is bounded,
and timeout, permission, or signal failures still propagate.

`check_process.py` owns native campaign subprocesses on macOS and Linux. Commands
have an absolute working directory and a finite timeout. Each process group is
closed on completion, failure, timeout, or handled interruption; outer runners
allow more termination time than their nested checkers before forced cleanup.
Interactive callers must
also bound pipe reads. Children must remain in the owned group or use the same
cleanup protocol for their own group. SIGKILL cannot unwind Python cleanup.
Windows requires a job-object implementation before these campaigns can promise
equivalent ownership. Disposable-process tests exercise failures, inherited pipes,
lingering descendants, SIGTERM, and a deliberately disabled-cleanup control.

The semantic campaigns accept an explicit stock CLI for focused work. They check
that it is executable, print its SHA-256 identity, and require that identity to
remain unchanged through a successful campaign. The composition checker also
requires a new work directory with a supplied CLI; it refuses an existing one.
Without supplied artifacts, both checkers build in a fresh temporary target.
The full gate builds one stock CLI after its Cargo checks and supplies it to both
campaigns sequentially. Each campaign checks its hash before and after execution.
The gate removes the shared target and composition databases during finalization.

### Isolate native allocation reuse

The [native reuse caller](fixtures/native-allocation-reuse.c) uses ordinary C
`malloc` and `free`, with no engine or Rust allocator observer. Its two modes use
the same 3,817,440-byte request. `cold` measures it in a fresh process. `reuse`
first allocates and frees 3,899,392 bytes, then measures the smaller request.
The seed size is the usable extent observed in the wide-join diagnostic; this
controlled history does not replay the original query's full allocation history.

Run from the repository root on macOS or GNU/Linux:

```sh
pipesql_reuse_dir=$(mktemp -d)
cc -std=c11 -O2 -Wall -Wextra -Werror tools/fixtures/native-allocation-reuse.c \
  -o "$pipesql_reuse_dir/native-reuse"
"$pipesql_reuse_dir/native-reuse" cold
"$pipesql_reuse_dir/native-reuse" reuse
rm -r -- "$pipesql_reuse_dir"
```

Each successful invocation prints one JSON record after freeing its allocations.
`usable` and `seed_usable` come from `malloc_size` on Darwin or
`malloc_usable_size` on GNU/Linux. `reused` compares saved integer addresses and
never dereferences a freed pointer. The program rejects malformed invocations,
allocation failures and extents smaller than their requests; it does not require
reuse or a particular rounded size from every allocator configuration.

Compare fresh processes, including the cold control. On the measured Darwin
configuration, reuse reproduces the oversized extent independently of PipeSQL.
The [evidence](../notes/evidence.md#native-allocation-reuse) records both platforms'
observations. A matching extent supports a native-reuse explanation for one
allocation; it does not identify the original freed block or account for every
byte of the combined query's deficit. Keep that query's strict diagnostic and its
unqualified usable-heap/RSS status. This optional experiment is not a passing
engine admission campaign.

## Maintaining a tool

Keep executable work behind an explicit entry point. Fail on missing or ambiguous
inputs; never select an artifact by modification time. Use a fresh owned output
directory and preserve the original subprocess failure. Keep command arguments as
lists, with explicit working directories and timeouts.

Before consolidating helpers, distinguish shared mechanics from intentionally
independent evidence. Preserve cases and negative controls. Update the ordinary
gate, source-manifest coverage, and this inventory when command ownership changes.
Do not introduce a second workflow framework around the sequential gate.
