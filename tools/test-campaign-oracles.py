#!/usr/bin/env python3
"""Challenge campaign interpretation without building or invoking the engine."""

from contextlib import redirect_stdout
import io
import hashlib
import json
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch

TOOLS = Path(__file__).resolve().parent
COMPOSITION = runpy.run_path(str(TOOLS / "check-composable-aggregates.py"))
ALLOCATION = runpy.run_path(str(TOOLS / "check-diagnostic-allocation.py"))
GRAPH = runpy.run_path(str(TOOLS / "check-catalog-graph.py"))


class CatalogSeed(unittest.TestCase):
    def test_seed_only_preserves_inspection_and_reports_artifact_identities(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            driver = work / "driver"
            driver.write_bytes(b"caller")
            library = work / "target/release/libpipesql.rlib"
            library.parent.mkdir(parents=True)
            library.write_bytes(b"library")
            seed = work / "seed"
            build = Mock(return_value=(b"source", driver))
            inspect = Mock(return_value=(seed, {}))
            remainder = Mock(side_effect=AssertionError("unexpected full campaign"))
            output = io.StringIO()
            with patch.dict(GRAPH["campaign"].__globals__, {
                "build_driver": build,
                "create_seed": inspect,
                "check_genesis_lease_and_fixture": remainder,
            }), redirect_stdout(output):
                GRAPH["campaign"](work, seed_only=True)
            build.assert_called_once_with(work)
            inspect.assert_called_once_with(work, driver)
            remainder.assert_not_called()
            summary = json.loads(output.getvalue().removeprefix("catalog seed passed: "))
            self.assertEqual(summary, {
                "source_manifest_sha256": hashlib.sha256(b"source").hexdigest(),
                "driver_sha256": hashlib.sha256(b"caller").hexdigest(),
                "library_sha256": hashlib.sha256(b"library").hexdigest(),
                "seed": str(seed),
            })

    def test_failed_seed_never_reports_success(self):
        failure = subprocess.CalledProcessError(101, ["driver", "seed", "setup"])
        output = io.StringIO()
        with patch.dict(GRAPH["campaign"].__globals__, {
            "build_driver": Mock(return_value=(b"source", Path("driver"))),
            "create_seed": Mock(side_effect=failure),
        }), redirect_stdout(output), self.assertRaises(subprocess.CalledProcessError) as raised:
            GRAPH["campaign"](Path("unused"), seed_only=True)
        self.assertIs(raised.exception, failure)
        self.assertEqual(output.getvalue(), "")


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
    def test_ownership_requires_joined_and_allocator_observations(self):
        marker = ("joined shapes passed: 2 budgets; complete rows, step ownership and release\n"
                  "analytic shapes passed: 11 cases; rows, attribution and release\n"
                  "wide set shapes passed: 6 cases; complete rows, step ownership and release")
        for output, missing in [("", True), (marker, False)]:
            failures = []
            run = Mock(return_value=subprocess.CompletedProcess([], 0, output, ""))
            # Exercise interpretation only; the timeout control and native
            # negative subprocesses have their own retained execution checks.
            def native(command, **options):
                if command[0] == "/usr/bin/time":
                    raise subprocess.TimeoutExpired(command, 1, output="observed child live")
                return subprocess.CompletedProcess(command, 1, "", "injected rejection")
            with self.subTest(output=output), patch.dict(
                ALLOCATION["check_ownership"].__globals__, {"run_process": native}
            ), patch.object(ALLOCATION["sys"], "platform", "linux"), redirect_stdout(io.StringIO()):
                ALLOCATION["check_ownership"](Path("unused"), run, failures)
            self.assertEqual("incomplete joined allocation ownership checks" in failures, missing)
            self.assertIn((("joined-shapes", "joined-shapes"), {}), run.call_args_list)
            self.assertIn((("analytic-shapes", "analytic-path384", 384), {}), run.call_args_list)
            self.assertIn((("wide-set-shapes", "wide-sets-path384", 384), {}), run.call_args_list)
            self.assertEqual(sum(message.startswith("incomplete wide set allocation checks:")
                                 for message in failures), 2 if missing else 0)
            self.assertEqual(sum(message.startswith("incomplete analytic allocation checks:")
                                 for message in failures), 2 if missing else 0)
            self.assertIn((("legacy-constant-shapes", "legacy-constants-path384", 384), {}),
                          run.call_args_list)
            self.assertEqual(sum(message.startswith("incomplete legacy constant allocation checks:")
                                 for message in failures), 2)
            self.assertIn((("prepared-aggregate-shapes", "prepared-aggregates-path384", 384), {}),
                          run.call_args_list)
            self.assertEqual(sum(message.startswith("incomplete prepared aggregate allocation checks:")
                                 for message in failures), 2)
            # A zero exit and other completion markers cannot stand in for
            # observing each explicitly selected native allocation regime.
            self.assertEqual(sum(message.startswith("missing reader allocator control:")
                                 for message in failures), 4)
            self.assertIn((("reader-allocation-shapes", "reader-mapped-path384", 384),
                           {"mmap_threshold": 131_072}), run.call_args_list)
            self.assertIn((("reader-allocation-shapes", "reader-arena-path384", 384),
                           {"mmap_threshold": 67_108_864}), run.call_args_list)

    def test_pathname_scope_runs_only_its_cells_and_common_mutex_control(self):
        run = Mock(return_value=subprocess.CompletedProcess(
            [], 0, "native mutex contention passed without Rust allocation\n", ""
        ))
        sweep = Mock()
        ownership = Mock(side_effect=AssertionError("unrequested ownership campaign"))
        with patch.object(ALLOCATION["sys"], "platform", "linux"), patch.dict(
            ALLOCATION["main"].__globals__, {
                "build_driver": Mock(),
                "catalog_allocation_limit": Mock(return_value=128),
                "run_cell": run,
                "check_allocation_prefixes": sweep,
                "check_ownership": ownership,
            }
        ), redirect_stdout(io.StringIO()):
            ALLOCATION["main"](["--pathname-only"])
        ownership.assert_not_called()
        self.assertEqual(run.call_count, 1)
        self.assertEqual(run.call_args.args[2:], ("mutex", "mutex"))
        self.assertEqual(sweep.call_args.args[1], [
            ("create-expanded", "short", None), ("open-expanded", "short", None)
        ])
        self.assertFalse(sweep.call_args.args[3])

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
