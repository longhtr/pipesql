#!/usr/bin/env python3
"""Check native layout and thread-stack premises using the installed C SDK.

Build and execute filesystem-abi.c and native-stack.c in temporary storage. Their
literal layout assertions and oversized-thread control are independent of Rust
bindings; no engine code runs. Invoke directly or through check.py. A compiler
error, failed assertion or timeout fails the check and releases its owned output.
"""
from pathlib import Path
import argparse
import tempfile

from check_process import run as run_process

ROOT = Path(__file__).resolve().parent.parent


def main(argv=None):
    argparse.ArgumentParser(description=__doc__).parse_args(argv)
    with tempfile.TemporaryDirectory(prefix="pipesql-abi-") as directory:
        for fixture in ["filesystem-abi", "native-stack"]:
            binary = Path(directory) / fixture
            run_process(
                [
                    "cc",
                    "-std=c11",
                    "-pthread",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    str(ROOT / f"tools/fixtures/{fixture}.c"),
                    "-o",
                    str(binary),
                ],
                check=True,
                timeout=30,
                cwd=ROOT,
            )
            run_process([str(binary)], check=True, timeout=10, cwd=ROOT)


if __name__ == "__main__":
    main()
