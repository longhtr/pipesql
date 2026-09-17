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
```

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
storage for databases and a retained Cargo cache for build artifacts. The image is
`pipesql-verification-rust:1.98.1-time`, pinned to
`sha256:520be9ff830f944e49a3319cbf6f8ccfb2c1f21631947de50290efb98038e282`.
It runs as uid/gid 1000 with one CPU, 2 GiB and no network. Docker's `--init`
handles orphaned processes. Source is copied from a read-only mount; Linux Cargo
artifacts stay warm under `target/dev-linux`. Results are copied back before
the owned container is removed, and failed inputs are retained. See
[platforms](docs/operations.md#choose-storage-that-meets-the-engines-assumptions) for storage and qualification limits.

`cargo dev test all` runs the maintained campaigns sequentially, followed by fresh
small and above-memory analytical workflows. It freezes source, uses a separate
warm Cargo target and keeps the individual campaign logs. A failed campaign stops
the sequence. Successful completion also requires unchanged source and the inner
runner's completion record. Run it in Linux with `cargo dev linux test all`.
Nightly sanitizer diagnostics and host Docker-owner controls remain separate.

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

### Further engine investigations

Keep performance work separate from the source reorganization. Measure grouping
I/O, scalar evaluation, DISTINCT sorting, one-row join emission and wide-text join
admission with complete independent answers and forced spill. Derive capacities
from simultaneous owners and required progress; observed peaks alone cannot prove
arbitrary allocator or process-memory bounds. Removing legacy loading changes
product scope and requires its own decision.
