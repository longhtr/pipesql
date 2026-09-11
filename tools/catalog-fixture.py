#!/usr/bin/env python3
"""Independently encode the catalog parent of the retained schema fixture."""
import hashlib
from pathlib import Path
import runpy
import struct
import sys

ROOT = Path(__file__).resolve().parent.parent
PATH = ROOT / "tests/fixtures/catalog-schema/catalog.bin"
SCHEMA = runpy.run_path(str(ROOT / "tools/catalog-schema-fixture.py"))


def expected():
    schema = SCHEMA["expected"]()
    header = bytearray(64)
    header[:8] = b"PSQLCATL"
    struct.pack_into("<II", header, 8, 6, 1)
    header[16:32] = bytes([7]) * 16
    struct.pack_into("<QI", header, 32, 4, 1)
    table = bytearray(128)
    struct.pack_into("<Q", table, 0, 0x0102030405060708)
    table[8] = 5
    table[16:21] = b"facts"
    struct.pack_into("<QIII", table, 48, 1, 1, len(schema), SCHEMA["checksum"](schema))
    return bytes(header + table)


if __name__ == "__main__":
    data = expected()
    if sys.argv[1:] == ["--write"]:
        PATH.write_bytes(data)
    elif sys.argv[1:]:
        raise SystemExit("usage: catalog-fixture.py [--write]")
    elif PATH.read_bytes() != data:
        raise SystemExit("catalog fixture differs from independent encoding")
    print(
        f"bytes={len(data)} crc32c={SCHEMA['checksum'](data):08x} sha256={hashlib.sha256(data).hexdigest()}"
    )
