#!/usr/bin/env python3
"""Check campaign discovery and argument handling without running the engine."""
from contextlib import ExitStack, contextmanager, redirect_stderr, redirect_stdout
import builtins
import importlib.util
import io
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parent
CAMPAIGNS = (
    "check.py",
    "check-aggregate-semantics.py",
    "check-composable-aggregates.py",
    "check-catalog-interruption.py",
    "check-cli-allocation.py",
    "check-diagnostic-allocation.py",
    "check-filesystem-abi.py",
    "check-native-initialization.py",
    "check-native-sync.py",
    "check-native-io.py",
    "check-catalog-graph.py",
    "catalog_graph.py",
)


@contextmanager
def forbid_campaign_work():
    """Reject process launches and campaign output creation, including on import."""
    with ExitStack() as stack:
        for target in (
            "subprocess.Popen",
            "subprocess.run",
            "subprocess.check_output",
            "os.system",
            "tempfile.TemporaryDirectory",
            "pathlib.Path.mkdir",
            "pathlib.Path.write_text",
            "pathlib.Path.write_bytes",
            "pathlib.Path.touch",
        ):
            stack.enter_context(patch(target, side_effect=AssertionError(target)))
        yield


@contextmanager
def without_unix_modules():
    """Expose accidental imports even when the test host provides Unix modules."""
    original_import = builtins.__import__

    def import_module(name, *args, **kwargs):
        if name in ("fcntl", "resource"):
            raise ModuleNotFoundError(name)
        return original_import(name, *args, **kwargs)

    with patch("builtins.__import__", side_effect=import_module):
        yield


def load_checker(name, *, optimize=0):
    spec = importlib.util.spec_from_file_location(name[:-3], TOOLS / name)
    module = importlib.util.module_from_spec(spec)
    exec(
        compile(
            (TOOLS / name).read_bytes(), str(TOOLS / name), "exec", optimize=optimize
        ),
        module.__dict__,
    )
    return module


class CampaignEntryPoints(unittest.TestCase):
    def test_import_does_not_start_a_campaign_or_parse_caller_arguments(self):
        with forbid_campaign_work(), without_unix_modules(), patch.object(
            sys, "argv", ["caller", "--unrelated"]
        ):
            for name in CAMPAIGNS:
                with self.subTest(checker=name):
                    self.assertTrue(callable(load_checker(name).main))

    def test_help_is_available_before_platform_checks_and_campaign_setup(self):
        with forbid_campaign_work(), without_unix_modules(), patch.object(
            sys, "platform", "win32"
        ), patch.object(sys, "dont_write_bytecode", True):
            for name in CAMPAIGNS:
                with self.subTest(checker=name):
                    output = io.StringIO()
                    with redirect_stdout(output), self.assertRaises(
                        SystemExit
                    ) as stopped:
                        load_checker(name).main(["--help"])
                    self.assertEqual(stopped.exception.code, 0)
                    self.assertIn("usage:", output.getvalue())

    def test_unknown_arguments_fail_before_campaign_setup(self):
        with forbid_campaign_work(), patch.object(sys, "dont_write_bytecode", True):
            for name in CAMPAIGNS:
                with self.subTest(checker=name), redirect_stderr(
                    io.StringIO()
                ), self.assertRaises(SystemExit) as stopped:
                    load_checker(name).main(["--unknown-campaign-option"])
                self.assertEqual(stopped.exception.code, 2)

    def test_conflicting_allocation_scopes_are_rejected(self):
        with forbid_campaign_work(), redirect_stderr(io.StringIO()), self.assertRaises(
            SystemExit
        ) as stopped:
            load_checker("check-diagnostic-allocation.py").main(
                ["--catalog-only", "--ownership-only"]
            )
        self.assertEqual(stopped.exception.code, 2)

    def test_conflicting_native_io_scopes_are_rejected(self):
        with forbid_campaign_work(), redirect_stderr(io.StringIO()), self.assertRaises(
            SystemExit
        ) as stopped:
            load_checker("check-native-io.py").main(
                ["--repeated-only", "--derived-only"]
            )
        self.assertEqual(stopped.exception.code, 2)

    def test_native_campaigns_refuse_unsupported_platforms(self):
        with forbid_campaign_work(), without_unix_modules():
            for name, platform in (
                ("check-native-initialization.py", "linux"),
                ("check-native-sync.py", "win32"),
                ("check-native-io.py", "win32"),
                ("check-catalog-graph.py", "win32"),
            ):
                with self.subTest(checker=name), patch.object(
                    sys, "platform", platform
                ), redirect_stderr(io.StringIO()), self.assertRaises(
                    SystemExit
                ) as stopped:
                    load_checker(name).main([])
                self.assertEqual(stopped.exception.code, 2)

    def test_inspector_refuses_unavailable_locking_before_reading_database(self):
        with forbid_campaign_work(), without_unix_modules(), patch.object(
            sys, "platform", "win32"
        ), patch.object(
            Path, "lstat", side_effect=AssertionError("database read")
        ), redirect_stderr(
            io.StringIO()
        ):
            self.assertEqual(
                load_checker("catalog_graph.py").main(["/missing/database"]), 2
            )

    def test_assertion_campaigns_refuse_optimized_python(self):
        with forbid_campaign_work(), redirect_stderr(io.StringIO()):
            for name in CAMPAIGNS:
                if name in ("catalog_graph.py", "check-filesystem-abi.py"):
                    continue
                with self.subTest(checker=name), self.assertRaises(
                    SystemExit
                ) as stopped:
                    load_checker(name, optimize=1).main([])
                self.assertNotEqual(stopped.exception.code, 0)

    @unittest.skipUnless(os.name == "posix", "POSIX symlink output")
    def test_graph_campaigns_do_not_follow_dangling_output_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            destination = root / "not-created"
            output = root / "output"
            output.symlink_to(destination, target_is_directory=True)
            for name in ("check-catalog-graph.py", "check-catalog-interruption.py"):
                checker = load_checker(name)
                with self.subTest(checker=name), patch.object(
                    sys, "platform", "darwin"
                ), patch.object(
                    checker, "campaign", side_effect=AssertionError("campaign started")
                ), self.assertRaises(
                    FileExistsError
                ):
                    checker.main(["--output", str(output)])
                self.assertFalse(destination.exists())

    def test_composition_requires_a_cli_and_new_work_directory_together(self):
        with forbid_campaign_work(), redirect_stderr(io.StringIO()), self.assertRaises(
            SystemExit
        ) as stopped:
            load_checker("check-composable-aggregates.py").main(["/missing/cli"])
        self.assertEqual(stopped.exception.code, 2)

    def test_missing_cli_is_rejected_before_campaign_output(self):
        with forbid_campaign_work():
            for name, arguments in (
                ("check-aggregate-semantics.py", ["/missing/cli"]),
                ("check-composable-aggregates.py", ["/missing/cli", "/new/cases"]),
            ):
                with self.subTest(checker=name), self.assertRaisesRegex(
                    ValueError, "artifact"
                ):
                    load_checker(name).main(arguments)

    def test_guard_detects_accidental_executable_work(self):
        for source in (
            "import subprocess; subprocess.run(['cargo', 'build'])",
            "import tempfile; tempfile.TemporaryDirectory()",
            "from pathlib import Path; Path('campaign').mkdir()",
        ):
            with self.subTest(source=source), forbid_campaign_work(), self.assertRaises(
                AssertionError
            ):
                exec(source, {"__name__": "accidental_import"})


if __name__ == "__main__":
    unittest.main()
