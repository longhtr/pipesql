#!/usr/bin/env python3
"""Independent format-1 golden encoder/checker; never imported by PipeSQL."""

import hashlib
import struct

CONTROL_BYTES = 128
ROOT_BYTES = 4096
WAL_BYTES = 512
HEADER_BYTES = 4096
DESCRIPTOR_CAPACITY = 704
DESCRIPTOR_BYTES = DESCRIPTOR_CAPACITY * 16
PAYLOAD_OFFSET = 16384
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


def encode_control() -> bytes:
    result = bytearray(CONTROL_BYTES)
    result[0:8] = b"PIPESQL\0"
    put_u32(result, 8, 1)
    put_u32(result, 12, CONTROL_BYTES)
    result[16:32] = DATABASE_ID
    put_u32(result, 36, crc32c(bytes(result)))
    return bytes(result)


def decode_control(source: bytes) -> None:
    assert len(source) == CONTROL_BYTES
    assert source[0:8] == b"PIPESQL\0"
    assert read_u32(source, 8) == 1
    assert read_u32(source, 12) == CONTROL_BYTES
    assert source[16:32] != bytes(16)
    assert read_u32(source, 32) == 0
    assert source[40:] == bytes(88)
    assert read_u32(source, 36) == crc32c(with_zero(source, 36, 40))


def encode_unit() -> tuple[bytes, int, int, int]:
    values = [
        struct.pack("<d", 1.0),
        struct.pack("<d", 2.0),
        struct.pack("<d", 0.5),
        struct.pack("<i", 0),
    ]
    projected = b"".join(values)
    projected_crc = crc32c(projected)
    descriptor_area = bytearray(DESCRIPTOR_BYTES)
    offset = PAYLOAD_OFFSET
    for index, value in enumerate(values):
        struct.pack_into(
            "<QII", descriptor_area, index * 16, offset, len(value), crc32c(value)
        )
        offset += len(value)
    descriptor_crc = crc32c(bytes(descriptor_area))
    header = bytearray(HEADER_BYTES)
    header[0:8] = b"PSQLUNIT"
    put_u32(header, 8, 1)
    put_u32(header, 12, HEADER_BYTES)
    header[16:32] = DATABASE_ID
    put_u64(header, 32, 1)
    put_u64(header, 40, 1)
    put_u64(header, 48, 1)
    put_u64(header, 56, offset)
    put_u64(header, 64, HEADER_BYTES)
    put_u32(header, 72, 4)
    put_u32(header, 76, DESCRIPTOR_CAPACITY)
    put_u64(header, 80, DESCRIPTOR_BYTES)
    put_u64(header, 88, PAYLOAD_OFFSET)
    put_u32(header, 96, 4)
    put_u32(header, 104, descriptor_crc)
    put_u32(header, 112, projected_crc)
    region = PAYLOAD_OFFSET
    block = 0
    columns = [(1, 1, 8, 32768), (2, 1, 8, 32768), (3, 1, 8, 32768), (4, 2, 4, 65536)]
    for index, (column_id, kind, width, rows_per_block) in enumerate(columns):
        base = 128 + index * 40
        struct.pack_into(
            "<IIIIIIQQ",
            header,
            base,
            column_id,
            kind,
            width,
            rows_per_block,
            block,
            1,
            region,
            width,
        )
        region += width
        block += 1
    metadata_input = bytes(header) + bytes(descriptor_area)
    metadata_input = with_zero(metadata_input, 100, 112)
    metadata_crc = crc32c(metadata_input)
    put_u32(header, 108, metadata_crc)
    put_u32(header, 100, crc32c(with_zero(bytes(header), 100, 104)))
    padding = bytes(PAYLOAD_OFFSET - HEADER_BYTES - DESCRIPTOR_BYTES)
    unit = bytes(header) + bytes(descriptor_area) + padding + b"".join(values)
    assert len(unit) == offset == 16412
    return unit, metadata_crc, projected_crc, descriptor_crc


def decode_unit(source: bytes) -> tuple[int, int]:
    assert len(source) >= PAYLOAD_OFFSET
    header = source[:HEADER_BYTES]
    descriptors = source[HEADER_BYTES : HEADER_BYTES + DESCRIPTOR_BYTES]
    assert header[0:8] == b"PSQLUNIT"
    assert read_u32(header, 8) == 1 and read_u32(header, 12) == HEADER_BYTES
    assert header[16:32] == DATABASE_ID
    assert read_u64(header, 32) == 1 and read_u64(header, 40) == 1
    rows = read_u64(header, 48)
    assert rows == 1 and read_u64(header, 56) == len(source)
    assert read_u64(header, 64) == HEADER_BYTES
    used = read_u32(header, 72)
    assert used == 4 and read_u32(header, 76) == DESCRIPTOR_CAPACITY
    assert read_u64(header, 80) == DESCRIPTOR_BYTES
    assert read_u64(header, 88) == PAYLOAD_OFFSET and read_u32(header, 96) == 4
    assert header[116:128] == bytes(12) and header[288:] == bytes(HEADER_BYTES - 288)
    assert source[HEADER_BYTES + DESCRIPTOR_BYTES : PAYLOAD_OFFSET] == bytes(1024)
    descriptor_crc = read_u32(header, 104)
    assert descriptor_crc == crc32c(descriptors)
    metadata_input = with_zero(header + descriptors, 100, 112)
    assert read_u32(header, 108) == crc32c(metadata_input)
    assert read_u32(header, 100) == crc32c(with_zero(header, 100, 104))
    expected_columns = [
        (1, 1, 8, 32768),
        (2, 1, 8, 32768),
        (3, 1, 8, 32768),
        (4, 2, 4, 65536),
    ]
    offset = PAYLOAD_OFFSET
    projected = bytearray()
    for index, expected in enumerate(expected_columns):
        base = 128 + index * 40
        column = struct.unpack_from("<IIIIIIQQ", header, base)
        assert column[:4] == expected
        assert column[4:] == (index, 1, offset, expected[2])
        descriptor = struct.unpack_from("<QII", descriptors, index * 16)
        assert descriptor[0] == offset and descriptor[1] == expected[2]
        payload = source[offset : offset + descriptor[1]]
        assert len(payload) == descriptor[1] and descriptor[2] == crc32c(payload)
        projected += payload
        offset += descriptor[1]
    assert descriptors[used * 16 :] == bytes(DESCRIPTOR_BYTES - used * 16)
    assert offset == len(source)
    assert read_u32(header, 112) == crc32c(bytes(projected))
    return read_u32(header, 108), read_u32(header, 112)


def encode_wal(
    unit_length: int, metadata_crc: int, projected_crc: int, input_crc: int
) -> bytes:
    result = bytearray(WAL_BYTES)
    result[0:8] = b"PSQLWAL\0"
    put_u32(result, 8, 1)
    put_u32(result, 12, WAL_BYTES)
    result[16:32] = DATABASE_ID
    result[32:56] = TRANSACTION_ID
    put_u64(result, 56, 1)
    put_u64(result, 64, 1)
    put_u64(result, 72, 1)
    put_u64(result, 80, unit_length)
    put_u32(result, 88, metadata_crc)
    put_u32(result, 92, projected_crc)
    put_u32(result, 96, input_crc)
    put_u32(result, 100, crc32c(bytes(result)))
    return bytes(result)


def decode_wal(
    source: bytes,
    unit_length: int,
    metadata_crc: int,
    projected_crc: int,
    input_crc: int,
) -> None:
    assert len(source) == WAL_BYTES and source[0:8] == b"PSQLWAL\0"
    assert read_u32(source, 8) == 1 and read_u32(source, 12) == WAL_BYTES
    assert source[16:32] == DATABASE_ID and source[32:56] == TRANSACTION_ID
    assert (read_u64(source, 56), read_u64(source, 64), read_u64(source, 72)) == (
        1,
        1,
        1,
    )
    assert read_u64(source, 80) == unit_length
    assert (read_u32(source, 88), read_u32(source, 92), read_u32(source, 96)) == (
        metadata_crc,
        projected_crc,
        input_crc,
    )
    assert source[104:] == bytes(WAL_BYTES - 104)
    assert read_u32(source, 100) == crc32c(with_zero(source, 100, 104))


def encode_root(replica: int, unit_length: int, metadata_crc: int) -> bytes:
    result = bytearray(ROOT_BYTES)
    result[0:8] = b"PSQLROOT"
    put_u32(result, 8, 1)
    put_u32(result, 12, ROOT_BYTES)
    result[16:32] = DATABASE_ID
    result[32] = replica
    put_u64(result, 40, 1)
    put_u64(result, 48, WAL_BYTES)
    result[56:80] = TRANSACTION_ID
    put_u64(result, 80, 1)
    put_u64(result, 88, 1)
    put_u64(result, 96, unit_length)
    put_u32(result, 104, metadata_crc)
    put_u32(result, 108, crc32c(bytes(result)))
    return bytes(result)


def decode_root(
    source: bytes, replica: int, unit_length: int, metadata_crc: int
) -> None:
    assert len(source) == ROOT_BYTES and source[0:8] == b"PSQLROOT"
    assert read_u32(source, 8) == 1 and read_u32(source, 12) == ROOT_BYTES
    assert source[16:32] == DATABASE_ID and source[32] == replica
    assert source[33:40] == bytes(7)
    assert (read_u64(source, 40), read_u64(source, 48)) == (1, WAL_BYTES)
    assert source[56:80] == TRANSACTION_ID
    assert (read_u64(source, 80), read_u64(source, 88), read_u64(source, 96)) == (
        1,
        1,
        unit_length,
    )
    assert read_u32(source, 104) == metadata_crc
    assert source[112:] == bytes(ROOT_BYTES - 112)
    assert read_u32(source, 108) == crc32c(with_zero(source, 108, 112))


def refresh_unit_checksums(source: bytearray) -> bytes:
    header = source[:HEADER_BYTES]
    descriptors = source[HEADER_BYTES : HEADER_BYTES + DESCRIPTOR_BYTES]
    descriptor_crc = crc32c(bytes(descriptors))
    header[100:112] = bytes(12)
    metadata_crc = crc32c(bytes(header) + bytes(descriptors))
    put_u32(header, 104, descriptor_crc)
    put_u32(header, 108, metadata_crc)
    put_u32(header, 100, crc32c(with_zero(bytes(header), 100, 104)))
    source[:HEADER_BYTES] = header
    return bytes(source)


def must_reject_semantic_corruptions(
    control: bytes,
    root: bytes,
    wal: bytes,
    unit: bytes,
    metadata_crc: int,
    projected_crc: int,
    input_crc: int,
) -> int:
    cases = []
    changed = bytearray(control)
    put_u32(changed, 32, 1)
    put_u32(changed, 36, crc32c(with_zero(bytes(changed), 36, 40)))
    cases.append((bytes(changed), decode_control))

    changed = bytearray(root)
    put_u64(changed, 40, 2)
    put_u32(changed, 108, crc32c(with_zero(bytes(changed), 108, 112)))
    cases.append(
        (bytes(changed), lambda value: decode_root(value, 0, len(unit), metadata_crc))
    )

    changed = bytearray(wal)
    changed[104] = 1
    put_u32(changed, 100, crc32c(with_zero(bytes(changed), 100, 104)))
    cases.append(
        (
            bytes(changed),
            lambda value: decode_wal(
                value, len(unit), metadata_crc, projected_crc, input_crc
            ),
        )
    )

    changed = bytearray(unit)
    first_offset = read_u64(changed, HEADER_BYTES)
    put_u64(changed, HEADER_BYTES + 16, first_offset)
    cases.append((refresh_unit_checksums(changed), decode_unit))

    changed = bytearray(unit)
    changed[HEADER_BYTES + 4 * 16] = 1
    cases.append((refresh_unit_checksums(changed), decode_unit))

    changed = bytearray(unit)
    changed[288] = 1
    cases.append((refresh_unit_checksums(changed), decode_unit))

    for source, decoder in cases:
        try:
            decoder(source)
        except AssertionError:
            continue
        raise AssertionError("accepted checksum-consistent semantic corruption")
    return len(cases)


def must_reject_mutations(name: str, source: bytes, decoder) -> int:
    rejected = 0
    for index in range(len(source)):
        changed = bytearray(source)
        changed[index] ^= 0x80
        try:
            decoder(bytes(changed))
        except AssertionError:
            rejected += 1
    assert rejected == len(source), (name, rejected, len(source))
    for length in range(len(source)):
        try:
            decoder(source[:length])
        except (AssertionError, struct.error):
            pass
        else:
            raise AssertionError((name, "accepted prefix", length))
    try:
        decoder(source + b"\0")
    except AssertionError:
        pass
    else:
        raise AssertionError((name, "accepted trailing byte"))
    return rejected


def row_equations(rows: int) -> tuple[int, int, int, int, int]:
    double_blocks = (rows + 32767) // 32768
    date_blocks = (rows + 65535) // 65536
    descriptors = 3 * double_blocks + date_blocks
    payload = rows * 28
    unit = PAYLOAD_OFFSET + payload
    return double_blocks, date_blocks, descriptors, payload, unit


def main() -> None:
    assert crc32c(b"123456789") == 0xE3069283
    vectors = {
        0: (0, 0, 0, 0, 16384),
        1: (1, 1, 4, 28, 16412),
        32768: (1, 1, 4, 917504, 933888),
        65536: (2, 1, 7, 1835008, 1851392),
        6001215: (184, 92, 644, 168034020, 168050404),
        6500000: (199, 100, 697, 182000000, 182016384),
    }
    for rows, expected in vectors.items():
        assert row_equations(rows) == expected
    assert row_equations(6500001)[2] <= DESCRIPTOR_CAPACITY

    control = encode_control()
    unit, metadata_crc, projected_crc, descriptor_crc = encode_unit()
    input_crc = crc32c(b"golden-input-row\n")
    wal = encode_wal(len(unit), metadata_crc, projected_crc, input_crc)
    root_a = encode_root(0, len(unit), metadata_crc)
    root_b = encode_root(1, len(unit), metadata_crc)
    decode_control(control)
    decode_unit(unit)
    decode_wal(wal, len(unit), metadata_crc, projected_crc, input_crc)
    decode_root(root_a, 0, len(unit), metadata_crc)
    decode_root(root_b, 1, len(unit), metadata_crc)

    decoders = {
        "CONTROL": (control, decode_control),
        "ROOT.A": (
            root_a,
            lambda value: decode_root(value, 0, len(unit), metadata_crc),
        ),
        "ROOT.B": (
            root_b,
            lambda value: decode_root(value, 1, len(unit), metadata_crc),
        ),
        "WAL": (
            wal,
            lambda value: decode_wal(
                value, len(unit), metadata_crc, projected_crc, input_crc
            ),
        ),
        "UNIT": (unit, decode_unit),
    }
    rejected = 0
    for name, (content, decoder) in decoders.items():
        rejected += must_reject_mutations(name, content, decoder)
    semantic_rejected = must_reject_semantic_corruptions(
        control, root_a, wal, unit, metadata_crc, projected_crc, input_crc
    )

    for name, (content, _) in decoders.items():
        print(
            f"{name} bytes={len(content)} sha256={hashlib.sha256(content).hexdigest()}"
        )
    print(
        f"metadata_crc32c={metadata_crc:08x} descriptor_crc32c={descriptor_crc:08x} projected_crc32c={projected_crc:08x} input_crc32c={input_crc:08x}"
    )
    print(
        f"single_byte_mutations_rejected={rejected} checksum_consistent_semantic_rejected={semantic_rejected}"
    )
    print(f"row_vectors={len(vectors)} max_descriptors={vectors[6500000][2]}")


if __name__ == "__main__":
    main()
