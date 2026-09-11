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
| `snapshot-fixtures.py`, `candidate-fixtures.py` | Current legacy and retired/rejected snapshot vectors. |
| `catalog-root-fixture.py`, `catalog-fixture.py`, `catalog-schema-fixture.py` | Catalog namespace roots, catalogs, and schemas. |
| `table-data-fixture.py`, `native-unit-fixture.py` | Table indexes and typed units. |
| `aggregate-fixtures.py` | Independently encoded semantic campaign inputs. |

The independent legacy encoders and Q1 comparator live in
[`oracles/`](oracles/README.md). Historical manifests must be replayed at their
recorded revision after a path move.

## Native and allocation callers

| Runner | Owned boundary |
| --- | --- |
| `check-filesystem-abi.py` | Native SDK/decoder agreement. |
| `check-diagnostic-allocation.py` | Public library construction, errors, queries, catalog recovery, and composed ownership under allocator refusal. |
| `check-cli-allocation.py` | CLI startup, parsing, output, publication tokens, and allocation refusal. |
| `check-native-initialization.py` | Darwin pathname observation and native refusal. |
| `check-native-sync.py` | Linked synchronization calls, refusal, and healed outcomes. |
| `check-native-io.py` | Linked byte-I/O calls, partial progress, refusal, and healed outcomes. |
| `check-catalog-interruption.py` | Stock catalog append/recovery process-termination cuts. |
| `check-catalog-graph.py` | Independent persisted-graph checks and negative controls. |

These runners compile and execute code. C/Rust callers live in
`fixtures/`; they are development scaffolding with their own unsafe and process
ownership, not shipped adapters.

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

## Maintaining a tool

Keep executable work behind an explicit entry point. Fail on missing or ambiguous
inputs; never select an artifact by modification time. Use a fresh owned output
directory and preserve the original subprocess failure. Keep command arguments as
lists, with explicit working directories and timeouts.

Before consolidating helpers, distinguish shared mechanics from intentionally
independent evidence. Preserve cases and negative controls. Update the ordinary
gate, source-manifest coverage, and this inventory when command ownership changes.
Do not introduce a second workflow framework around the sequential gate.
