#!/usr/bin/env python3
"""Strict comparison of independent Q1 CSV and stock PipeSQL typed CLI rows."""

from __future__ import annotations

import csv
import hashlib
import re
import struct
import sys
from pathlib import Path

MAX_BYTES = 4_194_304
MAX_ROWS = 8_836
MAX_INPUT_ROWS = 6_500_000


def read_bounded(path: Path) -> list[str]:
    with path.open("rb") as source:
        data = source.read(MAX_BYTES + 1)
    assert len(data) <= MAX_BYTES
    return data.decode("utf-8").splitlines()


def single_value(lines: list[str], prefix: str) -> str:
    values = [line[len(prefix) :] for line in lines if line.startswith(prefix)]
    assert len(values) == 1, (prefix, values)
    return values[0]


def natural(text: str, maximum: int) -> int:
    assert re.fullmatch(r"0|[1-9][0-9]*", text), text
    value = int(text)
    assert value <= maximum, value
    return value


def bits(text: str) -> int:
    assert re.fullmatch(r"[0-9a-f]{16}", text), text
    return int(text, 16)


def validate_double_display(text: str, encoded: int) -> None:
    # Decimal rendering is redundant public output, not permission to ignore it.
    # NaN text cannot preserve a payload/sign; every other value must round-trip
    # to the encoded bits, including signed zero and the two infinities.
    exponent = encoded & 0x7FF0000000000000
    fraction = encoded & 0x000FFFFFFFFFFFFF
    if exponent == 0x7FF0000000000000:
        if fraction:
            assert text == "NaN", text
        else:
            assert text == ("-inf" if encoded >> 63 else "inf"), text
        return
    assert re.fullmatch(r"-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?", text), text
    observed = struct.unpack("<Q", struct.pack("<d", float(text)))[0]
    assert observed == encoded, (text, f"{encoded:016x}")


def validate_rows(rows) -> None:
    prior = None
    for first, second, values, count in rows:
        for key in (first, second):
            assert len(key) == 1 and 0x20 <= key[0] <= 0x7E and key != b"|", key
        pair = (first, second)
        assert prior is None or prior < pair, "groups must be unique and ordered"
        prior = pair
        assert len(values) == 7 and 1 <= count <= MAX_INPUT_ROWS


def parse_oracle(lines: list[str]) -> list[tuple[bytes, bytes, tuple[int, ...], int]]:
    total = natural(single_value(lines, "rows="), MAX_INPUT_ROWS)
    demanded = natural(single_value(lines, "demanded="), total)
    groups = natural(single_value(lines, "groups="), MAX_ROWS)
    bits(single_value(lines, "fingerprint="))
    rows = []
    for line in lines:
        if line.startswith(("rows=", "demanded=", "groups=", "fingerprint=")):
            continue
        fields = next(csv.reader([line], strict=True))
        assert len(fields) == 18, line
        assert len(rows) < MAX_ROWS
        values = []
        for index in range(3, 17, 2):
            assert fields[index].startswith("bits=")
            encoded = bits(fields[index][5:])
            validate_double_display(fields[index - 1], encoded)
            values.append(encoded)
        count = natural(fields[16], MAX_INPUT_ROWS)
        assert fields[17].startswith("count_bits=")
        assert bits(fields[17][11:]) == count
        rows.append((fields[0].encode(), fields[1].encode(), tuple(values), count))
    assert len(rows) == groups
    assert sum(row[3] for row in rows) == demanded
    validate_rows(rows)
    return rows


def parse_production(
    lines: list[str],
) -> list[tuple[bytes, bytes, tuple[int, ...], int]]:
    assert single_value(lines, "status=") == "queried"
    assert single_value(lines, "column_count=") == "10"
    columns = single_value(lines, "columns=").split("|")
    expected = ["string:required"] * 2 + ["double:nullable"] * 7 + ["int64:required"]
    assert len(columns) == len(expected)
    assert [column.split(":", 1)[1] for column in columns] == expected
    rows = []
    for line in lines:
        if not line.startswith("row="):
            continue
        assert len(rows) < MAX_ROWS
        values = line[4:].split("|")
        assert len(values) == 10
        keys = []
        for value in values[:2]:
            assert value.startswith("string:") and re.fullmatch(
                r"[0-9a-f]{2}", value[7:]
            )
            keys.append(bytes.fromhex(value[7:]))
        doubles = []
        for value in values[2:9]:
            fields = value.split(":")
            assert len(fields) == 3 and fields[0] == "double" and fields[1]
            encoded = bits(fields[2])
            validate_double_display(fields[1], encoded)
            doubles.append(encoded)
        assert values[9].startswith("int64:")
        count = natural(values[9][6:], MAX_INPUT_ROWS)
        rows.append((keys[0], keys[1], tuple(doubles), count))
    validate_row_count(lines, len(rows))
    validate_rows(rows)
    return rows


def canonical_sha256(rows: list[tuple[bytes, bytes, tuple[int, ...], int]]) -> str:
    canonical = bytearray()
    for return_flag, line_status, values, count in rows:
        canonical.extend(return_flag)
        canonical.extend(line_status)
        for value in values:
            canonical.extend(struct.pack("<Q", value))
        canonical.extend(struct.pack("<q", count))
    return hashlib.sha256(canonical).hexdigest()


def validate_row_count(lines: list[str], rows: int) -> None:
    assert natural(single_value(lines, "row_count="), MAX_ROWS) == rows


def validate_all_groups(
    lines: list[str],
) -> list[tuple[bytes, bytes, tuple[int, ...], int]]:
    production = parse_production(lines)
    keys = bytes(byte for byte in range(0x20, 0x7F) if byte != ord("|"))
    expected_bits = tuple(
        struct.unpack("<Q", struct.pack("<d", value))[0]
        for value in [1.0, 2.0, 1.0, 1.25, 1.0, 2.0, 0.5]
    )
    expected = [
        (bytes([first]), bytes([second]), expected_bits, 1)
        for first in keys
        for second in keys
    ]
    assert len(expected) == MAX_ROWS and production == expected
    return production


def main() -> None:
    if not __debug__:
        raise SystemExit("Q1 comparison requires Python assertions")
    assert (
        len(sys.argv) == 3
    ), "usage: q1_compare.py ORACLE PRODUCTION | --all-groups PRODUCTION"
    production_lines = read_bounded(Path(sys.argv[2]))
    if sys.argv[1] == "--all-groups":
        production = validate_all_groups(production_lines)
    else:
        oracle = parse_oracle(read_bounded(Path(sys.argv[1])))
        production = parse_production(production_lines)
        assert oracle == production
    print(f"rows={len(production)}")
    print(f"values={len(production) * 10}")
    print(f"canonical_sha256={canonical_sha256(production)}")


if __name__ == "__main__":
    main()
