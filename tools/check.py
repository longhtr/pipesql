#!/usr/bin/env python3
"""Run an ordered verification checkpoint against a private source export.

`stages` lists commands and deadlines; `execute` supervises them, stops at the
first failure and records each outcome. The full scope builds one compatible
stock artifact for its campaigns after ordinary Cargo tests; core omits those
campaigns. Neither scope may pass if source or shared artifacts changed, or
owned build cleanup failed. Receipts and logs remain in the new output directory.

Invoke through check.sh. Source discovery/export belongs to source-manifest.py,
process lifetime to check_process.py and stock build identity to check_support.py.
Case meanings and expected answers stay in their individual campaigns.
"""

import argparse
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import tempfile
import time

from check_process import run as run_process
from check_support import artifact_manifest, source_revision

ROOT = Path(__file__).resolve().parent.parent


@dataclass(frozen=True)
class Stage:
    name: str
    timeout: float
    command: list[str]


def stages(scope, output, root=ROOT):
    """Keep the gate's commands and deadlines visible in execution order."""
    python = [sys.executable, "-B"]
    fixtures = sorted(
        str(p.relative_to(root)) for p in (root / "tools/fixtures").glob("*.rs")
    )
    # Full host synchronization costs more than a VM disk using host fsync.
    # Keep finite supervision while allowing the measured storage-heavy paths.
    common = [
        Stage("rust-version", 30, ["rustc", "-vV"]),
        Stage("cargo-version", 30, ["cargo", "--version"]),
        Stage("native-compiler", 30, ["cc", "--version"]),
        Stage("rust-format", 60, ["cargo", "fmt", "--all", "--check"]),
        Stage(
            "caller-format", 60, ["rustfmt", "--edition", "2024", "--check", *fixtures]
        ),
        Stage("maintenance", 120, [*python, "tools/check-maintenance.py"]),
        Stage("filesystem-abi", 60, [*python, "tools/check-filesystem-abi.py"]),
        Stage(
            "rounding-vectors",
            60,
            [*python, "tools/aggregate-rounding-vectors.py", "--check"],
        ),
        Stage("attempt-identity", 120, [*python, "tools/models/attempt_identity.py"]),
        Stage(
            "attempt-publication", 120, [*python, "tools/models/attempt_publication.py"]
        ),
        Stage(
            "clippy",
            300,
            [
                "cargo",
                "clippy",
                "--offline",
                "--locked",
                "--release",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        Stage(
            "rust-tests",
            900,
            [
                "cargo",
                "test",
                "--offline",
                "--locked",
                "--release",
                "--workspace",
                "--all-targets",
                "--no-fail-fast",
                "--",
                "--test-threads=1",
            ],
        ),
        Stage(
            "rustdoc",
            180,
            [
                "cargo",
                "doc",
                "--offline",
                "--locked",
                "--release",
                "--workspace",
                "--no-deps",
            ],
        ),
        Stage(
            "doc-tests",
            180,
            [
                "cargo",
                "test",
                "--offline",
                "--locked",
                "--release",
                "--workspace",
                "--doc",
            ],
        ),
    ]
    # Stock features must not inherit the test target's dev-dependency features.
    binary = str(output / "stock/target/release/pipesql")
    native = [
        Stage(
            "stock-cli",
            180,
            [*python, "tools/check_support.py", str(output / "stock")],
        ),
        Stage(
            "aggregate-semantics",
            180,
            [*python, "tools/check-aggregate-semantics.py", binary],
        ),
        Stage(
            "aggregate-composition",
            480,
            [
                *python, "tools/check-composable-aggregates.py",
                binary, str(output / "composition"),
            ],
        ),
        Stage(
            "public-allocation", 1800, [*python, "tools/check-diagnostic-allocation.py"]
        ),
        Stage("cli-allocation", 600, [*python, "tools/check-cli-allocation.py"]),
        Stage(
            "native-initialization",
            180,
            [*python, "tools/check-native-initialization.py"],
        ),
        Stage("native-sync", 480, [*python, "tools/check-native-sync.py"]),
        Stage("native-io", 900, [*python, "tools/check-native-io.py"]),
        Stage(
            "catalog-interruption",
            600,
            [*python, "tools/check-catalog-interruption.py"],
        ),
        Stage("catalog-graph", 300, [*python, "tools/check-catalog-graph.py"]),
    ]
    return common + native if scope == "full" else common


source_tools = runpy.run_path(str(ROOT / "tools/source-manifest.py"))
source_manifest = source_tools["source_manifest"]
source_export = source_tools["source_export"]


def prepare_output(requested, root):
    if requested is None:
        output = Path(tempfile.mkdtemp(prefix="pipesql-gate-")).resolve()
        if root == output or root in output.parents:
            output.rmdir()
            raise ValueError(
                "temporary gate output must be outside the checkout; set --output"
            )
        return output
    if requested.is_symlink():
        raise FileExistsError(f"gate output already exists as a symlink: {requested}")
    output = requested.resolve()
    if output == root or root in output.parents:
        raise ValueError("gate output must be outside the checkout")
    output.mkdir()  # Never overwrite a previous observation.
    return output


def execute(steps, *, root, output, scope, revision=None):
    """Stop at the first failure; always record input integrity and cleanup."""
    target = output / "target"
    environment = {
        **os.environ,
        "CARGO_TARGET_DIR": str(target),
        "RUSTFLAGS": (os.environ.get("RUSTFLAGS", "") + " -D warnings").strip(),
        "RUSTDOCFLAGS": (os.environ.get("RUSTDOCFLAGS", "") + " -D warnings").strip(),
        "PYTHONDONTWRITEBYTECODE": "1",
    }
    environment.pop("PIPESQL_STOCK_BUILD", None)
    before = None
    stock_record = None
    receipt = {
        "scope": scope,
        "platform": sys.platform,
        "system": list(os.uname()),
        "python": sys.version,
        "root": str(root),
        "revision": revision,
        "rustflags": environment["RUSTFLAGS"],
        "rustdocflags": environment["RUSTDOCFLAGS"],
        "rustup_toolchain": environment.get("RUSTUP_TOOLCHAIN"),
        "cargo_build_target": environment.get("CARGO_BUILD_TARGET"),
        "termination_timeout_seconds": 30,
        "status": "failed",
        "stages": [],
    }
    complete = False
    try:
        if not steps:
            raise ValueError("a gate must contain verification stages")
        before = source_manifest(root)
        (output / "inputs-before.sha256").write_text(before)
        receipt["source_sha256"] = hashlib.sha256(before.encode()).hexdigest()
        for index, step in enumerate(steps, 1):
            log_name = f"{index:02d}-{step.name}.log"
            record = {
                "name": step.name,
                "command": step.command,
                "timeout": step.timeout,
                "log": log_name,
                "status": "failed",
            }
            receipt["stages"].append(record)
            print(f"check: {step.name} (log: {output / log_name})", flush=True)
            started = time.monotonic()
            try:
                with (output / log_name).open("x") as log:
                    result = run_process(
                        step.command,
                        cwd=root,
                        timeout=step.timeout,
                        terminate_timeout=30,
                        check=True,
                        env=environment,
                        stdout=log,
                        stderr=subprocess.STDOUT,
                    )
                if step.name == "stock-cli":
                    stock_record = json.loads((output / "stock/stock.json").read_text())
                    receipt["stock_build"] = stock_record
                    environment["PIPESQL_STOCK_BUILD"] = str(output / "stock")
                record.update(status="passed", returncode=result.returncode)
            except BaseException as error:
                record["error"] = str(error)
                record["error_type"] = type(error).__name__
                if isinstance(error, subprocess.CalledProcessError):
                    record["returncode"] = error.returncode
                raise
            finally:
                record["elapsed_seconds"] = round(time.monotonic() - started, 3)
            print(f"passed: {step.name} ({record['elapsed_seconds']}s)", flush=True)
        complete = True
    except BaseException as error:
        receipt["error"] = {"type": type(error).__name__, "message": str(error)}
        raise
    finally:
        finalization_errors = []
        try:
            after = source_manifest(root)
            (output / "inputs-after.sha256").write_text(after)
            receipt["inputs_unchanged"] = before is not None and before == after
            if before is None:
                finalization_errors.append("initial input manifest was not recorded")
            elif before != after:
                finalization_errors.append("build/gate inputs changed during the run")
        except Exception as error:
            finalization_errors.append(f"cannot verify final inputs: {error}")
        if stock_record is not None:
            try:
                if artifact_manifest(output / "stock/target/release") != stock_record["artifacts"]:
                    raise ValueError("shared stock artifacts changed during the gate")
            except Exception as error:
                finalization_errors.append(str(error))
        owned_outputs = [target, output / "stock", output / "composition"]
        if root == output / "source":
            owned_outputs.append(root)
        for owned in owned_outputs:
            try:
                if owned.exists():
                    shutil.rmtree(owned)
            except Exception as error:
                finalization_errors.append(f"cannot remove owned {owned.name}: {error}")
        receipt["finalization_errors"] = finalization_errors
        receipt["status"] = (
            "passed" if complete and not finalization_errors else "failed"
        )
        (output / "result.json").write_text(json.dumps(receipt, indent=2) + "\n")
        print(f"gate {receipt['status']}: {output / 'result.json'}", flush=True)
        if complete and finalization_errors:
            raise RuntimeError("; ".join(finalization_errors))
    return receipt


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        type=Path,
        help="new result directory outside the checkout; default: a retained temporary directory",
    )
    parser.add_argument(
        "--scope",
        choices=("full", "core"),
        default="full",
        help="core omits public/native campaigns and is not a complete gate",
    )
    options = parser.parse_args(argv)
    if not __debug__:
        parser.error("verification requires Python assertions")
    if sys.platform not in ("darwin", "linux"):
        parser.error(
            "native process ownership is currently implemented for macOS and Linux"
        )
    revision = source_revision(ROOT)
    output = prepare_output(options.output, ROOT)
    print(f"gate scope={options.scope} output={output}", flush=True)
    frozen = output / "source"
    try:
        source_export(ROOT, frozen)
        execute(
            stages(options.scope, output, frozen),
            root=frozen,
            output=output,
            scope=options.scope,
            revision=revision,
        )
    except subprocess.CalledProcessError as error:
        print(
            f"failed command: {error.cmd}; see {output / 'result.json'}",
            file=sys.stderr,
        )
        raise SystemExit(
            error.returncode if error.returncode > 0 else 128 - error.returncode
        )
    except subprocess.TimeoutExpired as error:
        print(f"timed out: {error.cmd}; see {output / 'result.json'}", file=sys.stderr)
        raise SystemExit(124)
    finally:
        if frozen.exists():
            shutil.rmtree(frozen)


if __name__ == "__main__":
    main()
