#!/usr/bin/env python3
"""Fresh public-library allocation refusal, bounded and supervised outside Cargo.

The allocator is unsafe caller scaffolding; engine unsafe_code=forbid remains.
Each cell first measures its expected-outcome allocation census, then denies after
EVERY permitted prefix, including zero and a full-prefix control. Workload cells
cross refusal with real construction permission failure and demanded corruption;
healed public reopen/retry checks follow the continuing-fault safety phase.
"""
from functools import partial
from pathlib import Path
import os
import hashlib
import re
import subprocess
import tempfile
import argparse
import sys

from check_process import run as run_process
from check_support import build_library, dependency


ROOT = Path(__file__).resolve().parent.parent


def parse_options(argv):
    arguments = argparse.ArgumentParser(description=__doc__)
    scope = arguments.add_mutually_exclusive_group()
    scope.add_argument(
        "--catalog-only",
        action="store_true",
        help="focused catalog development check; the default retains every existing cell",
    )
    scope.add_argument(
        "--catalog-recovery-only",
        action="store_true",
        help="focused public recovery/allocation check",
    )
    scope.add_argument(
        "--ownership-only",
        action="store_true",
        help="focused synchronized public ownership check",
    )
    scope.add_argument(
        "--pathname-only",
        action="store_true",
        help="focused Linux expanded-suffix scratch allocation check",
    )
    arguments.add_argument(
        "--controls-only",
        action="store_true",
        help="development census only; not a passing allocation sweep",
    )
    options = arguments.parse_args(argv)
    if options.pathname_only and sys.platform != "linux":
        arguments.error("expanded-suffix allocation checks require GNU/Linux")
    if not __debug__:
        arguments.error("allocation checks require Python assertions")
    return options


def catalog_allocation_limit():
    catalog_limit_match = re.search(
        r"^pub\(super\) const ALLOCATION_LIMIT: usize = (\d+);$",
        (ROOT / "tools/fixtures/catalog-allocation.rs").read_text(),
        re.MULTILINE,
    )
    assert (
        catalog_limit_match is not None
    ), "catalog caller must declare its allocation ceiling"
    return int(catalog_limit_match.group(1))


def build_driver(work):
    build_library(work)
    native = dependency(work / "target/release", "pipesql_filesystem")
    run_process(
        [
            "rustc",
            "--edition=2024",
            "-O",
            "-D",
            "warnings",
            "--extern",
            f"pipesql={work/'target/release/libpipesql.rlib'}",
            "--extern",
            f"pipesql_filesystem={native}",
            "-L",
            f"dependency={work/'target/release/deps'}",
            str(ROOT / "tools/fixtures/diagnostic-allocation.rs"),
            "-o",
            str(work / "driver"),
        ],
        cwd=ROOT,
        check=True,
        timeout=60,
    )
    for label, path in [
        ("rlib", work / "target/release/libpipesql.rlib"),
        ("driver", work / "driver"),
    ]:
        print(
            f"public allocation {label} sha256={hashlib.sha256(path.read_bytes()).hexdigest()}",
            flush=True,
        )
    print(
        f"public allocation RUSTFLAGS={os.environ.get('RUSTFLAGS', '')!r}",
        flush=True,
    )


def run_cell(work, failures, mode, label, database_bytes=None, mmap_threshold=None):
    root = work / label
    if database_bytes is not None:
        parent = root / ("x" * 200)
        parent.mkdir(parents=True)
        length = (
            database_bytes - len(os.fsencode(parent)) - 1 - len(b"/database")
        )
        assert 1 <= length <= 255
        root = parent / ("y" * length)
        assert len(os.fsencode(root / "database")) == database_bytes
    try:
        command = [str(work / "driver"), str(root), mode]
        if mode == "ownership":
            command = [
                "/usr/bin/time",
                "-l" if os.uname().sysname == "Darwin" else "-v",
                *command,
            ]
        environment = None
        if mmap_threshold is not None:
            assert sys.platform == "linux" and mode == "reader-allocation-shapes"
            environment = {
                **os.environ,
                "MALLOC_MMAP_THRESHOLD_": str(mmap_threshold),
                "GLIBC_TUNABLES": f"glibc.malloc.mmap_threshold={mmap_threshold}",
            }
        result = run_process(
            command, cwd=ROOT, capture_output=True, text=True, timeout=20,
            env=environment,
        )
    finally:
        # Timeout or assertion failure must not strand an owned directory ACL.
        if (
            mode.startswith("catalog-recover-permission")
            and (root / "database").exists()
        ):
            run_process(
                ["/bin/chmod", "-N", str(root / "database")],
                check=True,
                timeout=5,
                cwd=ROOT,
            )
    if result.returncode != 0:
        failures.append(
            f"{label}: exit={result.returncode}\n{result.stdout}{result.stderr}"
        )
    if mode.startswith(("filesystem-", "open-")):
        for name in ["CONTROL", "ROOT.A", "ROOT.B", "WAL"]:
            before = root / name
            after = root / "database" / name
            if (
                not before.is_file()
                or not after.is_file()
                or before.read_bytes() != after.read_bytes()
            ):
                failures.append(
                    f"{label}: missing/changed authoritative {name}"
                )
    if mode.startswith(("q1-", "q6-")):
        for index, name in enumerate(
            [
                "CONTROL",
                "ROOT.A",
                "ROOT.B",
                "WAL",
                "units/0000000000000001.unit",
            ]
        ):
            snapshot = (
                "fault" if (root / "fault-active").is_file() else "before"
            )
            before = root / f"{snapshot}-{index}"
            after = root / "database" / name
            if (
                not before.is_file()
                or not after.is_file()
                or before.read_bytes() != after.read_bytes()
            ):
                failures.append(
                    f"{label}: missing/changed query authority {name}"
                )
    return result


def check_ownership(work, run, failures):
    for label, length in [("short", None), ("path384", 384)]:
        analytic = run("analytic-shapes", f"analytic-{label}", length)
        print(analytic.stdout + analytic.stderr, end="", flush=True)
        if "analytic shapes passed: 11 cases; rows, attribution and release" not in analytic.stdout:
            failures.append(f"incomplete analytic allocation checks: {label}")
        prepared = run("prepared-aggregate-shapes", f"prepared-aggregates-{label}", length)
        print(prepared.stdout + prepared.stderr, end="", flush=True)
        if "prepared aggregate shapes passed: 14 accepted and 54 rejected; attribution, rows and release" not in prepared.stdout:
            failures.append(f"incomplete prepared aggregate allocation checks: {label}")
        legacy = run("legacy-constant-shapes", f"legacy-constants-{label}", length)
        print(legacy.stdout + legacy.stderr, end="", flush=True)
        if "legacy constant shapes passed: 6 direct and 4 extrema cases; rows, attribution and release" not in legacy.stdout:
            failures.append(f"incomplete legacy constant allocation checks: {label}")
    joined = run("joined-shapes", "joined-shapes")
    print(joined.stdout + joined.stderr, end="", flush=True)
    if "joined shapes passed: 2 budgets; complete rows, step ownership and release" not in joined.stdout:
        failures.append("incomplete joined allocation ownership checks")
    mixed = run("mixed-grouping-shapes", "mixed-grouping-shapes")
    print(mixed.stdout + mixed.stderr, end="", flush=True)
    if "mixed shapes completed: 40 cases, deficits=0" not in mixed.stdout:
        failures.append("incomplete mixed grouping allocation checks")
    grouped = run("grouped-allocation-shapes", "grouped-allocation-shapes")
    print(grouped.stdout + grouped.stderr, end="", flush=True)
    if "grouped allocation shapes passed: buffers=514 hash-layouts=91" not in grouped.stdout:
        failures.append("incomplete grouped allocation shape checks")
    reader_regimes = [("default", None)]
    if sys.platform == "linux":
        # Disable adaptive thresholds in these fresh callers: large payloads
        # exercise both mapped and arena-backed allocation paths.
        reader_regimes += [("mapped", 131_072), ("arena", 67_108_864)]
    for regime, threshold in reader_regimes:
        for label, length in [("short", None), ("path384", 384)]:
            readers = run(
                "reader-allocation-shapes", f"reader-{regime}-{label}", length,
                mmap_threshold=threshold,
            )
            print(f"reader allocator regime={regime} pathname={label}", flush=True)
            print(readers.stdout + readers.stderr, end="", flush=True)
            if threshold is not None and f"reader allocator threshold={threshold} observed" not in readers.stdout:
                failures.append(f"missing reader allocator control: {regime}/{label}")
            if "reader shapes passed: 12 fixed-width and 8 STRING ordering/distinct cases; rows, admission and release" not in readers.stdout:
                failures.append(f"incomplete typed reader allocation checks: {regime}/{label}")
    shapes = run("append-allocation-shapes", "append-allocation-shapes")
    print(shapes.stdout + shapes.stderr, end="", flush=True)
    if (
        "append allocation shapes passed: workspace=460865 references=4096" not in shapes.stdout
        or "append shapes passed: full-width maximum-column growth reuse publication release" not in shapes.stdout
    ):
        failures.append("incomplete append allocation shape census")
    # Killing only /usr/bin/time would leave this child holding the output
    # pipes open. Cleanup must close them and return within its own timeout.
    try:
        run_process(
            [
                "/usr/bin/time",
                "-l" if os.uname().sysname == "Darwin" else "-v",
                sys.executable,
                "-c",
                "import time; print('observed child live', flush=True); time.sleep(60)",
            ],
            cwd=ROOT,
            timeout=1,
            capture_output=True,
            text=True,
        )
    except subprocess.TimeoutExpired as error:
        if "observed child live" not in str(error.output):
            failures.append("timing timeout control never reached its child")
        else:
            print(
                "ownership timing timeout: child output closed after group cleanup",
                flush=True,
            )
    else:
        failures.append("timing timeout control unexpectedly completed")
    for label, length in [("short", None), ("path384", 384)]:
        result = run("ownership", f"ownership-{label}", length)
        print(result.stdout + result.stderr, end="", flush=True)
        if (
            "ownership overlap passed:" not in result.stdout
            or "ownership hash-held" not in result.stdout
        ):
            failures.append(
                f"ownership-{label}: missing composed ownership completion"
            )
    for mode, expected in [
        ("analytic-attribution-negative", "execution ownership attribution: analytic-admitted"),
        ("prepared-aggregate-attribution-negative", "prepared ownership attribution"),
        ("legacy-constant-attribution-negative", "execution ownership attribution: legacy-text"),
        ("joined-attribution-negative", "joined usable ownership attribution"),
        ("append-allocation-shapes-negative", "append allocation rounding"),
        ("ownership-negative", "complete-row oracle"),
        ("ownership-attribution-negative", "prepared ownership attribution"),
    ]:
        negative = run_process(
            [str(work / "driver"), str(work / mode), mode],
            capture_output=True,
            text=True,
            timeout=20,
            cwd=ROOT,
        )
        if negative.returncode == 0 or expected not in negative.stderr:
            failures.append(
                f"{mode}: expected rejection at {expected}: "
                f"{negative.stdout}{negative.stderr}"
            )
        else:
            print(f"{mode}: rejected by {expected}", flush=True)


def allocation_cells(options):
    # 383/384 deliberately straddle std's pinned C-string stack/heap threshold;
    # descendant paths cross it independently of their root. No long-path bypass.
    cells = [
        ("filesystem", "short", None),
        ("filesystem", "path383", 383),
        ("filesystem", "path384", 384),
        ("create", "short", None),
        ("create", "path384", 384),
        ("open", "short", None),
        ("open", "path384", 384),
        ("load", "short", None),
        ("load", "path384", 384),
        ("q1", "short", None),
        ("q1", "path384", 384),
        ("q6", "short", None),
        ("q6", "path384", 384),
        ("load-readonly", "short", None),
        ("load-readonly", "path384", 384),
        ("q1-corrupt", "short", None),
        ("q1-corrupt", "path384", 384),
        ("q6-corrupt", "short", None),
        ("q6-corrupt", "path384", 384),
    ]
    pathname_cells = [("create-expanded", "short", None), ("open-expanded", "short", None)]
    if sys.platform == "linux":
        cells += pathname_cells
    if options.pathname_only:
        return pathname_cells
    catalog_cells = [("catalog", "short", None), ("catalog", "path384", 384)]
    recovery_cells = [
        (f"catalog-recover-{kind}", label, length)
        for kind in ["empty", "data", "corrupt", "permission"]
        for label, length in [("short", None), ("path384", 384)]
    ]
    if os.uname().sysname != "Darwin":
        recovery_cells = [
            cell
            for cell in recovery_cells
            if cell[0] != "catalog-recover-permission"
        ]
        print(
            "repair-rename permission: Darwin ACL case unavailable; no corresponding platform claim",
            flush=True,
        )
    cells = (
        recovery_cells
        if options.catalog_recovery_only
        else (
            catalog_cells
            if options.catalog_only
            else cells + catalog_cells + recovery_cells
        )
    )
    return cells


def required_outcomes(operation):
    required = []
    if operation in {"create-expanded", "open-expanded"}:
        required = ["returned pathname scratch allocation refusal", "pathname scratch healed", f"returned healthy {operation}"]
    elif operation.startswith("catalog-recover-"):
        expected = (
            "expected"
            if operation
            in ["catalog-recover-corrupt", "catalog-recover-permission"]
            else "healthy"
        )
        required = [
            "returned catalog recovery allocation refusal",
            f"returned {expected} {operation}",
            "catalog recovery healed generation=",
        ]
        if operation == "catalog-recover-permission":
            required.append(
                "catalog repair replacement was not promoted to authority"
            )
    elif operation == "catalog":
        required = [
            "returned catalog allocation refusal",
            "returned healthy catalog",
            "catalog healed rows=",
            "catalog phase=order-execute",
            "catalog phase=joined-order-execute",
            "catalog phase=repeated-execute",
            "catalog phase=repeated-step",
            "catalog phase=derived-prepare",
            "catalog phase=derived-execute",
            "catalog phase=derived-step",
            "catalog phase=union-prepare",
            "catalog phase=union-execute",
            "catalog phase=union-step",
            "catalog phase=union-distinct-prepare",
            "catalog phase=union-distinct-execute",
            "catalog phase=union-distinct-step",
            "catalog phase=except-prepare",
            "catalog phase=except-execute",
            "catalog phase=except-step",
            "catalog phase=count-only-prepare",
            "catalog phase=count-only-execute",
            "catalog phase=count-only-step",
            "catalog phase=division-prepare",
            "catalog phase=division-execute",
            "catalog phase=division-step",
            "catalog phase=distinct-prepare",
            "catalog phase=distinct-execute",
            "catalog phase=distinct-step",
            "catalog query same-handle healed rows=4 distinct=3 writer=usable",
            "catalog query retained scan; scratch requires reopen",
        ]
    elif operation == "load":
        required = [
            "returned definite load allocation refusal",
            "returned load resource refusal",
            "returned load admission/issuance recovery debt",
            "returned load cleanup debt",
            "returned ambiguous load with original token",
            "returned healthy load",
        ]
    elif operation == "load-readonly":
        required = [
            "returned crossed permission/allocation cleanup debt",
            "returned expected load-readonly",
        ]
    elif operation.startswith(("q1", "q6")):
        required = [
            "returned typed query allocation refusal",
            "returned query resource refusal",
        ]
    return required


def check_allocation_prefixes(run, cells, catalog_limit, controls_only, failures):
    for operation, cell, length in cells:
        cell = f"{operation}-{cell}"
        result = run(f"{operation}-control", f"{cell}-control", length)
        census = re.search(
            rf"^{operation} allocations=(\d+)$", result.stdout, re.MULTILINE
        )
        if result.returncode != 0 or census is None:
            raise SystemExit(
                f"{cell}: control/census failed: {result.stdout}{result.stderr}"
            )
        allocations = int(census.group(1))
        bound = catalog_limit if operation == "catalog" else 128
        if allocations > bound:
            raise SystemExit(
                f"{cell}: exceeds {bound}-allocation campaign bound: {allocations}"
            )
        print(
            f"{cell}: allocation census={allocations}\ncontrol {cell}: {result.stdout.strip()}",
            flush=True,
        )
        observed = result.stdout
        # Prove that the fault-enabled caller admits the complete healthy prefix
        # before spending time on the refusal sweep.
        for prefix in (
            [] if controls_only else [allocations, *range(allocations)]
        ):
            label = f"{cell}-after-{prefix}"
            prior_failures = len(failures)
            result = run(f"{operation}-after-{prefix}", label, length)
            observed += result.stdout
            if result.returncode == 0:
                refusals = re.search(
                    rf"^{operation} refusals=(\d+)$", result.stdout, re.MULTILINE
                )
                expected_refusal = prefix < allocations
                if (
                    refusals is None
                    or (int(refusals.group(1)) > 0) != expected_refusal
                ):
                    failures.append(
                        f"{label}: allocation-fault coverage disagrees with census"
                    )
                if operation == "filesystem":
                    healthy = "returned not-found for unissued attempt"
                elif operation in [
                    "load-readonly",
                    "q1-corrupt",
                    "q6-corrupt",
                    "catalog-recover-corrupt",
                    "catalog-recover-permission",
                ]:
                    healthy = f"returned expected {operation}"
                else:
                    healthy = f"returned healthy {operation}"
                if not expected_refusal and healthy not in result.stdout:
                    failures.append(
                        f"{label}: full-allocation control did not complete"
                    )
                print(f"returned {label}: {result.stdout.strip()}", flush=True)
            if prefix == allocations and len(failures) != prior_failures:
                raise SystemExit(
                    "full-prefix preflight failed:\n"
                    + "\n".join(failures[prior_failures:])
                )
        required = required_outcomes(operation)
        for outcome in [] if controls_only else required:
            if outcome not in observed:
                failures.append(f"{cell}: required outcome not reached: {outcome}")


def main(argv=None):
    options = parse_options(argv)
    catalog_limit = catalog_allocation_limit()
    with tempfile.TemporaryDirectory(prefix="pipesql-diagnostic-gate-") as directory:
        work = Path(directory).resolve()
        build_driver(work)
        failures = []
        run = partial(run_cell, work, failures)
        native_result = run("mutex", "mutex")
        if (
            "native mutex contention passed without Rust allocation"
            not in native_result.stdout
        ):
            failures.append("missing native mutex contention evidence")
        print(native_result.stdout, end="", flush=True)

        if options.ownership_only or not (
            options.catalog_only
            or options.catalog_recovery_only
            or options.controls_only
            or options.pathname_only
        ):
            check_ownership(work, run, failures)
            if options.ownership_only:
                if failures:
                    raise SystemExit("\n".join(failures))
                print("public allocation: composed ownership cells passed", flush=True)
                raise SystemExit(0)

        for mode in (
            []
            if (options.catalog_only or options.catalog_recovery_only or options.pathname_only)
            else ["control", "deny", "format-control", "format-deny"]
        ):
            run(mode, mode)

        check_allocation_prefixes(
            run, allocation_cells(options), catalog_limit, options.controls_only, failures
        )
        if failures:
            raise SystemExit("public allocation regression:\n" + "\n".join(failures))
    if options.pathname_only and not options.controls_only:
        print("public allocation: expanded pathname allocation prefixes passed", flush=True)
        return
    print(
        "public allocation: controls only; no prefix-sweep claim"
        if options.controls_only
        else (
            "public allocation: selected recovery cells passed"
            if options.catalog_recovery_only
            else (
                "public allocation: selected cells passed (catalog only)"
                if options.catalog_only
                else "public allocation: diagnostics and short/long lifecycle/load/query/catalog/recovery refusal passed"
            )
        )
    )


if __name__ == "__main__":
    main()
