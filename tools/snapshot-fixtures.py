"""Independent format-4 vectors; no production imports or filesystem mutation.

Native geometry is unchanged from the retained format-2 encoder. Regenerate its
version-dependent checksums, then build snapshot authority independently. The
loaded vector commits attempt 2, preserving attempt 1 as a settled abort gap.
"""

import runpy
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LEGACY = runpy.run_path(str(ROOT / "tools/oracles/attempt_reuse.py"))
CRC = LEGACY["crc32c"]
DATABASE = bytes(range(16))


def vectors():
    if not __debug__:
        raise SystemExit("fixture oracle requires assertions")
    result = {}
    for name, rows in (("UNIT", 1), ("EMPTY.UNIT", 0)):
        unit = bytearray(LEGACY["encode_unit"](rows)[0])
        struct.pack_into("<I", unit, 8, 4)
        unit = LEGACY["refresh_unit_checksums"](unit)
        result[name] = bytes(unit)
    unit = result["UNIT"]
    metadata_crc = struct.unpack_from("<I", unit, 108)[0]
    control = bytearray(128)
    control[:8] = b"PIPESQL\0"
    struct.pack_into("<II", control, 8, 4, 128)
    control[16:32] = DATABASE
    struct.pack_into("<I", control, 36, CRC(control))
    result["CONTROL"] = bytes(control)
    for name, magic, size, replica in (
        ("ROOT.A", b"PSQLROOT", 4096, 0),
        ("ROOT.B", b"PSQLROOT", 4096, 1),
        ("WAL", b"PSQLWAL\0", 512, 0),
    ):
        data = bytearray(size)
        data[:8] = magic
        struct.pack_into("<II", data, 8, 4, size)
        data[16:32] = DATABASE
        data[32] = replica
        struct.pack_into("<Q", data, 40, 1)
        data[56:80] = DATABASE + struct.pack("<Q", 2)
        struct.pack_into("<QQQ", data, 80, 1, 1, len(unit))
        struct.pack_into("<I", data, 104, metadata_crc)
        struct.pack_into("<Q", data, 112, 2)
        struct.pack_into("<I", data, 108, CRC(data))
        result[name] = bytes(data)
    return result


def write_snapshot(path, rows):
    """Encode a valid format-4 snapshot independently of the engine decoder."""
    assert all(len(row) == 7 for row in rows)
    crc = CRC
    path.mkdir()
    (path / "units").mkdir()
    (path / "private").mkdir()
    snapshot = vectors()
    header = bytearray(snapshot["UNIT"][:4096])
    descriptors = bytearray(1344 * 16)
    columns = [
        b"".join(struct.pack("<Q", row[column]) for row in rows)
        for column in range(4)
    ]
    columns += [
        bytes(row[4] for row in rows),
        bytes(row[5] for row in rows),
        b"".join(struct.pack("<i", row[6]) for row in rows),
    ]
    payload = b"".join(columns)
    offset, index = 28672, 0
    for column, data in enumerate(columns):
        width = 8 if column < 4 else (1 if column < 6 else 4)
        # String blocks share the DOUBLE row geometry in this stored format.
        rows_per_block = 65536 if column == 6 else 32768
        block_bytes = rows_per_block * width
        count = (len(rows) + rows_per_block - 1) // rows_per_block
        old = struct.unpack_from("<IIIIIIQQ", header, 128 + column * 40)
        struct.pack_into(
            "<IIIIIIQQ",
            header,
            128 + column * 40,
            old[0],
            old[1],
            width,
            rows_per_block,
            index,
            count,
            offset,
            len(data),
        )
        for first in range(0, len(data), block_bytes):
            block = data[first : first + block_bytes]
            struct.pack_into(
                "<QII",
                descriptors,
                index * 16,
                offset + first,
                len(block),
                crc(block),
            )
            index += 1
        offset += len(data)
    struct.pack_into("<QQ", header, 48, len(rows), offset)
    struct.pack_into("<I", header, 72, index)
    struct.pack_into("<I", header, 112, crc(payload))
    unit = LEGACY["refresh_unit_checksums"](
        header + descriptors + bytes(3072) + payload
    )
    assert len(unit) == offset
    metadata_crc = struct.unpack_from("<I", unit, 108)[0]
    for name in ("ROOT.A", "ROOT.B", "WAL"):
        value = bytearray(snapshot[name])
        struct.pack_into("<QQ", value, 88, len(rows), len(unit))
        struct.pack_into("<I", value, 104, metadata_crc)
        struct.pack_into("<I", value, 108, 0)
        struct.pack_into("<I", value, 108, crc(value))
        (path / name).write_bytes(value)
    (path / "CONTROL").write_bytes(snapshot["CONTROL"])
    (path / "LOCK").write_bytes(b"")
    (path / "units/0000000000000001.unit").write_bytes(unit)
