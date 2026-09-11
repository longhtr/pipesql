#!/usr/bin/env python3
"""Independent catalog-root graph with successes 3/5 and issued abort gap 6."""
import hashlib
from pathlib import Path
import runpy
import struct
import sys

ROOT = Path(__file__).resolve().parent.parent
BASE = ROOT / "tests/fixtures/catalog-roots"
SCHEMA = runpy.run_path(str(ROOT / "tools/catalog-schema-fixture.py"))
DATA = runpy.run_path(str(ROOT / "tools/table-data-fixture.py"))
CRC = SCHEMA["checksum"]


def expected():
    source = DATA["expected"]()
    index = bytearray(source["table-data.bin"])
    struct.pack_into("<QI", index, 40, 5, 3)
    catalog = bytearray(source["data-catalog.bin"])
    struct.pack_into("<QI", catalog, 32, 5, 4)
    struct.pack_into("<Q", catalog, 112, 3)  # schema creator: successful attempt 3
    struct.pack_into("<QIII", catalog, 136, 5, 3, len(index), CRC(index))
    successes = bytearray(64)
    successes[:8] = b"PSQLSUCC"
    struct.pack_into("<I", successes, 8, 6)
    successes[16:32] = bytes([7]) * 16
    struct.pack_into("<QQI", successes, 32, 2, 5, 5)
    struct.pack_into("<Q", successes, 56, 5)
    successes.extend(struct.pack("<QQ", 3, 5))
    output = {
        "catalog.bin": bytes(catalog),
        "table-data.bin": bytes(index),
        "successes.bin": bytes(successes),
    }
    control = bytearray(128)
    control[:8] = b"PIPESQL\0"
    struct.pack_into("<II", control, 8, 7, 128)
    control[16:32] = bytes([7]) * 16
    struct.pack_into("<I", control, 36, CRC(control))
    output["CONTROL"] = bytes(control)
    for name, size, magic, role, generation, issued in [
        ("ROOT.A", 4096, b"PSQLROOT", 0, 2, 6),
        ("ROOT.B", 4096, b"PSQLROOT", 1, 2, 6),
        ("WAL", 512, b"PSQLWAL\0", 0, 2, 6),
        ("GENESIS", 4096, b"PSQLROOT", 0, 0, 2),
    ]:
        record = bytearray(size)
        record[:8] = magic
        struct.pack_into("<II", record, 8, 7, size)
        record[16:32] = bytes([7]) * 16
        record[32] = role
        struct.pack_into("<Q", record, 40, generation)
        struct.pack_into("<Q", record, 112, issued)
        if generation:
            record[56:72] = bytes([7]) * 16
            struct.pack_into("<Q", record, 72, 5)
            struct.pack_into("<QIII", record, 128, 5, 4, len(catalog), CRC(catalog))
            struct.pack_into("<QIII", record, 152, 5, 5, len(successes), CRC(successes))
        struct.pack_into("<I", record, 108, CRC(record))
        output[name] = bytes(record)
    return output


if __name__ == "__main__":
    if sys.argv[1:] not in ([], ["--write"]):
        raise SystemExit("usage: catalog-root-fixture.py [--write]")
    if sys.argv[1:]:
        BASE.mkdir(parents=True, exist_ok=True)
    for name, data in expected().items():
        path = BASE / name
        if sys.argv[1:]:
            path.write_bytes(data)
        elif path.read_bytes() != data:
            raise SystemExit(f"{name} differs from independent encoding")
        print(
            f"{name} bytes={len(data)} crc32c={CRC(data):08x} sha256={hashlib.sha256(data).hexdigest()}"
        )
