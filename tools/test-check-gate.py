#!/usr/bin/env python3
"""Verify gate receipts and isolation with disposable commands, without Cargo."""
from contextlib import redirect_stderr, redirect_stdout
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import check as gate


@unittest.skipUnless(os.name == "posix", "POSIX gate process ownership")
class GateOwnership(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="pipesql-gate-test-")
        self.addCleanup(self.directory.cleanup)
        self.work = Path(self.directory.name).resolve()
        self.root = self.work / "source"
        self.root.mkdir()
        for name in (
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "README.md",
            "THIRD_PARTY.md",
        ):
            (self.root / name).write_text("fixture\n")
        self.output = gate.prepare_output(self.work / "results", self.root)

    def stage(self, name, source, timeout=5):
        return gate.Stage(name, timeout, [sys.executable, "-B", "-c", source])

    def execute(self, stages):
        with redirect_stdout(io.StringIO()):
            return gate.execute(
                stages,
                root=self.root,
                output=self.output,
                scope="test",
                revision="fixture",
            )

    def receipt(self):
        return json.loads((self.output / "result.json").read_text())

    def test_success_records_commands_and_inputs_and_removes_only_owned_target(self):
        unrelated = self.work / "unrelated"
        unrelated.write_text("keep me")
        stage = self.stage(
            "environment",
            """
import os
from pathlib import Path
assert Path.cwd() == Path(os.environ['EXPECTED_ROOT'])
target = Path(os.environ['CARGO_TARGET_DIR'])
target.mkdir()
(target / 'artifact').write_text('disposable build output')
assert '-D warnings' in os.environ['RUSTFLAGS']
assert '-D warnings' in os.environ['RUSTDOCFLAGS']
print('observed output')
""",
        )
        with patch.dict(
            os.environ,
            {"EXPECTED_ROOT": str(self.root), "CARGO_TARGET_DIR": str(unrelated)},
        ):
            result = self.execute([stage])
        self.assertEqual(result["status"], "passed")
        self.assertTrue(result["inputs_unchanged"])
        self.assertEqual(result["revision"], "fixture")
        self.assertEqual(result["stages"][0]["command"], stage.command)
        self.assertEqual(result["stages"][0]["returncode"], 0)
        self.assertEqual(
            (self.output / "01-environment.log").read_text(), "observed output\n"
        )
        self.assertFalse((self.output / "target").exists())
        self.assertEqual(unrelated.read_text(), "keep me")
        self.assertEqual(
            (self.output / "inputs-before.sha256").read_bytes(),
            (self.output / "inputs-after.sha256").read_bytes(),
        )

    def test_failure_preserves_status_and_stops_before_next_stage(self):
        stages = [
            self.stage(
                "failure",
                "import sys; print('case failed', file=sys.stderr); sys.exit(7)",
            ),
            self.stage("must-not-run", "raise AssertionError('ran past failure')"),
        ]
        with self.assertRaises(subprocess.CalledProcessError) as failed:
            self.execute(stages)
        self.assertEqual(failed.exception.returncode, 7)
        result = self.receipt()
        self.assertEqual(result["status"], "failed")
        self.assertEqual(len(result["stages"]), 1)
        self.assertEqual(result["stages"][0]["returncode"], 7)
        self.assertIn("case failed", (self.output / "01-failure.log").read_text())

    def test_timeout_has_a_failed_receipt(self):
        with self.assertRaises(subprocess.TimeoutExpired):
            self.execute(
                [
                    self.stage(
                        "timeout",
                        "import time; print('ready', flush=True); time.sleep(60)",
                        timeout=0.3,
                    )
                ]
            )
        result = self.receipt()
        self.assertEqual(result["status"], "failed")
        self.assertEqual(result["stages"][0]["error_type"], "TimeoutExpired")
        self.assertIn("ready", (self.output / "01-timeout.log").read_text())

    @patch.object(gate, "run_process", side_effect=KeyboardInterrupt)
    def test_interruption_is_never_a_passing_gate(self, run):
        with self.assertRaises(KeyboardInterrupt):
            self.execute([self.stage("interrupted", "pass")])
        result = self.receipt()
        self.assertEqual(result["status"], "failed")
        self.assertEqual(result["stages"][0]["error_type"], "KeyboardInterrupt")

    def test_changed_input_invalidates_successful_commands(self):
        stage = self.stage(
            "mutation",
            "from pathlib import Path; Path('README.md').write_text('changed')",
        )
        with self.assertRaisesRegex(RuntimeError, "inputs changed"):
            self.execute([stage])
        result = self.receipt()
        self.assertEqual(result["status"], "failed")
        self.assertFalse(result["inputs_unchanged"])
        self.assertEqual(result["stages"][0]["status"], "passed")

    @patch.object(gate.shutil, "rmtree", side_effect=OSError("cleanup refused"))
    def test_cleanup_failure_invalidates_successful_commands(self, remove):
        stage = self.stage(
            "build",
            "import os; from pathlib import Path; Path(os.environ['CARGO_TARGET_DIR']).mkdir()",
        )
        with self.assertRaisesRegex(RuntimeError, "cleanup refused"):
            self.execute([stage])
        self.assertEqual(self.receipt()["status"], "failed")
        self.assertEqual(remove.call_args.args[0], self.output / "target")

    def test_existing_and_checkout_outputs_are_rejected(self):
        with self.assertRaises(FileExistsError):
            gate.prepare_output(self.output, self.root)
        with self.assertRaisesRegex(ValueError, "outside the checkout"):
            gate.prepare_output(self.root / "results", self.root)
        link = self.work / "output-link"
        link.symlink_to(self.work / "missing")
        with self.assertRaises(FileExistsError):
            gate.prepare_output(link, self.root)
        self.assertFalse((self.work / "missing").exists())

    def test_empty_gate_cannot_pass(self):
        with self.assertRaisesRegex(ValueError, "verification stages"):
            self.execute([])
        self.assertEqual(self.receipt()["status"], "failed")

    def test_temporary_output_cannot_bypass_checkout_exclusion(self):
        before = set(self.root.iterdir())
        with patch.object(
            gate.tempfile, "tempdir", str(self.root)
        ), self.assertRaisesRegex(ValueError, "outside the checkout"):
            gate.prepare_output(None, self.root)
        self.assertEqual(set(self.root.iterdir()), before)

    def test_command_line_preserves_failed_stage_exit_status(self):
        with patch.object(
            gate, "source_revision", return_value="fixture-revision"
        ), patch.object(gate, "prepare_output", return_value=self.output), patch.object(
            gate, "execute", side_effect=subprocess.CalledProcessError(7, ["fixture"])
        ), redirect_stdout(
            io.StringIO()
        ), redirect_stderr(
            io.StringIO()
        ), self.assertRaises(
            SystemExit
        ) as failed:
            gate.main(["--scope", "core"])
        self.assertEqual(failed.exception.code, 7)

    def test_invalid_source_has_a_failed_receipt(self):
        (self.root / "README.md").unlink()
        with self.assertRaises(FileNotFoundError):
            self.execute([self.stage("must-not-run", "pass")])
        result = self.receipt()
        self.assertEqual(result["status"], "failed")
        self.assertEqual(result["stages"], [])
        self.assertEqual(result["error"]["type"], "FileNotFoundError")

    def test_core_scope_cannot_include_or_replace_native_campaigns(self):
        core = gate.stages("core")
        full = gate.stages("full")
        self.assertEqual(full[: len(core)], core)
        self.assertEqual(
            {stage.name for stage in full[len(core) :]},
            {
                "aggregate-semantics",
                "aggregate-composition",
                "public-allocation",
                "cli-allocation",
                "native-initialization",
                "native-sync",
                "native-io",
                "catalog-interruption",
                "catalog-graph",
            },
        )
        self.assertIn("rust-tests", {stage.name for stage in core})
        self.assertIn("doc-tests", {stage.name for stage in core})
        for stage in full:
            self.assertGreater(stage.timeout, 0)
        caller_format = next(stage for stage in core if stage.name == "caller-format")
        self.assertIn("tools/fixtures/composed-ownership.rs", caller_format.command)


if __name__ == "__main__":
    unittest.main()
