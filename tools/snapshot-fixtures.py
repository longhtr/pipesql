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
