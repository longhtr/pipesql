#!/usr/bin/env python3
"""Exercise command failures and process cleanup with disposable Python children."""
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import check_process as processes

TOOLS = Path(__file__).resolve().parent

HEARTBEAT_CHILD = """
from pathlib import Path
import time

while True:
    Path('heartbeat').write_text(str(time.monotonic_ns()))
    time.sleep(.01)
"""

HEARTBEAT_PARENT = """
from pathlib import Path
import subprocess
import sys
import time

subprocess.Popen([sys.executable, '-B', '-c', sys.argv[1]])
while not Path('heartbeat').exists():
    time.sleep(.01)
"""


@unittest.skipUnless(os.name == "posix", "POSIX process-group implementation")
class ProcessOwnership(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="pipesql-process-test-")
        self.addCleanup(self.directory.cleanup)
        self.work = Path(self.directory.name).resolve()

    def close_test_group(self, pid):
        processes._signal_group(SimpleNamespace(pid=pid), signal.SIGKILL)

    def run_python(self, source, **options):
        return processes.run(
            [sys.executable, "-B", "-c", source],
            cwd=self.work,
            timeout=options.pop("timeout", 5),
            capture_output=True,
            **options,
        )

    def test_working_directory_environment_and_output(self):
        result = self.run_python(
            "import os; print(os.getcwd()); print(os.environ['CAMPAIGN_CASE'])",
            env={**os.environ, "CAMPAIGN_CASE": "a value with spaces"},
            text=True,
            check=True,
        )
        self.assertEqual(
            result.stdout.splitlines(), [str(self.work), "a value with spaces"]
        )
        self.assertEqual(result.stderr, "")

    def test_failed_command_preserves_status_and_both_outputs(self):
        with self.assertRaises(subprocess.CalledProcessError) as failed:
            self.run_python(
                "import sys; print('case input'); print('refused', file=sys.stderr); sys.exit(7)",
                check=True,
            )
        self.assertEqual(failed.exception.returncode, 7)
        self.assertEqual(failed.exception.stdout, b"case input\n")
        self.assertEqual(failed.exception.stderr, b"refused\n")

    def test_failed_launch_restores_signal_handler(self):
        previous = signal.getsignal(signal.SIGTERM)
        with self.assertRaises(FileNotFoundError):
            processes.run([str(self.work / "missing")], cwd=self.work, timeout=1)
        self.assertEqual(signal.getsignal(signal.SIGTERM), previous)

    def test_cleanup_failure_keeps_command_failure_and_closes_pipes(self):
        streams = []

        def failed_cleanup(child, terminate_timeout):
            self.assertEqual(child.returncode, 7)
            streams.extend([child.stdout, child.stderr])
            raise RuntimeError("cleanup failed")

        with patch.object(
            processes, "_close_group", side_effect=failed_cleanup
        ), self.assertRaisesRegex(RuntimeError, "cleanup failed") as failed:
            self.run_python("import sys; print('case input'); sys.exit(7)", check=True)
        command_error = failed.exception.__context__
        self.assertIsInstance(command_error, subprocess.CalledProcessError)
        self.assertEqual(command_error.returncode, 7)
        self.assertEqual(command_error.stdout, b"case input\n")
        self.assertTrue(all(stream.closed for stream in streams))

    def test_timeout_closes_a_grandchild_that_inherits_output_pipes(self):
        previous = signal.getsignal(signal.SIGTERM)
        source = (
            "import subprocess, sys, time; "
            "subprocess.Popen([sys.executable, '-c', "
            "\"import time; print('grandchild ready', flush=True); time.sleep(60)\"]); "
            "time.sleep(60)"
        )
        started = time.monotonic()
        with self.assertRaises(subprocess.TimeoutExpired) as timed_out:
            self.run_python(source, timeout=1)
        self.assertIn(b"grandchild ready", timed_out.exception.output)
        self.assertLess(time.monotonic() - started, 9)
        self.assertEqual(signal.getsignal(signal.SIGTERM), previous)

    def test_scope_failure_and_success_remove_lingering_children(self):
        # A heartbeat makes a surviving descendant observable independently of
        # the helper's return status or its internal process-group calls.
        command = [sys.executable, "-B", "-c", HEARTBEAT_PARENT, HEARTBEAT_CHILD]
        for fail, disable_cleanup in ((False, False), (True, False), (False, True)):
            with self.subTest(failure=fail, negative_control=disable_cleanup):
                heartbeat = self.work / "heartbeat"
                heartbeat.unlink(missing_ok=True)
                try:
                    real_signal = processes._signal_group
                    with patch.object(
                        processes,
                        "_signal_group",
                        side_effect=(
                            (lambda *args: None) if disable_cleanup else real_signal
                        ),
                    ):
                        with processes.owned_process(command, cwd=self.work) as child:
                            self.addCleanup(self.close_test_group, child.pid)
                            self.assertEqual(child.wait(timeout=5), 0)
                            if fail:
                                raise RuntimeError("caller failed")
                except RuntimeError as error:
                    self.assertTrue(fail)
                    self.assertEqual(str(error), "caller failed")
                before = heartbeat.read_text()
                time.sleep(0.1)
                after = heartbeat.read_text()
                if disable_cleanup:
                    self.assertNotEqual(
                        after, before, "negative control must expose a live child"
                    )
                    self.close_test_group(child.pid)
                else:
                    self.assertEqual(after, before)

    def test_sigterm_unwinds_and_closes_owned_child(self):
        import select

        source = (
            f"import sys; sys.path.insert(0, {str(TOOLS)!r}); import check_process; "
            "check_process.run([sys.executable, '-c', "
            '"import os, time; print(os.getpid(), flush=True); time.sleep(60)"], '
            f"cwd={str(self.work)!r}, timeout=60)"
        )
        with processes.owned_process(
            [sys.executable, "-B", "-c", source],
            cwd=self.work,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            terminate_timeout=10,
        ) as parent:
            ready, _, _ = select.select([parent.stdout], [], [], 5)
            self.assertTrue(ready)
            child_pid = int(parent.stdout.readline())
            self.addCleanup(self.close_test_group, child_pid)
            parent.terminate()
            stdout, stderr = parent.communicate(timeout=9)
            self.assertEqual(parent.returncode, 128 + signal.SIGTERM, (stdout, stderr))

    def test_outer_owner_allows_nested_forced_cleanup_to_finish(self):
        import select

        child_source = """
import os
import signal
import time
signal.signal(signal.SIGTERM, signal.SIG_IGN)
print(os.getpid(), flush=True)
time.sleep(60)
"""
        parent_source = (
            f"import sys; sys.path.insert(0, {str(TOOLS)!r}); import check_process; "
            f"check_process.run([sys.executable, '-c', {child_source!r}], "
            f"cwd={str(self.work)!r}, timeout=60)"
        )
        with self.assertRaisesRegex(RuntimeError, "outer interrupted"):
            with processes.owned_process(
                [sys.executable, "-B", "-c", parent_source],
                cwd=self.work,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                terminate_timeout=10,
            ) as parent:
                ready, _, _ = select.select([parent.stdout], [], [], 5)
                self.assertTrue(ready)
                child_pid = int(parent.stdout.readline())
                self.addCleanup(self.close_test_group, child_pid)
                raise RuntimeError("outer interrupted")
        self.assertEqual(parent.returncode, 128 + signal.SIGTERM)

    @patch.object(processes.subprocess, "Popen")
    def test_invalid_invocation_refuses_before_launch(self, launch):
        for command, options in (
            ("cargo build", {}),
            (["cargo", "build"], {"shell": True}),
            (["cargo", "build"], {"start_new_session": False}),
            (["cargo", "build"], {"capture_output": True, "stdout": subprocess.PIPE}),
        ):
            with self.subTest(command=command, options=options), self.assertRaises(
                ValueError
            ):
                processes.run(command, cwd=self.work, timeout=1, **options)
        for timeout in (0, -1, float("inf"), float("nan")):
            with self.subTest(timeout=timeout), self.assertRaises(ValueError):
                processes.run(["cargo", "build"], cwd=self.work, timeout=timeout)
        with self.assertRaises(ValueError):
            processes.run(["cargo", "build"], cwd="relative", timeout=1)
        launch.assert_not_called()

    def test_darwin_zombie_group_is_distinct_from_live_permission_failure(self):
        child = SimpleNamespace(pid=123)
        with patch.object(
            processes.os, "killpg", side_effect=PermissionError
        ), patch.object(processes.sys, "platform", "darwin"), patch.object(
            processes.subprocess, "run"
        ) as inspect:
            for states in ("123 Z\n", "456 S\n", ""):
                inspect.return_value.stdout = states
                processes._signal_group(child, signal.SIGKILL)
            for states in ("123 S\n", "123 Z\n123 R\n"):
                inspect.return_value.stdout = states
                with self.assertRaises(PermissionError):
                    processes._signal_group(child, signal.SIGKILL)
            inspect.side_effect = subprocess.TimeoutExpired("ps", 5)
            with self.assertRaises(subprocess.TimeoutExpired):
                processes._signal_group(child, signal.SIGKILL)


if __name__ == "__main__":
    unittest.main()
