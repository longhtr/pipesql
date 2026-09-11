#!/usr/bin/env python3
"""Independent projected native-unit bytes for the catalog integration trial."""
import hashlib
from pathlib import Path
import runpy
import struct
import sys

ROOT = Path(__file__).resolve().parent.parent
PATH = ROOT / "tests/fixtures/catalog-schema/native-unit.bin"
SCHEMA = runpy.run_path(str(ROOT / "tools/catalog-schema-fixture.py"))


def expected():
    # Physical order is amount, note; the retained schema declares note, amount.
    doubles = bytes([15]) + struct.pack(
        "<QQQQ",
        0x8000000000000000,
        0x7FF0000000000001,
        0x7FF0000000000000,
        0xFFEFFFFFFFFFFFFF,
    )
    text = "雪é\0🙂".encode("utf-8")
    strings = bytes([14]) + struct.pack("<IIIII", 0, 0, 0, 3, len(text)) + text
    header = bytearray(64)
    header[:8] = b"PSQLDATA"
    struct.pack_into("<II", header, 8, 6, 2)
    header[16:32] = bytes([7]) * 16
    struct.pack_into("<QQIII", header, 32, 0x0102030405060708, 3, 2, 4, 128)
    descriptors = bytearray()
    offset = 128
    for identity, kind, nullable, payload in [(3, 2, 0, doubles), (29, 3, 1, strings)]:
        entry = bytearray(32)
        struct.pack_into("<IBB", entry, 0, identity, kind, nullable)
        struct.pack_into(
            "<QII", entry, 8, offset, len(payload), SCHEMA["checksum"](payload)
        )
        descriptors.extend(entry)
        offset += len(payload)
    return bytes(header + descriptors + doubles + strings)


if __name__ == "__main__":
    data = expected()
    if sys.argv[1:] == ["--write"]:
        PATH.write_bytes(data)
    elif sys.argv[1:]:
        raise SystemExit("usage: native-unit-fixture.py [--write]")
    elif PATH.read_bytes() != data:
        raise SystemExit("native unit differs from independent encoding")
    print(
        f"bytes={len(data)} metadata_crc32c={SCHEMA['checksum'](data[:128]):08x} sha256={hashlib.sha256(data).hexdigest()}"
    )
