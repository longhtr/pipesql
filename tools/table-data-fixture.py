#!/usr/bin/env python3
"""Independent table-data index and complete catalog parent for the unit fixture."""
import hashlib
from pathlib import Path
import runpy
import struct
import sys

ROOT = Path(__file__).resolve().parent.parent
SCHEMA = runpy.run_path(str(ROOT / "tools/catalog-schema-fixture.py"))
CATALOG = runpy.run_path(str(ROOT / "tools/catalog-fixture.py"))
UNIT = runpy.run_path(str(ROOT / "tools/native-unit-fixture.py"))


def expected():
    native = UNIT["expected"]()
    header = bytearray(64)
    header[:8] = b"PSQLTBLD"
    struct.pack_into("<II", header, 8, 6, 1)
    header[16:32] = bytes([7]) * 16
    struct.pack_into("<QQI", header, 32, 0x0102030405060708, 4, 2)
    struct.pack_into("<Q", header, 56, 4)
    entry = bytearray(48)
    struct.pack_into(
        "<QIIIIQ", entry, 0, 3, 2, 4, len(native), SCHEMA["checksum"](native[:128]), 0
    )
    index = bytes(header + entry)
    catalog = bytearray(CATALOG["expected"]())
    struct.pack_into("<QI", catalog, 32, 5, 1)
    struct.pack_into("<QIII", catalog, 136, 4, 2, len(index), SCHEMA["checksum"](index))
    struct.pack_into("<QI", catalog, 160, 4, 1)
    return {"table-data.bin": index, "data-catalog.bin": bytes(catalog)}


if __name__ == "__main__":
    if sys.argv[1:] not in ([], ["--write"]):
        raise SystemExit("usage: table-data-fixture.py [--write]")
    for name, data in expected().items():
        path = ROOT / "tests/fixtures/catalog-schema" / name
        if sys.argv[1:] == ["--write"]:
            path.write_bytes(data)
        elif path.read_bytes() != data:
            raise SystemExit(f"{name} differs from independent encoding")
        print(
            f"{name} bytes={len(data)} crc32c={SCHEMA['checksum'](data):08x} sha256={hashlib.sha256(data).hexdigest()}"
        )
