#!/usr/bin/env python3
"""Check documentation, Python tools, and independent fixtures without a Rust build."""

from pathlib import Path
import os
import subprocess
import sys

from check_process import run as run_process

ROOT = Path(__file__).resolve().parent.parent


def main():
    if not __debug__:
        raise SystemExit("maintenance checks require Python assertions")
    sources = sorted((ROOT / "tools").rglob("*.py"))
    for source in sources:
        compile(source.read_bytes(), str(source), "exec")
    print(f"Python syntax checked: {len(sources)} files", flush=True)
    checks = sorted(path.name for path in (ROOT / "tools").glob("test-*.py"))
    checks += ["check-docs.py", "source-manifest.py", "check-fixtures.py"]
    for name in checks:
        print(f"check: {name}", flush=True)
        run = run_process if os.name == "posix" else subprocess.run
        ownership = {"terminate_timeout": 15} if os.name == "posix" else {}
        run(
            [sys.executable, "-B", str(ROOT / "tools" / name)],
            cwd=ROOT,
            check=True,
            timeout=60,
            stdout=subprocess.DEVNULL if name == "source-manifest.py" else None,
            **ownership,
        )


if __name__ == "__main__":
    main()
