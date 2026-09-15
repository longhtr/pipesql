#!/usr/bin/env python3
"""Run stock queries against literal aggregate overflow and DOUBLE expectations.

An independent snapshot encoder supplies the input bits; cases.json supplies the
expected bits, NaN classification or overflow. Successful cases require one row
and complete CLI status records. Overflow requires the failing process status,
its diagnostic and no published row or success marker.

Run directly, optionally supplying an identified stock CLI path; otherwise build
one in owned temporary storage. Print each observation as JSON and fail on any
mismatch. The composition campaign covers combinations beyond these boundaries.
"""

import argparse
import hashlib
import json
from pathlib import Path
import runpy
import tempfile

from check_process import run as run_process
from check_support import build_cli, require_executable

ROOT = Path(__file__).resolve().parent.parent


def matches_case(case, result):
    """Compare one literal arithmetic expectation with its CLI observation."""
    expected = case["expected"]
    matched = False
    if expected == "overflow":
        matched = (
            result.returncode == 1
            and not any(
                line.startswith("row=") or line == "status=queried"
                for line in result.stdout.splitlines()
            )
            and "arithmetic overflow" in result.stderr
        )
    elif result.returncode == 0:
        lines = result.stdout.splitlines()
        completion = [
            line for line in lines if line.startswith(("row_count=", "status="))
        ]
        expected_completion = ["status=querying", "row_count=1", "status=queried"]
        if (
            completion != expected_completion
            or lines[:1] != expected_completion[:1]
            or lines[-2:] != expected_completion[1:]
        ):
            return False
        rows = [
            line[4:].split("|")
            for line in result.stdout.splitlines()
            if line.startswith("row=")
        ]
        if len(rows) == 1:
            value = rows[0][case["column"]].split(":")
            if len(value) == 3 and value[0] == "double":
                bits = int(value[2], 16)
                if expected == "nan":
                    matched = (
                        bits & 0x7FF0000000000000 == 0x7FF0000000000000
                        and bits & 0x000FFFFFFFFFFFFF != 0
                    )
                elif expected == "infinity":
                    matched = bits == 0x7FF0000000000000
                else:
                    matched = bits == int(expected, 16)
    return matched


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "stock_cli",
        nargs="?",
        type=Path,
        help="identified stock CLI; otherwise build in a fresh target",
    )
    options = parser.parse_args(argv)
    if not __debug__:
        parser.error("aggregate checks require Python assertions")
    if options.stock_cli is not None:
        options.stock_cli = require_executable(options.stock_cli.resolve())
    fixture = runpy.run_path(str(ROOT / "tools/snapshot-fixtures.py"))["write_snapshot"]
    cases = json.loads(
        (ROOT / "tests/fixtures/aggregate-semantics/cases.json").read_text()
    )
    assert len(cases) == 24

    with tempfile.TemporaryDirectory(
        prefix="pipesql-aggregate-semantics-"
    ) as temporary:
        work = Path(temporary)
        if options.stock_cli is not None:
            binary = options.stock_cli
        else:
            binary = build_cli(work) / "pipesql"
        identity = hashlib.sha256(binary.read_bytes()).hexdigest()
        print(f"aggregate artifact sha256={identity}", flush=True)
        failures = []
        for case in cases:
            database = work / case["name"]
            rows = [[int(value, 16) for value in row] for row in case["rows"]]
            assert 1 <= len(rows) <= 4 and all(len(row) == 4 for row in rows)
            fixture(
                database,
                [row + [ord("A"), ord("F"), 0] for row in rows],
            )
            query = (
                ROOT
                / (
                    "tests/fixtures/upstream/q1-upstream.pipe.sql"
                    if case["query"] == "q1"
                    else "tests/fixtures/q6.pipe.sql"
                )
            ).read_text()
            if case["query"] == "q6":
                query = query.replace("1994-01-01", "1970-01-01").replace("0.08", "1.0")
            query_path = work / (case["name"] + ".sql")
            query_path.write_text(query)
            result = run_process(
                [
                    str(binary),
                    "query",
                    "--database",
                    str(database),
                    "--memory-limit-bytes",
                    "2000000",
                    "--temp-limit-bytes",
                    "1000000",
                    "--query-file",
                    str(query_path),
                ],
                capture_output=True,
                text=True,
                timeout=30,
                cwd=ROOT,
            )
            expected = case["expected"]
            matched = matches_case(case, result)
            print(
                json.dumps(
                    {
                        "name": case["name"],
                        "expected": expected,
                        "matched": matched,
                        "exit": result.returncode,
                        "stdout": result.stdout,
                        "stderr": result.stderr,
                    }
                ),
                flush=True,
            )
            if not matched:
                failures.append(case["name"])
        print(json.dumps({"cases": len(cases), "failures": failures}), flush=True)
        if failures:
            raise SystemExit(1)
        assert (
            hashlib.sha256(binary.read_bytes()).hexdigest() == identity
        ), "CLI changed during campaign"


if __name__ == "__main__":
    main()
