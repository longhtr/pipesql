#!/usr/bin/env python3
"""Exercise stock creation across native path-resolution failures and overlap.

Build an identified native observer and stock-linked Rust caller. Fresh processes
cover platform pathname spellings, selected resolver entries and a delayed first
actor while a second proceeds. The caller checks resolved names, exact failures,
subsequent creation and byte-preserving reopen. Counts prove each boundary ran;
this is a finite native scheduling campaign, not general concurrency qualification."""

from pathlib import Path
import argparse
import errno
import hashlib
import os
import subprocess
import sys
import tempfile

from check_support import build_library, native_library, observer_environment, rust_driver

from check_process import run as run_process


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args(argv)
    if not __debug__:
        parser.error("native initialization checks require Python assertions")
    if sys.platform not in {"darwin", "linux"}:
        parser.error("native initialization checks require macOS or Linux")

    import resource

    def limits():
        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))

    with tempfile.TemporaryDirectory(prefix="pipesql-native-init-") as directory:
        work = Path(directory).resolve()
        observer = native_library(work, "native-initialization.c", "native_probe")
        release = build_library(work)
        rust_driver(
            work, release, "native-initialization.rs", "native_probe", ("libc",)
        )
        for path in [observer, work / "driver", release / "libpipesql.rlib"]:
            print(
                f"native initialization {path.name} sha256={hashlib.sha256(path.read_bytes()).hexdigest()}",
                flush=True,
            )
        count = 0
        spellings = (
            ["private", "data-mount", "symlink33"]
            if sys.platform == "darwin"
            else ["private", "symlink33", "suffix40"]
        )
        for spelling in spellings:
            sites = ["root"] if sys.platform == "darwin" else ["root", "component"]
            if sys.platform == "linux" and spelling != "private":
                sites.append("symlink")
            for site in sites:
                for mode in [1, 2, 3, 4]:
                    for error in (
                        [errno.EIO]
                        if mode in [1, 3]
                        else [errno.EINTR, errno.EIO, errno.ENOMEM, errno.EACCES]
                    ):
                        cell = work / f"{spelling}-{site}-{mode}-{error}"
                        cell.mkdir()
                        # Darwin preserves its explicit data-mount spelling. Both
                        # targets must resolve a 33-link chain to the same parent.
                        expected = (
                            str(cell)
                            if spelling == "private" or sys.platform == "linux"
                            else "/System/Volumes/Data" + str(cell)
                        )
                        requested = expected
                        if spelling in {"symlink33", "suffix40"}:
                            links = 40 if spelling == "suffix40" else 33
                            for index in range(links):
                                target = f"s{index+1}" if index < links - 1 else "."
                                if spelling == "suffix40":
                                    target += "/." * 1900
                                os.symlink(target, cell / f"s{index}")
                            requested += "/s0"
                        child = run_process(
                            [
                                str(work / "driver"),
                                requested,
                                expected,
                                str(mode),
                                str(error),
                                site,
                            ],
                            env=observer_environment(observer),
                            stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE,
                            preexec_fn=limits,
                            timeout=30,
                            cwd=work,
                        )
                        assert (
                            child.returncode == 0 and b"outcomes=checked" in child.stdout
                        ), (spelling, site, mode, error, child)
                        print(spelling, child.stdout.decode().strip(), flush=True)
                        count += 1
        print(
            f"native initialization: {count} fresh-process cells; public outcomes, names and healed bytes passed",
            flush=True,
        )


if __name__ == "__main__":
    main()
