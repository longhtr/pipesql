# Working on PipeSQL

Use the pinned Rust toolchain. Cargo dependencies are vendored, so builds and
checks run offline after toolchain installation. Keep the target directory between
edits. On a small development machine, use one Cargo job for large builds.
Native checks also require a C compiler and platform headers for the small SDK
bindings, plus `/usr/bin/time` for process observations. Install the command-line
SDK on macOS; Linux checks use GNU libc development headers and GNU time.

```sh
cargo build --release --offline --locked -j 1
```

## Test the changed behavior

`cargo dev check` selects local documentation links, formatting, all-target Clippy
and three CLI lifecycle tests. Use `cargo dev tidy` to check links alone.

Private tests live beside their implementation. Public API tests are in `test/`;
`test/data/` holds stored inputs and expected answers. Cargo declares the integration
targets explicitly because it does not automatically discover this directory name.

Start with the smallest relevant test. Use release mode when stack size, allocation
geometry or timing is part of the case:

```sh
cargo test --offline --locked --lib query::parser::tests::
cargo test --release --offline --locked --test public window_sum::
```

Check that the filter selected tests. For an exact test name, add `-- --exact`.
A successful command that ran zero tests establishes nothing about the change.
Choose the broader checkpoint before implementation. A local edit needs its
affected behavior checked; completed persistence, native, resource or concurrency
work needs the relevant campaigns together. Reuse unchanged passing results unless
a failure or unresolved concern justifies another run.

Keep expected answers beside the behavior being checked. Share input construction
and process ownership, but do not use the production calculation to derive an
independent answer. Stored inputs under `test/data/` retain their provenance and
licenses. Change expected bytes only when the intended format or rejection rule
changes, with an independent explanation.

Challenge the test as well as the implementation. A result checker must reject a
wrong complete answer and a correct prefix followed by failure. Vary NULLs,
duplicates, width, order and failure position independently when the behavior
depends on them. For blocking operators, force spill and replay and check cleanup
after refusal or cancellation. Verify demanded errors and spans across producer
boundaries; matching final rows alone can miss an incorrect evaluation schedule.
When a harness can hang, supervise its deliberate failures in another process.
Keep replay inputs and configuration with the output; a seed cannot reproduce
native thread scheduling or recover a deleted input.

Temporary-directory and direct-child guards live in `test/support/`. Create the
directory owner first so child processes and open handles stop using it before
cleanup. The child guard does not own descendants; the development runner's
process supervisor does. Each test still checks readiness and completion. Native
stack observations and allocation campaigns establish different facts from
ordinary semantic tests, even when they share inputs.

The Rust runner builds its database drivers through Cargo and judges their output
in a separate process. Named cases keep focused checks available:

```sh
cargo dev --help
cargo dev test analytics --list
cargo dev test analytics --case small
cargo dev test recovery --case append-boundaries
cargo dev test composition --case report
cargo dev test allocation --case jsonl
cargo dev test allocation --case windows
cargo dev test allocation --case joins
cargo dev test io --case windows
cargo dev test io --case joins
cargo dev test io --case join-keys
cargo dev test io --case join-key-controls
cargo dev test io --case join-strings
cargo dev test io --case join-string-controls
cargo dev test io --case census
cargo dev test io --case census-joins
```

The allocation `joins` selection refuses optional workspace replacement while
a text left join retains its first input. It checks complete pairs and unmatched
rows, cancellation and reuse, plus missing-observer, attribution and false-pair
controls. Linux requires written spill from both inputs; macOS checks native
allocations, descriptors and reservations.

The I/O `joins` selection checks complete inner and left join pairs, including
NULL keys, unmatched rows, nullable values and retained UTF-8/NUL text. Duplicate
groups cross native read buffers. Every observed transfer position is swept for
short progress and errors; failed output must be a valid prefix of an independently
checked retry. Healthy text cases also cancel after duplicate-group replay.

`join-keys` adds DOUBLE and DATE text and numeric joins. Source preflight checks
original floating-point bits and date days before faults are armed; separate
answer checks reject NULL and NaN matches and require matches across signed
zeros. Large equal-key groups force replay across native read buffers. Healthy
cases cancel after two complete traversals of the same large group. The selection
sweeps every observed transfer position and includes focused integer regression
controls. `join-key-controls` runs the source, complete-answer, cancellation and
observer controls first, without the new key types' full fault sweeps. Select one
full sweep with `join-double`, `join-date`, `join-double-numeric`, or
`join-date-numeric` when only that fixture changed.

`join-strings` checks complete STRING key bytes and equality across empty text,
embedded NUL, long shared prefixes and distinct Unicode representations. Both
text and fixed-result projections use scalar matching because their retained
key is STRING. Large groups exceed both native read buffers; cancellation must
reach two complete traversals of one group. The selection includes existing
integer, DOUBLE and DATE regression controls. `join-string-controls` omits the
new fixtures' full fault sweeps; `join-string` and `join-string-numeric` each
select one full sweep.

The I/O `census` selection separately observes actual native call and transfer
bytes for checked text scans, sorts and windows. It includes two-, three- and
32-row peers, large groups and uneven partitions; it does not time queries or
inject faults. Standard byte/offset controls check its observer first.
`census-joins` observes the same inputs as the `join-replay` measurement below,
including matching dimension ORDER and fact scan controls. It checks the native
observer first and derives each required complete row count independently.

The analytical workflow checks complete typed results. Recovery checks native
interruption and independently decoded stored state. The abstract models explore
small histories under stated persistence assumptions; they do not execute the
engine. Their introductions explain what each check establishes.

## Change an owner

Trace a normal operation and a consequential failure before changing it. Follow
column identity through projections, reservations through buffer release, and
publication outcomes through recovery. Keep definite abort, possible commit and
failed cleanup distinct. A passing result check alone cannot establish those rules.

Choose boundaries around decisions and resource lifetimes. A helper or abstraction
should remove a real repeated hazard without hiding state changes. Bound bytes as
well as rows; check arithmetic before allocation, narrowing or mutation. Return
typed errors for input, resource and OS failures; use assertions for programming
invariants. Keep unsafe native code behind small interfaces with stated premises.

Put public behavior beside APIs and consequential reasons beside enforcement.
Keep separate guides for tasks that cross code owners. Remove duplicate inventories
and obsolete commands after checking callers, retained data and attribution. Keep
unfinished work in [Remaining work](#remaining-work) and run details in generated
output.

Before performance work, identify a likely cost and compare matching optimized
artifacts on representative inputs. Check complete answers in the same run.
Include spill where relevant; a microbenchmark does not establish an end-to-end
improvement. Preserve pinned offline dependencies and their licenses when changing
the build. Review a dependency's allocation, panic, I/O and maintenance costs too.

## Broader checks

```sh
cargo fmt --all --check
cargo clippy --offline --locked --workspace --all-targets -j 1 -- -D warnings
cargo dev test rust
cargo dev test documentation
cargo dev test platform
cargo dev linux test rust
cargo dev linux test analytics --case scaled
```

`cargo dev test documentation` builds warnings-denied library and CLI documentation
in separate warm targets, verifies both introductions and runs documentation tests.

`cargo dev test platform` checks system-header layouts, native mutex behavior and
thread stack bounds, including an oversized-thread control. It measures the
thread extent, not the deepest stack use of a database operation.
It also checks that every native observer mode rejects overlap and permits a
new mode after the previous one stops.

On the Docker host, `cargo dev test linux-runner` checks the container owner itself:
success and failure, missing or mismatched completion, interruption, saved diagnostics
and container removal. It is separate from tests inside Linux because those containers
have no Docker access.

Linux is primary. The Linux command freezes the intended Git inventory before
starting the container. Stage newly authored files, or use `git add -N`, so they
are included. It refuses source changes during copying. The container uses native
storage for databases and a retained Cargo cache for build artifacts.

Provision `pipesql-verification-rust:1.98.1-time` once on the Docker host:

```sh
docker build --pull=false -f dev/verification.Dockerfile -t pipesql-verification-rust:1.98.1-time dev
```

The [recipe](dev/verification.Dockerfile) pins the official Rust 1.98.1 base by
registry digest and installs GNU time from a fixed Debian package snapshot.
The base supplies the C compiler and GNU libc headers; the recipe also installs
Clippy and rustfmt for the pinned toolchain. Provisioning needs network access.
Keep an existing verification image and its warm targets; rebuilding is needed
only when provisioning is missing or the recipe deliberately changes.

Before copying source, the runner resolves the local image tag to a Linux content
identity and saves Docker's inspection as `image.json`. It creates the container
by that identity with pulling disabled, so a later tag change cannot select a
different image. A rebuilt image may have a different identity; each run records
the actual one instead of requiring a historical host's image ID.

The container runs as uid/gid 1000 with one CPU, 2 GiB and no network. Docker's `--init`
handles orphaned processes. Source is copied from a read-only mount; Linux Cargo
artifacts stay warm under `target/dev-linux`. Results are copied back before
the owned container is removed, and failed inputs are retained. The copied
`environment.txt` records the kernel, user, distribution, compiler, Cargo,
Clippy, rustfmt, C compiler, libc and GNU time versions before checks begin. See
[platforms](docs/operations.md#choose-storage-that-meets-the-engines-assumptions) for storage and qualification limits.

`cargo dev test all` runs the maintained campaigns sequentially, followed by fresh
small and above-memory analytical workflows. It freezes source, uses a separate
warm Cargo target and keeps the individual campaign logs. A failed campaign stops
the sequence. Successful completion also requires unchanged source and the inner
runner's completion record. Run it in Linux with `cargo dev linux test all`.
Nightly sanitizer diagnostics and host Docker-owner controls remain separate.

The optional Parquet reader image also supports fresh transfer qualification:

```sh
cargo dev linux test parquet-transfer
```

Provision that image as described under [measurement](#measure-analytical-work).
This selection is separate from `test all`. PyArrow generates eight uncompressed
v1/v2 inputs with 777 or 8,192 rows, descending IDs, different row-group sizes and
page targets, and 65,536-byte strings. The driver reverses target column order and
checks each input with both 7-byte and 127-byte caller reads. Receipt failure,
final-source failure and final cancellation must preserve an existing sentinel row, remove private data objects,
release reservations and handles, and resolve as aborted after reopen. A healthy
retry must resolve as committed and retain every input row.

Two qualification-only inputs use 513-row groups and nine adjacent maximum-length
strings. Their 1 MiB page and group bounds admit the actual encoded input; ordinary
cases retain their smaller bounds. The independent native inspector checks that
the first 512-row lend splits after seven strings, followed by the group remainder.
Wrong values at these cuts and an incorrect cut with the same row total must fail.

The same selection exports 513 rows through a permuted 64-column mixed-type
projection, with 257- and 255-row groups and short writer fragments. PyArrow checks
the exact schema, every value and raw DOUBLE bits; readable wrong-column and
complete-prefix files must be rejected. Query admission refuses a 65th output.
Late row and footer limits must release resources, and a retry must reproduce the
complete output bytes.

Two further PyArrow inputs qualify imports at the 64-column limit. Repeated
types have distinct values and NULL positions; input, declaration and result
column orders differ. The 513-row input uses a single group for v1 and groups of
257 and 256 for v2. A final source error must leave a nonempty table intact before
reopen and retry. The supervisor checks every reopened value, receipt outcomes,
resource release, and controls for same-type column aliases and changed DOUBLE bits.

The supervisor checks complete reopened JSON Lines against independently authored
values, including raw DOUBLE bits. This avoids relying on a Parquet round trip.
Wrong or incomplete answers, incorrect outcome reports, disabled assertions, a
missing generator and a disabled source fault must fail their controls. These
checks exercise the API's commit resolution on the recorded container storage;
they do not extend power-loss qualification or establish an import performance
baseline.

## Measure analytical work

Run checked examples on frozen Linux inputs with the same image and resource limits:

```sh
cargo dev linux measure --case all
```

Select `grouping`, `composed-grouping`, `numeric-grouping`, `filtered-grouping`,
`string-grouping`,
`sets`, `scalar`, `scans`, `computed`, `operators`, `windows`, `window-payloads`, `joins`, `join-replay`,
`numeric-joins`, `scaled-numeric-joins`, `report` or
`exports` or `csv-imports` for a smaller matrix. `numeric-grouping` compares INT64 and DOUBLE SUM/AVG
over 8,192 rows, 32 or 4,096 groups, even or skewed input, and 1.2 MB or 2 MB of
memory. Each profile has a checked warm-up and five query samples in each of three
fresh processes. The default grouping example retains its extrema query with the
same repeated-query checks. `filtered-grouping` uses the same group counts,
distributions and budgets with an integer computation shared by filtering and
output. It retains exactly the even keys and checks every count, total and
computed value. Large groups at 1.2 MB must write spill; spilled queries must
cancel, release their resources and retry.
`numeric-joins` checks fixed-width inner and left joins over 8,192 left rows,
32 or 4,096 keys, one or eight right rows per key, and 2.6 MB or 12 MB budgets.
Nullable keys create unmatched rows; nullable values and both row identities are
checked for every pair. Each of three fresh processes runs a checked warm-up,
five timed queries, cancellation after written spill and a complete retry.
`scaled-numeric-joins` uses the same matrix over 262,144 left rows and requires
written scratch data larger than the selected memory budget. Temporary space is
capped at 16 MB for the small matrix and 64 MB for the larger matrix. Each query
has a 20-million-step bound; the supervisor allows five minutes per process.
`composed-grouping` measures the self-join, grouping and descending-order query
at 2.2 MB and 12 MB. Each of three fresh processes checks a warm-up and five
complete query samples, requires written spill on Linux, then cancels and retries.
Its exact schema and all 4,096 typed groups are checked. This matrix is also
included in `grouping`.
`string-grouping` compares string extrema over integer keys with SUM/COUNT over
text keys. Sixteen profiles vary 4 or 256 groups, 8-byte or 65,536-byte text, and
4 MB or 80 MB budgets. Each key occurs twice in descending input passes; text
keys share a prefix and end in an eight-digit identity. Four-row input batches
stay fixed. Every fresh process runs a warm-up and five complete checked queries;
Linux observes written spill outside timing and spilled cases must cancel and
retry without retained owners. Three processes per profile preserve all samples.
This matrix is also included in `grouping`.
`sets` compares EXCEPT and INTERSECT, each DISTINCT and ALL, with 8,192 left and
4,096 right rows, partially overlapping classes and unequal duplicate counts.
Its 32 scenarios vary 32 or 4,096 classes, 8-byte or 1,024-byte strings, and 4 MB
or 12 MB of memory. Each checks complete typed row multiplicities, actual spill,
cancellation and retry, with five samples in each of three fresh processes.
`scans` compares unfiltered scans with predicates matching zero, sparse, half and
all rows at 8 MB, using the same 8,192- and 65,536-row integer inputs written
in 64-, 512- or 4,096-row fact batches. Result batches contain at most 256 rows;
each sample reports its observed batch count. Three fresh
processes per shape each run a warm-up and five checked query samples. Every
selected identity and value is checked; empty results still require completion,
no scratch files and released owners. Timings include answer checks and cursor
disposal, without cache eviction.
`computed` measures a numeric value used by a filter and then output, alongside
its double, after ORDER BY or JOIN. Twelve profiles vary 8,192 or 65,536 facts,
one or eight right matches and 4 MB or 12 MB budgets. Each checks every selected
identity and value, written spill, cancellation and retry, with five samples in
three fresh processes. These cases exercise repeated row-level demand; timings
include answer checks and cursor disposal.
`operators` runs each blocking profile in three fresh processes, with five
checked query samples per process; its scan controls run once.
`join-replay` is a focused selection, separate from `all`. Its 24 join shapes
vary one or eight matches, 32 or 4,096 keys, 8-, 1,024- or 2,048-byte dimension
text, and 2.6 MB or 12 MB budgets over 8,192 facts. Twelve dimension ORDER
controls use the same right rows at 12 MB, alongside one fact scan control.
ORDER uses text comparison and omits the join key; it controls right input and
sorting costs rather than reproducing the complete join plan. All 37 shapes
run in three fresh processes with five checked samples each. The new 2,048-byte
width permits 256 MB of temporary space for overlapping dimension merge files;
existing widths retain 128 MB. Complete pair identities, text and multiplicity,
actual spill, cancellation, release and retry are required before timing is
accepted. Native call and byte observations run separately through `census-joins`.
`windows` selects partition COUNT and peer-inclusive running SUM from the operator
cases with constrained growth (1 MB for COUNT, 1.2 MB for SUM) and a 2 MB
control. Each runs in three fresh processes, with five checked query samples per
process.

`window-payloads` retains 8-byte or 1,024-byte fact text through partition COUNT
and peer-inclusive SUM. Its 32 window shapes vary 8,192 or 65,536 rows, 32 or
4,096 keys, and 4 MB or 12 MB budgets. Eight matching scan and ORDER controls
use 32 keys and 12 MB. All 40 shapes run in three fresh processes with five
checked samples each. Wide payloads include UTF-8 and NUL bytes; every identity,
text byte and typed result is checked. Warm-ups require actual Linux spill for
blocking profiles, followed by cancellation, release and complete retry. The
256 MB temporary allowance admits two wide sort files during merge; it remains
separate from reservations and process RSS. Timings include validation and cursor
disposal, excluding setup, preparation and the warm-up file probe.

`joins` uses 8,192 facts with one or eight right matches per key, 32 or
4,096 keys, 8-byte or 1,024-byte payloads and 2.6 MB or 12 MB of memory. It checks
every distinct pair in the same three-process, five-sample schedule. The
[measurement runner](dev/src/measure.rs) builds each optimized artifact once and
records its hash. `selection.json` maps scenario IDs to arguments and repetitions;
`ID-ROUND.time` records GNU time's whole-process CPU time, RSS and filesystem
observations. Numbered stdout/stderr files contain the examples' query samples.
Source, image and environment records belong to the enclosing Linux run.

`report` runs the composed event report over even and skewed inputs at 8 MB and
32 MB in three fresh processes. Each performs an initial checked execution and
untimed spill/cancellation/retry qualification, then five complete samples of the
same prepared report. Sample times include typed answer checks and cursor disposal;
input/model construction, preparation, filesystem probes and reclamation are
separate. Every sample records its time and sampled reservations. Missing or
duplicated samples, wrong input descriptions and invalid values fail the supervisor.

The untimed retry observes nonempty Linux scratch files; cancellation also waits
for written scratch, then requires terminal failure, descriptor closure and
released accounts. The supervisor requires this observation alongside samples and
phase times. macOS example runs retain account checks without claiming Linux file
observation. No cache eviction is attempted.

`exports` measures typed JSON Lines from scans and ordered queries over 8,192 rows.
Its 24 profiles vary 8-byte or 1,024-byte plain text, 1,024-byte escaped Unicode
text, 64-row or 256-row input batches, and writer fragments of 127 or 1,024 bytes.
Each has a warm-up and five exports in three fresh processes. Empty and 257-row
controls run first. The supervisor checks every schema, scalar bit pattern, NULL,
text value, row identity and completion, including positional duplicate names.
It rejects wrong values, duplicate or missing rows, prefixes and trailing data.
A flush error after completion bytes remains a failed export; disabling that
fault must fail its control.

`csv-imports` compares 4 and 64 columns, fixed-width and mixed text schemas, and
64 and 256 decoder rows per batch over 8,192 input rows. Each of three fresh
processes runs a warm-up and five samples on fresh databases. Import from resident
CSV bytes is timed; input construction, declaration, receipt resolution,
close/reopen and complete typed result checks run outside the timer. The checker
preserves independent NULL patterns and exact DOUBLE bits. Engine limits are
32 MB of memory and 64 MB of temporary space; reported memory is the reservation
after import, not a peak. The runner records input hashes and requires identical
bytes across rounds and batch sizes. This selection does not measure filesystem
input throughput or qualify stronger power-loss durability.

The same selection includes `long-text` at both column counts and batch sizes.
Its 128 rows place 65,536-byte Unicode values in successive column bursts, keeping
each generated input below 8 MB. Decoder text storage is capped at 4 MiB; text
capacity and native per-column byte cuts can shorten a requested row batch.
The complete checker compares the Unicode bytes and column suffix, and rejects
a same-length change near the end of an otherwise valid import.

`parquet` is an optional measurement selection, excluded from `all`. Provision
its [reader image](dev/parquet.Dockerfile) after the ordinary verification image:

```sh
docker build --pull=false -f dev/parquet.Dockerfile -t pipesql-parquet-rust:1.98.1-arrow22.0.0 dev
cargo dev linux measure --case parquet
```

The layer installs the SHA-256-pinned PyArrow 22.0.0 wheel for the image's Python
3.11 and architecture. Provisioning needs network access; checks run offline with
the same container owner and limits. Engine dependencies, ordinary Cargo checks
and the primary image do not require PyArrow.

The matrix includes empty and 257-row controls followed by 24 profiles over 8,192
rows. Each larger profile has five exports in three fresh processes. Six baselines
combine scans or ordered spill with narrow, wide plain or wide escaped Unicode
text. Each comparison changes one baseline argument: group rows (311 to 128),
wide group text (131,072 to 65,536 bytes), or narrow/escaped input batches (256 to
64 rows) and writer fragments (1,024 to 127 bytes). Narrow groups use 4,096 text
bytes, so row capacity determines their cuts. Encoder capacity competes with query
memory at the fixed 4 MB budget; a changed capacity can also change query cost.
The external reader checks unique column names, schema, every value including raw
DOUBLE bits, row identities and exact row/text group cuts. Corrupted values, duplicate or missing rows, truncated files, damaged
footers, disabled assertions and a missing checker must fail. The producer also
requires exact resource errors, cancellation, late query failure, release and
retry. A readable file after a failed final flush remains an API error.

`parquet-wide` measures 64-column scan exports with the same optional reader image:

```sh
cargo dev linux measure --case parquet-wide
```

A 513-row control precedes three 8,192-row profiles. The large baseline uses
257-row groups and 4,096-byte writer fragments; comparisons change group rows to
1,024 or writer fragments to 127. Each large profile runs in three fresh processes,
with one warm-up and five samples per process. The export API through final flush
is timed; setup, file creation, release checks and independent reading are outside.
Runs use a 64 MB engine budget and report their native stack extent. PyArrow checks
the permuted mixed schema, every value and raw DOUBLE bit, exact group cuts and
complete files. Reader controls reject substituted columns, changed zero sign bits
and valid-looking prefixes. These scan measurements do not establish spill costs.

`parquet-imports` is another optional selection, excluded from `all`:

```sh
cargo dev linux measure --case parquet-imports
```

It first runs the fresh import qualification, then measures the original six inputs.
The two cases with clustered maximum strings are excluded from timing.
The two 777-row inputs run once with 4,096-byte caller reads. Two 8,192-row inputs
run in three fresh processes with read fragments of 127, 4,096 or 65,536 bytes.
Two more hold the v1 page target and write batch fixed while changing row groups
from 257 to 255 or 256 rows, using 4,096-byte reads in three fresh processes.
Every process has a warm-up and five samples, each starting from a fresh database
with the same declared schema and sentinel row. The API interval includes footer
validation, decoding, native writes and final publication, with observed reader
calls and bytes. Source opening, database setup, reopen and result export are
outside that interval. Complete independent result checks happen afterward.

Compare read sizes within the same input file. The original two larger inputs also differ
in page version, row groups and page targets, so their contrast does not isolate
one of those effects. `selection.json` records the schedule; `measurements.json`
retains every API sample, output hash and separate validation time. After timing,
the independent offline inspector validates the live graph and every payload.
The supervisor records referenced native data files with their row counts and
sizes, including the sentinel unit; catalogs and other metadata are excluded. A
corrupted native payload must fail inspection, and its restored bytes must pass.
Unit layout is observed behavior, not a constraint on future optimizations. Whole-process
GNU time includes setup, reopen and result export. No cache eviction is attempted;
these filesystem timings retain the container storage limitation.

Export sample times include query execution, encoding, writer calls and final
flush. Independent parsing happens afterward; `ID-ROUND.validation.json` records
its time and the producer's separate API outcome. Parquet file validation times
exclude Python startup and hashing; the command log records the whole external
reader process. Whole-process time and RSS cover
the producer, including setup and failure controls, but exclude this supervisor
validation. Writer callbacks sample engine reservations; they can miss peaks before
output begins. Caller output files and OS buffering are separate costs. Warm-ups
observe written scratch, and failures must release query resources before retry.

Compare matching cases and all their samples. Example introductions define timing
boundaries. Apart from exports, query times include independent answer checks and
result disposal. Process measurements also include setup, warm-up and cleanup
checks. Input batch sizes stay fixed across a comparison; no cache eviction is attempted.
Reservations are separate from process RSS. Operator and grouping warm-ups check
written scratch through Linux file descriptors, outside measured samples. Cases
that spill also check cancellation, release and retry. Ordinary macOS example
runs retain answer and reservation checks without claiming this Linux observation.

## Check a filesystem

`cargo dev linux test identity` exercises container-native storage, including
replacement and failed-observation controls. To investigate another mount from
GNU/Linux, supply a new directory with no other writer:

```sh
cargo dev identity /absolute/test-mount/new-directory
```

The caller owns its contents afterward. The command has a 60-second deadline and
retains stdout/stderr in its printed result directory. Record image, kernel,
libc, user and mount configuration; compare fresh paths on the suspect mount and
native storage.

The [witness](dev/driver/src/identity.rs) replaces two files 200 times and compares
pathname and descriptor identities without application mutation between ordinary
observations. It checks every content byte. A mismatch reports all six device/inode
pairs; observation failure is separate. Require successful exit and
`400 file replacements checked`. Neither a bounded pass nor a timeout clears an
intermittent counterexample. This check does not qualify disk synchronization.

## Native sanitizer diagnostics

An installed nightly is required. The runner freezes source, checks a clean
process and a deliberate heap-bounds fault, then compares the selected tests
under the pinned compiler, plain nightly and AddressSanitizer:

```sh
cargo dev test sanitizer --case pathname --toolchain nightly
```

Use `mutex` for the smaller native mutex selection or `controls` to check
instrumentation alone. These scopes use the prebuilt standard library. They
check the selected native boundaries, not whole-engine memory safety.

`memory` includes selected import, query, export and recovery paths under
AddressSanitizer. `concurrency` uses ThreadSanitizer for selected shared-state
paths. Both require `--standard-library-vendor` and rebuild the standard library;
neither establishes coverage of every engine path or native runtime library.

To rebuild the standard library too, provision the selected nightly's `rust-src`
and vendor its dependencies outside the checkout. Then add
`--standard-library-vendor /absolute/dependency/directory` to the command.
The runner preserves engine dependency versions, records source and dependency
hashes, and requires Cargo to identify a rebuilt standard library in each nightly
build. The pinned-compiler comparison continues to use its installed library.

Linux diagnostics use a separate image. Provisioning needs network access;
test runs remain offline:

```sh
docker build --pull=false -f dev/sanitizer.Dockerfile -t pipesql-diagnostic-rust:nightly-2026-09-06-std dev
cargo dev linux test sanitizer --case pathname --toolchain nightly-2026-09-06
```

The runner resolves the diagnostic image to its content identity before creating
the container. The ordinary verification image remains unchanged.

## Investigate a failure

For a closed declared-table database or a quiescent copy, independently inspect
stored metadata and complete rows:

```sh
(set -C; cargo dev inspect /absolute/database > /absolute/new-graph.json)
```

The inspector takes the cooperating database lease without waiting and never
repairs files. It validates namespace version 7 and object version 6 before
printing JSON. Require exit zero; a failed output write can leave a partial file.
The caller owns that file. Unreferenced objects are reported, not authorized for
deletion, and settled roots do not establish power-loss durability.

Default limits are 4,096 objects, 64 MiB of cumulative reads and 1,000,000 decoded
values. Override them with `--max-objects`, `--max-read-bytes` and `--max-values`,
up to 1,048,576 objects, 1 GiB and 10,000,000 values. Exceeding a diagnostic limit
fails inspection; it does not establish corruption. These bounds constrain work
and retained input, not process memory. DOUBLE cells retain their hexadecimal bits;
DATE cells use signed days from 1970-01-01. See the
[independent reader](dev/src/oracle/catalog.rs) for decoding and authority rules.

Output defaults to the OS temporary directory. To retain runs across temporary-file
cleanup, create a persistent directory and set `PIPESQL_RUN_ROOT` to its absolute
path before invoking the runner. The parent directory must already exist.

The runner prints its output directory before launching work. `commands.jsonl`
records arguments, working directory and explicit environment overrides/removals
before execution, then status, elapsed time and omitted-output counts.
Numbered stdout/stderr files preserve diagnostics. Cargo artifact records
include hashes and build profiles. A failed workflow retains its input databases
and exports; a successful one removes those working directories and retains logs.

For Linux, the outer directory contains the container command and copied results.
Failed runs also retain `source/` with `.pipesql-source.json`, its exact file list
and hashes. From that directory, `cargo dev linux test ...` can replay the inputs
without Git. Replay rejects files that differ from the recorded hashes; make new
changes in the working checkout instead.
Check the first failing command and its case input before expanding the test
selection. Keep query completion, expected
rows, publication outcome and cleanup as separate questions.

## Remaining work

### Analytical performance

Small-peer read-ahead changes need repeatable end-to-end gains alongside matching
scan and ORDER controls. Record-length rounding reduces transfers for three-row
wide peers but leaves conflicting timings in other shapes; retain the 4 KiB hint.
Include two-, three-, large and uneven peers with complete retained-text answers,
and preserve fresh reads and validation during replay.

Duplicate text joins retain the 16 KiB first-group read hint. Rounding to the
first record and allowing a following key reduces transfers for eight wide
matches, but adds prefetch for one match and leaves ordinary-join timings
unresolved. Compare one and many matches with matching ORDER and scan controls
before changing this hint. Rewinds must reread and validate the visited group
and its following-key record.

Parquet row-value staging needs a repeatable benefit across wide scans and text
exports. A per-batch borrowed-value array improved wide output but regressed text
exports; retain direct lookups. Preserve row preflight before flushing a group,
and include single-row cursor batches when measuring scratch initialization.

Skipping text-bound scans for fixed-width batches needs a repeatable complete-import
benefit. The local shortcut did not improve the narrow and wide `csv-imports`
matrix under alternating artifact checks; retain the shared partition scan.

Required-integer sort comparison specialization needs a repeatable benefit
without slowing wide text sorting. The tested local shortcut improved integer
ORDER and DISTINCT while regressing the text control; retain the shared decoder.

Combining join input encoding and admission still needs a repeatable end-to-end
benefit. Fewer progress transitions alone do not justify that change.

Grouped evaluator reuse needs a repeatable benefit without a composed-query
regression. Sharing work between filtering and output can help the targeted
computed case while slowing ordinary composed queries. Keep each evaluator local
to its immutable group and preserve cached NULLs and filtered-away errors.

Further grouped-result decoding changes need a repeatable end-to-end benefit
across spill and composed workloads. Isolated spill gains do not settle mixed
in-memory and composed results.

Computed-demand measurements now cover filtering and output after sorting and
joining. A cached-result shortcut needs a repeatable benefit on those cases
without a regression in ordinary joins; conflicting ordinary-workload timings
leave it unqualified.

Scan column traversal changes need a repeatable end-to-end benefit alongside the
checked selective scans. Include mixed-type and long-text exports; a small scan
improvement alone does not settle a conflicting ordered-export measurement.

Schema field-access changes remain gated on matching narrow and wide import,
reopen and typed-scan measurements. Wide-table gains alone are insufficient when
narrow-workload results remain unresolved. Preserve initial validation, immutable
borrows and persistent column identity; avoid a schema cache for this work.

Further CSV scanner optimization must account for quote-dense input as well as
ordinary text. Bulk record copying can improve wide fields while slowing the
quote state machine; preserve the first-excess offset tests and require measured
end-to-end gains without that regression.

Do not shrink global hash capacities to improve small reports: high-cardinality
and wide-key grouping also need the available memory. A measured cost does not
by itself justify an execution rewrite or a new cache or allocation framework.

Additional SQL features and legacy-loader removal remain separate work.
Stronger power-loss qualification remains deferred.
