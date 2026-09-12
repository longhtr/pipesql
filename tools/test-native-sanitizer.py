#!/usr/bin/env python3
"""Challenge sanitizer interpretation and failure receipts without a compiler."""
from contextlib import redirect_stdout
import io
import json
import os
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest
from unittest.mock import patch

CHECK = runpy.run_path(str(Path(__file__).with_name("check-native-sanitizer.py")))


class Interpretation(unittest.TestCase):
    def result(self, code, stdout="", stderr=""):
        return subprocess.CompletedProcess(["control"], code, stdout, stderr)

    def test_clean_and_intended_fault_are_distinct_success_conditions(self):
        verify = CHECK["verify_control"]
        verify("clean", self.result(0, "address control clean\n"))
        verify("fault", self.result(86, stderr="ERROR: AddressSanitizer: heap-buffer-overflow"))
        for mode, result in [
            ("clean", self.result(0)),
            ("clean", self.result(0, "address control clean", "LeakSanitizer report")),
            ("fault", self.result(0, stderr="ERROR: AddressSanitizer: heap-buffer-overflow")),
            ("fault", self.result(86, stderr="ordinary assertion failed")),
            ("fault", self.result(86, stderr="ERROR: AddressSanitizer: stack-buffer-overflow")),
        ]:
            with self.subTest(mode=mode, result=result), self.assertRaises(ValueError):
                verify(mode, result)

    def test_empty_missing_duplicate_and_ignored_tests_are_rejected(self):
        lines = [f"test {name} ... ok" for name in sorted(CHECK["MUTEX_TESTS"])]
        summary = "test result: ok. 4 passed; 0 failed; 0 ignored;"
        CHECK["verify_tests"]("\n".join([*lines, summary]))
        for text in ["", summary, "\n".join(lines),
                     "\n".join([*lines[:-1], summary]),
                     "\n".join([*lines, lines[0], summary]),
                     "\n".join([*lines, summary.replace("0 ignored", "1 ignored")])]:
            with self.subTest(text=text), self.assertRaises(ValueError):
                CHECK["verify_tests"](text)

    def test_executable_requires_one_owned_cargo_artifact(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory).resolve() / "target"
            target.mkdir()
            binary = target / "test"
            binary.write_bytes(b"executable fixture")
            binary.chmod(0o700)
            event = {"reason": "compiler-artifact", "target": {"name": "pipesql_filesystem"},
                     "profile": {"test": True}, "executable": str(binary)}
            line = json.dumps(event)
            self.assertEqual(CHECK["test_executable"](line, target), binary)
            for text in ["", line + "\n" + line,
                         json.dumps({**event, "executable": str(target.parent / "other")})]:
                with self.subTest(text=text), self.assertRaises(ValueError):
                    CHECK["test_executable"](text, target)

    def test_environment_cannot_silently_disable_instrumentation(self):
        with patch.dict(os.environ, {"RUSTFLAGS": "ambient", "ASAN_OPTIONS": "exitcode=0",
                                    "RUSTC_WRAPPER": "wrapper", "LD_PRELOAD": "inject"}):
            env = CHECK["environment"](Path("/owned"), True)
        self.assertEqual(env["ASAN_OPTIONS"], CHECK["ASAN_OPTIONS"])
        self.assertEqual(env["RUSTFLAGS"], "-Dwarnings -Zsanitizer=address")
        self.assertNotIn("RUSTC_WRAPPER", env)
        self.assertNotIn("LD_PRELOAD", env)

    def failed_run(self, outcome):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory).resolve() / "result"
            globals_ = CHECK["main"].__globals__
            with patch.dict(globals_, {
                "source_revision": lambda root: "fixture",
                "run_process": outcome,
            }), redirect_stdout(io.StringIO()):
                with self.assertRaises((ValueError, subprocess.TimeoutExpired)):
                    CHECK["main"](["--toolchain", "nightly", "--output", str(output)])
            receipt = json.loads((output / "result.json").read_text())
            self.assertEqual(receipt["status"], "failed")
            self.assertTrue(receipt["inputs_unchanged"])
            self.assertFalse(receipt["finalization_errors"])
            self.assertFalse((output / "build").exists())
            return receipt, (output / "native-compiler.stderr").read_text()

    def test_subprocess_failure_is_retained_with_cleanup(self):
        receipt, stderr = self.failed_run(lambda *a, **k: self.result(7, stderr="compiler failed"))
        self.assertEqual(receipt["commands"][0]["returncode"], 7)
        self.assertEqual(stderr, "compiler failed")

    def test_timeout_preserves_partial_output_and_cleanup(self):
        def timeout(*args, **kwargs):
            raise subprocess.TimeoutExpired(["compiler"], 30, output=b"partial", stderr=b"timed out")
        receipt, stderr = self.failed_run(timeout)
        self.assertTrue(receipt["commands"][0]["timed_out"])
        self.assertEqual(stderr, "timed out")


if __name__ == "__main__":
    unittest.main()
