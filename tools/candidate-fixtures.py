"""Independent retained candidate vectors (rejected format 3 and replacement 5).

No production imports or filesystem writes. Native geometry and checksum domain
are unchanged; version-dependent unit checksums are regenerated independently.
"""

import runpy
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LEGACY = runpy.run_path(str(ROOT / "tools/oracles/multi_table.py"))
CRC = LEGACY["crc32c"]
DATABASE = bytes(range(16))


def vectors(version):
    if not __debug__:
        raise SystemExit("fixture oracle requires assertions")
    assert version in (3, 5)
    units = {table: LEGACY["encode_unit"](table, 1) for table in (1, 2)}
    if version == 5:
        for table, (unit, _, projection) in units.items():
            changed = bytearray(unit)
            struct.pack_into("<I", changed, 8, 5)
            changed = LEGACY["refresh_unit"](changed)
            units[table] = (
                changed,
                struct.unpack_from("<I", changed, 108)[0],
                projection,
            )
    result = {"LINEITEM.UNIT": units[1][0], "PART.UNIT": units[2][0]}
    if version == 3:
        result.update(
            {
                "CONTROL": LEGACY["encode_control"](),
                "ROOT.A": LEGACY["encode_root"](0, units),
                "ROOT.B": LEGACY["encode_root"](1, units),
                "WAL": LEGACY["encode_wal"](units),
                "EMPTY.ROOT.A": LEGACY["encode_empty_root"](0),
            }
        )
        return result
    control = bytearray(128)
    control[:8] = b"PIPESQL\0"
    struct.pack_into("<II", control, 8, 5, 128)
    control[16:32] = DATABASE
    struct.pack_into("<I", control, 36, CRC(control))
    result["CONTROL"] = bytes(control)
    for name, magic, size, replica, data in (
        ("ROOT.A", b"PSQLROOT", 4096, 0, True),
        ("ROOT.B", b"PSQLROOT", 4096, 1, True),
        ("WAL", b"PSQLWAL\0", 512, 0, True),
        ("EMPTY.ROOT.A", b"PSQLROOT", 4096, 0, False),
    ):
        value = bytearray(size)
        value[:8] = magic
        struct.pack_into("<II", value, 8, 5, size)
        value[16:32] = DATABASE
        value[32] = replica
        if data:
            struct.pack_into("<Q", value, 40, 1)
            value[56:80] = DATABASE + struct.pack("<Q", 1)
            struct.pack_into("<I", value, 80, 2)
            for table, base, input_crc in ((1, 88, 0x11223344), (2, 128, 0x55667788)):
                unit, metadata, projected = units[table]
                struct.pack_into("<I", value, base, table)
                struct.pack_into(
                    "<QQIII",
                    value,
                    base + 8,
                    1,
                    len(unit),
                    metadata,
                    projected,
                    input_crc,
                )
            struct.pack_into("<Q", value, 176, 1)
        struct.pack_into("<I", value, 168, CRC(value))
        result[name] = bytes(value)
    return result
