#!/usr/bin/env python3
"""Independent bytes for the unpromoted catalog schema experiment.

No Rust encoder is invoked. With --write, create the fixture; otherwise compare
it byte-for-byte. Numeric format 6 is an experimental discriminator, not stability.
"""
import hashlib
from pathlib import Path
import struct
import sys

ROOT = Path(__file__).resolve().parent.parent
PATH = ROOT / "tests/fixtures/catalog-schema/columns.bin"


def expected():
    header = bytearray(64)
    header[:8] = b"PSQLSCHM"
    struct.pack_into("<II", header, 8, 6, 2)
    header[16:32] = bytes([7]) * 16
    struct.pack_into("<Q", header, 32, 0x0102030405060708)
    columns = []
    for identity, name, kind, nullable in [(29, b"note", 3, 1), (3, b"amount", 2, 0)]:
        column = bytearray(48)
        struct.pack_into("<IBBB", column, 0, identity, kind, nullable, len(name))
        column[8 : 8 + len(name)] = name
        columns.append(column)
    return bytes(header + b"".join(columns))


def checksum(data):
    crc = 0xFFFFFFFF
    for byte in data:
        crc ^= byte
        for _ in range(8):
            crc = (crc >> 1) ^ (0x82F63B78 if crc & 1 else 0)
    return crc ^ 0xFFFFFFFF


if __name__ == "__main__":
    data = expected()
    if sys.argv[1:] == ["--write"]:
        PATH.write_bytes(data)
    elif sys.argv[1:]:
        raise SystemExit("usage: catalog-schema-fixture.py [--write]")
    elif PATH.read_bytes() != data:
        raise SystemExit("schema fixture differs from independent encoding")
    print(
        f"bytes={len(data)} crc32c={checksum(data):08x} sha256={hashlib.sha256(data).hexdigest()}"
    )
