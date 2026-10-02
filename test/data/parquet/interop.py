#!/usr/bin/env python3
"""Generate or check Parquet fixtures with the independent PyArrow implementation.

Use PyArrow 22.0.0 in a separate development environment. Default mode checks
byte-for-byte reproduction; --write replaces only this generator's named files.
--read-export FILE checks PipeSQL output against the same literal input values.
--read-boundary-export FILE checks the 777-row export and its row-group cuts.
--read-profile and --read-profile-file check fresh export_cost selections.
--describe reports the reader environment without generating fixtures.
--write-imports NEW_DIRECTORY generates fresh inputs for import qualification.
--write-wide-imports NEW_DIRECTORY generates permuted 64-column import inputs.
--read-wide-export DIRECTORY checks 64-column output and deliberate reader controls.
--read-wide-measure DIRECTORY ROWS GROUP_ROWS checks six complete timed exports.
The ordinary offline gate uses retained fixtures and does not import PyArrow.
"""

import argparse
import datetime
import hashlib
import json
import sys
import time
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
    assert double_bits(table) == BITS


def double_bits(table, name="number"):
    actual = []
    for chunk in table.column(name).chunks:
        validity, data = chunk.buffers()
        for index in range(len(chunk)):
            ordinal = chunk.offset + index
            valid = validity is None or (validity[ordinal // 8] >> (ordinal % 8)) & 1
            actual.append(struct.unpack_from("<Q", data, ordinal * 8)[0] if valid else None)
    return actual


def boundary_table(pa, rows=777, dense=False):
    """Typed boundary values with unrelated NULL periods and column widths."""
    ids = list(range(rows))
    integers = [value for value in INTEGERS if value is not None]
    dates = [value for value in DAYS if value is not None]
    amounts = [None if i % 11 == 3 else integers[i % 7] for i in ids]
    bits = [None if i % 13 == 5 else BITS[i % 7] for i in ids]
    days = [None if i % 19 == 7 else dates[i % 7] for i in ids]
    notes = ["x" * 65536 if dense and i >= rows - 9 else
             None if i % 7 == 2 else
             "x" * 65536 if i in (255, 645) else
             "" if i % 17 == 4 else f"{i}:é🙂\n\0" + "q" * (i % 101) for i in ids]
    numbers = [None if value is None else struct.unpack("<d", struct.pack("<Q", value))[0]
               for value in bits]
    return pa.Table.from_arrays([
        pa.array(ids, type=pa.int64()), pa.array(amounts, type=pa.int64()),
        pa.array(numbers, type=pa.float64(), from_pandas=False),
        pa.array(days, type=pa.date32()), pa.array(notes, type=pa.string()),
    ], schema=expected_table(pa).schema)


def check_boundary_values(pa, actual):
    expected = boundary_table(pa)
    assert actual.schema == expected.schema and actual.num_rows == 777
    for name in ("id", "amount", "day", "note"):
        assert actual.column(name).to_pylist() == expected.column(name).to_pylist(), name
    assert double_bits(actual) == double_bits(expected)


def check_boundary_export(pa, pq, path):
    check_boundary_values(pa, pq.read_table(path, page_checksum_verification=True))
    metadata = pq.read_metadata(path)
    assert [metadata.row_group(i).num_rows for i in range(metadata.num_row_groups)] == [255, 1, 311, 78, 2, 130]


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
    for version in ("1.0", "2.0"):
        path = directory / f"typed-boundaries-v{version[0]}.parquet"
        pq.write_table(boundary_table(pa), path, version="2.6", data_page_version=version,
                       compression=None, use_dictionary=False, row_group_size=389,
                       data_page_size=1024, write_batch_size=37, write_page_checksum=True)
        check_boundary_values(pa, pq.read_table(path, page_checksum_verification=True))
        paths.append(path)
    return paths


def profile_value(row, profile):
    amounts = [-9223372036854775808, 9223372036854775807, -1, 0, 1,
               -9007199254740993, 9007199254740993]
    bits = [0x0000000000000000, 0x8000000000000000, 0x3ff0000000000000,
            0xbff0000000000000, 0x0000000000000001, 0x7fefffffffffffff,
            0x7ff0000000000000, 0xfff0000000000000, 0x7ff8000000000123,
            0xfff8000000000042]
    dates = ["0001-01-01", "1969-12-31", "1970-01-01", "2000-02-29",
             "1900-03-01", "9999-12-31"]
    if row % 13 == 0:
        text = None
    elif row % 17 == 0:
        text = ""
    elif profile == "escaped1024":
        text = '\x00\x0a\x09\x22\x5cé🙂' * 92 + 'xxxx' + f'{row:08d}'
    else:
        text = ('x' * 1016 if profile == "plain1024" else '') + f'{row:08d}'
    return [row, None if row % 5 == 0 else amounts[row % 7],
            None if row % 7 == 0 else bits[row % 10],
            None if row % 11 == 0 else dates[row % 6], text]


def profile_selection(arguments):
    path, query, profile, rows, group_rows, group_text = arguments
    rows, group_rows, group_text = map(int, (rows, group_rows, group_text))
    assert query in ("scan", "order")
    assert profile in ("plain8", "plain1024", "escaped1024")
    assert rows in (0, 257, 8192) and group_rows in (128, 311)
    assert group_text in (4096, 65536, 131072)
    return Path(path), (query, profile, rows, group_rows, group_text)


def check_profile_file(pa, pq, path, selection):
    query, profile, rows, group_rows, group_text = selection
    size = path.stat().st_size
    assert 12 <= size <= 64_000_000, "file length"
    start = time.perf_counter_ns()
    schema = pa.schema([pa.field("id", pa.int64(), nullable=False),
                        pa.field("amount", pa.int64()), pa.field("number", pa.float64()),
                        pa.field("day", pa.date32()), pa.field("label", pa.string())])
    seen = bytearray(rows)
    position = 0
    expected_groups = []
    retained_rows = retained_text = 0
    with pq.ParquetFile(path, pre_buffer=False, page_checksum_verification=True,
                        thrift_string_size_limit=16_777_216,
                        thrift_container_size_limit=1_000_000) as reader:
        metadata = reader.metadata
        assert reader.schema_arrow.equals(schema, check_metadata=False), "schema"
        assert metadata.num_rows == rows and metadata.num_columns == 5, "row/column count"
        assert metadata.serialized_size <= 65_536 and metadata.num_row_groups <= 256, "metadata limits"
        groups = []
        for ordinal in range(metadata.num_row_groups):
            group = metadata.row_group(ordinal)
            assert 0 < group.num_rows <= group_rows and group.num_columns == 5, "group bounds"
            for column in range(5):
                chunk = group.column(column)
                assert chunk.num_values == group.num_rows, "column count"
                assert chunk.compression == "UNCOMPRESSED", "compression"
                assert set(chunk.encodings) == {"PLAIN", "RLE"}, "encodings"
            groups.append(group.num_rows)
            # One selected group is bounded; raw buffers preserve NaN payloads
            # and signed zero without conversion through Python floating point.
            table = reader.read_row_group(ordinal, use_threads=False)
            values = [table.column(name).to_pylist() for name in ("id", "amount", "day", "label")]
            raw_bits = double_bits(table)
            assert table.num_rows == group.num_rows, "decoded group count"
            for index in range(table.num_rows):
                row = values[0][index]
                assert isinstance(row, int) and 0 <= row < rows and not seen[row], "row identity"
                if query == "order":
                    assert row == position, "row order"
                expected = profile_value(row, profile)
                day = values[2][index]
                actual = [row, values[1][index], raw_bits[index],
                          None if day is None else day.isoformat(), values[3][index]]
                assert actual == expected, f"row {row} values"
                seen[row] = 1
                position += 1
                # Use independent input text lengths after checking the whole row.
                # Scan order is unrestricted; capacity cuts follow its actual IDs.
                text_bytes = 0 if expected[4] is None else len(expected[4].encode("utf-8"))
                assert text_bytes <= group_text, "row text capacity"
                if retained_rows == group_rows or retained_text + text_bytes > group_text:
                    expected_groups.append(retained_rows)
                    retained_rows = retained_text = 0
                retained_rows += 1
                retained_text += text_bytes
        if retained_rows:
            expected_groups.append(retained_rows)
        assert position == rows and all(seen), "complete row identities"
        assert groups == expected_groups, "row/text group cuts"
    elapsed = time.perf_counter_ns() - start
    return {"file": path.name, "rows": rows, "bytes": size, "groups": groups,
            "validation_ns": elapsed, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def profile_reader_controls(pa, pq, path, selection):
    query, profile, rows, group_rows, group_text = selection
    assert query == "order" and rows == 257
    scan = ("scan", profile, rows, group_rows, group_text)
    with pq.ParquetFile(path, pre_buffer=False) as reader:
        table = reader.read(use_threads=False)
        cuts = [reader.metadata.row_group(i).num_rows for i in range(reader.num_row_groups)]
    raw = path.read_bytes()
    rejected = []
    with tempfile.TemporaryDirectory(prefix="pipesql-parquet-controls-") as temporary:
        temporary = Path(temporary)

        def write_table(name, changed):
            destination = temporary / (name + ".parquet")
            with pq.ParquetWriter(destination, changed.schema, compression=None,
                                  use_dictionary=False, write_statistics=False,
                                  data_page_version="1.0") as writer:
                start = 0
                for count in cuts:
                    writer.write_table(changed.slice(start, count))
                    start += count
            return destination

        def reject(name, destination, expected):
            # Missing files, reader installation and OS failures are not corrupt
            # data. Only a value/shape assertion or Parquet format error qualifies.
            assert destination.is_file()
            try:
                check_profile_file(pa, pq, destination, expected)
            except (AssertionError, pa.ArrowInvalid):
                rejected.append(name)
            else:
                raise AssertionError(f"reader accepted {name}")

        amounts = table.column("amount").to_pylist()
        amounts[1] = 0
        changed = table.set_column(1, table.schema.field(1), pa.array(amounts, type=pa.int64()))
        reject("wrong_value", write_table("wrong-value", changed), scan)
        duplicate = table.take(pa.array([0, 0] + list(range(2, rows)), type=pa.int64()))
        reject("duplicate_row", write_table("duplicate", duplicate), scan)
        reject("missing_row", write_table("missing", table.slice(0, rows - 1)), scan)
        for name, contents in [("prefix", raw[:len(raw) // 2]),
                               ("missing_footer", raw[:-8]),
                               ("footer_magic", raw[:-1] + b"x")]:
            destination = temporary / (name + ".parquet")
            destination.write_bytes(contents)
            reject(name, destination, scan)
        reordered = table.take(pa.array([1, 0] + list(range(2, rows)), type=pa.int64()))
        destination = write_table("reordered", reordered)
        check_profile_file(pa, pq, destination, scan)
        reject("ordered_rows", destination, selection)
    assert len(rejected) == 7
    return rejected


def read_profile(pa, pq, arguments, single):
    path, selection = profile_selection(arguments)
    rows = selection[2]
    failed_exports = []
    if single:
        files = [check_profile_file(pa, pq, path, selection)]
        rejected = []
    else:
        names = ["warmup"] + [f"sample-{i}" for i in range(5)] + ["retry"]
        if rows:
            names.append("failure-flush")
        files = [check_profile_file(pa, pq, path / (name + ".parquet"), selection) for name in names]
        failed = [] if not rows else ["write", "cancel", "rows", "bytes", "groups", "text", "metadata"]
        if rows == 8192:
            failed.append("query")
        for name in failed:
            file = path / f"failure-{name}.parquet"
            # These failures precede a footer. Complete-looking failed flushes
            # were checked separately above and remain API errors in the producer.
            assert file.is_file() and 0 < file.stat().st_size <= 64_000_000
            failed_exports.append({"file": file.name, "bytes": file.stat().st_size})
            try:
                pq.ParquetFile(file, pre_buffer=False).close()
            except pa.ArrowInvalid:
                pass
            else:
                raise AssertionError(f"failed export has readable footer: {name}")
        rejected = profile_reader_controls(pa, pq, path / "warmup.parquet", selection) if rows == 257 else []
    print(json.dumps({"status": "checked", "reader": VERSION, "query": selection[0],
                      "profile": selection[1], "rows": rows, "group_rows": selection[3],
                      "group_text": selection[4], "files": files, "failed_exports": failed_exports,
                      "reader_rejections": rejected}, sort_keys=True))


def write_imports(pa, pq, directory):
    directory.mkdir()
    inputs = []
    # Page sizes are PyArrow targets; a single 65,536-byte text value may exceed
    # them. Write batches and row groups deliberately have different boundaries.
    for version, rows, group_rows, page, batch, dense in [
            ("1.0", 777, 389, 1024, 37, False),
            ("2.0", 777, 257, 4096, 113, False),
            ("1.0", 8192, 257, 4096, 37, False),
            ("2.0", 8192, 389, 1024, 113, False),
            ("1.0", 8192, 255, 4096, 37, False),
            ("1.0", 8192, 256, 4096, 37, False),
            ("1.0", 777, 513, 1024, 37, True),
            ("2.0", 777, 513, 4096, 113, True)]:
        path = directory / f"input-{len(inputs)}.parquet"
        expected = boundary_table(pa, rows, dense).take(
            pa.array(range(rows - 1, -1, -1), type=pa.int64()))
        pq.write_table(expected, path, version="2.6", data_page_version=version,
                       compression=None, use_dictionary=False, row_group_size=group_rows,
                       data_page_size=page, write_batch_size=batch, write_page_checksum=True)
        with pq.ParquetFile(path, pre_buffer=False, page_checksum_verification=True) as reader:
            actual = reader.read(use_threads=False)
            assert actual.schema == expected.schema and actual.num_rows == rows
            assert actual.column("id").to_pylist() == list(range(rows - 1, -1, -1))
            for column in ("id", "amount", "day", "note"):
                assert actual.column(column).to_pylist() == expected.column(column).to_pylist(), column
            assert double_bits(actual) == double_bits(expected)
            groups = [reader.metadata.row_group(i).num_rows for i in range(reader.num_row_groups)]
            assert groups == [min(group_rows, rows - start) for start in range(0, rows, group_rows)]
            assert reader.metadata.serialized_size <= 65_536
            for ordinal in range(reader.num_row_groups):
                group = reader.metadata.row_group(ordinal)
                assert group.total_byte_size <= (1_048_576 if dense else 196_608)
                for column in range(5):
                    chunk = group.column(column)
                    assert chunk.num_values == group.num_rows
                    assert chunk.compression == "UNCOMPRESSED"
                    assert set(chunk.encodings) == {"PLAIN", "RLE"}
        size = path.stat().st_size
        assert 12 <= size <= 2_000_000
        inputs.append({"file": path.name, "version": version, "rows": rows,
                       "group_rows": group_rows, "page_target": page, "write_batch": batch,
                       "groups": groups, "bytes": size, "dense": dense,
                       "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
    print(json.dumps({"status": "generated", "reader": VERSION,
                      "row_order": "descending", "inputs": inputs}, sort_keys=True))


def wide_value(row, column):
    """Expected source cells; the exporter projects them in a different order."""
    if column % 3 != 0 and (row + 7 * column) % 11 == 0:
        return None
    kind = column % 4
    if kind == 0:
        return (-(1 << 63) + column if row % 7 == 0 else
                (1 << 63) - 1 - column if row % 7 == 1 else row * 1000 + column)
    if kind == 1:
        fixed = [0, 1 << 63, 0x7ff0000000000000, 0xfff0000000000000,
                 0x7ff8000000000100 | column, 1 + column]
        if row % 8 < 6:
            return fixed[row % 8]
        value = row * 64 + column
        return struct.unpack("<Q", struct.pack("<d", value if row % 8 == 6 else -value))[0]
    if kind == 2:
        days = (-719162 + column if row % 3 == 0 else
                2932896 - column if row % 3 == 1 else row * 37 + column)
        return datetime.date(1970, 1, 1) + datetime.timedelta(days=days)
    return "" if row % 13 == 0 else f"c{column:02}:r{row}:雪\0é🙂"


def write_wide_imports(pa, pq, directory):
    directory.mkdir()
    order = [(17 * ordinal + 7) % 64 for ordinal in range(64)]
    types = [pa.int64(), pa.float64(), pa.date32(), pa.string()]
    schema = pa.schema([pa.field(f"c{column:02}", types[column % 4], nullable=column % 3 != 0)
                        for column in order])
    arrays = []
    for column in order:
        values = []
        for row in range(512, -1, -1):
            value = row if column == 0 else wide_value(row, column)
            if value is not None and column % 4 == 2:
                value = datetime.date(2000 if row % 2 == 0 else 9999, 1, column // 4 + 1)
            if value is not None and column % 4 == 1:
                value = struct.unpack("<d", struct.pack("<Q", value))[0]
            values.append(value)
        arrays.append(pa.array(values, type=types[column % 4], from_pandas=False))
    expected = pa.Table.from_arrays(arrays, schema=schema)
    inputs = []
    for version, group_rows, page, batch in [("1.0", 513, 1024, 37), ("2.0", 257, 4096, 113)]:
        path = directory / f"wide-{len(inputs)}.parquet"
        pq.write_table(expected, path, version="2.6", data_page_version=version,
                       compression=None, use_dictionary=False, row_group_size=group_rows,
                       data_page_size=page, write_batch_size=batch, write_page_checksum=True)
        with pq.ParquetFile(path, pre_buffer=False, page_checksum_verification=True) as reader:
            actual = reader.read(use_threads=False)
            assert actual.schema == schema and actual.num_rows == 513
            for column in order:
                name = f"c{column:02}"
                if column % 4 == 1:
                    assert double_bits(actual, name) == double_bits(expected, name)
                else:
                    assert actual.column(name).to_pylist() == expected.column(name).to_pylist()
            groups = [reader.metadata.row_group(i).num_rows for i in range(reader.num_row_groups)]
            assert groups == ([513] if group_rows == 513 else [257, 256])
            assert reader.metadata.serialized_size <= 65536
            for ordinal in range(reader.num_row_groups):
                group = reader.metadata.row_group(ordinal)
                assert group.total_byte_size <= 1048576
                for column in range(64):
                    chunk = group.column(column)
                    assert chunk.num_values == group.num_rows and chunk.compression == "UNCOMPRESSED"
                    assert set(chunk.encodings) == {"PLAIN", "RLE"}
        inputs.append({"file": path.name, "version": version, "rows": 513, "columns": 64,
                       "groups": groups, "page_target": page, "write_batch": batch,
                       "bytes": path.stat().st_size, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
    print(json.dumps({"status": "wide-generated", "reader": VERSION, "inputs": inputs}, sort_keys=True))


def check_wide_file(pa, pq, path, group_rows, rows=513):
    columns = [(17 * ordinal + 7) % 64 for ordinal in range(64)]
    names = [f"v{i:02}" if i < 8 else f"c{column:02}" for i, column in enumerate(columns)]
    types = [pa.int64(), pa.float64(), pa.date32(), pa.string()]
    schema = pa.schema([pa.field(names[i], types[column % 4], nullable=column % 3 != 0)
                        for i, column in enumerate(columns)])
    expected_groups = [min(group_rows, rows - start) for start in range(0, rows, group_rows)]
    with pq.ParquetFile(path, pre_buffer=False, page_checksum_verification=True) as reader:
        assert reader.schema_arrow.equals(schema, check_metadata=False), "wide schema"
        assert reader.metadata.num_columns == 64 and reader.metadata.num_rows == rows, "wide extent"
        assert reader.metadata.num_row_groups == len(expected_groups), "wide group count"
        position = 0
        for ordinal, expected_rows in enumerate(expected_groups):
            group = reader.metadata.row_group(ordinal)
            assert group.num_rows == expected_rows and group.num_columns == 64, "wide group extent"
            for column in range(64):
                chunk = group.column(column)
                assert chunk.path_in_schema == names[column], "wide column path"
                assert chunk.num_values == expected_rows and chunk.compression == "UNCOMPRESSED"
                assert set(chunk.encodings) == {"PLAIN", "RLE"}, "wide encodings"
            table = reader.read_row_group(ordinal, use_threads=False)
            assert table.num_rows == expected_rows
            for output, source in enumerate(columns):
                actual = (double_bits(table, names[output]) if source % 4 == 1
                          else table.column(output).to_pylist())
                expected = [wide_value(row, source) for row in range(position, position + expected_rows)]
                assert actual == expected, f"wide values: group={ordinal}, column={output}"
            position += expected_rows
        assert position == rows
        return {"file": path.name, "rows": position, "columns": 64, "groups": expected_groups,
                "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def wide_reader_controls(pa, pq, directory, source, rows, group_rows):
    original = pq.read_table(source, page_checksum_verification=True)
    rejected = []
    with tempfile.TemporaryDirectory(prefix="wide-controls-", dir=directory) as temporary:
        # Preserve schema while substituting a different same-type column.
        wrong = original.set_column(0, original.schema.field(0), original.column(4))
        # Output 2 is source column 41. Its first present value is positive zero;
        # negative zero compares equal numerically but has different DOUBLE bits.
        numbers = original.column(2).to_pylist()
        assert numbers[0] == 0.0 and double_bits(original, original.column_names[2])[0] == 0
        numbers[0] = -0.0
        wrong_bits = original.set_column(2, original.schema.field(2), pa.array(numbers, type=pa.float64()))
        for name, table in [("wrong-column", wrong), ("double-bits", wrong_bits),
                            ("prefix", original.slice(0, rows - 1))]:
            path = Path(temporary) / f"{name}.parquet"
            pq.write_table(table, path, compression=None, use_dictionary=False,
                           row_group_size=group_rows, write_page_checksum=True)
            assert pq.read_table(path, page_checksum_verification=True).num_rows == table.num_rows
            try:
                check_wide_file(pa, pq, path, group_rows, rows)
            except AssertionError as error:
                expected_error = "wide extent" if name == "prefix" else "wide values:"
                assert expected_error in str(error), f"wrong rejection for {name}: {error}"
                rejected.append(name)
            else:
                raise AssertionError(f"wide checker accepted {name}")
    assert rejected == ["wrong-column", "double-bits", "prefix"]
    return rejected


def read_wide_export(pa, pq, directory):
    files = [check_wide_file(pa, pq, directory / name, group) for name, group in
             [("wide-257.parquet", 257), ("wide-255.parquet", 255), ("retry.parquet", 257)]]
    assert files[0]["sha256"] == files[2]["sha256"], "wide retry bytes"
    rejected = wide_reader_controls(pa, pq, directory, directory / "wide-257.parquet", 513, 257)
    assert check_wide_file(pa, pq, directory / "wide-257.parquet", 257) == files[0]
    print(json.dumps({"status": "wide-verified", "files": files, "rejected": rejected}, sort_keys=True))


def read_wide_measure(pa, pq, arguments):
    directory, rows, group_rows = Path(arguments[0]), int(arguments[1]), int(arguments[2])
    assert rows in (513, 8192) and group_rows in (257, 1024)
    files = [check_wide_file(pa, pq, directory / f"sample-{sample}.parquet", group_rows, rows)
             for sample in range(6)]
    assert len({record["sha256"] for record in files}) == 1, "wide repeated bytes"
    rejected = (wide_reader_controls(pa, pq, directory, directory / "sample-0.parquet", rows, group_rows)
                if rows == 513 else [])
    assert check_wide_file(pa, pq, directory / "sample-0.parquet", group_rows, rows) == files[0]
    print(json.dumps({"status": "wide-measured", "files": files, "rejected": rejected}, sort_keys=True))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true")
    readers = parser.add_mutually_exclusive_group()
    readers.add_argument("--read-export", type=Path)
    readers.add_argument("--read-boundary-export", type=Path)
    readers.add_argument("--read-profile", nargs=6,
                         metavar=("DIRECTORY", "QUERY", "PROFILE", "ROWS", "GROUP_ROWS", "GROUP_TEXT"))
    readers.add_argument("--read-profile-file", nargs=6,
                         metavar=("FILE", "QUERY", "PROFILE", "ROWS", "GROUP_ROWS", "GROUP_TEXT"))
    readers.add_argument("--describe", action="store_true")
    readers.add_argument("--write-imports", type=Path)
    readers.add_argument("--read-wide-export", type=Path)
    readers.add_argument("--read-wide-measure", nargs=3, metavar=("DIRECTORY", "ROWS", "GROUP_ROWS"))
    readers.add_argument("--write-wide-imports", type=Path)
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
    if args.write_imports:
        write_imports(pa, pq, args.write_imports)
        return
    if args.write_wide_imports:
        write_wide_imports(pa, pq, args.write_wide_imports)
        return
    if args.read_wide_measure:
        read_wide_measure(pa, pq, args.read_wide_measure)
        return
    if args.read_wide_export:
        read_wide_export(pa, pq, args.read_wide_export)
        return
    if args.describe:
        print(json.dumps({"python": sys.version, "pyarrow": pa.__version__, "module": pa.__file__}, sort_keys=True))
        return
    if args.read_profile or args.read_profile_file:
        read_profile(pa, pq, args.read_profile or args.read_profile_file, bool(args.read_profile_file))
        return
    if args.read_export:
        check_values(pa, pq.read_table(args.read_export, page_checksum_verification=True))
        print("External Parquet reader: complete schema, values, NULLs and DOUBLE bits passed")
        return
    if args.read_boundary_export:
        check_boundary_export(pa, pq, args.read_boundary_export)
        print("External Parquet reader: 777 complete typed rows and row-group boundaries passed")
        return
    target = ROOT
    check_values(pa, pq.read_table(target / "pipesql-v1.parquet", page_checksum_verification=True))
    check_boundary_export(pa, pq, target / "pipesql-typed-boundaries.parquet")
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
        print("Retained PipeSQL exports: independent complete-value checks passed")
        print(f"PyArrow {VERSION}: {len(paths)} independent Parquet fixtures {'written' if args.write else 'reproduced'}")


if __name__ == "__main__":
    main()
