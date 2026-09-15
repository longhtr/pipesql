#!/usr/bin/env python3
"""Check native byte-I/O failures and partial transfers through stock public calls.

An identified observer counts regular-file read/write/pread/pwrite calls. Healthy
controls require the expected call kinds, so a spill campaign cannot silently
become memory-only. Fresh processes then refuse observed positions, shorten real
transfers and fail their continuations; the caller checks outcomes and reuse.
Focused modes cover their named workloads only. --controls-only reports a census,
not the failure campaign. Subprocess supervision and builds use shared helpers."""
from pathlib import Path
import argparse
import errno
import hashlib
import re
import sys
import tempfile

from check_support import (
    build_library,
    native_library,
    observer_environment,
    rust_driver,
)

from check_process import run as run_process


def main(argv=None):
    arguments = argparse.ArgumentParser(description=__doc__)
    scope = arguments.add_mutually_exclusive_group()
    scope.add_argument(
        "--repeated-only",
        action="store_true",
        help="focused repeated-aggregation campaign",
    )
    scope.add_argument(
        "--derived-only", action="store_true", help="focused derived-join campaign"
    )
    arguments.add_argument(
        "--controls-only",
        action="store_true",
        help="I/O census only; not failure coverage",
    )
    options = arguments.parse_args(argv)
    if not __debug__:
        arguments.error("native I/O checks require Python assertions")
    if sys.platform not in ("darwin", "linux"):
        arguments.error("native I/O checks require macOS or Linux")
    with tempfile.TemporaryDirectory(prefix="pipesql-native-io-") as temporary:
        work = Path(temporary).resolve()
        observer = native_library(work, "native-io.c", "io_probe")
        release = build_library(work)
        rust_driver(work, release, "native-io.rs", "io_probe", ())
        for path in [observer, work / "driver", release / "libpipesql.rlib"]:
            print(
                f"native I/O {path.name} sha256={hashlib.sha256(path.read_bytes()).hexdigest()}",
                flush=True,
            )
        failures = []
        count = 0

        def run(mode, kind, at, burst, error):
            nonlocal count
            cell = work / f"{mode}-{kind}-{at}-{burst}-{error}"
            cell.mkdir()
            child = run_process(
                [
                    str(work / "driver"),
                    str(cell),
                    mode,
                    str(kind),
                    str(at),
                    str(burst),
                    str(error),
                ],
                env=observer_environment(observer),
                text=True,
                capture_output=True,
                timeout=20,
                cwd=work,
            )
            print(
                f"{mode} {kind=} {at=} {burst=} {error=} exit={child.returncode}\n{child.stdout}{child.stderr}",
                flush=True,
            )
            match = re.search(r"calls=(\d+) refused=(\d+) partial=(\d+)", child.stdout)
            if child.returncode != 0 or match is None:
                failures.append((mode, kind, at, burst, error))
            count += 1
            return tuple(map(int, match.groups())) if match else None

        for kind in range(4):
            assert run("standard", kind, 1, 3, errno.EINTR) == (4, 3, 0)
        partial_mask = 0
        expected_kinds = {
            # Creation only reads and writes initial files. Recovery's positioned
            # fence readback remains covered by load/recover, not fresh creation.
            "create": 3,
            "load": 15,
            "recover": 7,
            "q6": 5,
            "q1": 5,
            "repeated": 12,
            "derived": 12,
        }
        if options.repeated_only:
            expected_kinds = {"repeated": 12}
        if options.derived_only:
            expected_kinds = {"derived": 12}
        for mode in expected_kinds:
            for kind in range(4):
                healthy = run(mode, kind, 0, 0, errno.EIO)
                assert healthy is not None and healthy[0] <= 256
                assert (healthy[0] > 0) == bool(expected_kinds[mode] & (1 << kind)), (
                    mode,
                    kind,
                    healthy,
                )
                print(
                    f"native I/O census {mode} {kind=} calls={healthy[0]}", flush=True
                )
                if options.controls_only:
                    continue
                for at in range(1, healthy[0] + 1):
                    for error in [errno.EINTR, errno.EIO, 0]:
                        for burst in [1, 3]:
                            observed = run(mode, kind, at, burst, error)
                            if observed is None or (error != 0 and observed[1] == 0):
                                failures.append(
                                    (mode, kind, at, error, "fault not reached")
                                )
                            if observed is not None and error == 0 and observed[2] > 0:
                                partial_mask |= 1 << kind
                if healthy[0] > 0:
                    # At the first observed call each fixture has nonempty file data.
                    # First make real byte progress, then refuse its continuation.
                    for error in [-errno.EINTR, -errno.EIO]:
                        for burst in [1, 3]:
                            crossed = run(mode, kind, 1, burst, error)
                            if crossed is None or crossed[1] == 0 or crossed[2] == 0:
                                failures.append(
                                    (
                                        mode,
                                        kind,
                                        error,
                                        "partial/error crossing not reached",
                                    )
                                )
        if not options.controls_only:
            expected_partial = (
                12 if options.repeated_only or options.derived_only else 15
            )
            assert (
                partial_mask == expected_partial
            ), "every selected positive-partial I/O path must run"
        if failures:
            raise SystemExit(f"native I/O regressions: {failures}")
        if options.controls_only:
            print(f"native I/O: {count} controls; no failure-coverage claim")
        else:
            print(
                f"native I/O: {count} cells; all observed public I/O positions, error propagation, positive partial progress and healed outcomes checked"
            )


if __name__ == "__main__":
    main()
