#!/usr/bin/env python3
"""Challenge campaign interpretation without building or invoking the engine."""

from contextlib import redirect_stdout
import copy
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
INTERRUPTION = runpy.run_path(str(TOOLS / "check-catalog-interruption.py"))


class ReportHistory(unittest.TestCase):
    def test_interrupted_graph_requires_typed_rows_and_exact_history(self):
        graph = dict(issued=6, successes=[1, 2, 3, 4], generation=4, tables=[
            dict(id=1, name="events", columns=[
                dict(id=1, name="id", type=1, nullable=False),
                dict(id=2, name="dimension_id", type=1, nullable=True),
                dict(id=3, name="happened", type=4, nullable=True),
                dict(id=4, name="amount", type=1, nullable=True),
                dict(id=5, name="measurement", type=2, nullable=True),
            ], rows=copy.deepcopy(INTERRUPTION["REPORT_EVENTS"][:8])),
            dict(id=2, name="dimensions", columns=[
                dict(id=1, name="id", type=1, nullable=False),
                dict(id=2, name="label", type=3, nullable=False),
            ], rows=[[1, "north"], [2, "south"], [2, "南"], [3, ""]]),
        ])
        check = INTERRUPTION["check_report_graph"]
        check(graph, 1, False)
        for state, retried in [(0, False), (2, False), (1, True)]:
            with self.subTest(state=state, retried=retried), self.assertRaises(AssertionError):
                check(graph, state, retried)
        for path, value in [
            (["issued"], 5),
            (["successes"], [1, 2, 3, 4, 6]),
            (["generation"], 5),
            (["tables"], graph["tables"][:1]),
            (["tables", 0, "id"], 2),
            (["tables", 0, "columns", 2, "id"], 5),
            (["tables", 0, "columns", 2, "type"], 1),
            (["tables", 0, "columns", 2, "nullable"], False),
            (["tables", 0, "rows", 0, 2], 10957),
            (["tables", 0, "rows", 3, 4], {"double_bits": "0000000000000000"}),
            (["tables", 0, "rows"], graph["tables"][0]["rows"][:-1]),
            (["tables", 1, "rows"], [[1, "north"], [2, "south"], [3, ""]]),
        ]:
            altered = copy.deepcopy(graph)
            owner = altered
            for key in path[:-1]:
                owner = owner[key]
            owner[path[-1]] = value
            with self.subTest(path=path), self.assertRaises(AssertionError):
                check(altered, 1, False)
        healed = copy.deepcopy(graph)
        healed.update(issued=7, successes=[1, 2, 3, 4, 7], generation=5)
        healed["tables"][0]["rows"].append(copy.deepcopy(INTERRUPTION["REPORT_EVENTS"][8]))
        check(healed, 1, True)
        healed["tables"][0]["rows"][-1][3] = 2
        with self.assertRaises(AssertionError):
            check(healed, 1, True)


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
            "row=int64:1\nrow_count=10\nstatus=queried\n",
            "row=int64:1\nrow_count=1\nstatus=queried-extra\n",
            "row=int64:1\nrow_count=1\nrow_count=1\nstatus=queried\n",
            "row=int64:1\nrow_count=1\nstatus=queried\nstatus=queried\n",
        ):
            with self.subTest(output=output), patch.object(
                checker, "run", return_value=subprocess.CompletedProcess([], 0, output, "")
            ), self.assertRaises(AssertionError):
                checker.composed("negative", "unused", [["int64:1"]])
        self.assertEqual(checker.observations, [])

    def test_query_checker_records_exact_completion(self):
        checker = COMPOSITION["QueryChecks"](Path("unused"), Path("unused"))
        output = "row=int64:1\nrow_count=1\nstatus=queried\n"
        with patch.object(
            checker, "run", return_value=subprocess.CompletedProcess([], 0, output, "")
        ):
            checker.composed("valid", "unused", [["int64:1"]])
        self.assertEqual(checker.observations, [{
            "case": "valid", "rows": 1,
            "sha256": hashlib.sha256(output.encode()).hexdigest(),
        }])

    def test_query_checker_requires_exact_schema(self):
        checker = COMPOSITION["QueryChecks"](Path("unused"), Path("unused"))
        for columns in ("other:int64:required", "v:double:required", "v:int64:nullable", ""):
            output = f"columns={columns}\nrow=int64:1\nrow_count=1\nstatus=queried\n"
            with self.subTest(columns=columns), patch.object(
                checker, "run", return_value=subprocess.CompletedProcess([], 0, output, "")
            ), self.assertRaises(AssertionError):
                checker.composed("schema", "unused", [["int64:1"]], columns="v:int64:required")
        self.assertEqual(checker.observations, [])

    def test_query_checker_rejects_failed_process_even_with_complete_output(self):
        checker = COMPOSITION["QueryChecks"](Path("unused"), Path("unused"))
        result = subprocess.CompletedProcess(
            [], 1, "row=int64:1\nrow_count=1\nstatus=queried\n", "failure"
        )
        with patch.object(checker, "run", return_value=result), self.assertRaises(AssertionError):
            checker.composed("failed", "unused", [["int64:1"]])


class AllocationInterpretation(unittest.TestCase):
    def test_event_report_requires_every_prefix_and_terminal_event(self):
        def sample(allocations, frees):
            return (f"Samples {{ allocations: {allocations}, frees: {frees}, "
                    "requested_headroom: 16, usable_headroom: 16 }")

        lines = ["transient ownership calibration passed: hidden allocation detected; entry/exit agree"]
        for phase in ["preparation", "construction"]:
            for prefix in range(3):
                calls, refusals = (prefix + 1, 1) if prefix < 2 else (2, 0)
                lines.append(f"event report {phase} prefix={prefix} calls={calls} refusals={refusals} samples={sample(prefix, prefix)}")
            lines.append(f"event report {phase} passed: prefixes=0..=2; live errors and release")
        for history in range(3):
            for phase, allocations, frees in [
                ("prepare", 2, 1), ("partial drop", 3, 3), ("cancelled", 2, 2),
                ("execute/finish/drop", 3, 3), ("prepared drop", 0, 1),
            ]:
                lines.append(f"event report {phase}: {sample(allocations, frees)}")
            lines.append(f"event report history={history} rows=13 release=complete")
        lines.append("event report histories passed: 3 complete runs")
        valid = "\n".join(lines) + "\n"
        check = ALLOCATION["complete_event_report"]
        self.assertTrue(check(valid))
        for invalid in [
            "event report histories passed: 3 complete runs\n",
            valid.replace("prefix=1", "prefix=0"),
            valid.replace("refusals=1", "refusals=0"),
            valid.replace("allocations: 3, frees: 3", "allocations: 3, frees: 1"),
            valid.replace("requested_headroom: 16", "requested_headroom: -1"),
            valid.replace("usable_headroom: 16", "usable_headroom: -1"),
            valid.replace("rows=13", "rows=12"),
            valid.replace("history=2", "history=1"),
            valid.replace("event report cancelled:", "unobserved cancelled:"),
            valid.replace(lines[0], ""),
            valid + lines[-1] + "\n",
        ]:
            with self.subTest(output=invalid):
                self.assertFalse(check(invalid))

    def test_preparation_refusal_trace_requires_each_prefix_and_healthy_control(self):
        census = "join preparation census allocations=3\n"
        rows = [f"join preparation prefix={prefix} calls={min(prefix + 1, 3)} "
                f"refusals={int(prefix < 3)} samples=observed\n" for prefix in range(4)]
        complete = ("wide left join preparation failures passed: prefixes=0..=3; "
                    "live errors, owned span and release\n")
        check = ALLOCATION["complete_preparation_failures"]
        valid = census + "".join(rows) + complete
        self.assertTrue(check(valid))
        for invalid in [
            "", "".join(rows) + complete, census + "".join(rows),
            census + "".join(rows[1:]) + complete,
            census + "".join(rows[:-1]) + complete,
            census + "".join(rows[:2] + rows[1:]) + complete,
            census + "".join(reversed(rows)) + complete,
            valid.replace("prefix=1 calls=2 refusals=1", "prefix=1 calls=2 refusals=0"),
            valid.replace("prefix=3 calls=3 refusals=0", "prefix=3 calls=4 refusals=1"),
            valid.replace("prefixes=0..=3", "prefixes=0..=4"),
        ]:
            with self.subTest(output=invalid):
                self.assertFalse(check(invalid))

    def test_construction_refusal_trace_requires_owned_prefixes_and_control(self):
        census = "join construction census allocations=3\n"
        rows = ["join construction prefix=0 calls=1 refusals=1 samples=none\n"]
        rows += [f"join construction prefix={prefix} calls={min(prefix + 1, 3)} "
                 f"refusals={int(prefix < 3)} samples=Samples {{ allocations: {prefix}, frees: {prefix}, "
                 "requested_headroom: 32, usable_headroom: 16 }\n" for prefix in range(1, 4)]
        complete = ("wide left join construction failures passed: prefixes=0..=3; "
                    "live errors and release\n")
        valid = census + "".join(rows) + complete
        check = ALLOCATION["complete_construction_failures"]
        self.assertTrue(check(valid))
        for invalid in [
            "", "".join(rows) + complete, census + "".join(rows),
            census + "".join(rows[1:]) + complete,
            census + "".join(rows[:-1]) + complete,
            census + "".join(rows[:2] + rows[1:]) + complete,
            census + "".join(reversed(rows)) + complete,
            valid + complete, census + valid,
            valid.replace("prefix=1 calls=2 refusals=1", "prefix=1 calls=2 refusals=0"),
            valid.replace("prefix=3 calls=3 refusals=0", "prefix=3 calls=4 refusals=1"),
            valid.replace("prefixes=0..=3", "prefixes=0..=4"),
            valid.replace("samples=none", "samples=missing"),
            valid.replace("allocations: 1", "allocations: 0"),
            valid.replace("frees: 2", "frees: 1"),
            valid.replace("requested_headroom: 32", "requested_headroom: -1"),
            valid.replace("usable_headroom: 16", "usable_headroom: -1"),
        ]:
            with self.subTest(output=invalid):
                self.assertFalse(check(invalid))

    def test_execution_failure_trace_requires_every_demand_and_live_release(self):
        expressions = ["LOG10(ABS(r.id-3))", "SAFE_DIVIDE(1, LOG10(ABS(r.id-3)))",
                       "COALESCE(NULLIF(1, 1), LOG10(ABS(r.id-3)))"]
        rows = [f"wide left join demanded failure: expression={expression}; step=20; temporary=4096; "
                "Samples { allocations: 0, frees: 10, requested_headroom: 32, usable_headroom: 16 }\n"
                for expression in expressions]
        complete = ("wide left join execution failures passed: 3 demanded errors; "
                    "external work, owned spans and release\n")
        valid = "".join(rows) + complete
        check = ALLOCATION["complete_execution_failures"]
        self.assertTrue(check(valid))
        for invalid in [
            "", complete, "".join(rows), "".join(rows[1:]) + complete,
            "".join(rows[:-1]) + complete, "".join(reversed(rows)) + complete,
            valid + complete, rows[0] + valid,
            valid.replace("step=20", "step=1"),
            valid.replace("step=20", "step=200000"),
            valid.replace("temporary=4096", "temporary=0"),
            valid.replace("frees: 10", "frees: 0"),
            valid.replace("requested_headroom: 32", "requested_headroom: -1"),
            valid.replace("usable_headroom: 16", "usable_headroom: -1"),
        ]:
            with self.subTest(output=invalid):
                self.assertFalse(check(invalid))

    def test_partial_results_require_prefixes_and_every_terminal_observation(self):
        rows = [f"partial result case={name} rows={count} steps=4000 "
                "prepare_allocations=7 execute_allocations=26 terminal_frees=18 "
                "prepared_frees=2 requested_headroom=32 usable_headroom=16 release=complete\n"
                for name, count in [("overflow", 256), ("cancelled", 1), ("finished", 257)]]
        complete = "partial result ownership passed: 3 cases; rows, terminal events and release\n"
        check = ALLOCATION["complete_partial_results"]
        valid = "".join(rows) + complete
        self.assertTrue(check(valid))
        for invalid in [
            "", complete, "".join(rows), "".join(rows[1:]) + complete,
            "".join(rows[:-1]) + complete, "".join(reversed(rows)) + complete,
            valid + complete, rows[0] + valid,
            valid + rows[0].replace("case=overflow", "case=unknown"),
            valid + "partial result case=malformed\n",
            valid.replace("overflow rows=256", "overflow rows=255"),
            valid.replace("cancelled rows=1", "cancelled rows=0"),
            valid.replace("cancelled rows=1", "cancelled rows=257"),
            valid.replace("finished rows=257", "finished rows=256"),
            valid.replace("steps=4000", "steps=1"),
            valid.replace("steps=4000", "steps=20000"),
            valid.replace("prepare_allocations=7", "prepare_allocations=0"),
            valid.replace("execute_allocations=26", "execute_allocations=0"),
            valid.replace("terminal_frees=18", "terminal_frees=0"),
            valid.replace("prepared_frees=2", "prepared_frees=0"),
            valid.replace("requested_headroom=32", "requested_headroom=-1"),
            valid.replace("usable_headroom=16", "usable_headroom=-1"),
            valid.replace("release=complete", "release=incomplete"),
        ]:
            with self.subTest(output=invalid):
                self.assertFalse(check(invalid))

    def test_ownership_requires_joined_and_allocator_observations(self):
        marker = ("joined shapes passed: 2 budgets; complete rows, step ownership and release\n"
                  "analytic shapes passed: 19 cases; rows, attribution and release\n"
                  "wide set shapes passed: 6 cases; complete rows, step ownership and release\n"
                  "wide left join passed: 64 columns, 11 pairs; rows, ownership and release")
        calibration = "transient ownership calibration passed: hidden allocation detected; entry/exit agree"
        lifecycle = "wide left join lifecycle passed: preparation, finished release, two abandonments"
        for output, missing, missing_calibration, missing_lifecycle in [
            ("", True, True, True), (marker, False, True, True),
            (marker + "\n" + calibration, False, False, True),
            (marker + "\n" + calibration + "\n" + lifecycle, False, False, False)
        ]:
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
            self.assertIn((("partial-result-shapes", "partial-results-path384", 384), {}), run.call_args_list)
            self.assertIn((("event-report-history", "event-report-path384", 384), {}), run.call_args_list)
            self.assertEqual(sum(message.startswith("incomplete event report allocation histories:")
                                 for message in failures), 2)
            self.assertEqual(sum(message.startswith("incomplete partial result ownership checks:")
                                 for message in failures), 2)
            self.assertIn((("analytic-shapes", "analytic-path384", 384), {}), run.call_args_list)
            self.assertIn((("wide-set-shapes", "wide-sets-path384", 384), {}), run.call_args_list)
            self.assertIn((("wide-left-join-shape", "wide-left-join-path384", 384), {}), run.call_args_list)
            self.assertEqual(sum(message.startswith("incomplete wide left join ownership checks:")
                                 for message in failures), 2 if missing else 0)
            self.assertEqual(sum(message.startswith("incomplete transient ownership calibration:")
                                 for message in failures), 2 if missing_calibration else 0)
            self.assertEqual(sum(message.startswith("incomplete transient join lifecycle:")
                                 for message in failures), 2 if missing_lifecycle else 0)
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
            self.assertEqual(sum(message.startswith("incomplete join preparation refusal trace:")
                                 for message in failures), 2)
            self.assertEqual(sum(message.startswith("incomplete join execution failure trace:")
                                 for message in failures), 2)
            # A zero exit and other completion markers cannot stand in for
            # observing each explicitly selected native allocation regime.
            self.assertEqual(sum(message.startswith("missing reader allocator control:")
                                 for message in failures), 4)
            self.assertIn((("reader-allocation-shapes", "reader-mapped-path384", 384),
                           {"mmap_threshold": 131_072}), run.call_args_list)
            self.assertIn((("reader-allocation-shapes", "reader-arena-path384", 384),
                           {"mmap_threshold": 67_108_864}), run.call_args_list)

    def test_pathname_scope_requires_capacity_and_mutex_controls(self):
        capacity = ("allocation capacity passed: 3 preflight refusals; "
                    "empty, denied, exact and spare-capacity controls\n")
        mutex = "native mutex contention passed without Rust allocation\n"
        for capacity_output, capacity_exit, mutex_output, error in [
            (capacity, 0, mutex, None),
            ("", 0, mutex, "missing allocation capacity preflight evidence"),
            (capacity, 101, mutex, "allocation-capacity: exit=101"),
            (capacity, 0, "", "missing native mutex contention evidence"),
        ]:
            def execute(command, **options):
                output, status = {
                    "allocation-capacity": (capacity_output, capacity_exit),
                    "mutex": (mutex_output, 0),
                }[command[-1]]
                return subprocess.CompletedProcess(command, status, output, "")

            native = Mock(side_effect=execute)
            sweep = Mock()
            ownership = Mock(side_effect=AssertionError("unrequested ownership campaign"))
            with self.subTest(error=error), patch.object(
                ALLOCATION["sys"], "platform", "linux"
            ), patch.dict(ALLOCATION["main"].__globals__, {
                "build_driver": Mock(),
                "catalog_allocation_limit": Mock(return_value=128),
                "run_process": native,
                "check_allocation_prefixes": sweep,
                "check_ownership": ownership,
            }), redirect_stdout(io.StringIO()):
                if error is None:
                    ALLOCATION["main"](["--pathname-only"])
                else:
                    with self.assertRaisesRegex(SystemExit, error):
                        ALLOCATION["main"](["--pathname-only"])
            ownership.assert_not_called()
            self.assertEqual([call.args[0][-1] for call in native.call_args_list],
                             ["allocation-capacity", "mutex"])
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
