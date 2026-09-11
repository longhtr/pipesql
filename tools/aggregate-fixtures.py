"""Independent, valid format-4 aggregate fixtures; at most four rows per case."""

import hashlib
from pathlib import Path
import runpy
import struct

ROOT = Path(__file__).resolve().parent.parent
MODEL = runpy.run_path(str(ROOT / "tools/snapshot-fixtures.py"))
CRC = MODEL["CRC"]


def fixture(path, rows):
    """Encode a valid format-4 snapshot independently of the engine decoder."""
    assert 1 <= len(rows) <= 4
    assert all(len(row) == 4 for row in rows)
    path.mkdir()
    (path / "units").mkdir()
    (path / "private").mkdir()
    vectors = MODEL["vectors"]()
    header = bytearray(vectors["UNIT"][:4096])
    descriptors = bytearray(1344 * 16)
    columns = [
        b"".join(struct.pack("<Q", row[column]) for row in rows) for column in range(4)
    ]
    columns += [b"A" * len(rows), b"F" * len(rows), bytes(len(rows) * 4)]
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
                "<QII", descriptors, index * 16, offset + first, len(block), CRC(block)
            )
            index += 1
        offset += len(data)
    struct.pack_into("<QQ", header, 48, len(rows), offset)
    struct.pack_into("<I", header, 72, index)
    struct.pack_into("<I", header, 112, CRC(payload))
    unit = MODEL["LEGACY"]["refresh_unit_checksums"](
        header + descriptors + bytes(3072) + payload
    )
    assert len(unit) == offset
    metadata_crc = struct.unpack_from("<I", unit, 108)[0]
    for name in ("ROOT.A", "ROOT.B", "WAL"):
        value = bytearray(vectors[name])
        struct.pack_into("<QQ", value, 88, len(rows), len(unit))
        struct.pack_into("<I", value, 104, metadata_crc)
        struct.pack_into("<I", value, 108, 0)
        struct.pack_into("<I", value, 108, CRC(value))
        (path / name).write_bytes(value)
    (path / "CONTROL").write_bytes(vectors["CONTROL"])
    (path / "LOCK").write_bytes(b"")
    (path / "units/0000000000000001.unit").write_bytes(unit)
    return {
        str(p.relative_to(path)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted(path.rglob("*"))
        if p.is_file()
    }
