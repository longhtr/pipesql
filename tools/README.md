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
| `check-composable-aggregates.py` | Independent public composition corpus through the stock CLI. |
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
| Library allocation | `allocation_cells` selects operations and path bounds; `required_outcomes` lists required observations. `check_allocation_prefixes` runs the healthy prefix before refusal positions. `check_ownership` owns composed-reader/writer controls. |
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
| `check-diagnostic-allocation.py` | Public library construction, errors, queries, catalog recovery, and composed ownership under allocator refusal. |
| `check-cli-allocation.py` | CLI startup, parsing, output, publication tokens, and allocation refusal. |
| `check-native-initialization.py` | Darwin root-stat and Linux lstat/readlink observation, refusal, and overlapping callers. |
| `check-native-sync.py` | Linked synchronization calls, refusal, and healed outcomes. |
| `check-native-io.py` | Linked byte-I/O calls, partial progress, refusal, and healed outcomes. |
| `check-native-sanitizer.py` | Explicit nightly AddressSanitizer controls and native-mutex tests, compared with stock and uninstrumented nightly builds. |
| `check-catalog-interruption.py` | Stock catalog append/recovery process-termination cuts. |
| `check-catalog-graph.py` | Independent persisted-graph checks and negative controls. |

These runners compile and execute code. C/Rust callers live in
`fixtures/`; they are development scaffolding with their own unsafe and process
ownership, not shipped adapters.

Use `python3 -B tools/check-diagnostic-allocation.py --ownership-only` to
reconcile prepared queries, parked readers, and an append. The
[resource equations](../docs/resources.md#interpret-composed-memory-observations)
explain logical charges, requested/usable bytes, caller storage, and observer
limits. It checks reader usable extents against admission, including one- and
64-column INT64, DOUBLE, and DATE ORDER BY/DISTINCT cases with complete nullable
results and final release. Composed-reader bound failures are reported after the
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

## Maintaining a tool

Keep executable work behind an explicit entry point. Fail on missing or ambiguous
inputs; never select an artifact by modification time. Use a fresh owned output
directory and preserve the original subprocess failure. Keep command arguments as
lists, with explicit working directories and timeouts.

Before consolidating helpers, distinguish shared mechanics from intentionally
independent evidence. Preserve cases and negative controls. Update the ordinary
gate, source-manifest coverage, and this inventory when command ownership changes.
Do not introduce a second workflow framework around the sequential gate.
