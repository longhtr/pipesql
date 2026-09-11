#!/usr/bin/env python3
"""Observe actual linked native sync calls and refusal; no power-loss claim."""
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
    dependency,
    rust_driver,
)

from check_process import run as run_process


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args(argv)
    if not __debug__:
        parser.error("native sync checks require Python assertions")
    if sys.platform not in ("darwin", "linux"):
        parser.error("native sync checks require macOS or Linux")
    with tempfile.TemporaryDirectory(prefix="pipesql-native-sync-") as temporary:
        work = Path(temporary).resolve()
        observer = native_library(work, "native-sync.c", "sync_probe")
        release = build_library(work)
        native = dependency(release, "pipesql_filesystem")
        rust_driver(
            work, release, "native-sync.rs", "sync_probe", ("pipesql_filesystem",)
        )
        for path in [
            observer,
            work / "driver",
            work / "target/release/libpipesql.rlib",
            native,
        ]:
            print(
                f"native sync {path.name} sha256={hashlib.sha256(path.read_bytes()).hexdigest()}",
                flush=True,
            )
        failures = []
        count = 0

        def run(mode, at, burst, error):
            nonlocal count
            cell = work / f"{mode}-{at}-{burst}-{error}"
            cell.mkdir()
            child = run_process(
                [
                    str(work / "driver"),
                    str(cell),
                    mode,
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
                f"{mode} {at=} {burst=} {error=} exit={child.returncode}\n{child.stdout}{child.stderr}",
                flush=True,
            )
            match = re.search(r"calls=(\d+) refused=(\d+) weak=(\d+)", child.stdout)
            if child.returncode != 0 or match is None:
                failures.append((mode, at, burst, error))
            count += 1
            return tuple(map(int, match.groups())) if match else None

        assert run("standard", 1, 3, errno.EINTR) == (4, 3, 0)
        for mode in ["file", "directory", "create", "load", "recover"]:
            healthy = run(mode, 0, 0, errno.EIO)
            assert healthy is not None and 0 < healthy[0] <= 128
            for at in range(1, healthy[0] + 1):
                for error in [errno.EINTR, errno.EIO, errno.ENOTSUP]:
                    for burst in [1, 3]:
                        observed = run(mode, at, burst, error)
                        if observed is None or observed[1] == 0:
                            failures.append(
                                (mode, at, burst, error, "fault not reached")
                            )
        assert run("query", 1, 3, errno.EINTR) == (0, 0, 0)
        if failures:
            raise SystemExit(f"native sync regressions: {failures}")
        print(
            f"native sync: {count} cells; single-attempt primitives, public outcomes and healed resolution/retry; no weaker fallback"
        )


if __name__ == "__main__":
    main()
