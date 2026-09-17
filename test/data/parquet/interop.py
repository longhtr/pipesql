#!/usr/bin/env python3
"""Generate or check Parquet fixtures with the independent PyArrow implementation.

Use PyArrow 22.0.0 in a separate development environment. Default mode checks
byte-for-byte reproduction; --write replaces only this generator's named files.
--read-export FILE checks PipeSQL output against the same literal input values.
The ordinary offline gate uses retained fixtures and does not import PyArrow.
"""

import argparse
import datetime
from pathlib import Path
import struct
import tempfile

ROOT = Path(__file__).resolve().parent
VERSION = "22.0.0"

# Expected values are authored here, independently of PipeSQL's parser/encoder.
INTEGERS = [-(1 << 63), (1 << 63) - 1, None, -1, 0, 1, 42, -42]
BITS = [0, 0x8000000000000000, 0x7FF0000000000000, 0xFFF0000000000000,
        0x7FF8000000001234, 0xFFF0000000000001, 1, None]
DAYS = [-719162, 2932896, None, -1, 0, 1, 11016, 18262]
TEXT = ["", None, "é🙂", 'a,\n"\\\0', r"\N", "x" * 65536, "tail", "end"]


def expected_table(pa):
    numbers = [None if bits is None else struct.unpack("<d", struct.pack("<Q", bits))[0]
               for bits in BITS]
    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("amount", pa.int64()),
        pa.field("number", pa.float64()),
        pa.field("day", pa.date32()),
        pa.field("note", pa.string()),
    ])
    return pa.Table.from_arrays([
        pa.array(range(1, 9), type=pa.int64()),
        pa.array(INTEGERS, type=pa.int64()),
        pa.array(numbers, type=pa.float64(), from_pandas=False),
        pa.array(DAYS, type=pa.date32()),
        pa.array(TEXT, type=pa.string()),
    ], schema=schema)


def check_values(pa, table):
    assert table.column_names == ["id", "amount", "number", "day", "note"]
    assert table.num_rows == 8 and table.num_columns == 5
    assert [field.type for field in table.schema] == [pa.int64(), pa.int64(), pa.float64(), pa.date32(), pa.string()]
    assert [field.nullable for field in table.schema] == [False, True, True, True, True]
    assert table.column("id").to_pylist() == list(range(1, 9))
    assert table.column("amount").to_pylist() == INTEGERS
    assert table.column("note").to_pylist() == TEXT
    epoch = datetime.date(1970, 1, 1)
    assert table.column("day").to_pylist() == [None if day is None else epoch + datetime.timedelta(days=day) for day in DAYS]
    actual = []
    for chunk in table.column("number").chunks:
        validity, data = chunk.buffers()
        for index in range(len(chunk)):
            ordinal = chunk.offset + index
            valid = validity is None or (validity[ordinal // 8] >> (ordinal % 8)) & 1
            actual.append(struct.unpack_from("<Q", data, ordinal * 8)[0] if valid else None)
    assert actual == BITS, (actual, BITS)


def generate(pa, pq, directory):
    table = expected_table(pa)
    profiles = [
        ("plain-v1", "1.0", None, False),
        ("plain-v2", "2.0", None, False),
        ("unsupported-snappy", "1.0", "snappy", False),
        ("unsupported-dictionary", "1.0", None, True),
    ]
    paths = []
    for name, page_version, compression, dictionary in profiles:
        path = directory / f"{name}.parquet"
        pq.write_table(table, path, version="2.6", data_page_version=page_version,
                       compression=compression, use_dictionary=dictionary,
                       row_group_size=3, data_page_size=256, write_page_checksum=True)
        check_values(pa, pq.read_table(path, page_checksum_verification=True))
        paths.append(path)
    path = directory / "unsupported-nested.parquet"
    nested = pa.table({"nested": pa.array([[1, None], []], type=pa.list_(pa.int64()))})
    pq.write_table(nested, path, compression=None, use_dictionary=False)
    paths.append(path)
    path = directory / "plain-batches.parquet"
    batches = pa.Table.from_arrays([pa.array(range(600), type=pa.int64())],
                                  schema=pa.schema([pa.field("id", pa.int64(), nullable=False)]))
    pq.write_table(batches, path, compression=None, use_dictionary=False,
                   row_group_size=600, data_page_size=256, write_batch_size=100,
                   write_page_checksum=True)
    assert pq.read_table(path, page_checksum_verification=True).column("id").to_pylist() == list(range(600))
    paths.append(path)
    return paths


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true")
    parser.add_argument("--read-export", type=Path)
    args = parser.parse_args()
    if not __debug__:
        parser.error("fixture checks require Python assertions")
    try:
        import pyarrow as pa
        import pyarrow.parquet as pq
    except ImportError:
        parser.error(f"install pyarrow=={VERSION} in a separate development environment")
    if pa.__version__ != VERSION:
        parser.error(f"expected PyArrow {VERSION}, found {pa.__version__}")
    if args.read_export:
        check_values(pa, pq.read_table(args.read_export, page_checksum_verification=True))
        print("External Parquet reader: complete schema, values, NULLs and DOUBLE bits passed")
        return
    target = ROOT
    check_values(pa, pq.read_table(target / "pipesql-v1.parquet", page_checksum_verification=True))
    with tempfile.TemporaryDirectory(prefix="pipesql-parquet-fixtures-") as temporary:
        paths = generate(pa, pq, Path(temporary))
        if args.write:
            target.mkdir(exist_ok=True)
        for path in paths:
            retained = target / path.name
            if args.write:
                retained.write_bytes(path.read_bytes())
            else:
                assert retained.read_bytes() == path.read_bytes(), retained
        print("Retained PipeSQL export: independent complete-value check passed")
        print(f"PyArrow {VERSION}: {len(paths)} independent Parquet fixtures {'written' if args.write else 'reproduced'}")


if __name__ == "__main__":
    main()
