#!/usr/bin/env python3
"""Stock native catalog process interruption at observed native mutation cuts.

Fresh copies isolate each cut. Visible writes survive termination; this does not
model power loss, torn writes, kernel failure or arbitrary concurrent mutations.
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
        elif mode in ("append", "recover"):
            print(f"{label} trace: " + json.dumps(events), flush=True)
        if expected in (0, 86):
            graph = catalog_graph.inspect(db)
            observed_state = 2 if mode == "append" and cut == 0 else state
            retried = mode == "verify"
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

    def copy(source, label):
        target = work / label
        shutil.copytree(source, target)
        return target

    seed = work / "seed"
    run(seed, "setup", "setup")
    complete = copy(seed, "append-complete")
    events = run(complete, "append", "append-control")
    assert 0 < len(events) <= 512
    assert {event[1] for event in events} >= {"write", "pwrite", "sync", "rename"}
    publications = [
        int(event[0]) for event in events if event[1:] == ["rename", "after", "A"]
    ]
    assert len(publications) == 2, publications  # Issuance, then data publication.
    issuance, commit = publications
    run(copy(complete, "acknowledged"), "verify", "acknowledged", state=2)
    states = set()
    recovery_seeds = {}
    for cut in range(1, len(events) + 1):
        state = 0 if cut < issuance else 1 if cut < commit else 2
        states.add(state)
        db = copy(seed, f"append-cut-{cut}")
        reached = run(db, "append", f"append-cut-{cut}", cut, state, 86)
        assert reached == events[:cut], (cut, reached, events[:cut])
        # Preserve pre-publication and mixed-root published states before healing.
        if cut in (commit - 1, commit):
            recovery_seeds[state] = copy(db, f"recovery-seed-{state}")
        run(db, "verify", f"append-verify-{cut}", state=state)
    assert states == {0, 1, 2} and set(recovery_seeds) == {1, 2}
    recovery_count = 0
    recovery_kinds = set()
    for state, source in recovery_seeds.items():
        db = copy(source, f"recovery-control-{state}")
        recovery = run(db, "recover", f"recovery-control-{state}", state=state)
        assert 0 < len(recovery) <= 256
        recovery_kinds.update(event[1] for event in recovery)
        for cut in range(1, len(recovery) + 1):
            db = copy(source, f"recovery-{state}-cut-{cut}")
            reached = run(db, "recover", f"recovery-{state}-cut-{cut}", cut, state, 86)
            assert reached == recovery[:cut], (state, cut)
            run(db, "verify", f"recovery-{state}-verify-{cut}", state=state)
            recovery_count += 1
    assert recovery_kinds >= {"unlink", "rename", "write", "sync"}
    wrong = copy(complete, "wrong-history")
    run(wrong, "recover", "wrong-history", state=1, expected=101)
    assert "independent generation history" in records[-1]["stderr"]
    for mode, marker in [
        ("wrong-rows", "independent row history"),
        ("wrong-receipt", "missing durable receipt"),
    ]:
        run(copy(complete, mode), mode, mode, state=2, expected=101)
        assert marker in records[-1]["stderr"], records[-1]
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
        append_cuts=len(events),
        recovery_cuts=recovery_count,
        independent_graph_checks=sum("graph" in record for record in records),
        issuance_after=issuance,
        commit_after=commit,
        states=sorted(states),
        artifacts={
            name: hashlib.sha256((work / name).read_bytes()).hexdigest()
            for name in [
                "driver",
                observer.name,
                "target/release/libpipesql.rlib",
            ]
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
