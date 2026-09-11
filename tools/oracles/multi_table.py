#!/usr/bin/env python3
"""Independent format-3 golden/layout checker;never imported by PipeSQL."""

import hashlib
import struct

VERSION = 3
CONTROL_BYTES = 128
ROOT_BYTES = 4_096
WAL_BYTES = 512
HEADER_BYTES = 4_096
DESCRIPTOR_CAPACITY = 1_536
DESCRIPTOR_BYTES = DESCRIPTOR_CAPACITY * 16
PAYLOAD_OFFSET = 28_672
LINEITEM = 1
PART = 2
PART_SORTED_UNIQUE = 1
LINEITEM_MAX_ROWS = 6_500_000
PART_MAX_ROWS = 200_000
PART_RUN_ROWS = 32_768
PART_RUN_RECORD_BYTES = 34
PART_RUN_HEADER_BYTES = 64
PART_SORT_ROW_BYTES = 40
LOAD_SOURCE_CHUNK_BYTES = 65_536
LOAD_ROW_BYTES = 512
LOAD_OUTPUT_BUFFER_BYTES = 65_536
LOAD_FIXED_OWNER_BYTES = 65_536
DATABASE_ID = bytes(range(16))
TRANSACTION_ID = DATABASE_ID + struct.pack("<Q", 1)


def crc32c(data: bytes) -> int:
    crc = 0xFFFFFFFF
    for byte in data:
        crc ^= byte
        for _ in range(8):
            mask = -(crc & 1) & 0xFFFFFFFF
            crc = ((crc >> 1) ^ (0x82F63B78 & mask)) & 0xFFFFFFFF
    return crc ^ 0xFFFFFFFF


def put_u32(target: bytearray, offset: int, value: int) -> None:
    struct.pack_into("<I", target, offset, value)


def put_u64(target: bytearray, offset: int, value: int) -> None:
    struct.pack_into("<Q", target, offset, value)


def read_u32(source: bytes, offset: int) -> int:
    return struct.unpack_from("<I", source, offset)[0]


def read_u64(source: bytes, offset: int) -> int:
    return struct.unpack_from("<Q", source, offset)[0]


def with_zero(source: bytes, start: int, end: int) -> bytes:
    result = bytearray(source)
    result[start:end] = bytes(end - start)
    return bytes(result)


def ceiling_div(value: int, divisor: int) -> int:
    return 0 if value == 0 else (value - 1) // divisor + 1


def lineitem_layout(rows: int) -> tuple[int, int]:
    assert 0 <= rows <= LINEITEM_MAX_ROWS
    descriptors = 7 * ceiling_div(rows, 32_768) + ceiling_div(rows, 65_536)
    assert descriptors <= DESCRIPTOR_CAPACITY
    return descriptors, PAYLOAD_OFFSET + rows * 46


def part_layout(rows: int) -> tuple[int, int]:
    assert 0 <= rows <= PART_MAX_ROWS
    descriptors = ceiling_div(rows, 32_768) + ceiling_div(rows, 8_192)
    assert descriptors <= DESCRIPTOR_CAPACITY
    return descriptors, PAYLOAD_OFFSET + rows * 33


LINEITEM_COLUMNS = [
    (1, 1, 8, 32_768, struct.pack("<d", 1.0)),
    (2, 1, 8, 32_768, struct.pack("<d", 2.0)),
    (3, 1, 8, 32_768, struct.pack("<d", 0.5)),
    (4, 1, 8, 32_768, struct.pack("<d", 0.25)),
    (5, 3, 1, 32_768, b"R"),
    (6, 3, 1, 32_768, b"A"),
    (7, 2, 4, 65_536, struct.pack("<i", 0)),
    (8, 4, 8, 32_768, struct.pack("<Q", 7)),
]
PART_TYPE = b"PROMO TEST"
PART_COLUMNS = [
    (1, 4, 8, 32_768, struct.pack("<Q", 7)),
    (2, 5, 25, 8_192, PART_TYPE + bytes(25 - len(PART_TYPE))),
]


def columns_for(table: int):
    if table == LINEITEM:
        return LINEITEM_COLUMNS, 0
    if table == PART:
        return PART_COLUMNS, PART_SORTED_UNIQUE
    raise AssertionError("unknown table")


def block_crc(
    table: int,
    column: int,
    kind: int,
    block: int,
    row_start: int,
    row_count: int,
    payload: bytes,
) -> int:
    context = struct.pack(
        "<8s16sQIIIIQII",
        b"PSQLBLK3",
        DATABASE_ID,
        1,
        table,
        column,
        kind,
        block,
        row_start,
        row_count,
        len(payload),
    )
    assert len(context) == 64
    return crc32c(context + payload)


def projected_crc(table: int, rows: int) -> int:
    assert rows in (0, 1)
    if rows == 0:
        return crc32c(b"")
    if table == LINEITEM:
        return crc32c(b"".join(column[4] for column in LINEITEM_COLUMNS))
    return crc32c(struct.pack("<Q", 7) + bytes([len(PART_TYPE)]) + PART_TYPE)


def encode_control() -> bytes:
    result = bytearray(CONTROL_BYTES)
    result[:8] = b"PIPESQL\0"
    put_u32(result, 8, VERSION)
    put_u32(result, 12, CONTROL_BYTES)
    result[16:32] = DATABASE_ID
    put_u32(result, 36, crc32c(bytes(result)))
    return bytes(result)


def decode_control(source: bytes) -> None:
    assert len(source) == CONTROL_BYTES and source[:8] == b"PIPESQL\0"
    assert read_u32(source, 8) == VERSION and read_u32(source, 12) == CONTROL_BYTES
    assert source[16:32] == DATABASE_ID and source[32:36] == bytes(4)
    assert source[40:] == bytes(CONTROL_BYTES - 40)
    assert read_u32(source, 36) == crc32c(with_zero(source, 36, 40))


def encode_unit(table: int, rows: int) -> tuple[bytes, int, int]:
    assert rows in (0, 1)
    columns, flags = columns_for(table)
    payloads = [column[4] for column in columns] if rows else []
    descriptor_area = bytearray(DESCRIPTOR_BYTES)
    offset = PAYLOAD_OFFSET
    for index, ((column, kind, _width, _rows_per_block, _value), payload) in enumerate(
        zip(columns, payloads)
    ):
        checksum = block_crc(table, column, kind, 0, 0, 1, payload)
        struct.pack_into(
            "<QII", descriptor_area, index * 16, offset, len(payload), checksum
        )
        offset += len(payload)

    descriptor_crc = crc32c(bytes(descriptor_area))
    header = bytearray(HEADER_BYTES)
    header[:8] = b"PSQLUNIT"
    put_u32(header, 8, VERSION)
    put_u32(header, 12, HEADER_BYTES)
    header[16:32] = DATABASE_ID
    put_u64(header, 32, 1)
    put_u32(header, 40, table)
    put_u32(header, 44, flags)
    put_u64(header, 48, rows)
    put_u64(header, 56, offset)
    put_u64(header, 64, HEADER_BYTES)
    put_u32(header, 72, rows * len(columns))
    put_u32(header, 76, DESCRIPTOR_CAPACITY)
    put_u64(header, 80, DESCRIPTOR_BYTES)
    put_u64(header, 88, PAYLOAD_OFFSET)
    put_u32(header, 96, len(columns))
    put_u32(header, 104, descriptor_crc)
    put_u32(header, 112, projected_crc(table, rows))

    region = PAYLOAD_OFFSET
    descriptor = 0
    for index, (column, kind, width, rows_per_block, _value) in enumerate(columns):
        base = 128 + index * 40
        block_count = rows
        region_bytes = rows * width
        struct.pack_into(
            "<IIIIIIQQ",
            header,
            base,
            column,
            kind,
            width,
            rows_per_block,
            descriptor,
            block_count,
            region,
            region_bytes,
        )
        descriptor += block_count
        region += region_bytes
    assert region == offset
    metadata_crc = crc32c(with_zero(bytes(header), 100, 112) + bytes(descriptor_area))
    put_u32(header, 108, metadata_crc)
    put_u32(header, 100, crc32c(with_zero(bytes(header), 100, 104)))
    unit = bytes(header) + bytes(descriptor_area) + b"".join(payloads)
    assert len(unit) == offset
    return unit, metadata_crc, projected_crc(table, rows)


def decode_unit(
    source: bytes, expected_table: int, expected_rows: int
) -> tuple[int, int]:
    columns, flags = columns_for(expected_table)
    expected_descriptors, expected_bytes = (
        lineitem_layout(expected_rows)
        if expected_table == LINEITEM
        else part_layout(expected_rows)
    )
    assert len(source) == expected_bytes and source[:8] == b"PSQLUNIT"
    header = source[:HEADER_BYTES]
    descriptors = source[HEADER_BYTES:PAYLOAD_OFFSET]
    assert read_u32(header, 8) == VERSION and read_u32(header, 12) == HEADER_BYTES
    assert header[16:32] == DATABASE_ID and read_u64(header, 32) == 1
    assert read_u32(header, 40) == expected_table and read_u32(header, 44) == flags
    assert read_u64(header, 48) == expected_rows and read_u64(header, 56) == len(source)
    assert read_u64(header, 64) == HEADER_BYTES
    assert read_u32(header, 72) == expected_descriptors
    assert read_u32(header, 76) == DESCRIPTOR_CAPACITY
    assert read_u64(header, 80) == DESCRIPTOR_BYTES
    assert read_u64(header, 88) == PAYLOAD_OFFSET
    assert read_u32(header, 96) == len(columns)
    assert header[116:128] == bytes(12)
    assert header[128 + len(columns) * 40 :] == bytes(
        HEADER_BYTES - 128 - len(columns) * 40
    )
    assert read_u32(header, 100) == crc32c(with_zero(header, 100, 104))
    assert read_u32(header, 104) == crc32c(descriptors)
    assert read_u32(header, 108) == crc32c(with_zero(header, 100, 112) + descriptors)

    offset = PAYLOAD_OFFSET
    descriptor_index = 0
    for index, (column, kind, width, rows_per_block, value) in enumerate(columns):
        base = 128 + index * 40
        block_count = expected_rows
        region_bytes = expected_rows * width
        assert struct.unpack_from("<IIIIIIQQ", header, base) == (
            column,
            kind,
            width,
            rows_per_block,
            descriptor_index,
            block_count,
            offset,
            region_bytes,
        )
        if expected_rows:
            descriptor = struct.unpack_from("<QII", descriptors, descriptor_index * 16)
            assert descriptor[:2] == (offset, width)
            payload = source[offset : offset + width]
            assert payload == value
            assert descriptor[2] == block_crc(
                expected_table, column, kind, 0, 0, 1, payload
            )
        descriptor_index += block_count
        offset += region_bytes
    assert descriptors[descriptor_index * 16 :] == bytes(
        DESCRIPTOR_BYTES - descriptor_index * 16
    )
    assert offset == len(source)
    assert read_u32(header, 112) == projected_crc(expected_table, expected_rows)
    return read_u32(header, 108), read_u32(header, 112)


def graph_entry(
    target: bytearray, base: int, table: int, unit: bytes, metadata_crc: int
) -> None:
    put_u32(target, base, table)
    put_u64(target, base + 8, 1)
    put_u64(target, base + 16, len(unit))
    put_u32(target, base + 24, metadata_crc)


def encode_root(replica: int, units: dict[int, tuple[bytes, int, int]]) -> bytes:
    result = bytearray(ROOT_BYTES)
    result[:8] = b"PSQLROOT"
    put_u32(result, 8, VERSION)
    put_u32(result, 12, ROOT_BYTES)
    result[16:32] = DATABASE_ID
    result[32] = replica
    put_u64(result, 40, 1)
    put_u64(result, 48, WAL_BYTES)
    result[56:80] = TRANSACTION_ID
    put_u32(result, 80, 2)
    graph_entry(result, 88, LINEITEM, units[LINEITEM][0], units[LINEITEM][1])
    graph_entry(result, 120, PART, units[PART][0], units[PART][1])
    put_u32(result, 152, crc32c(bytes(result)))
    return bytes(result)


def encode_empty_root(replica: int) -> bytes:
    result = bytearray(ROOT_BYTES)
    result[:8] = b"PSQLROOT"
    put_u32(result, 8, VERSION)
    put_u32(result, 12, ROOT_BYTES)
    result[16:32] = DATABASE_ID
    result[32] = replica
    put_u32(result, 152, crc32c(bytes(result)))
    return bytes(result)


def decode_root(
    source: bytes,
    replica: int,
    units: dict[int, tuple[bytes, int, int]],
    empty: bool = False,
) -> None:
    assert len(source) == ROOT_BYTES and source[:8] == b"PSQLROOT"
    assert read_u32(source, 8) == VERSION and read_u32(source, 12) == ROOT_BYTES
    assert source[16:32] == DATABASE_ID and source[32] == replica
    assert source[33:40] == bytes(7)
    assert read_u32(source, 152) == crc32c(with_zero(source, 152, 156))
    assert source[156:] == bytes(ROOT_BYTES - 156)
    if empty:
        assert read_u64(source, 40) == 0 and source[48:152] == bytes(104)
        return
    assert read_u64(source, 40) == 1 and read_u64(source, 48) == WAL_BYTES
    assert source[56:80] == TRANSACTION_ID and read_u32(source, 80) == 2
    assert source[84:88] == bytes(4)
    for base, table in ((88, LINEITEM), (120, PART)):
        unit, metadata_crc, _projected_crc = units[table]
        assert read_u32(source, base) == table and source[base + 4 : base + 8] == bytes(
            4
        )
        assert read_u64(source, base + 8) == 1
        assert read_u64(source, base + 16) == len(unit)
        assert read_u32(source, base + 24) == metadata_crc
        assert source[base + 28 : base + 32] == bytes(4)


def wal_entry(
    target: bytearray,
    base: int,
    table: int,
    unit: tuple[bytes, int, int],
    input_crc: int,
) -> None:
    payload, metadata_crc, projected = unit
    put_u32(target, base, table)
    put_u64(target, base + 8, 1)
    put_u64(target, base + 16, len(payload))
    put_u32(target, base + 24, metadata_crc)
    put_u32(target, base + 28, projected)
    put_u32(target, base + 32, input_crc)


def encode_wal(units: dict[int, tuple[bytes, int, int]]) -> bytes:
    result = bytearray(WAL_BYTES)
    result[:8] = b"PSQLWAL\0"
    put_u32(result, 8, VERSION)
    put_u32(result, 12, WAL_BYTES)
    result[16:32] = DATABASE_ID
    result[32:56] = TRANSACTION_ID
    put_u64(result, 56, 1)
    put_u32(result, 64, 2)
    wal_entry(result, 72, LINEITEM, units[LINEITEM], 0x11223344)
    wal_entry(result, 112, PART, units[PART], 0x55667788)
    put_u32(result, 152, crc32c(bytes(result)))
    return bytes(result)


def decode_wal(source: bytes, units: dict[int, tuple[bytes, int, int]]) -> None:
    assert len(source) == WAL_BYTES and source[:8] == b"PSQLWAL\0"
    assert read_u32(source, 8) == VERSION and read_u32(source, 12) == WAL_BYTES
    assert source[16:32] == DATABASE_ID and source[32:56] == TRANSACTION_ID
    assert read_u64(source, 56) == 1 and read_u32(source, 64) == 2
    assert source[68:72] == bytes(4)
    for base, table, input_crc in ((72, LINEITEM, 0x11223344), (112, PART, 0x55667788)):
        payload, metadata_crc, projected = units[table]
        assert read_u32(source, base) == table and source[base + 4 : base + 8] == bytes(
            4
        )
        assert read_u64(source, base + 8) == 1
        assert read_u64(source, base + 16) == len(payload)
        assert read_u32(source, base + 24) == metadata_crc
        assert read_u32(source, base + 28) == projected
        assert read_u32(source, base + 32) == input_crc
        assert source[base + 36 : base + 40] == bytes(4)
    assert read_u32(source, 152) == crc32c(with_zero(source, 152, 156))
    assert source[156:] == bytes(WAL_BYTES - 156)


def refresh_root(source: bytearray) -> bytes:
    put_u32(source, 152, crc32c(with_zero(bytes(source), 152, 156)))
    return bytes(source)


def refresh_wal(source: bytearray) -> bytes:
    put_u32(source, 152, crc32c(with_zero(bytes(source), 152, 156)))
    return bytes(source)


def refresh_unit(source: bytearray) -> bytes:
    header = bytearray(source[:HEADER_BYTES])
    descriptors = bytes(source[HEADER_BYTES:PAYLOAD_OFFSET])
    put_u32(header, 104, crc32c(descriptors))
    put_u32(header, 108, crc32c(with_zero(bytes(header), 100, 112) + descriptors))
    put_u32(header, 100, crc32c(with_zero(bytes(header), 100, 104)))
    source[:HEADER_BYTES] = header
    return bytes(source)


def reject_semantic_corruption(
    root: bytes,
    wal: bytes,
    units: dict[int, tuple[bytes, int, int]],
) -> int:
    cases = []
    changed = bytearray(root)
    put_u32(changed, 88, PART)
    cases.append((refresh_root(changed), lambda value: decode_root(value, 0, units)))
    changed = bytearray(wal)
    put_u32(changed, 112, LINEITEM)
    cases.append((refresh_wal(changed), lambda value: decode_wal(value, units)))
    changed = bytearray(units[PART][0])
    put_u32(changed, 44, 0)
    cases.append((refresh_unit(changed), lambda value: decode_unit(value, PART, 1)))
    changed = bytearray(units[LINEITEM][0])
    put_u32(changed, 128, 2)
    cases.append((refresh_unit(changed), lambda value: decode_unit(value, LINEITEM, 1)))
    changed = bytearray(units[PART][0])
    changed[128 + len(PART_COLUMNS) * 40] = 1
    cases.append((refresh_unit(changed), lambda value: decode_unit(value, PART, 1)))
    for source, decoder in cases:
        try:
            decoder(source)
        except AssertionError:
            continue
        raise AssertionError("accepted checksum-consistent semantic corruption")
    return len(cases)


def reject_mutations(source: bytes, decoder) -> int:
    indexes = (
        range(len(source))
        if len(source) <= 512
        else sorted(
            set(range(160))
            | set(range(len(source) - 64, len(source)))
            | set(range(0, len(source), max(1, len(source) // 256)))
        )
    )
    rejected = 0
    for index in indexes:
        changed = bytearray(source)
        changed[index] ^= 1
        try:
            decoder(bytes(changed))
        except AssertionError:
            rejected += 1
            continue
        raise AssertionError(f"accepted mutation at {index}")
    return rejected


def temporary_peak(lineitem_rows: int, part_rows: int) -> int:
    _line_descriptors, line_unit = lineitem_layout(lineitem_rows)
    _part_descriptors, part_unit = part_layout(part_rows)
    line_payload = lineitem_rows * 46
    line_assembly = line_payload + lineitem_rows * 8 + PAYLOAD_OFFSET
    runs = (
        part_rows * PART_RUN_RECORD_BYTES
        + ceiling_div(part_rows, PART_RUN_ROWS) * PART_RUN_HEADER_BYTES
    )
    part_phase = line_unit + runs + part_unit
    return max(line_assembly, part_phase)


def sha256(source: bytes) -> str:
    return hashlib.sha256(source).hexdigest()


def main() -> None:
    control = encode_control()
    line = encode_unit(LINEITEM, 1)
    part = encode_unit(PART, 1)
    units = {LINEITEM: line, PART: part}
    root_a = encode_root(0, units)
    root_b = encode_root(1, units)
    empty_root_a = encode_empty_root(0)
    wal = encode_wal(units)

    decode_control(control)
    decode_unit(line[0], LINEITEM, 1)
    decode_unit(part[0], PART, 1)
    decode_root(root_a, 0, units)
    decode_root(root_b, 1, units)
    decode_root(empty_root_a, 0, units, empty=True)
    decode_wal(wal, units)

    mutations = 0
    mutations += reject_mutations(control, decode_control)
    mutations += reject_mutations(root_a, lambda value: decode_root(value, 0, units))
    mutations += reject_mutations(wal, lambda value: decode_wal(value, units))
    mutations += reject_mutations(
        line[0], lambda value: decode_unit(value, LINEITEM, 1)
    )
    mutations += reject_mutations(part[0], lambda value: decode_unit(value, PART, 1))
    semantic = reject_semantic_corruption(root_a, wal, units)

    artifacts = {
        "CONTROL": control,
        "EMPTY.ROOT.A": empty_root_a,
        "ROOT.A": root_a,
        "ROOT.B": root_b,
        "WAL": wal,
        "LINEITEM.UNIT": line[0],
        "PART.UNIT": part[0],
    }
    for name, source in artifacts.items():
        print(f"sha256 {name} {sha256(source)}")

    for rows in (0, 1, 32_768, 65_536, LINEITEM_MAX_ROWS):
        descriptors, unit_bytes = lineitem_layout(rows)
        print(f"lineitem rows={rows} descriptors={descriptors} unit_bytes={unit_bytes}")
    for rows in (0, 1, 8_192, 32_768, PART_MAX_ROWS):
        descriptors, unit_bytes = part_layout(rows)
        print(f"part rows={rows} descriptors={descriptors} unit_bytes={unit_bytes}")
    print(f"sf1_temporary_peak={temporary_peak(6_001_215, 200_000)}")
    print(f"maximum_temporary_peak={temporary_peak(LINEITEM_MAX_ROWS, PART_MAX_ROWS)}")
    load_memory = (
        PART_RUN_ROWS * PART_SORT_ROW_BYTES
        + LOAD_SOURCE_CHUNK_BYTES
        + LOAD_ROW_BYTES
        + LOAD_OUTPUT_BUFFER_BYTES
        + LOAD_FIXED_OWNER_BYTES
    )
    print(f"modeled_load_memory={load_memory}")
    assert load_memory == 1_507_840 and load_memory <= 2_000_000
    print(f"mutations_rejected={mutations}")
    print(f"semantic_corruptions_rejected={semantic}")
    print(f"block_context_bytes={struct.calcsize('<8s16sQIIIIQII')}")


if __name__ == "__main__":
    main()
