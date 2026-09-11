#!/usr/bin/env python3
"""Reproduce retained codec fixtures using independent Python encoders.

The encoders are independent development oracles, not production imports. This
checks provenance drift, not recovery safety or the full corruption campaigns.
"""

import runpy
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def check_fixtures() -> None:
    if not __debug__:
        raise SystemExit(
            "fixture oracles require Python assertions (no -O/PYTHONOPTIMIZE)"
        )
    count = 0
    for revision, oracle, directory in [
        (1, "single_table", "rejected-single-table-format"),
        (2, "attempt_reuse", "rejected-attempt-reuse-format"),
    ]:
        probe = runpy.run_path(str(ROOT / f"tools/oracles/{oracle}.py"))
        unit, metadata, projection, _ = (
            probe["encode_unit"]() if revision == 1 else probe["encode_unit"](1)
        )
        source = (
            b"golden-input-row\n"
            if revision == 1
            else b"1|2|3|4|1|2|0.5|0.25|A|F|1970-01-01|12|13|14|15|16|\n"
        )
        expected = {
            "CONTROL": probe["encode_control"](),
            "ROOT.A": probe["encode_root"](0, len(unit), metadata),
            "ROOT.B": probe["encode_root"](1, len(unit), metadata),
            "WAL": probe["encode_wal"](
                len(unit), metadata, projection, probe["crc32c"](source)
            ),
            "UNIT": unit,
        }
        if revision == 2:
            expected["EMPTY.UNIT"] = probe["encode_unit"](0)[0]
        fixtures = ROOT / "tests/fixtures" / directory
        assert {path.name for path in fixtures.iterdir()} == set(expected), directory
        for name, generated in expected.items():
            path = fixtures / name
            assert not path.is_symlink() and path.is_file(), path
            assert path.stat().st_size == len(generated), path
            assert path.read_bytes() == generated, path
            count += 1
    current = runpy.run_path(str(ROOT / "tools/snapshot-fixtures.py"))["vectors"]()
    directory = ROOT / "tests/fixtures/current-single-table-format"
    assert {path.name for path in directory.iterdir()} == set(current)
    for name, generated in current.items():
        path = directory / name
        assert (
            path.is_file() and not path.is_symlink() and path.read_bytes() == generated
        ), path
        count += 1
    candidate = runpy.run_path(str(ROOT / "tools/candidate-fixtures.py"))["vectors"]
    for version, name in [
        (3, "rejected-multi-table-format"),
        (5, "candidate-multi-table-format"),
    ]:
        expected = candidate(version)
        directory = ROOT / "tests/fixtures" / name
        assert {path.name for path in directory.iterdir()} == set(expected)
        for name, generated in expected.items():
            path = directory / name
            assert (
                path.is_file()
                and not path.is_symlink()
                and path.read_bytes() == generated
            ), path
            count += 1
    catalog = runpy.run_path(str(ROOT / "tools/catalog-root-fixture.py"))["expected"]()
    directory = ROOT / "tests/fixtures/catalog-roots"
    assert {path.name for path in directory.iterdir()} == set(catalog)
    for name, generated in catalog.items():
        path = directory / name
        assert (
            path.is_file() and not path.is_symlink() and path.read_bytes() == generated
        ), path
        count += 1
    print(f"independently reproduced codec fixtures={count}")


if __name__ == "__main__":
    check_fixtures()
