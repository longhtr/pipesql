#!/usr/bin/env python3
"""Check stock catalog recovery after termination at observed native mutation cuts.

Record a healthy append trace, then replay every before/after event on a fresh
seed copy. Root-A replacements distinguish issuance from data publication.
Interrupted recovery gets its own cut sequence. Rust callers verify literal
rows, report groups and token outcomes after reopen and retry; the independent
Python inspector separately checks raw graph contents. Wrong histories, rows
and receipts must be rejected by the controls.

The full gate invokes this campaign on frozen source. A standalone run owns
disposable outputs unless --output requests a new retained replay directory.
Visible writes survive termination: this does not model power loss, torn writes,
kernel failure or arbitrary concurrent mutation. See tools/README.md for scope.
"""
from pathlib import Path
import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile

from check_process import run as run_process
from check_support import (
    build_library,
    native_library,
    observer_environment,
    rust_driver,
    source_revision,
)

ROOT = Path(__file__).resolve().parent.parent


# Literal raw values are independent of the Rust input helper and query evaluator.
REPORT_EVENTS = [
    [0, 1, 10956, 10, {"double_bits": "3fe0000000000000"}],
    [1, 2, 11016, 20, {"double_bits": "3ff8000000000000"}],
    [2, None, 11016, 30, None],
    [3, 99, 11016, None, {"double_bits": "8000000000000000"}],
    [4, 3, None, 5, {"double_bits": "4000000000000000"}],
    [5, 1, 11323, None, {"double_bits": "4004000000000000"}],
    [6, 2, 11322, -5, {"double_bits": "4008000000000000"}],
    [7, 1, None, 7, {"double_bits": "bff0000000000000"}],
    [8, 1, 10956, -2, {"double_bits": "4010000000000000"}],
    [9, 2, 11323, 40, {"double_bits": "4012000000000000"}],
    [10, 99, 11016, 3, {"double_bits": "4014000000000000"}],
    [11, None, None, 11, None],
    [12, 3, 11016, 0, {"double_bits": "4018000000000000"}],
    [13, 1, 11323, 13, {"double_bits": "401a000000000000"}],
    [14, 2, None, None, {"double_bits": "401c000000000000"}],
    [15, 3, 11322, 9, {"double_bits": "c000000000000000"}],
]


def check_report_graph(graph, state, retried):
    assert state in (0, 1, 2)
    successes = [1, 2, 3, 4] + ([6] if state == 2 else [])
    issued = 5 if state == 0 else 6
    rows = REPORT_EVENTS[:16 if state == 2 else 8]
    if retried:
        issued += 1
        successes += [issued]
        rows = rows + [REPORT_EVENTS[8]]
    assert graph["issued"] == issued and graph["successes"] == successes, graph
    assert graph["generation"] == len(successes), graph
    tables = {table["name"]: table for table in graph["tables"]}
    assert len(graph["tables"]) == 2 and set(tables) == {"events", "dimensions"}, graph
    events, dimensions = tables["events"], tables["dimensions"]
    assert (events["id"], dimensions["id"]) == (1, 2)
    assert events["columns"] == [
        dict(id=1, name="id", type=1, nullable=False),
        dict(id=2, name="dimension_id", type=1, nullable=True),
        dict(id=3, name="happened", type=4, nullable=True),
        dict(id=4, name="amount", type=1, nullable=True),
        dict(id=5, name="measurement", type=2, nullable=True),
    ]
    assert dimensions["columns"] == [
        dict(id=1, name="id", type=1, nullable=False),
        dict(id=2, name="label", type=3, nullable=False),
    ]
    assert events["rows"] == rows, "independent report raw rows"
    assert dimensions["rows"] == [[1, "north"], [2, "south"], [2, "南"], [3, ""]]


def campaign(work):
    import catalog_graph

    sources = run_process(
        [sys.executable, str(ROOT / "tools/source-manifest.py")],
        cwd=ROOT,
        timeout=30,
        check=True,
        stdout=subprocess.PIPE,
    ).stdout
    (work / "sources.sha256").write_bytes(sources)
    observer = native_library(work, "catalog-interruption.c", "interruption")
    release = build_library(work)
    rust_driver(work, release, "catalog-interruption.rs", "interruption")
    env = observer_environment(observer)
    records = []

    def run(db, mode, label, cut=0, state=0, expected=0):
        trace = work / f"{label}.trace"
        command = [
            str(work / "driver"),
            str(db),
            mode,
            str(trace),
            str(cut),
            str(state),
        ]
        result = run_process(
            command, env=env, capture_output=True, text=True, timeout=20, cwd=ROOT
        )
        record = dict(
            label=label,
            mode=mode,
            cut=cut,
            state=state,
            exit=result.returncode,
            stdout=result.stdout,
            stderr=result.stderr,
            trace=trace.name,
        )
        records.append(record)
        (work / "cases.json").write_text(json.dumps(records, indent=2) + "\n")
        assert result.returncode == expected, record
        events = [line.split() for line in trace.read_text().splitlines()]
        for index, event in enumerate(events, 1):
            assert len(event) == 4 and int(event[0]) == index, event
        if cut:
            assert len(events) == cut, record
        elif mode.removeprefix("report-") in ("append", "recover"):
            print(f"{label} trace: " + json.dumps(events), flush=True)
        if expected == 0:
            assert f"catalog interruption {mode} passed state={state}" in result.stdout, record
        if expected in (0, 86):
            graph = catalog_graph.inspect(db)
            report = mode.startswith("report-")
            operation = mode.removeprefix("report-")
            observed_state = 2 if operation == "append" and cut == 0 else state
            retried = operation == "verify"
            if report:
                check_report_graph(graph, observed_state, retried)
            else:
                successes = [1, 2] + ([4] if observed_state == 2 else [])
                issued = 3 if observed_state == 0 else 4
                wanted_rows = [[7, "snow 雪\0"], [2**63 - 1, None]]
                if observed_state == 2:
                    wanted_rows += [[11, "snow 雪\0"], [13, None]]
                if retried:
                    issued += 1
                    successes += [issued]
                    wanted_rows += [[17, "snow 雪\0"], [19, None]]
                assert graph["issued"] == issued and graph["successes"] == successes, (
                    label,
                    graph,
                )
                assert graph["generation"] == len(successes), (label, graph)
                assert len(graph["tables"]) == 1 and graph["tables"][0]["name"] == "facts"
                assert graph["tables"][0]["rows"] == wanted_rows, (label, graph)
            if expected == 0:
                assert graph["roots_settled"] and not graph["cleanup_names"], (
                    label,
                    graph,
                )
            record["graph"] = {
                key: graph[key]
                for key in (
                    "issued",
                    "generation",
                    "successes",
                    "roots_settled",
                    "reachable",
                    "unreferenced",
                    "read_bytes",
                    "decoded_values",
                )
            }
            (work / "cases.json").write_text(json.dumps(records, indent=2) + "\n")
        print(
            f"{label}: exit={result.returncode} events={len(events)} state={state}",
            flush=True,
        )
        return events

    def history(prefix):
        def observe(db, mode, label, cut=0, state=0, expected=0):
            return run(db, prefix + mode, prefix + label, cut, state, expected)

        def fresh(source, label):
            target = work / (prefix + label)
            shutil.copytree(source, target)
            return target

        first_record = len(records)
        seed = work / (prefix + "seed")
        observe(seed, "setup", "setup")
        complete = fresh(seed, "append-complete")
        events = observe(complete, "append", "append-control")
        assert 0 < len(events) <= 512
        assert {event[1] for event in events} >= {"write", "pwrite", "sync", "rename"}
        publications = [
            int(event[0]) for event in events if event[1:] == ["rename", "after", "A"]
        ]
        assert len(publications) == 2, publications  # Issuance, then data publication.
        issuance, commit = publications
        observe(fresh(complete, "acknowledged"), "verify", "acknowledged", state=2)
        states = set()
        recovery_seeds = {}
        for cut in range(1, len(events) + 1):
            state = 0 if cut < issuance else 1 if cut < commit else 2
            states.add(state)
            db = fresh(seed, f"append-cut-{cut}")
            reached = observe(db, "append", f"append-cut-{cut}", cut, state, 86)
            assert reached == events[:cut], (cut, reached, events[:cut])
            # Preserve pre-publication and mixed-root published states before healing.
            if cut in (commit - 1, commit):
                recovery_seeds[state] = fresh(db, f"recovery-seed-{state}")
            observe(db, "verify", f"append-verify-{cut}", state=state)
        assert states == {0, 1, 2} and set(recovery_seeds) == {1, 2}
        recovery_count = 0
        recovery_kinds = set()
        for state, source in recovery_seeds.items():
            db = fresh(source, f"recovery-control-{state}")
            recovery = observe(db, "recover", f"recovery-control-{state}", state=state)
            assert 0 < len(recovery) <= 256
            recovery_kinds.update(event[1] for event in recovery)
            for cut in range(1, len(recovery) + 1):
                db = fresh(source, f"recovery-{state}-cut-{cut}")
                reached = observe(db, "recover", f"recovery-{state}-cut-{cut}", cut, state, 86)
                assert reached == recovery[:cut], (state, cut)
                observe(db, "verify", f"recovery-{state}-verify-{cut}", state=state)
                recovery_count += 1
        assert recovery_kinds >= {"unlink", "rename", "write", "sync"}
        wrong = fresh(complete, "wrong-history")
        observe(wrong, "recover", "wrong-history", state=1, expected=101)
        assert ("independent report generation history" if prefix else "independent generation history") in records[-1]["stderr"]
        for mode, marker in [
            ("wrong-rows", "report group history" if prefix else "independent row history"),
            ("wrong-receipt", "missing durable receipt"),
        ]:
            observe(fresh(complete, mode), mode, mode, state=2, expected=101)
            assert marker in records[-1]["stderr"], records[-1]
        return dict(
            append_cuts=len(events), recovery_cuts=recovery_count,
            independent_graph_checks=sum("graph" in record for record in records[first_record:]),
            issuance_after=issuance, commit_after=commit, states=sorted(states),
        )

    facts = history("")
    report = history("report-")
    assert (
        sources
        == run_process(
            [sys.executable, str(ROOT / "tools/source-manifest.py")],
            cwd=ROOT,
            timeout=30,
            check=True,
            stdout=subprocess.PIPE,
        ).stdout
    )
    result = dict(
        base=source_revision(ROOT),
        source_manifest_sha256=hashlib.sha256(sources).hexdigest(),
        **facts,
        report=report,
        artifacts={
            path.name: hashlib.sha256(path.read_bytes()).hexdigest()
            for path in [work / "driver", observer, release / "libpipesql.rlib"]
        },
    )
    (work / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print("catalog interruption passed: " + json.dumps(result), flush=True)


def main(argv=None):
    sys.dont_write_bytecode = True
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        type=Path,
        help="new directory for retained replay inputs, traces and results",
    )
    options = parser.parse_args(argv)
    if not __debug__:
        parser.error("interruption checks require Python assertions")
    if sys.platform not in ("darwin", "linux"):
        parser.error("catalog interruption requires macOS or Linux")

    if options.output:
        options.output = options.output.absolute()
        options.output.mkdir()  # Never overwrite a replay package.
        campaign(options.output)
    else:
        with tempfile.TemporaryDirectory(
            prefix="pipesql-catalog-interruption-"
        ) as directory:
            campaign(Path(directory).resolve())


if __name__ == "__main__":
    main()
