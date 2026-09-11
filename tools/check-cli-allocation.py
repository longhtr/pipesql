#!/usr/bin/env python3
"""Real CLI source under allocator refusal; stock CLI descriptor/byte checks."""
from functools import partial
from typing import NamedTuple
from pathlib import Path
import argparse
import hashlib
import os
import re
import shutil
import subprocess
import tempfile
import sys

from check_process import run as run_process
from check_support import build_cli, dependency

ROOT = Path(__file__).resolve().parent.parent


def limits():
    import resource

    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    resource.setrlimit(resource.RLIMIT_NOFILE, (128, 128))


TRANSACTION_TOKEN = "000102030405060708090a0b0c0d0e0f0200000000000000"


def options(database):
    return [
        "--database",
        str(database),
        "--memory-limit-bytes",
        "2000000",
        "--temp-limit-bytes",
        "1000000",
    ]


def run_cli(work, args, mode=None, argv0=None, **kwargs):
    binary = work / "target/release/pipesql" if mode is None else work / "probe"
    return run_process(
        [str(binary) if argv0 is None else argv0, *args],
        executable=binary,
        env={**os.environ, **({"PIPESQL_CLI_PROBE": mode} if mode else {})},
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        preexec_fn=limits,
        timeout=20,
        cwd=ROOT,
        **kwargs,
    )


def build_probes(work):
    target = work / "target"
    build_cli(work)
    dependencies = target / "release/deps"
    compile = [
        "rustc",
        "--edition=2024",
        "-O",
        "-D",
        "warnings",
        "--extern",
        f"pipesql={target/'release/libpipesql.rlib'}",
    ]
    for name in ["pipesql_filesystem", "libc"]:
        artifact = dependency(target / "release", name)
        compile += ["--extern", f"{name}={artifact}"]
    compile += [
        "-L",
        f"dependency={dependencies}",
        str(ROOT / "tools/fixtures/cli-allocation.rs"),
        "-o",
        str(work / "probe"),
    ]
    run_process(compile, cwd=ROOT, check=True, timeout=60)
    stock = target / "release/pipesql"
    for artifact in [stock, work / "probe"]:
        print(
            f"CLI {artifact.name} sha256={hashlib.sha256(artifact.read_bytes()).hexdigest()}",
            flush=True,
        )
    print(f"CLI RUSTFLAGS={os.environ.get('RUSTFLAGS', '')!r}", flush=True)

    return compile


def parser_cases(work):
    # Parse errors allocate nothing after ownership of argv has transferred.
    cases = [
        [],
        ["wrong"],
        ["create"],
        ["create", b"\xff", "x"],
        [""],
        ["x" * 4097],
    ]
    token = TRANSACTION_TOKEN
    for flag in [
        "--database",
        "--input",
        "--query-file",
        "--transaction",
        "--memory-limit-bytes",
        "--temp-limit-bytes",
        "--unknown",
        "é" * 2048,
    ]:
        cases.append(["create", flag])
    for operation in ["create", "open", "load", "query", "resolve"]:
        base = [operation, *options(work / "absent")]
        if operation == "load":
            base += ["--input", str(work / "input.tbl")]
        if operation == "query":
            base += ["--query-file", str(ROOT / "tests/fixtures/q6.pipe.sql")]
        if operation == "resolve":
            base += ["--transaction", token]
        cases.append(base)
        for flag, value in [
            ("--database", "/duplicate"),
            ("--input", "/input"),
            ("--query-file", "/query"),
            ("--transaction", token),
            ("--memory-limit-bytes", "1"),
            ("--temp-limit-bytes", "1"),
            ("--unknown", "x"),
        ]:
            cases.append([*base, flag, value])
    cases.append(["resolve", *options(work / "absent")])
    for value in [
        token.upper(),
        "",
        token[:-1],
        token + "0",
        "g" * 48,
        b"\xff" * 48,
        "0" * 48,
        "0" * 32 + "01" + "0" * 14,
        token[:32] + "0" * 16,
        "0x" + token,
        " " + token,
        token + " ",
    ]:
        cases.append(["resolve", *options(work / "absent"), "--transaction", value])
    for flag in ["--memory-limit-bytes", "--temp-limit-bytes"]:
        for value in ["", "-1", "bad", "18446744073709551616", b"\xff", "x" * 4097]:
            cases.append(["create", flag, value])
        for value in ["0", "18446744073709551615"]:
            base = ["create", *options(work / "absent")]
            base[base.index(flag) + 1] = value
            cases.append(base)
    return cases


def check_parser(run, cases):
    for index, args in enumerate(cases):
        control = run(args, "parse-control")
        refused = run(args, "parse-deny")
        assert control.returncode == refused.returncode == 0, (
            index,
            control,
            refused,
        )
        assert control.stdout == refused.stdout, (
            index,
            control.stdout,
            refused.stdout,
        )
        assert b"cli allocations=0 refusals=0" in refused.stdout, (
            index,
            refused.stdout,
        )
    print(
        f"CLI parser: {len(cases)} control/deny pairs, zero allocations", flush=True
    )


def census(result):
    match = re.search(
        rb"^cli allocations=(\d+) refusals=(\d+)$", result.stdout, re.M
    )
    assert match is not None, (result.returncode, result.stdout, result.stderr)
    calls, refused = map(int, match.groups())
    assert calls <= 128
    return calls, refused


def check_native_capture(run):
    # Native capture plus diagnostic output, including empty argv[0], ignored long
    # argv[0], raw bytes, exact/next byte and count bounds. No engine effect here.
    native_cases = [
        ([], ""),
        (["", ""], ""),
        (["create", "--database"], "x" * 5000),
        (["create", b"\xff", "x"], ""),
        (["create", "é" * 2048], ""),
        (["x" * 4097], ""),
        ([""] * 11, ""),
        ([""] * 12, ""),
    ]
    cells = 0
    for args, argv0 in native_cases:
        control = run(args, "entry-control", argv0=argv0)
        stock_result = run(args, argv0=argv0)
        assert control.returncode == stock_result.returncode == 2
        assert control.stderr == stock_result.stderr
        calls, _ = census(control)
        for prefix in range(calls + 1):
            result = run(args, f"entry-after-{prefix}", argv0=argv0)
            assert result.returncode == 2, (args, prefix, result)
            _, refused = census(result)
            assert (refused > 0) == (prefix < calls)
            if prefix == calls:
                assert result.stderr == control.stderr
            cells += 1

    return cells


def digest(database):
    return {
        str(p.relative_to(database)): p.read_bytes()
        for p in database.rglob("*")
        if p.is_file()
    }


class DatabaseFixtures(NamedTuple):
    input_path: Path
    empty: Path
    loaded: Path
    history: Path
    tokens: dict[str, str]


def create_databases(work, run):
    input_path = work / "input.tbl"
    input_path.write_bytes(b"1|2|3|4|1|100|0.08|8|R|F|1994-01-01|12|13|14|15|16|\n")
    empty = work / "empty"
    assert run(["create", *options(empty)]).returncode == 0
    loaded = work / "loaded"
    shutil.copytree(empty, loaded)
    assert (
        run(["load", *options(loaded), "--input", str(input_path)]).returncode == 0
    )

    # Independent retained history gives both outcomes without relying on the
    # production writer to manufacture an aborted oracle.
    history = work / "history"
    (history / "units").mkdir(parents=True)
    (history / "private").mkdir()
    (history / "LOCK").write_bytes(b"")
    fixture = ROOT / "tests/fixtures/current-single-table-format"
    for name in ["CONTROL", "ROOT.A", "ROOT.B", "WAL"]:
        shutil.copyfile(fixture / name, history / name)
    shutil.copyfile(fixture / "UNIT", history / "units/0000000000000001.unit")
    token = TRANSACTION_TOKEN
    tokens = {
        "resolve-durable": token,
        "resolve-repair": token,
        "resolve-aborted": token[:32] + "0100000000000000",
        "resolve-unknown": token[:32] + "0300000000000000",
    }
    return DatabaseFixtures(input_path, empty, loaded, history, tokens)


def heal_ambiguous(run, input_path, healed_ambiguity, database, result):
    ambiguous = re.search(
        rb"commit outcome is ambiguous for transaction ([0-9a-f]{48}):",
        result.stderr,
    )
    if not ambiguous:
        return None
    old = ambiguous.group(1).decode()
    resolution_args = ["resolve", *options(database), "--transaction", old]
    healed = run(resolution_args)
    assert healed.returncode == 0, healed
    assert b"transaction=" + ambiguous.group(1) + b"\n" in healed.stdout
    match = re.search(rb"^resolution=(aborted|durable)$", healed.stdout, re.M)
    assert match is not None, healed
    outcome = match.group(1).decode()
    healed_ambiguity.add(outcome)
    if outcome == "aborted":
        retry = run(["load", *options(database), "--input", str(input_path)])
        assert retry.returncode == 0, retry
        resolved_again = run(resolution_args)
        assert (
            resolved_again.returncode == 0
            and b"resolution=aborted\n" in resolved_again.stdout
        )
    return outcome


def check_publication(work, compile, empty, input_path, heal_ambiguous):
    # Allocation refusal reaches durable ambiguity here. Native rename refusal
    # separately covers the first data-root replacement, before it takes effect.
    # Four renames are the issuance A/B and data A/B publication protocol.
    observer = work / "libcli_publication.dylib"
    run_process(
        [
            "clang",
            "-std=c11",
            "-O2",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Wconversion",
            "-dynamiclib",
            str(ROOT / "tools/fixtures/cli-publication.c"),
            f"-Wl,-install_name,{observer}",
            "-o",
            str(observer),
        ],
        check=True,
        timeout=30,
        cwd=ROOT,
    )
    native_compile = list(compile)
    native_compile[
        native_compile.index(str(ROOT / "tools/fixtures/cli-allocation.rs"))
    ] = str(ROOT / "tools/fixtures/cli-publication.rs")
    native_compile[-1] = str(work / "publication")
    native_compile += ["-L", str(work), "-l", "dylib=cli_publication"]
    run_process(native_compile, check=True, timeout=60, cwd=ROOT)
    for artifact in [observer, work / "publication"]:
        print(
            f"CLI {artifact.name} sha256={hashlib.sha256(artifact.read_bytes()).hexdigest()}",
            flush=True,
        )
    for cut in [0, 3, 4]:
        database = work / f"rename-cut-{cut}"
        shutil.copytree(empty, database)
        result = run_process(
            [
                str(work / "publication"),
                "load",
                *options(database),
                "--input",
                str(input_path),
            ],
            env={
                **os.environ,
                "PIPESQL_RENAME_CUT": str(cut),
                "DYLD_INSERT_LIBRARIES": str(observer),
            },
            capture_output=True,
            preexec_fn=limits,
            timeout=20,
            cwd=ROOT,
        )
        assert result.returncode == (0 if cut == 0 else 1), result
        assert f"observed_renames={cut or 4}\n".encode() in result.stderr, result
        if cut:
            assert heal_ambiguous(database, result) == (
                "aborted" if cut == 3 else "durable"
            ), result
        print(
            f"CLI publication rename cut={cut}: {result.stderr.decode().strip()}",
            flush=True,
        )


def check_operations(work, run, fixtures, heal_ambiguous):
    input_path, empty, loaded, history, tokens = fixtures
    cells = 0
    for operation in ["create", "open", "load", "q1", "q6", *tokens]:

        def cell(label, mode):
            database = work / label
            if operation != "create":
                source = (
                    history
                    if operation in tokens
                    else empty if operation == "load" else loaded
                )
                shutil.copytree(source, database)
            before = (
                digest(database)
                if operation in ["open", "q1", "q6", *tokens]
                else None
            )
            if operation == "resolve-repair":
                (database / "ROOT.B").unlink()
            verb = (
                "resolve"
                if operation in tokens
                else "query" if operation.startswith("q") else operation
            )
            args = [verb, *options(database)]
            if operation == "load":
                args += ["--input", str(input_path)]
            if operation in tokens:
                args += ["--transaction", tokens[operation]]
            if operation.startswith("q"):
                query = ROOT / (
                    "tests/fixtures/upstream/q1-upstream.pipe.sql"
                    if operation == "q1"
                    else "tests/fixtures/q6.pipe.sql"
                )
                args += ["--query-file", str(query)]
            result = run(args, mode)
            if before is not None and operation != "resolve-repair":
                assert digest(database) == before, (label, "settled bytes changed")
            if operation in tokens:
                if result.returncode == 0:
                    expected = (
                        b"aborted" if operation == "resolve-aborted" else b"durable"
                    )
                    assert b"resolution=" + expected + b"\n" in result.stdout, (
                        label,
                        result,
                    )
                    assert (
                        b"transaction=" + tokens[operation].encode() + b"\n"
                        in result.stdout
                    )
                healed = run(args)
                assert healed.returncode == (
                    1 if operation == "resolve-unknown" else 0
                ), (label, healed)
                assert digest(database) == before, (
                    label,
                    "healed repair differs from independent fixture",
                )
                if operation == "resolve-unknown":
                    assert (
                        b"resolution=" not in result.stdout
                        and b"resolution=" not in healed.stdout
                    )
                    assert b"transaction was not found" in healed.stderr
            if operation == "load":
                heal_ambiguous(database, result)
            return result

        control = cell(f"{operation}-control", "entry-control")
        expected_code = 1 if operation == "resolve-unknown" else 0
        assert control.returncode == expected_code, control.stderr
        calls, _ = census(control)
        print(f"CLI {operation} census={calls}", flush=True)
        for prefix in range(calls + 1):
            result = cell(f"{operation}-after-{prefix}", f"entry-after-{prefix}")
            _, refused = census(result)
            assert (refused > 0) == (prefix < calls), (operation, prefix, result)
            assert result.returncode in ([1, 2] if refused else [expected_code]), (
                operation,
                prefix,
                result,
            )
            cells += 1
    return cells


def check_output_sinks(work, run, loaded, history):
    stock = work / "target/release/pipesql"
    token = TRANSACTION_TOKEN
    # Inject closure AFTER runtime sanitization at the narrow caller boundary.
    # The same CLI entry must refuse stdout before mutation and never reuse fd 2
    # as a diagnostic sink after it has become an engine descriptor.
    closed_database = work / "closed-output"
    result = run(["create", *options(closed_database)], "entry-closed-stdout")
    assert result.returncode == 1 and not closed_database.exists(), result
    result = run(["create", *options(closed_database)], "entry-closed-stderr")
    assert result.returncode == 0, result
    before = digest(closed_database)
    result = run(["create", *options(closed_database)], "entry-closed-stderr")
    assert result.returncode == 1 and digest(closed_database) == before, result
    shutil.rmtree(closed_database)

    # Stock executable, with shell-side descriptor closure before Rust bootstrap.
    def closed_stdout():
        limits()
        os.close(1)

    result = run_process(
        [str(stock), "create", *options(closed_database)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        preexec_fn=closed_stdout,
        timeout=20,
        cwd=ROOT,
    )
    # Pinned Rust startup sanitizes closed inherited slots to /dev/null before
    # main. This is an accepted sink, not a CLI write error. Keep this discovered
    # counterexample to the initial assumption that shell closure implies EBADF.
    assert result.returncode == 0 and closed_database.exists(), result
    shutil.rmtree(closed_database)

    def closed_stderr():
        limits()
        os.close(2)

    result = run_process(
        [str(stock), "create", *options(closed_database)],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        preexec_fn=closed_stderr,
        timeout=20,
        cwd=ROOT,
    )
    assert result.returncode == 0 and b"status=created" in result.stdout, result
    before = digest(closed_database)
    result = run_process(
        [str(stock), "create", *options(closed_database)],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        preexec_fn=closed_stderr,
        timeout=20,
        cwd=ROOT,
    )
    assert result.returncode == 1 and digest(closed_database) == before, result
    reader, writer = os.pipe()
    os.close(reader)
    try:
        result = run_process(
            [str(stock), "open", *options(loaded)],
            stdout=writer,
            stderr=subprocess.PIPE,
            preexec_fn=limits,
            timeout=20,
            cwd=ROOT,
        )
        assert result.returncode == 1 and b"broken pipe" in result.stderr, result
    finally:
        os.close(writer)
    reader, writer = os.pipe()
    os.close(reader)
    try:
        result = run_process(
            [str(stock), "resolve", *options(history), "--transaction", token],
            stdout=writer,
            stderr=subprocess.PIPE,
            preexec_fn=limits,
            timeout=20,
            cwd=ROOT,
        )
        assert result.returncode == 1 and b"broken pipe" in result.stderr, result
    finally:
        os.close(writer)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args(argv)
    if not __debug__:
        parser.error("allocation checks require Python assertions")
    if sys.platform != "darwin":
        parser.error("CLI publication/allocation observers require macOS")
    with tempfile.TemporaryDirectory(prefix="pipesql-cli-gate-") as directory:
        work = Path(directory).resolve()
        compile = build_probes(work)
        run = partial(run_cli, work)
        check_parser(run, parser_cases(work))
        cells = check_native_capture(run)
        fixtures = create_databases(work, run)
        input_path, empty, loaded, history, _ = fixtures
        healed_ambiguity = set()
        heal = partial(heal_ambiguous, run, input_path, healed_ambiguity)
        check_publication(work, compile, empty, input_path, heal)
        cells += check_operations(
            work, run, fixtures, heal
        )
        assert healed_ambiguity == {"aborted", "durable"}, healed_ambiguity
        print(
            "CLI ambiguous-load tokens: stock resolution reaches aborted and durable; aborted stays aborted after retry",
            flush=True,
        )

        check_output_sinks(work, run, loaded, history)
        print(
            f"CLI: {cells} allocation-prefix cases; Rust owners/descriptors released; stock closed/broken sinks passed",
            flush=True,
        )


if __name__ == "__main__":
    main()
