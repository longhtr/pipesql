#!/usr/bin/env python3
"""Check build orchestration without compiling or executing the engine."""
from pathlib import Path
import os
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import check_support as support


class SourceRevision(unittest.TestCase):
    @patch.object(support, "run_process")
    def test_revision_context_has_an_explicit_directory_and_timeout(self, run):
        run.return_value = subprocess.CompletedProcess("git", 0, "a" * 40 + "\n", "")
        self.assertEqual(support.source_revision(support.ROOT), "a" * 40)
        run.assert_called_once_with(
            ["git", "rev-parse", "--verify", "HEAD"],
            cwd=support.ROOT,
            timeout=30,
            capture_output=True,
            text=True,
        )

    @patch.object(support, "run_process")
    def test_export_and_missing_git_have_no_invented_revision(self, run):
        run.return_value = subprocess.CompletedProcess(
            "git", 128, "HEAD\n", "not a repository"
        )
        self.assertIsNone(support.source_revision(support.ROOT))
        run.side_effect = FileNotFoundError("git")
        self.assertIsNone(support.source_revision(support.ROOT))

    @patch.object(support, "run_process")
    def test_timeout_permissions_and_signals_are_not_missing_metadata(self, run):
        for error in (subprocess.TimeoutExpired("git", 30), PermissionError("git")):
            with self.subTest(error=type(error).__name__):
                run.side_effect = error
                with self.assertRaises(type(error)):
                    support.source_revision(support.ROOT)
        run.side_effect = None
        run.return_value = subprocess.CompletedProcess("git", -15, "", "")
        with self.assertRaises(subprocess.CalledProcessError):
            support.source_revision(support.ROOT)

    @patch.object(support, "run_process")
    def test_relative_source_is_rejected_before_launch(self, run):
        with self.assertRaisesRegex(ValueError, "absolute"):
            support.source_revision(Path("relative"))
        run.assert_not_called()


class BuildCommands(unittest.TestCase):
    def setUp(self):
        platform = patch.object(support.sys, "platform", "darwin")
        platform.start()
        self.addCleanup(platform.stop)
        self.directory = tempfile.TemporaryDirectory(prefix="pipesql-build-test-")
        self.addCleanup(self.directory.cleanup)
        self.work = Path(self.directory.name).resolve() / "owned campaign"
        self.work.mkdir()
        self.release = self.work / "target/release"

    def library_fixture(self):
        self.release.mkdir(parents=True, exist_ok=True)
        (self.release / "libpipesql.rlib").write_bytes(b"stock library fixture")

    def driver_inputs(self, observer="sync_probe"):
        self.library_fixture()
        (self.work / f"lib{observer}.dylib").write_bytes(b"observer fixture")

    def compiler_output(self, command, **options):
        output = Path(command[-1])
        self.assertEqual(output.read_bytes(), b"", "compiler must own a fresh name")
        output.write_bytes(b"compiled fixture")
        output.chmod(0o700)

    @patch.object(support, "run_process")
    def test_build_is_offline_locked_and_isolated(self, run):
        run.side_effect = lambda *args, **kwargs: self.library_fixture()
        self.assertEqual(support.build_library(self.work), self.release)
        run.assert_called_once_with(
            [
                "cargo",
                "build",
                "--release",
                "--offline",
                "--locked",
                "--lib",
                "--target-dir",
                str(self.work / "target"),
            ],
            cwd=support.ROOT,
            check=True,
            timeout=60,
        )

    @patch.object(support, "run_process")
    def test_cli_build_requires_both_stock_artifacts(self, run):
        run.side_effect = lambda *args, **kwargs: self.library_fixture()
        with self.assertRaisesRegex(ValueError, "pipesql$"):
            support.build_cli(self.work)
        self.assertNotIn("--lib", run.call_args.args[0])
        self.assertEqual(run.call_args.kwargs["timeout"], 90)

    @patch.object(support, "run_process")
    def test_success_without_compiler_output_is_rejected(self, run):
        with self.assertRaisesRegex(ValueError, "libpipesql.rlib"):
            support.build_library(self.work)
        with self.assertRaisesRegex(ValueError, "libsync_probe.dylib"):
            support.native_library(self.work, "native-sync.c", "sync_probe")

    @patch.object(support, "run_process")
    def test_existing_target_is_rejected_before_build(self, run):
        (self.work / "target").mkdir()
        with self.assertRaises(FileExistsError):
            support.build_library(self.work)
        run.assert_not_called()

    @patch.object(support, "run_process")
    def test_native_warning_and_install_name_contract(self, run):
        run.side_effect = self.compiler_output
        output = support.native_library(self.work, "native-sync.c", "sync_probe")
        command = run.call_args.args[0]
        self.assertEqual(output, self.work / "libsync_probe.dylib")
        self.assertIn("-Wconversion", command)
        self.assertIn("-Werror", command)
        self.assertIn(f"-Wl,-install_name,{output}", command)
        self.assertEqual(command[-2:], ["-o", str(output)])
        self.assertEqual(
            run.call_args.kwargs, {"cwd": support.ROOT, "check": True, "timeout": 30}
        )

    @patch.object(support, "run_process")
    def test_linux_observer_build_and_loading(self, run):
        run.side_effect = self.compiler_output
        with patch.object(support.sys, "platform", "linux"):
            work = self.work.parent / "linux-observer"
            work.mkdir()
            output = support.native_library(work, "native-io.c", "io_probe")
            command = run.call_args.args[0]
            self.assertEqual(output, work / "libio_probe.so")
            self.assertEqual(command[0], "cc")
            for flag in ("-fPIC", "-shared", "-D_GNU_SOURCE", "-ldl", "-Werror", "-Wl,-soname,libio_probe.so"):
                self.assertIn(flag, command)
            self.assertNotIn("-dynamiclib", command)
            self.assertEqual(support.observer_environment(output)["LD_PRELOAD"], str(output))

    def test_loader_paths_reject_ambiguous_inputs(self):
        with self.assertRaisesRegex(ValueError, "absolute"):
            support.observer_environment(Path("relative.so"))
        with patch.object(support.sys, "platform", "linux"):
            for name in ("space library.so", "colon:library.so"):
                library = self.work.parent / name
                library.write_bytes(b"observer fixture")
                with self.assertRaisesRegex(ValueError, "list separator"):
                    support.observer_environment(library)
        library = self.work / "observer.dylib"
        library.write_bytes(b"observer fixture")
        self.assertEqual(
            support.observer_environment(library)["DYLD_INSERT_LIBRARIES"], str(library)
        )

    @patch.object(support, "run_process")
    def test_unsupported_observer_target_refuses_before_output(self, run):
        with patch.object(support.sys, "platform", "win32"):
            with self.assertRaisesRegex(ValueError, "macOS or Linux"):
                support.native_library(self.work, "native-io.c", "io_probe")
        run.assert_not_called()
        self.assertEqual(list(self.work.iterdir()), [])

    @patch.object(support, "run_process")
    def test_driver_links_only_explicit_artifacts(self, run):
        self.driver_inputs()
        deps = self.release / "deps"
        deps.mkdir()
        dependency = deps / "libpipesql_filesystem-fixture.rlib"
        dependency.write_bytes(b"filesystem fixture")
        run.side_effect = self.compiler_output
        output = support.rust_driver(
            self.work,
            self.release,
            "native-sync.rs",
            "sync_probe",
            ("pipesql_filesystem",),
        )
        command = run.call_args.args[0]
        self.assertIn(f"pipesql={self.release / 'libpipesql.rlib'}", command)
        self.assertIn(f"pipesql_filesystem={dependency}", command)
        self.assertIn("dylib=sync_probe", command)
        self.assertEqual(command[-2:], ["-o", str(output)])
        self.assertEqual(
            run.call_args.kwargs, {"cwd": support.ROOT, "check": True, "timeout": 60}
        )

    @patch.object(support, "run_process")
    def test_missing_driver_inputs_refuse_before_output_creation(self, run):
        with self.assertRaisesRegex(ValueError, "libpipesql.rlib"):
            support.rust_driver(self.work, self.release, "native-sync.rs", "sync_probe")
        self.assertFalse((self.work / "driver").exists())
        run.assert_not_called()

    @patch.object(support, "run_process")
    def test_existing_compiler_output_is_not_overwritten(self, run):
        output = self.work / "libsync_probe.dylib"
        output.write_bytes(b"prior observation")
        with self.assertRaises(FileExistsError):
            support.native_library(self.work, "native-sync.c", "sync_probe")
        self.assertEqual(output.read_bytes(), b"prior observation")
        run.assert_not_called()

    @unittest.skipUnless(os.name == "posix", "POSIX symlink output")
    @patch.object(support, "run_process")
    def test_dangling_output_link_is_not_followed(self, run):
        output = self.work / "libsync_probe.dylib"
        output.symlink_to(self.work / "missing")
        with self.assertRaises(FileExistsError):
            support.native_library(self.work, "native-sync.c", "sync_probe")
        self.assertTrue(output.is_symlink())
        run.assert_not_called()

    def test_missing_empty_and_ambiguous_dependencies_refuse(self):
        deps = self.release / "deps"
        deps.mkdir(parents=True)
        with self.assertRaisesRegex(ValueError, "found 0"):
            support.dependency(self.release, "libc")
        first = deps / "liblibc-one.rlib"
        first.touch()
        with self.assertRaisesRegex(ValueError, "nonempty"):
            support.dependency(self.release, "libc")
        first.write_bytes(b"dependency fixture")
        self.assertEqual(support.dependency(self.release, "libc"), first)
        (deps / "liblibc-two.rlib").write_bytes(b"another fixture")
        with self.assertRaisesRegex(ValueError, "found 2"):
            support.dependency(self.release, "libc")

    @unittest.skipUnless(os.name == "posix", "POSIX execute permission")
    def test_executable_input_requires_execute_permission(self):
        executable = self.work / "cli"
        executable.write_bytes(b"CLI fixture")
        executable.chmod(0o600)
        with self.assertRaisesRegex(ValueError, "not executable"):
            support.require_executable(executable)
        executable.chmod(0o700)
        self.assertEqual(support.require_executable(executable), executable)

    @patch.object(
        support, "run_process", side_effect=subprocess.CalledProcessError(7, "cargo")
    )
    def test_failed_build_is_not_reported_as_success(self, run):
        with self.assertRaises(subprocess.CalledProcessError):
            support.build_library(self.work)
        run.assert_called_once()

    @patch.object(
        support, "run_process", side_effect=subprocess.TimeoutExpired("rustc", 60)
    )
    def test_timeout_propagates_to_the_campaign(self, run):
        self.driver_inputs("io_probe")
        with self.assertRaises(subprocess.TimeoutExpired):
            support.rust_driver(self.work, self.release, "native-io.rs", "io_probe")
        run.assert_called_once()


if __name__ == "__main__":
    unittest.main()
