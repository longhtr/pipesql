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
| `sh tools/check.sh` | Sequential gate over a private source export, with stage logs and a result receipt. Accepts `--output` and `--scope`. | Yes |
| `python3 tools/check-linux-vm.py --image IMAGE --kernel KERNEL --output NEW_DIRECTORY` | Full-synchronization Linux VM gate and fresh examples; owns its images and containers. See [setup](../docs/testing.md#linux-verification-with-full-synchronization). | Yes |
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

Source discovery, hashing and export belong to [source-manifest.py](source-manifest.py).
Both platform runners use that owner; campaign expectations remain separate.

## Linux VM verification owners

[check-linux-vm.py](check-linux-vm.py) freezes inputs, prepares the provisioned
image, checks the controller's policy refusal, runs the guest and validates its
receipts before removing owned images and containers. The
[controller](fixtures/virtual-disk-sync-host.m) owns VM resources and enforces
full synchronization for verification profiles. Its smaller raw/catalog profiles
remain the latency diagnostic below.

[linux-verification-init.c](fixtures/linux-verification-init.c) owns privileged
mounts, the unprivileged child, descendant reaping and shutdown. It reports the
actual child status separately from completed cleanup. The
[guest commands](fixtures/linux-verification.sh) check placement and identity,
then invoke the existing gate and examples. No alternate database verification
implementation lives in the wrapper. [Completion tests](test-linux-vm.py) reject
missing, duplicate, reordered or failed status records and wrong-source or
incomplete gate receipts. [Testing](../docs/testing.md#linux-verification-with-full-synchronization)
owns the public command and its output interpretation.

## Models and semantic oracles

| Owner | Responsibility |
| --- | --- |
| [models/attempt_identity.py](models/attempt_identity.py) | Bounded issuance-prefix/success-index model against explicit receipts. |
| [models/attempt_publication.py](models/attempt_publication.py) | Root/fence representation model under stated persistence premises. |
| `aggregate-rounding-vectors.py` | Independent rational rounding vectors; `--check` compares retained inputs. |
| `check-aggregate-semantics.py` | Public stock CLI aggregate boundary campaign. |
| `check-composable-aggregates.py` | Independent calendar-year expectations and public composition corpus through the stock CLI: legacy aggregates, positional unions, and the composed event report over retained catalog fixtures. |
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

## Measure synchronization in a verification caller

Use this diagnostic when a storage-heavy campaign is slow. It builds the existing
allocation caller once and compares fresh stock and observed runs. The observer
in [sync-timing.c](fixtures/sync-timing.c) forwards each native synchronization
call unchanged and sums its elapsed nanoseconds. It measures F_FULLFSYNC on
macOS and fsync on Linux. Other synchronization primitives are counted separately.
Unexpected Darwin fcntl signatures fail closed.

Run from the repository root with the ordinary native prerequisites. The command
uses one Cargo job and removes its owned build and databases when finished:

```sh
CARGO_BUILD_JOBS=1 RUSTFLAGS='-D warnings' python3 -B - <<'PY'
from pathlib import Path
import re
import runpy
import sys
import tempfile
import time

sys.path.insert(0, "tools")
from check_process import run
from check_support import native_library, observer_environment

allocation = runpy.run_path("tools/check-diagnostic-allocation.py")
root = Path.cwd()
with tempfile.TemporaryDirectory(prefix="pipesql-sync-timing-") as name:
    work = Path(name).resolve()
    allocation["build_driver"](work)
    observer = native_library(work, "sync-timing.c", "sync_timing")
    for repeat in range(3):
        for mode in ("allocation-capacity", "catalog-control", "catalog-after-0"):
            outputs = []
            for measured in ((False, True) if repeat % 2 == 0 else (True, False)):
                path = work / f"case-{repeat}-{mode}-{int(measured)}"
                started = time.monotonic()
                result = run(
                    [str(work / "driver"), str(path), mode],
                    cwd=root, timeout=30, capture_output=True, text=True, check=True,
                    env=observer_environment(observer) if measured else None,
                )
                elapsed = time.monotonic() - started
                outputs.append(result.stdout)
                if measured:
                    record = re.fullmatch(
                        r"sync_timing calls=(\d+) nanoseconds=(\d+) other_calls=0\n",
                        result.stderr,
                    )
                    assert record, result.stderr
                    calls, nanoseconds = map(int, record.groups())
                    assert (calls == 0) == (mode == "allocation-capacity")
                    assert calls == 0 or nanoseconds > 0
                    print(mode, f"wall={elapsed:.6f}s", record[0].strip())
                else:
                    assert not result.stderr, result.stderr
                    print(mode, f"stock_wall={elapsed:.6f}s")
            assert outputs[0] == outputs[1], mode
PY
```

The existing caller checks its outcomes; the supervisor requires matching stdout,
a complete timing record, a zero-call control and positive observation of the
catalog calls. An observed run is diagnostic evidence, not another passing fault
campaign. Compare repeated timings and reverse run order before attributing a
small difference to the observer. Process startup, non-synchronization work and
cleanup outside the child are not included in the native-call sum. Concurrent
calls contribute summed durations, which can exceed process wall time.

A slow synchronization call locates time in the OS/storage path; it does not
identify which filesystem, virtualization or device layer caused the wait, nor
establish power-loss protection. The [timing investigation](../notes/evidence.md#verification-time-discrepancy)
records the tested environments and limits. Before comparing platforms, also
check the virtual-disk policy using the diagnostic below. Equal guest call counts
do not imply equal host synchronization guarantees.

## Compare virtual-disk synchronization guarantees

A guest `fsync` cannot provide a stronger guarantee than its virtual disk's host
policy. Apple Virtualization distinguishes [full synchronization](https://developer.apple.com/documentation/virtualization/vzdiskimagesynchronizationmode/full)
from [best-effort fsync](https://developer.apple.com/documentation/virtualization/vzdiskimagesynchronizationmode/fsync).
Use this optional diagnostic to change that policy while keeping the guest
kernel, program, CPU, memory, caching mode and initial disk bytes fixed.

The [host controller](fixtures/virtual-disk-sync-host.m) boots a Linux guest with
one CPU, 256 MiB, no network device and a read-only boot disk. The
[guest probe](fixtures/virtual-disk-sync-guest.c) runs as init to mount its private
devices, then measures with uid/gid 1000. Its raw-block workload separates 128
unchanged-data flushes from 128 aligned writes followed by flushes, with readback
checks. Its catalog workload runs the existing allocation caller's zero-sync,
healthy-catalog and prefix-zero refusal/healing controls. An entropy device keeps
initial random-seed waiting out of database startup. Boot and unmount are outside
the reported intervals; catalog intervals include fork, execution and wait.

Run on an Apple silicon Mac with the command-line SDK, codesign and Docker. Set
`PIPESQL_VM_IMAGE` to an already provisioned GNU arm64 image containing the pinned
Rust toolchain, Python, a static C toolchain, glibc and `mkfs.ext4`. Set
`PIPESQL_VM_KERNEL` to a local arm64 Linux kernel with virtio block/console/entropy,
devtmpfs and ext4 built in. This diagnostic does not download a kernel or image,
change Docker settings, or attach Docker's own disk. Each writable image below
is a new expendable file.

Run from the repository root. The command uses the existing library build helper,
requires every child to succeed, compares complete catalog output across modes,
reverses mode order on the middle repetition, and removes its container and files:

```sh
python3 -B - <<'PY'
from pathlib import Path
import os
import re
import shutil
import sys
import tempfile
import uuid

sys.path.insert(0, "tools")
from check_process import run

root = Path.cwd().resolve()
image = os.environ["PIPESQL_VM_IMAGE"]
kernel = Path(os.environ["PIPESQL_VM_KERNEL"]).resolve(strict=True)
container = "pipesql-sync-" + uuid.uuid4().hex[:12]

def checked(*command, **options):
    return run(list(map(str, command)), cwd=root, check=True, timeout=180, **options)

with tempfile.TemporaryDirectory(prefix="pipesql-virtual-sync-") as name:
    work = Path(name).resolve()
    started = False
    try:
        checked("docker", "image", "inspect", "--format", "{{.Id}}", image)
        checked("docker", "run", "-d", "--name", container,
                "--user", "1000:1000", "--cpus", "1", "--memory", "2g",
                "--memory-swap", "2g", "--network", "none",
                "-e", "CARGO_BUILD_JOBS=1", "-e", "RUSTFLAGS=-D warnings",
                "--mount", f"type=bind,source={root},target=/source,readonly",
                image, "sleep", "infinity")
        started = True
        checked("docker", "exec", "-w", "/source", container,
                "python3", "-B", "-c", r'''
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
sys.path.insert(0, "tools")
work = Path("/tmp/probe")
work.mkdir()
runpy.run_path("tools/check-diagnostic-allocation.py")["build_driver"](work)
boot = work / "root"
boot.mkdir()
for name in ("dev", "proc", "tmp"):
    (boot / name).mkdir()
subprocess.run(["cc", "-static", "-O2", "-Wall", "-Wextra", "-Wconversion",
                "-Werror", "tools/fixtures/virtual-disk-sync-guest.c",
                "-o", str(boot / "init")], check=True)
shutil.copy2(work / "driver", boot / "driver")
for name in ("/lib/ld-linux-aarch64.so.1",
             "/lib/aarch64-linux-gnu/libgcc_s.so.1",
             "/lib/aarch64-linux-gnu/libm.so.6",
             "/lib/aarch64-linux-gnu/libc.so.6"):
    destination = boot / name.lstrip("/")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(name, destination)
subprocess.run(["mkfs.ext4", "-q", "-F", "-d", str(boot),
                str(work / "boot.raw"), "65536"], check=True)
subprocess.run(["mkfs.ext4", "-q", "-F", str(work / "data.raw"), "32768"], check=True)
''')
        for image_name in ("boot.raw", "data.raw"):
            checked("docker", "cp", f"{container}:/tmp/probe/{image_name}", work / image_name)
        checked("cc", "-fobjc-arc", "-Wall", "-Wextra", "-Werror",
                "-framework", "Foundation", "-framework", "Virtualization",
                root / "tools/fixtures/virtual-disk-sync-host.m", "-o", work / "host")
        entitlement = work / "entitlements.plist"
        entitlement.write_text('<plist version="1.0"><dict>'
                               '<key>com.apple.security.virtualization</key>'
                               '<true/></dict></plist>')
        checked("codesign", "--force", "--sign", "-", "--entitlements", entitlement, work / "host")
        expected_catalog = None
        for repeat in range(3):
            modes = ("fsync", "full") if repeat != 1 else ("full", "fsync")
            for workload in ("raw", "catalog"):
                for mode in modes:
                    disk = work / f"{repeat}-{workload}-{mode}.raw"
                    if workload == "raw":
                        with disk.open("xb") as output:
                            output.truncate(16 * 1024 * 1024)
                    else:
                        shutil.copyfile(work / "data.raw", disk)
                    result = checked(work / "host", kernel, work / "boot.raw", disk,
                                     mode, workload, capture_output=True, text=True)
                    lines = result.stdout.splitlines()
                    assert lines.count("PROBE_OK") == 1, result
                    assert "IDENTITY uid=1000 gid=1000" in lines, result
                    assert not any("PROBE_ERROR" in line for line in lines), result
                    records = [line for line in lines if line.startswith(("RAW ", "CATALOG "))]
                    assert len(records) == (2 if workload == "raw" else 3), result
                    if workload == "catalog":
                        output = [line for line in lines if not line.startswith(
                            ("CATALOG ", "IDENTITY ", "PROBE_OK"))]
                        if expected_catalog is None:
                            expected_catalog = output
                        assert output == expected_catalog, (output, expected_catalog)
                    assert re.search(r"cache=2 sync=" + ("1" if mode == "full" else "2"),
                                     result.stderr), result
                    print(f"repeat={repeat} workload={workload} mode={mode}",
                          *records, sep="\n", flush=True)
                    disk.unlink()
        # An unformatted catalog disk must fail in the guest even when the VM
        # shuts down normally. A zero host exit code alone is insufficient.
        disk = work / "unformatted.raw"
        with disk.open("xb") as output:
            output.truncate(16 * 1024 * 1024)
        rejected = checked(work / "host", kernel, work / "boot.raw", disk,
                           "fsync", "catalog", capture_output=True, text=True)
        assert "PROBE_ERROR operation=mount-data" in rejected.stdout, rejected
        assert "PROBE_OK" not in rejected.stdout, rejected
        print("unformatted-disk rejection control passed", flush=True)
    finally:
        if started:
            checked("docker", "rm", "-f", container)
PY
```

Require the complete guest output and successful VM shutdown. A shutdown alone
can follow a failed guest check. These are controlled latency experiments, not
power-loss tests or additional passing fault campaigns. The raw-block test has
no guest filesystem; the catalog test uses ext4 directly, whereas Docker may add
a container filesystem layer. Neither provides a numerical prediction for an
entire gate or a general ranking of operating systems. The
[comparison record](../notes/evidence.md#virtual-disk-synchronization-root-cause)
connects the controlled result to the observed Docker configuration.

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
| `check-catalog-interruption.py` | Stock catalog append/recovery process-termination cuts for the original facts and composed-report histories; independent raw graph and grouped-answer checks. |
| `check-catalog-graph.py` | Independent persisted-graph checks, report payload demand and failed recovery, with structural and wrong-answer controls. |
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
run all nineteen cases; a one-byte attribution error must fail. BYTE_LENGTH and
CHAR_LENGTH each measure nullable Unicode text before and after analytic
spooling, with independent byte/scalar expectations, complete results,
requested/usable ownership and release checks. Numeric CAST runs before and after
analytic spooling, retaining exact DOUBLE values for integers 0–511. Calendar-year
extraction checks the DATE-to-INT64 result before and after the same spooling boundary.
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

The same selection runs `partial-result-shapes` from
[`result-ownership.rs`](fixtures/result-ownership.rs) at both pathname lengths.
Its ordered 257-row workload checks a 256-row prefix followed by addition
overflow, cancellation after a nonempty prefix, and complete healthy output.
Each phase uses the existing allocation-event observer: preparation, construction,
individual steps, repeated terminal state, owned-error transfer and prepared
release. Terminal steps must free real allocations; repeated terminal steps and
consuming the remaining result handle must allocate and free none. Owned errors
retain their category and literal span after the prepared plan drops.
Before execution, successful and failed logical-plan formatting into fixed caller
storage must each produce zero allocation and free events. The same observer's
nonzero preparation events serve as its positive control.
`complete_partial_results` requires all three distinct records, their row counts,
phase events, nonnegative requested/usable headroom and final release.
`partial-result-prefix-negative` changes the expected prefix count;
`partial-result-terminal-negative` leaves post-prefix steps unobserved. Both must
fail at their intended oracle. These checks extend observed histories without
claiming arbitrary allocator behavior or whole-process bounds.

The ownership selection runs `event-report-history` from the same caller at
both pathname lengths. Its sixteen typed events and literal thirteen-group answer
match the [report lesson](../docs/event-report.md). Three complete executions are
interleaved with partial-result drop and spill cancellation. Between the first
and second histories, preparation and constructor sweeps deny every allocation
prefix, including zero and a healthy full-prefix control. Errors remain live
while counters, reservations, descriptors and formatting are checked.
The existing observer measures successful allocations and owners immediately
before free, so balanced endpoint counters cannot hide early reservation release.
`complete_event_report` requires the complete prefix sequences, exact row counts,
phase order, nonnegative requested/usable headroom and balanced execution events.

Four controls must fail: `event-report-attribution-negative` counts an extra
resident owner; `event-report-preparation-negative` and
`event-report-construction-negative` omit observation for prefix one;
`event-report-terminal-negative` omits the cancelled terminal step. Retain these
controls alongside the existing combined-history native-reuse diagnostic. Passing
this report's bounded histories does not qualify arbitrary allocator reuse.

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
The composition campaign also builds the retained `event_report` example in its
owned work directory to seed typed report tables. Its expected rows are separate
literals; the example does not compute the campaign oracle.
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
