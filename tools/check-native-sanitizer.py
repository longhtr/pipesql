#!/usr/bin/env python3
"""Qualify AddressSanitizer observations for native mutexes or pathname handling.

The nightly toolchain must already be installed. Standard libraries and native
libraries and the kernel remain uninstrumented; this is not a race-freedom gate.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import time
import tomllib

from check import prepare_output, source_manifest
from check_process import run as run_process
from check_support import require_executable, source_revision

ROOT = Path(__file__).resolve().parent.parent
ASAN_OPTIONS = "halt_on_error=1:abort_on_error=0:exitcode=86:detect_leaks=1"
MUTEX_TESTS = {
    "mutex::tests::stationary_owner_moves_and_nonblocking_reentry_refuses",
    "mutex::tests::native_mutex_serializes_real_threads_and_drops_value_once",
    "mutex::tests::panic_poisoning_refuses_protected_state",
    "mutex::tests::forgotten_guard_keeps_native_storage_alive",
}

PATHNAME_TESTS = {
    "tests::decoder_bounds_and_mutations",
    "tests::decoder_reviewed_abi",
    "tests::native_canonical_names_match_reference_on_joined_workers",
    "tests::native_directory_names_refusal_and_independent_cursors",
    "tests::native_directory_cursors_are_independent_on_real_threads",
    "tests::native_path_metadata_matches_independent_std_and_opened_files",
    "tests::native_path_mutations_and_canonicalization_match_std",
    "tests::native_directory_open_errors_and_long_path",
    "tests::directory_total_bound_is_independent_of_per_call_work",
}
PATHNAME_PLATFORM_TESTS = {
    "Darwin": {
        "tests::decoder_flags_offsets_and_native_error",
        "tests::native_deep_absolute_links_preserve_the_33_link_boundary",
        "syscall::path::tests::native_budget_refuses_the_next_call_and_stays_exhausted",
        "syscall::path::tests::root_resolution_obeys_admission_before_native_effects",
        "syscall::path::tests::invalid_input_and_slot_growth_do_not_publish",
        "syscall::path::name_record::tests::offset_length_and_termination_are_independent",
        "syscall::path::name_record::tests::maximum_name_and_non_utf8_are_borrowed_without_allocation",
    },
    "Linux": {
        "tests::canonicalization_preserves_expanded_suffixes_with_a_short_final_name",
        "tests::canonicalization_preserves_native_directory_permission_errors",
        "tests::canonicalization_preserves_errors_at_the_native_name_ceiling",
        "syscall::linux_path::tests::native_admission_precedes_entry_and_preserves_native_errors",
        "syscall::linux_path::tests::suffix_refusal_preserves_pending_bytes_and_typed_cause",
    },
}


def environment(target, instrumented):
    env = dict(os.environ)
    for key in (
        "RUSTC", "RUSTDOC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
        "RUSTC_BOOTSTRAP", "CARGO_ENCODED_RUSTFLAGS", "RUSTDOCFLAGS",
        "ASAN_OPTIONS", "LSAN_OPTIONS", "TSAN_OPTIONS", "LD_PRELOAD",
        "DYLD_INSERT_LIBRARIES", "LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH",
    ):
        env.pop(key, None)
    env.update(
        CARGO_TARGET_DIR=str(target),
        RUSTFLAGS="-Dwarnings" + (" -Zsanitizer=address" if instrumented else ""),
        ASAN_OPTIONS=ASAN_OPTIONS,
        PYTHONDONTWRITEBYTECODE="1",
    )
    return env


def artifact(path):
    return {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def test_executable(stdout, target):
    candidates = []
    for line in stdout.splitlines():
        event = json.loads(line)
        if (event.get("reason") == "compiler-artifact"
                and event.get("target", {}).get("name") == "pipesql_filesystem"
                and event.get("profile", {}).get("test")
                and event.get("executable")):
            candidates.append(Path(event["executable"]).resolve())
    if len(candidates) != 1 or target.resolve() not in candidates[0].parents:
        raise ValueError("expected one filesystem test executable in the owned target")
    return require_executable(candidates[0])


def verify_discovery(stdout, expected):
    found = re.findall(r"^(\S+): test$", stdout, re.MULTILINE)
    if len(found) != len(expected) or set(found) != expected:
        raise ValueError("test discovery differs from the required scope")


def verify_tests(stdout, expected=MUTEX_TESTS):
    passed = re.findall(r"^test (\S+) \.\.\. ok$", stdout, re.MULTILINE)
    if len(passed) != len(expected) or set(passed) != expected:
        raise ValueError("test completion differs from the required scope")
    if f"test result: ok. {len(expected)} passed; 0 failed; 0 ignored;" not in stdout:
        raise ValueError("test summary does not establish completion")


def verify_control(mode, result):
    if mode == "clean":
        if result.returncode != 0 or result.stdout.strip() != "address control clean":
            raise ValueError("clean sanitizer control failed")
        if "Sanitizer" in result.stderr:
            raise ValueError("clean control reported a sanitizer diagnostic")
    elif mode == "fault":
        if result.returncode != 86 or "ERROR: AddressSanitizer: heap-buffer-overflow" not in result.stderr:
            raise ValueError("fault control did not establish instrumented heap-bounds detection")
    else:
        raise ValueError("unknown sanitizer control")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--toolchain", required=True, help="installed diagnostic nightly selector")
    parser.add_argument("--output", type=Path, required=True, help="new absolute directory outside the checkout")
    parser.add_argument("--scope", choices=("mutex", "pathname"), default="mutex",
                        help="native boundary to qualify (default: mutex)")
    options = parser.parse_args(argv)
    if not options.output.is_absolute():
        parser.error("--output must be absolute")
    if platform.system() not in ("Darwin", "Linux"):
        parser.error("this diagnostic currently supports macOS and GNU/Linux")
    required = MUTEX_TESTS if options.scope == "mutex" else (
        PATHNAME_TESTS | PATHNAME_PLATFORM_TESTS[platform.system()]
    )
    output = prepare_output(options.output, ROOT)
    work = output / "build"
    receipt = {
        "status": "failed", "scope": f"native-{options.scope}-address-sanitizer",
        "required_tests": sorted(required),
        "platform": platform.platform(), "revision": None,
        "libc": platform.libc_ver(), "python": platform.python_version(),
        "standard_library_instrumented": False, "native_pthread_instrumented": False,
        "native_libraries_instrumented": False, "kernel_instrumented": False,
        "asan_options": ASAN_OPTIONS, "commands": [], "artifacts": {},
    }

    def command(label, args, env, timeout=180, expected=0):
        record = {"label": label, "command": args, "timeout": timeout}
        receipt["commands"].append(record)
        print(f"check: {label}", flush=True)
        started = time.monotonic()
        try:
            result = run_process(args, cwd=ROOT, env=env, timeout=timeout,
                                 capture_output=True, text=True)
            (output / f"{label}.stdout").write_text(result.stdout)
            (output / f"{label}.stderr").write_text(result.stderr)
            record["returncode"] = result.returncode
            if result.returncode != expected:
                raise ValueError(f"{label}: exit {result.returncode}, expected {expected}; see logs")
            return result
        except subprocess.TimeoutExpired as error:
            for suffix, value in [("stdout", error.stdout), ("stderr", error.stderr)]:
                data = value or ""
                (output / f"{label}.{suffix}").write_bytes(data if isinstance(data, bytes) else data.encode())
            record["timed_out"] = True
            raise
        finally:
            record["elapsed_seconds"] = round(time.monotonic() - started, 3)

    before = None
    completed = False
    try:
        work.mkdir()
        receipt["revision"] = source_revision(ROOT)
        before = source_manifest(ROOT)
        (output / "inputs-before.sha256").write_text(before)
        receipt["source_sha256"] = hashlib.sha256(before.encode()).hexdigest()
        pinned = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
        selectors = {"stock": pinned, "diagnostic": options.toolchain}
        compilers = {}
        clean_env = environment(work / "metadata", False)
        command("native-compiler", ["cc", "--version"], clean_env, 30)
        for label, selector in selectors.items():
            info = command(f"{label}-compiler", ["rustc", f"+{selector}", "-vV"], clean_env, 30).stdout
            fields = dict(line.split(": ", 1) for line in info.splitlines() if ": " in line)
            if label == "diagnostic" and "nightly" not in fields.get("release", ""):
                raise ValueError("diagnostic compiler must identify itself as nightly")
            libdir = Path(command(f"{label}-libdir", ["rustc", f"+{selector}", "--print", "target-libdir"], clean_env, 30).stdout.strip())
            libraries = sorted(libdir.glob("libstd-*.rlib"))
            if len(libraries) != 1:
                raise ValueError("expected one prebuilt standard-library archive")
            compilers[label] = {**fields, "selector": selector, "standard_library": artifact(libraries[0])}
            if label == "diagnostic":
                runtimes = sorted(libdir.glob("*rt.asan.*"))
                if len(runtimes) != 1:
                    raise ValueError("expected one AddressSanitizer runtime")
                receipt["sanitizer_runtime"] = artifact(runtimes[0])
        receipt["compilers"] = compilers
        host = compilers["stock"]["host"]
        if host != compilers["diagnostic"]["host"] or host not in (
            "aarch64-apple-darwin", "x86_64-apple-darwin",
            "aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu",
        ):
            raise ValueError("stock and diagnostic compilers must share a supported native target")
        control = work / "address-control"
        asan_env = environment(work / "address", True)
        command("control-build", ["rustc", f"+{options.toolchain}", "--edition=2024", "--target", host,
                "-Zsanitizer=address", "-Copt-level=1", "-g", "-Dwarnings",
                str(ROOT / "tools/fixtures/sanitizer-control.rs"), "-o", str(control)], asan_env, 60)
        receipt["artifacts"]["control"] = artifact(require_executable(control))
        for mode in ("clean", "fault"):
            result = command(f"control-{mode}", [str(control), mode], asan_env, 20, 86 if mode == "fault" else 0)
            verify_control(mode, result)
        for label, selector, instrumented in (
            ("stock", pinned, False), ("nightly", options.toolchain, False),
            ("address", options.toolchain, True),
        ):
            target = work / label
            env = environment(target, instrumented)
            # Test aborts cannot strand native directory fixtures in ambient /tmp.
            temporary = work / f"{label}-temporary"
            temporary.mkdir()
            env.update(TMPDIR=str(temporary), TMP=str(temporary), TEMP=str(temporary))
            build = command(f"{label}-build", ["cargo", f"+{selector}", "test", "--release",
                    "--offline", "--locked", "--target", host, "-p", "pipesql-filesystem",
                    "--no-run", "--message-format=json"], env)
            binary = test_executable(build.stdout, target)
            receipt["artifacts"][label] = artifact(binary)
            if instrumented:
                linker = ["otool", "-L"] if platform.system() == "Darwin" else ["ldd"]
                command("native-dependencies", [*linker, str(binary)], env, 30)
            selection = [str(binary), *sorted(required), "--exact"]
            listing = command(f"{label}-discovery", [*selection, "--list"], env, 30)
            verify_discovery(listing.stdout, required)
            result = command(f"{label}-tests", [*selection, "--test-threads=1"], env, 30)
            verify_tests(result.stdout, required)
            if "Sanitizer" in result.stderr:
                raise ValueError(f"{label}: sanitizer report in otherwise successful test run")
        completed = True
    except BaseException as error:
        receipt["error"] = {"type": type(error).__name__, "message": str(error)}
        raise
    finally:
        errors = []
        try:
            after = source_manifest(ROOT)
            (output / "inputs-after.sha256").write_text(after)
            receipt["inputs_unchanged"] = before is not None and before == after
            if not receipt["inputs_unchanged"]:
                errors.append("source inputs changed or were not recorded")
        except Exception as error:
            errors.append(f"cannot verify source inputs: {error}")
        try:
            if work.exists():
                shutil.rmtree(work)
        except Exception as error:
            errors.append(f"cannot remove owned build directory: {error}")
        receipt["finalization_errors"] = errors
        receipt["status"] = "passed" if completed and not errors else "failed"
        (output / "result.json").write_text(json.dumps(receipt, indent=2) + "\n")
        print(f"sanitizer {receipt['status']}: {output / 'result.json'}", flush=True)
        if completed and errors:
            raise RuntimeError("; ".join(errors))


if __name__ == "__main__":
    main()
