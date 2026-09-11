#!/usr/bin/env python3
"""Stock graphs and deliberately malformed references for the independent checker."""
import argparse
from dataclasses import replace
import hashlib
import json
import os
from pathlib import Path
import shutil
import select
import subprocess
import sys
import tempfile
import time

import catalog_graph as g

from check_process import owned_process, run as run_process
from check_support import build_library, source_revision

ROOT = Path(__file__).resolve().parent.parent


def wait_for_lease(holder):
    """Read the caller's readiness marker without an unbounded readline."""
    deadline = time.monotonic() + 10
    marker = b"holding\n"
    received = b""
    while len(received) < len(marker):
        ready, _, _ = select.select(
            [holder.stdout], [], [], max(0, deadline - time.monotonic())
        )
        if not ready:
            raise TimeoutError("catalog caller did not acquire its lease")
        part = os.read(holder.stdout.fileno(), len(marker) - len(received))
        if not part:
            raise AssertionError(
                "catalog caller closed output before acquiring its lease"
            )
        received += part
    assert received == marker, received


def expected_rows():
    keys = [-(2**63), -1, 0, 1, 2, 3, 4, 5, 2**63 - 1]
    bits = [
        "7ff0000000000001",
        "8000000000000000",
        "7ff0000000000000",
        "fff0000000000000",
        "3ff0000000000000",
        "4000000000000000",
        "4008000000000000",
        None,
        "fff8000000001234",
    ]
    strings = ["雪\0", "", "é", "🙂", "same", "same", "a", None, "end"]
    return [
        [
            key,
            None if bits[i] is None else {"double_bits": bits[i]},
            strings[i],
            None if i == 7 else 2932896 if i == 8 else -719162,
        ]
        for i, key in enumerate(keys)
    ] * 2


def expected(result):
    assert (
        result["issued"] == 6
        and result["generation"] == 4
        and result["successes"] == [1, 2, 4, 5]
    )
    assert [(t["id"], t["name"]) for t in result["tables"]] == [
        (1, "facts"),
        (2, "empty"),
    ]
    assert result["tables"][0]["rows"] == expected_rows(), "independent row expectation"
    assert result["tables"][1]["rows"] == []
    assert result["tables"][0]["columns"] == [
        dict(id=i + 1, name=n, type=t, nullable=i > 0)
        for i, (n, t) in enumerate([("k", 1), ("d", 2), ("s", 3), ("day", 4)])
    ]
    assert result["tables"][1]["columns"] == [
        dict(id=1, name="x", type=3, nullable=True)
    ]


def put(data, at, value, width=4):
    data[at : at + width] = value.to_bytes(width, "little")


class Mutation:
    """Re-anchor mutations all the way to roots, so CRC failure cannot mask them."""

    def __init__(self, path):
        self.path = path
        root = (path / "ROOT.A").read_bytes()
        self.cat = g.reference(root[128:152])
        self.success = g.reference(root[152:176])
        self.catalog = bytearray((path / "units" / self.cat.name).read_bytes())
        self.schema = g.reference(self.catalog[112:136])
        self.index = g.reference(self.catalog[136:160])
        self.indexbytes = bytearray((path / "units" / self.index.name).read_bytes())

    def roots(self, edit):
        for name in ("ROOT.A", "ROOT.B", "WAL"):
            data = bytearray((self.path / name).read_bytes())
            edit(data)
            put(data, 108, 0)
            put(data, 108, g.crc32c(data))
            (self.path / name).write_bytes(data)

    def save_catalog(self):
        (self.path / "units" / self.cat.name).write_bytes(self.catalog)
        self.roots(lambda data: put(data, 144, g.crc32c(self.catalog)))

    def change(self, kind, edit):
        if kind == "catalog":
            edit(self.catalog)
            self.save_catalog()
            return
        if kind == "history":
            data = bytearray((self.path / "units" / self.success.name).read_bytes())
            edit(data)
            (self.path / "units" / self.success.name).write_bytes(data)
            self.roots(lambda root: put(root, 168, g.crc32c(data)))
            return
        if kind == "schema":
            data = bytearray((self.path / "units" / self.schema.name).read_bytes())
            edit(data)
            (self.path / "units" / self.schema.name).write_bytes(data)
            put(self.catalog, 128, g.crc32c(data))
            self.save_catalog()
            return
        if kind in ("unit", "payload", "last-payload"):
            at = 112 if kind == "last-payload" else 64
            identity = g.object_id(
                g.uint(self.indexbytes, at, 8), g.uint(self.indexbytes, at + 8)
            )
            data = bytearray((self.path / "units" / identity).read_bytes())
            edit(data)
            if kind != "unit":
                for offset in range(64, 192, 32):
                    begin, length = g.uint(data, offset + 8, 8), g.uint(
                        data, offset + 16
                    )
                    put(data, offset + 20, g.crc32c(data[begin : begin + length]))
            (self.path / "units" / identity).write_bytes(data)
            put(self.indexbytes, at + 20, g.crc32c(data[:192]))
        else:
            assert kind == "index"
            edit(self.indexbytes)
        (self.path / "units" / self.index.name).write_bytes(self.indexbytes)
        put(self.catalog, 152, g.crc32c(self.indexbytes))
        self.save_catalog()


def build_driver(work):
    source = run_process(
        [sys.executable, str(ROOT / "tools/source-manifest.py")],
        cwd=ROOT,
        timeout=30,
        check=True,
        stdout=subprocess.PIPE,
    ).stdout
    (work / "sources.sha256").write_bytes(source)
    build_library(work, timeout=120)
    driver = work / "driver"
    run_process(
        [
            "rustc",
            "--edition=2024",
            "-O",
            "-D",
            "warnings",
            str(ROOT / "tools/fixtures/catalog-graph.rs"),
            "--extern",
            f"pipesql={work/'target/release/libpipesql.rlib'}",
            "-L",
            f"dependency={work/'target/release/deps'}",
            "-o",
            str(driver),
        ],
        check=True,
        timeout=60,
        cwd=ROOT,
    )
    return source, driver


def create_seed(work, driver):
    seed = work / "seed"
    run_process([str(driver), str(seed), "setup"], check=True, timeout=20, cwd=ROOT)
    baseline = g.inspect(seed)
    expected(baseline)
    assert baseline["roots_settled"] and not baseline["cleanup_names"]
    (work / "graph.json").write_text(json.dumps(baseline, indent=2) + "\n")
    return seed, baseline


def check_genesis_lease_and_fixture(work, driver, seed):
    genesis = work / "genesis"
    run_process(
        [str(driver), str(genesis), "genesis"], check=True, timeout=20, cwd=ROOT
    )
    empty = g.inspect(genesis)
    assert (
        empty["generation"] == empty["issued"] == 0
        and empty["tables"] == empty["successes"] == empty["reachable"] == []
    )
    with owned_process(
        [str(driver), str(seed), "hold"],
        cwd=ROOT,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    ) as holder:
        wait_for_lease(holder)
        try:
            g.inspect(seed)
        except BlockingIOError:
            pass
        else:
            raise AssertionError("checker ignored live engine lease")
        holder.stdin.write(b"x")
        holder.stdin.flush()
        stdout, stderr = holder.communicate(timeout=10)
        assert holder.returncode == 0, (stdout, stderr)
    # Independent retained bytes deliberately reverse physical/declared column
    # order and use nonordinal IDs. This challenges an ordinal-only decoder.
    fixture = work / "independent-fixture"
    (fixture / "units").mkdir(parents=True)
    (fixture / "private").mkdir()
    (fixture / "LOCK").touch()
    for name in ("CONTROL", "ROOT.A", "ROOT.B", "WAL"):
        shutil.copyfile(ROOT / "tests/fixtures/catalog-roots" / name, fixture / name)
    for name, source_name in [
        ("0000000000000003-00000001.obj", "catalog-schema/columns.bin"),
        ("0000000000000003-00000002.obj", "catalog-schema/native-unit.bin"),
        ("0000000000000005-00000003.obj", "catalog-roots/table-data.bin"),
        ("0000000000000005-00000004.obj", "catalog-roots/catalog.bin"),
        ("0000000000000005-00000005.obj", "catalog-roots/successes.bin"),
    ]:
        shutil.copyfile(ROOT / "tests/fixtures" / source_name, fixture / "units" / name)
    independent = g.inspect(fixture)
    assert independent["successes"] == [3, 5] and independent["issued"] == 6
    assert [c["id"] for c in independent["tables"][0]["columns"]] == [29, 3]
    assert independent["tables"][0]["rows"] == [
        [None, {"double_bits": "8000000000000000"}],
        ["", {"double_bits": "7ff0000000000001"}],
        ["雪", {"double_bits": "7ff0000000000000"}],
        ["é\0🙂", {"double_bits": "ffefffffffffffff"}],
    ]
    print(
        "genesis, stock lease contention and independent physical column order passed",
        flush=True,
    )


class GraphChecks:
    """Own copied case directories and observations for one stock seed."""

    def __init__(self, work, seed, driver):
        self.work = work
        self.seed = seed
        self.driver = driver
        self.records = []

    def case(
        self, label, mutate=None, error=None, public=False,
        limits=g.Limits(), verify=expected,
    ):
        path = self.work / label
        shutil.copytree(self.seed, path)
        if mutate:
            mutate(Mutation(path))
        try:
            result = g.inspect(path, limits)
        except (g.Invalid, g.LimitExceeded, OSError) as failure:
            assert error and error in str(failure), (
                label,
                type(failure).__name__,
                str(failure),
                error,
            )
            record = dict(
                case=label, outcome=type(failure).__name__, reason=str(failure)
            )
        else:
            assert error is None, (label, "accepted malformed graph")
            verify(result)
            record = dict(
                case=label,
                outcome="valid",
                roots_settled=result["roots_settled"],
                reachable=len(result["reachable"]),
                unreferenced=len(result["unreferenced"]),
            )
        if public:
            result = run_process(
                [
                    str(self.driver),
                    str(path),
                    "query-reject" if public == "query" else "reject",
                ],
                capture_output=True,
                text=True,
                timeout=20,
                cwd=ROOT,
            )
            assert result.returncode == 0, (label, result.stdout, result.stderr)
            record["public_rejected"] = public
        self.records.append(record)
        (self.work / "cases.json").write_text(json.dumps(self.records, indent=2) + "\n")
        print(json.dumps(record), flush=True)


def check_budgets(check, baseline):
    assert g.crc32c(b"123456789") == 0xE3069283 and g.crc32c(b"") == 0
    check(
        "exact-budgets",
        limits=g.Limits(
            len(baseline["reachable"]) + len(baseline["unreferenced"]),
            baseline["read_bytes"],
            baseline["decoded_values"],
        ),
    )
    for field, value, reason in [
        ("objects", 1, "directory entry budget"),
        ("read_bytes", baseline["read_bytes"] - 1, "read byte budget"),
        ("values", baseline["decoded_values"] - 1, "decoded value budget"),
    ]:
        check(
            f"limit-{field}", error=reason, limits=replace(g.Limits(), **{field: value})
        )


def reference_mutations(seed):
    mutations = [
        (
            "wrong-schema",
            "catalog",
            lambda b: b.__setitem__(slice(112, 136), b[240:264]),
            "schema table identity",
            True,
        ),
        (
            "cycle",
            "catalog",
            lambda b: b.__setitem__(
                slice(112, 136), (seed / "ROOT.A").read_bytes()[128:152]
            ),
            "object alias or cycle",
            True,
        ),
        (
            "future-schema",
            "catalog",
            lambda b: put(b, 112, 7, 8),
            "future schema reference",
            True,
        ),
        (
            "duplicate-table",
            "catalog",
            lambda b: put(b, 192, 1, 8),
            "table identity/name length",
            True,
        ),
        (
            "row-count",
            "catalog",
            lambda b: put(b, 160, 17, 8),
            "table index header",
            True,
        ),
        (
            "schema-count",
            "schema",
            lambda b: put(b, 12, 65),
            "schema count/extent",
            True,
        ),
        (
            "catalog-count",
            "catalog",
            lambda b: put(b, 12, 65),
            "catalog count/extent",
            True,
        ),
        (
            "schema-padding",
            "schema",
            lambda b: put(b, 40, 1),
            "nonzero reserved bytes",
            True,
        ),
        (
            "column-identity",
            "schema",
            lambda b: put(b, 112, 1),
            "duplicate column identity/name",
            True,
        ),
        (
            "catalog-identity",
            "catalog",
            lambda b: put(b, 32, 4, 8),
            "catalog identity",
            True,
        ),
        (
            "repeated-unit",
            "index",
            lambda b: b.__setitem__(slice(112, 136), b[64:88]),
            "unit order/creator",
            True,
        ),
        ("row-gap", "index", lambda b: put(b, 136, 9, 8), "unit row coverage", True),
        (
            "payload-overlap",
            "unit",
            lambda b: put(b, 104, 192, 8),
            "column type/coverage",
            True,
        ),
        (
            "unit-column-alias",
            "unit",
            lambda b: put(b, 96, 1),
            "unit column identity",
            True,
        ),
        ("unit-type", "unit", lambda b: put(b, 68, 4, 1), "column type/coverage", True),
        ("history-order", "history", lambda b: put(b, 72, 1, 8), "history order", True),
        ("history-last", "history", lambda b: put(b, 56, 4, 8), "history header", True),
        (
            "null-number",
            "payload",
            lambda b: put(b, g.uint(b, 104, 8) + 1 + 7 * 8, 1, 8),
            "nonzero reserved bytes",
            "query",
        ),
        (
            "date-domain",
            "payload",
            lambda b: put(b, g.uint(b, 168, 8) + 1, 2932897),
            "DATE domain",
            "query",
        ),
        (
            "invalid-utf8",
            "payload",
            lambda b: put(b, g.uint(b, 136, 8) + 1 + 9 * 4, 255, 1),
            "string UTF-8",
            "query",
        ),
        (
            "string-offset",
            "payload",
            lambda b: put(b, g.uint(b, 136, 8) + 1, 1),
            "initial string offset",
            "query",
        ),
        (
            "validity-padding",
            "last-payload",
            lambda b: put(b, 192, 129, 1),
            "validity padding",
            "query",
        ),
    ]
    return mutations


def check_references(check, seed):
    for label, kind, edit, error, public in reference_mutations(seed):
        check(label, lambda m, k=kind, e=edit: m.change(k, e), error, public)


def check_namespace_and_authority(check):
    check("newer-corrupt-older-valid", newer_corrupt, "object checksum", True)
    check(
        "missing-schema",
        lambda m: (m.path / "units" / m.schema.name).unlink(),
        "missing reference",
        True,
    )
    check(
        "unknown-name",
        lambda m: (m.path / "unexpected").touch(),
        "namespace names",
        True,
    )
    check(
        "foreign-root",
        lambda m: m.roots(lambda b: put(b, 16, g.uint(b, 16) ^ 1)),
        "foreign snapshot identity",
        True,
    )
    check(
        "unknown-version",
        lambda m: m.roots(lambda b: put(b, 8, 8)),
        "unsupported authoritative version",
        True,
    )
    check(
        "overlong-reference",
        lambda m: m.roots(lambda b: put(b, 140, 8257)),
        "root reference extent",
        True,
    )
    check(
        "damaged-payload",
        lambda m: damage(m.path / "units" / g.object_id(4, 1), 192),
        "payload checksum",
        "query",
    )
    check(
        "damaged-root",
        lambda m: damage(m.path / "ROOT.A", 108),
        verify=lambda r: (
            expected(r),
            g.require(not r["roots_settled"], "expected repair"),
        ),
    )
    check("missing-root", lambda m: (m.path / "ROOT.B").unlink(), verify=expected)
    check(
        "both-roots-damaged",
        lambda m: (damage(m.path / "ROOT.A", 108), damage(m.path / "ROOT.B", 108)),
        "insufficient root authority",
        True,
    )
    check(
        "nonadjacent-roots",
        lambda m: change_root(m.path / "ROOT.A", lambda b: put(b, 112, 8, 8)),
        "nonadjacent roots",
        True,
    )
    check(
        "ahead-fence",
        lambda m: change_root(m.path / "WAL", lambda b: put(b, 112, 7, 8)),
        verify=expected,
    )
    check(
        "ahead-fence-missing-root",
        lambda m: (
            (m.path / "ROOT.B").unlink(),
            change_root(m.path / "WAL", lambda b: put(b, 112, 7, 8)),
        ),
        "insufficient root authority",
        True,
    )
    extra = "0000000000000003-00000001.obj"
    check(
        "unreferenced-object",
        lambda m: (m.path / "units" / extra).touch(),
        verify=lambda r: (
            expected(r),
            g.require(extra in r["unreferenced"], "missing unreferenced object"),
        ),
    )
    check(
        "symlink-object",
        lambda m: (m.path / "units" / extra).symlink_to(m.schema.name),
        "object ownership/extent",
        True,
    )
    check(
        "aliased-object",
        lambda m: os.link(m.path / "units" / m.schema.name, m.path / "units" / extra),
        "object ownership/extent",
        True,
    )
    check(
        "oversize-object",
        lambda m: sparse(m.path / "units" / extra, 33556545),
        "object ownership/extent",
        True,
    )


def check_cli_limits(seed):
    # The exported CLI preserves refusal categories and fails without partial JSON.
    for option, value in [
        ("--max-values", "1"),
        ("--max-read-bytes", "1"),
        ("--max-objects", "1"),
    ]:
        result = run_process(
            [
                sys.executable,
                str(ROOT / "tools/catalog_graph.py"),
                str(seed),
                option,
                value,
            ],
            capture_output=True,
            text=True,
            timeout=20,
            cwd=ROOT,
        )
        assert (
            result.returncode == 3
            and not result.stdout
            and result.stderr.startswith("limit:")
        )


def check_oracle_controls(baseline):
    # Wrong semantic expectations test the consumer, independently of byte mutation.
    wrong = json.loads(json.dumps(baseline))
    wrong["tables"][0]["rows"][0][0] = 0
    try:
        expected(wrong)
    except AssertionError:
        pass
    else:
        raise AssertionError("wrong row expectation accepted")
    wrong = json.loads(json.dumps(baseline))
    wrong["successes"] = [1, 2, 3, 5]
    try:
        expected(wrong)
    except AssertionError:
        pass
    else:
        raise AssertionError("wrong receipt expectation accepted")


def write_report(work, source, driver, records):
    assert (
        source
        == run_process(
            [sys.executable, str(ROOT / "tools/source-manifest.py")],
            cwd=ROOT,
            timeout=30,
            check=True,
            stdout=subprocess.PIPE,
        ).stdout
    )
    record = dict(
        base=source_revision(ROOT),
        cases=len(records),
        oracle_controls=2,
        cli_limits=3,
        genesis=True,
        lease_contention=True,
        independent_column_order=True,
        source_manifest_sha256=hashlib.sha256(source).hexdigest(),
        driver_sha256=hashlib.sha256(driver.read_bytes()).hexdigest(),
        library_sha256=hashlib.sha256(
            (work / "target/release/libpipesql.rlib").read_bytes()
        ).hexdigest(),
    )
    (work / "result.json").write_text(json.dumps(record, indent=2) + "\n")
    print("catalog graph passed: " + json.dumps(record), flush=True)


def campaign(work):
    source, driver = build_driver(work)
    seed, baseline = create_seed(work, driver)
    check_genesis_lease_and_fixture(work, driver, seed)
    checks = GraphChecks(work, seed, driver)
    check = checks.case
    check_budgets(check, baseline)
    check_references(check, seed)
    check_namespace_and_authority(check)
    check_cli_limits(seed)
    check_oracle_controls(baseline)
    write_report(work, source, driver, checks.records)


def newer_corrupt(mutation):
    # Retained older immutable objects are valid. Only the selected newer graph
    # is damaged; neither checker nor public open may hide it by falling back.
    mutation.roots(lambda data: put(data, 112, 5, 8))

    def old(data):
        put(data, 40, 3, 8)
        put(data, 72, 4, 8)
        for at, ordinal in [(128, 4), (152, 5)]:
            contents = (mutation.path / "units" / g.object_id(4, ordinal)).read_bytes()
            put(data, at, 4, 8)
            put(data, at + 8, ordinal)
            put(data, at + 12, len(contents))
            put(data, at + 16, g.crc32c(contents))

    change_root(mutation.path / "ROOT.B", old)
    damage(mutation.path / "units" / mutation.cat.name, 64)


def damage(path, offset):
    data = bytearray(path.read_bytes())
    data[offset] ^= 1
    path.write_bytes(data)


def change_root(path, edit):
    data = bytearray(path.read_bytes())
    edit(data)
    put(data, 108, 0)
    put(data, 108, g.crc32c(data))
    path.write_bytes(data)


def sparse(path, size):
    with path.open("wb") as stream:
        stream.truncate(size)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, help="new replay output directory")
    args = parser.parse_args(argv)
    if not __debug__:
        parser.error("catalog graph checks require Python assertions")
    if sys.platform not in ("darwin", "linux"):
        parser.error("catalog graph checks require macOS or Linux")
    if args.output:
        work = args.output.absolute()
        work.mkdir()
        campaign(work)
    else:
        with tempfile.TemporaryDirectory(prefix="pipesql-catalog-graph-") as directory:
            campaign(Path(directory).resolve())


if __name__ == "__main__":
    main()
