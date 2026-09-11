#!/usr/bin/env python3
"""Check the filesystem boundary against the installed native SDK, not Cargo bindings."""
from pathlib import Path
import argparse
import tempfile

from check_process import run as run_process

ROOT = Path(__file__).resolve().parent.parent


def main(argv=None):
    argparse.ArgumentParser(description=__doc__).parse_args(argv)
    with tempfile.TemporaryDirectory(prefix="pipesql-abi-") as directory:
        binary = Path(directory) / "abi"
        run_process(
            [
                "cc",
                "-std=c11",
                "-pthread",
                "-Wall",
                "-Wextra",
                "-Werror",
                str(ROOT / "tools/fixtures/filesystem-abi.c"),
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
