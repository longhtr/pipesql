#!/usr/bin/env python3
"""Public create/reopen across per-call Darwin root observation and refusal."""
from pathlib import Path
import argparse
import errno
import hashlib
import os
import subprocess
import sys
import tempfile

from check_support import build_library, native_library, rust_driver

from check_process import run as run_process


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args(argv)
    if not __debug__:
        parser.error("native initialization checks require Python assertions")
    if sys.platform != "darwin":
        parser.error("native initialization checks require macOS")

    import resource

    def limits():
        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))

    with tempfile.TemporaryDirectory(prefix="pipesql-native-init-") as directory:
        work = Path(directory).resolve()
        dylib = native_library(work, "native-initialization.c", "native_probe")
        release = build_library(work)
        rust_driver(
            work, release, "native-initialization.rs", "native_probe", ("libc",)
        )
        for path in [dylib, work / "driver", work / "target/release/libpipesql.rlib"]:
            print(
                f"native initialization {path.name} sha256={hashlib.sha256(path.read_bytes()).hexdigest()}",
                flush=True,
            )
        count = 0
        for spelling in ["private", "data-mount", "symlink33"]:
            for mode in [1, 2, 3, 4]:
                for error in (
                    [errno.EIO]
                    if mode in [1, 3]
                    else [errno.EINTR, errno.EIO, errno.ENOMEM, errno.EACCES]
                ):
                    cell = work / f"{spelling}-{mode}-{error}"
                    cell.mkdir()
                    # Explicit mount spelling and the 33-link input are previously
                    # accepted inputs that defeated a proposed F_GETPATH replacement.
                    expected = (
                        str(cell)
                        if spelling == "private"
                        else "/System/Volumes/Data" + str(cell)
                    )
                    requested = expected
                    if spelling == "symlink33":
                        for index in range(33):
                            os.symlink(
                                f"s{index+1}" if index < 32 else ".", cell / f"s{index}"
                            )
                        requested += "/s0"
                    child = run_process(
                        [
                            str(work / "driver"),
                            requested,
                            expected,
                            str(mode),
                            str(error),
                        ],
                        env={**os.environ, "DYLD_INSERT_LIBRARIES": str(dylib)},
                        stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE,
                        preexec_fn=limits,
                        timeout=30,
                        cwd=work,
                    )
                    assert (
                        child.returncode == 0 and b"outcomes=checked" in child.stdout
                    ), (spelling, mode, error, child)
                    print(spelling, child.stdout.decode().strip(), flush=True)
                    count += 1
        print(
            f"native initialization: {count} fresh-process cells; public outcomes, names and healed bytes passed",
            flush=True,
        )


if __name__ == "__main__":
    main()
