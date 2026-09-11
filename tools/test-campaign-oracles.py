#!/usr/bin/env python3
"""Challenge campaign interpretation without building or invoking the engine."""

from contextlib import redirect_stdout
import io
from pathlib import Path
import runpy
import subprocess
import unittest
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parent
COMPOSITION = runpy.run_path(str(TOOLS / "check-composable-aggregates.py"))
ALLOCATION = runpy.run_path(str(TOOLS / "check-diagnostic-allocation.py"))


class GroupExpectations(unittest.TestCase):
    def test_empty_global_retains_count_and_null_sum(self):
        expected = COMPOSITION["expected_groups"](
            [], [], [("count", 0), ("sum", 0)], None
        )
        self.assertEqual(expected, [["int64:0", "null"]])

    def test_group_order_precedes_limit(self):
        raw = COMPOSITION["raw"]
        rows = [[raw(10.0), 66], [raw(2.0), 65], [raw(4.0), 65]]
        expected = COMPOSITION["expected_groups"](
            rows, [1], [("sum", 0), ("count", 0)], (1, 1)
        )
        self.assertEqual(expected, [["string:42", "4024000000000000", "int64:1"]])

    def test_query_checker_rejects_wrong_rows_and_incomplete_output(self):
        checker = COMPOSITION["QueryChecks"](Path("unused"), Path("unused"))
        for output in (
            "row=int64:2\nrow_count=1\nstatus=queried\n",
            "row=int64:1\nrow_count=1\n",
            "row=int64:1\nrow_count=0\nstatus=queried\n",
        ):
            with self.subTest(output=output), patch.object(
                checker, "run", return_value=subprocess.CompletedProcess([], 0, output, "")
            ), self.assertRaises(AssertionError):
                checker.composed("negative", "unused", [["int64:1"]])
        self.assertEqual(checker.observations, [])

    def test_query_checker_rejects_failed_process_even_with_complete_output(self):
        checker = COMPOSITION["QueryChecks"](Path("unused"), Path("unused"))
        result = subprocess.CompletedProcess(
            [], 1, "row=int64:1\nrow_count=1\nstatus=queried\n", "failure"
        )
        with patch.object(checker, "run", return_value=result), self.assertRaises(AssertionError):
            checker.composed("failed", "unused", [["int64:1"]])


class AllocationInterpretation(unittest.TestCase):
    def run_sweep(self, *, controls_only=False, full_prefix_healthy=True, refusals=True):
        modes = []
        failures = []

        def run(mode, label, length):
            modes.append(mode)
            if mode == "open-control":
                output = "open allocations=2\nreturned healthy open\n"
            else:
                prefix = int(mode.rsplit("-", 1)[1])
                output = f"open refusals={int(refusals and prefix < 2)}\n"
                if prefix == 2 and full_prefix_healthy:
                    output += "returned healthy open\n"
            return subprocess.CompletedProcess([], 0, output, "")

        with redirect_stdout(io.StringIO()):
            ALLOCATION["check_allocation_prefixes"](
                run, [("open", "short", None)], 128, controls_only, failures
            )
        return modes, failures

    def test_full_prefix_precedes_every_refusal_position(self):
        modes, failures = self.run_sweep()
        self.assertEqual(modes, ["open-control", "open-after-2", "open-after-0", "open-after-1"])
        self.assertEqual(failures, [])

    def test_controls_only_does_not_run_prefixes(self):
        modes, failures = self.run_sweep(controls_only=True)
        self.assertEqual(modes, ["open-control"])
        self.assertEqual(failures, [])

    def test_missing_full_prefix_completion_stops_sweep(self):
        with self.assertRaisesRegex(SystemExit, "full-prefix preflight failed"):
            self.run_sweep(full_prefix_healthy=False)

    def test_missing_refusals_fail_coverage(self):
        _, failures = self.run_sweep(refusals=False)
        self.assertEqual(len(failures), 2)
        self.assertTrue(all("coverage disagrees with census" in failure for failure in failures))


if __name__ == "__main__":
    unittest.main()
