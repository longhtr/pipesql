"""Independent catalog/schema/unit/root fixture encoders.

Keep literal layouts and checksum calculations separate from production codecs
and the persisted-graph inspector. tools/check-fixtures.py compares these bytes
with retained fixtures; importing this module does no file I/O.
"""

import argparse
from pathlib import Path
import struct


def checksum(data):
    crc = 0xFFFFFFFF
    for byte in data:
        crc ^= byte
        for _ in range(8):
            crc = (crc >> 1) ^ (0x82F63B78 if crc & 1 else 0)
    return crc ^ 0xFFFFFFFF


def encode_schema():
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


def encode_catalog():
    schema_bytes = encode_schema()
    header = bytearray(64)
    header[:8] = b"PSQLCATL"
    struct.pack_into("<II", header, 8, 6, 1)
    header[16:32] = bytes([7]) * 16
    struct.pack_into("<QI", header, 32, 4, 1)
    table = bytearray(128)
    struct.pack_into("<Q", table, 0, 0x0102030405060708)
    table[8] = 5
    table[16:21] = b"facts"
    struct.pack_into("<QIII", table, 48, 1, 1, len(schema_bytes), checksum(schema_bytes))
    return bytes(header + table)


def encode_native_unit():
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
            "<QII", entry, 8, offset, len(payload), checksum(payload)
        )
        descriptors.extend(entry)
        offset += len(payload)
    return bytes(header + descriptors + doubles + strings)


def encode_table_data():
    native = encode_native_unit()
    header = bytearray(64)
    header[:8] = b"PSQLTBLD"
    struct.pack_into("<II", header, 8, 6, 1)
    header[16:32] = bytes([7]) * 16
    struct.pack_into("<QQI", header, 32, 0x0102030405060708, 4, 2)
    struct.pack_into("<Q", header, 56, 4)
    entry = bytearray(48)
    struct.pack_into(
        "<QIIIIQ", entry, 0, 3, 2, 4, len(native), checksum(native[:128]), 0
    )
    index = bytes(header + entry)
    catalog = bytearray(encode_catalog())
    struct.pack_into("<QI", catalog, 32, 5, 1)
    struct.pack_into("<QIII", catalog, 136, 4, 2, len(index), checksum(index))
    struct.pack_into("<QI", catalog, 160, 4, 1)
    return {"table-data.bin": index, "data-catalog.bin": bytes(catalog)}


def encode_roots():
    source = encode_table_data()
    index = bytearray(source["table-data.bin"])
    struct.pack_into("<QI", index, 40, 5, 3)
    catalog = bytearray(source["data-catalog.bin"])
    struct.pack_into("<QI", catalog, 32, 5, 4)
    struct.pack_into("<Q", catalog, 112, 3)  # schema creator: successful attempt 3
    struct.pack_into("<QIII", catalog, 136, 5, 3, len(index), checksum(index))
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
    struct.pack_into("<I", control, 36, checksum(control))
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
            struct.pack_into("<QIII", record, 128, 5, 4, len(catalog), checksum(catalog))
            struct.pack_into("<QIII", record, 152, 5, 5, len(successes), checksum(successes))
        struct.pack_into("<I", record, 108, checksum(record))
        output[name] = bytes(record)
    return output


def vectors():
    return {
        "catalog-schema": {
            "columns.bin": encode_schema(),
            "catalog.bin": encode_catalog(),
            "native-unit.bin": encode_native_unit(),
            **encode_table_data(),
        },
        "catalog-roots": encode_roots(),
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new directory for generated vectors")
    output = parser.parse_args(argv).output
    output.mkdir()  # Refuse existing paths, including dangling symlinks.
    for directory, files in vectors().items():
        destination = output / directory
        destination.mkdir()
        for name, data in files.items():
            (destination / name).write_bytes(data)
    print(f"generated catalog fixtures: {output.resolve()}")


if __name__ == "__main__":
    main()
