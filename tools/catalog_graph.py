#!/usr/bin/env python3
"""Read-only independent namespace-7 graph inspection of an offline database.

Uses no engine decoder or fixture encoder. Resource refusal is distinct from
corruption. Output preserves DOUBLE bits and represents aborted history as gaps
in the issued prefix, never by expanding a potentially u64-sized range.
"""
import argparse
from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import stat
import sys


class Invalid(ValueError):
    pass


class LimitExceeded(ValueError):
    pass


def require(condition, reason):
    if not condition:
        raise Invalid(reason)


def zero(data):
    require(not any(data), "nonzero reserved bytes")


def uint(data, at, width=4):
    require(0 <= at <= len(data) - width, "truncated integer")
    return int.from_bytes(data[at : at + width], "little")


def crc32c(data):
    # Independent reflected Castagnoli recurrence; no production table/import.
    crc = 0xFFFFFFFF
    for byte in data:
        crc ^= byte
        for _ in range(8):
            crc = (crc >> 1) ^ (0x82F63B78 if crc & 1 else 0)
    return crc ^ 0xFFFFFFFF


def protected(data, at):
    return crc32c(data[:at] + bytes(4) + data[at + 4 :]) == uint(data, at)


def object_id(attempt, ordinal):
    require(attempt > 0 and ordinal > 0, "zero object identity")
    return f"{attempt:016x}-{ordinal:08x}.obj"


@dataclass(frozen=True)
class Ref:
    attempt: int
    ordinal: int
    size: int
    crc: int

    @property
    def name(self):
        return object_id(self.attempt, self.ordinal)


def reference(data):
    require(len(data) == 24, "reference extent")
    zero(data[20:])
    ref = Ref(uint(data, 0, 8), uint(data, 8), uint(data, 12), uint(data, 16))
    ref.name
    require(ref.size >= 64, "reference below header")
    return ref


@dataclass(frozen=True)
class Snapshot:
    issued: int
    generation: int
    last: int
    catalog: Ref | None
    successes: Ref | None

    def follows(self, old):
        issuance = self.issued == old.issued + 1 and (
            self.generation,
            self.last,
            self.catalog,
            self.successes,
        ) == (old.generation, old.last, old.catalog, old.successes)
        commit = (
            self.issued == old.issued
            and self.generation == old.generation + 1
            and self.last == self.issued
            and old.last < self.last
        )
        return issuance or commit


def snapshot(data, database, role, wal=False):
    size = 512 if wal else 4096
    # Unknown recognizable versions must not be treated as repairable damage.
    if len(data) >= 12:
        require(uint(data, 8) == 7, "unsupported authoritative version")
    if len(data) != size or data[:8] != (b"PSQLWAL\0" if wal else b"PSQLROOT"):
        raise Invalid("damaged snapshot extent or magic")
    require(protected(data, 108), "damaged snapshot checksum")
    require(uint(data, 12) == size and data[32] == role, "snapshot extent/role")
    require(data[16:32] == database, "foreign snapshot identity")
    for lo, hi in [(33, 40), (48, 56), (80, 108), (120, 128), (176, size)]:
        zero(data[lo:hi])
    generation, issued = uint(data, 40, 8), uint(data, 112, 8)
    if generation == 0:
        zero(data[56:80])
        zero(data[128:176])
        return Snapshot(issued, 0, 0, None, None)
    require(data[56:72] == database, "foreign transaction identity")
    last = uint(data, 72, 8)
    cat, successes = reference(data[128:152]), reference(data[152:176])
    require(
        1 <= generation <= 1048576 and generation <= last <= issued,
        "success/issuance bounds",
    )
    require(
        cat.attempt <= last
        and successes.attempt == last
        and cat.name != successes.name,
        "root object identity",
    )
    require(
        cat.size <= 8256 and successes.size == 64 + 8 * generation,
        "root reference extent",
    )
    return Snapshot(issued, generation, last, cat, successes)


def name(data):
    require(
        re.fullmatch(rb"[A-Za-z_][A-Za-z_0-9]{0,31}", data) is not None, "invalid name"
    )
    return data.decode("ascii")


@dataclass(frozen=True)
class Limits:
    objects: int = 4096
    read_bytes: int = 64 * 1024 * 1024
    values: int = 1_000_000

    def __post_init__(self):
        if not (
            1 <= self.objects <= 1048576
            and 1 <= self.read_bytes <= 1024**3
            and 1 <= self.values <= 10_000_000
        ):
            raise LimitExceeded("invalid diagnostic limit")


class Checker:
    def __init__(self, directory, limits):
        self.path = Path(directory)
        self.limits = limits
        self.read_bytes = 0
        self.values = 0
        self.reached = set()
        self.objects = {}

    def read(self, path, maximum, exact=None):
        # Inspect before opening so FIFOs/devices cannot turn validation into I/O.
        before = path.lstat()
        require(stat.S_ISREG(before.st_mode), f"not a regular file: {path.name}")
        require(
            before.st_nlink == 1
            or path.name in ("ROOT.A", "ROOT.B", "CONTROL", "LOCK"),
            f"aliased mutable/object file: {path.name}",
        )
        require(
            before.st_size <= maximum and (exact is None or before.st_size == exact),
            f"file extent: {path.name}",
        )
        if self.read_bytes + before.st_size > self.limits.read_bytes:
            raise LimitExceeded("read byte budget")
        self.read_bytes += before.st_size
        with os.fdopen(
            os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), "rb"
        ) as stream:
            opened = os.fstat(stream.fileno())
            require(
                (opened.st_dev, opened.st_ino, opened.st_size)
                == (before.st_dev, before.st_ino, before.st_size),
                "file changed during open",
            )
            data = stream.read(before.st_size)
            require(len(data) == before.st_size, "short file read")
            after = os.fstat(stream.fileno())
            require(
                (opened.st_mtime_ns, opened.st_size)
                == (after.st_mtime_ns, after.st_size),
                "file changed during read",
            )
        return data

    def inventory(self, path, maximum):
        require(stat.S_ISDIR(path.lstat().st_mode), f"not a directory: {path.name}")
        entries = {}
        with os.scandir(path) as cursor:
            for entry in cursor:
                if len(entries) == maximum:
                    raise LimitExceeded(f"directory entry budget: {path.name}")
                entries[entry.name] = entry.stat(follow_symlinks=False)
        return entries

    def object(self, ref, maximum, magic, metadata=None):
        require(ref.name in self.objects, f"missing reference: {ref.name}")
        require(ref.name not in self.reached, f"object alias or cycle: {ref.name}")
        require(ref.size <= maximum, "role-specific object extent")
        self.reached.add(ref.name)
        data = self.read(self.path / "units" / ref.name, maximum, ref.size)
        require(
            data[:8] == magic and uint(data, 8) == 6,
            f"object magic/version: {ref.name}",
        )
        require(data[16:32] == self.database, f"foreign object identity: {ref.name}")
        require(
            crc32c(data if metadata is None else data[:metadata]) == ref.crc,
            f"object checksum: {ref.name}",
        )
        return data

    def schema(self, ref, table):
        data = self.object(ref, 3136, b"PSQLSCHM")
        require(uint(data, 32, 8) == table, f"schema table identity: {ref.name}")
        zero(data[40:64])
        n = uint(data, 12)
        require(1 <= n <= 64 and len(data) == 64 + 48 * n, "schema count/extent")
        columns = []
        for at in range(64, len(data), 48):
            row = data[at : at + 48]
            identity, kind, nullable, length = uint(row, 0), row[4], row[5], row[6]
            require(
                identity > 0 and 1 <= kind <= 4 and nullable <= 1 and 1 <= length <= 32,
                "column fields",
            )
            zero(row[7:8])
            zero(row[8 + length :])
            label = name(row[8 : 8 + length])
            require(
                all(
                    c["id"] != identity and c["name"].lower() != label.lower()
                    for c in columns
                ),
                "duplicate column identity/name",
            )
            columns.append(
                dict(id=identity, name=label, type=kind, nullable=bool(nullable))
            )
        return columns

    def unit(self, ref, rows, table, columns):
        require(1 <= rows <= 32768, "unit row count")
        if self.values + rows * len(columns) > self.limits.values:
            raise LimitExceeded("decoded value budget")
        self.values += rows * len(columns)
        metadata = 64 + 32 * len(columns)
        data = self.object(ref, 33556544, b"PSQLDATA", metadata)
        require(
            len(data) >= metadata
            and uint(data, 12) == len(columns)
            and uint(data, 32, 8) == table
            and uint(data, 40, 8) == ref.attempt
            and uint(data, 48) == ref.ordinal
            and uint(data, 52) == rows
            and uint(data, 56) == metadata,
            "unit identity/geometry",
        )
        zero(data[60:64])
        decoded = {}
        offset = metadata
        for at in range(64, metadata, 32):
            d = data[at : at + 32]
            identity = uint(d, 0)
            specs = [c for c in columns if c["id"] == identity]
            require(len(specs) == 1 and identity not in decoded, "unit column identity")
            col = specs[0]
            require(
                d[4] == col["type"]
                and d[5] == col["nullable"]
                and uint(d, 8, 8) == offset,
                "column type/coverage",
            )
            zero(d[6:8])
            zero(d[24:])
            length = uint(d, 16)
            bitmap = (rows + 7) // 8
            width = {1: 8, 2: 8, 4: 4}.get(col["type"])
            minimum = bitmap + (rows * width if width else (rows + 1) * 4)
            require(
                minimum <= length <= 524288
                and (width is None or length == minimum)
                and offset + length <= len(data),
                "payload extent",
            )
            payload = data[offset : offset + length]
            offset += length
            require(crc32c(payload) == uint(d, 20), "payload checksum")
            require(
                rows % 8 == 0 or payload[bitmap - 1] >> (rows % 8) == 0,
                "validity padding",
            )
            values = []
            textbase = bitmap + (rows + 1) * 4
            if width is None:
                require(uint(payload, bitmap) == 0, "initial string offset")
            previous = 0
            for row in range(rows):
                present = bool(payload[row // 8] & (1 << (row % 8)))
                require(present or col["nullable"], "NULL in nonnullable column")
                if width:
                    raw = payload[bitmap + row * width : bitmap + (row + 1) * width]
                    if not present:
                        zero(raw)
                        value = None
                    elif col["type"] == 2:
                        value = {"double_bits": raw[::-1].hex()}
                    else:
                        value = int.from_bytes(raw, "little", signed=True)
                        require(
                            col["type"] != 4 or -719162 <= value <= 2932896,
                            "DATE domain",
                        )
                else:
                    end = uint(payload, bitmap + (row + 1) * 4)
                    require(
                        previous <= end <= length - textbase
                        and end - previous <= 65536,
                        "string offsets/length",
                    )
                    require(present or previous == end, "NULL string payload")
                    try:
                        value = payload[textbase + previous : textbase + end].decode(
                            "utf-8"
                        )
                    except UnicodeDecodeError as error:
                        raise Invalid("string UTF-8") from error
                    previous = end
                    if not present:
                        value = None
                values.append(value)
            if width is None:
                require(previous == length - textbase, "trailing string bytes")
            decoded[identity] = values
        require(offset == len(data), "trailing unit bytes")
        return [[decoded[c["id"]][row] for c in columns] for row in range(rows)]

    def catalog(self, snapshot):
        ref = snapshot.catalog
        data = self.object(ref, 8256, b"PSQLCATL")
        require(
            uint(data, 32, 8) == ref.attempt and uint(data, 40) == ref.ordinal,
            "catalog identity",
        )
        zero(data[44:64])
        n = uint(data, 12)
        require(n <= 64 and len(data) == 64 + 128 * n, "catalog count/extent")
        tables = []
        previous = 0
        for at in range(64, len(data), 128):
            entry = data[at : at + 128]
            identity, length = uint(entry, 0, 8), entry[8]
            require(
                identity > previous and 1 <= length <= 32, "table identity/name length"
            )
            previous = identity
            zero(entry[9:16])
            zero(entry[16 + length : 48])
            zero(entry[108:])
            label = name(entry[16 : 16 + length])
            require(
                all(t["name"].lower() != label.lower() for t in tables),
                "duplicate table name",
            )
            schema = reference(entry[48:72])
            require(schema.attempt <= ref.attempt, "future schema reference")
            columns = self.schema(schema, identity)
            rows, units = uint(entry, 96, 8), uint(entry, 104)
            require(
                units <= 4096 and units <= rows <= units * 32768,
                "table row/unit counts",
            )
            output = []
            if units == 0:
                zero(entry[72:96])
            else:
                indexref = reference(entry[72:96])
                require(
                    indexref.attempt <= ref.attempt
                    and indexref.size == 64 + 48 * units,
                    "table index reference",
                )
                index = self.object(indexref, 196672, b"PSQLTBLD")
                require(
                    uint(index, 12) == units
                    and uint(index, 32, 8) == identity
                    and uint(index, 40, 8) == indexref.attempt
                    and uint(index, 48) == indexref.ordinal
                    and uint(index, 56, 8) == rows,
                    "table index header",
                )
                zero(index[52:56])
                prior = (0, 0)
                for start in range(64, len(index), 48):
                    unit = index[start : start + 48]
                    attempt, ordinal = uint(unit, 0, 8), uint(unit, 8)
                    count = uint(unit, 12)
                    require(
                        prior < (attempt, ordinal) and attempt <= indexref.attempt,
                        "unit order/creator",
                    )
                    require(uint(unit, 24, 8) == len(output), "unit row coverage")
                    zero(unit[32:])
                    prior = attempt, ordinal
                    output.extend(
                        self.unit(
                            Ref(attempt, ordinal, uint(unit, 16), uint(unit, 20)),
                            count,
                            identity,
                            columns,
                        )
                    )
                require(len(output) == rows, "table row total")
            tables.append(dict(id=identity, name=label, columns=columns, rows=output))
        return tables

    def graph(self):
        entries = self.inventory(self.path, 9)
        allowed = {
            "LOCK",
            "CONTROL",
            "ROOT.A",
            "ROOT.B",
            "WAL",
            "units",
            "private",
            "ROOT.A.next",
            "ROOT.B.next",
        }
        require(
            set(entries) <= allowed
            and {"LOCK", "CONTROL", "WAL", "units", "private"} <= set(entries),
            "namespace names",
        )
        self.read(self.path / "LOCK", 0, 0)
        control = self.read(self.path / "CONTROL", 128, 128)
        require(
            control[:8] == b"PIPESQL\0"
            and uint(control, 8) == 7
            and uint(control, 12) == 128
            and protected(control, 36),
            "CONTROL header/checksum",
        )
        zero(control[32:36])
        zero(control[40:])
        self.database = control[16:32]
        require(any(self.database), "zero database identity")
        snapshots = []
        for label, role in [("ROOT.A", 0), ("ROOT.B", 1), ("WAL", 0)]:
            if label not in entries:
                snapshots.append(None)
                continue
            size = 512 if label == "WAL" else 4096
            raw = self.read(self.path / label, size, size if label == "WAL" else None)
            try:
                snapshots.append(snapshot(raw, self.database, role, label == "WAL"))
            except Invalid as error:
                # Only damaged bytes may be ignored. Semantic/version/identity
                # violations are authoritative and never license an older graph.
                if not str(error).startswith("damaged snapshot"):
                    raise
                snapshots.append(None)
        a, b, fence = snapshots
        if a is not None and b is not None:
            require(a == b or a.follows(b) or b.follows(a), "nonadjacent roots")
            selected = max((a, b), key=lambda s: (s.issued, s.generation))
        else:
            selected = a or b
            require(
                selected is not None and fence == selected,
                "insufficient root authority",
            )
        require(
            fence is None or fence == selected or fence.follows(selected),
            "inconsistent fence",
        )
        private = self.inventory(self.path / "private", 2)
        for label in private:
            require(label in ("SCRATCH.A", "SCRATCH.B"), "unknown scratch name")
            self.read(self.path / "private" / label, 0, 0)
        for label in set(entries) & {"ROOT.A.next", "ROOT.B.next"}:
            self.read(self.path / label, 4096)
        self.objects = self.inventory(self.path / "units", self.limits.objects)
        for label, meta in self.objects.items():
            require(
                re.fullmatch(r"[0-9a-f]{16}-[0-9a-f]{8}\.obj", label) is not None,
                "object filename",
            )
            attempt, ordinal = int(label[:16], 16), int(label[17:25], 16)
            object_id(attempt, ordinal)
            require(attempt <= selected.issued, "object beyond issued prefix")
            require(
                stat.S_ISREG(meta.st_mode)
                and meta.st_nlink == 1
                and meta.st_size <= 33556544,
                "object ownership/extent",
            )
        successes = []
        tables = []
        if selected.generation:
            ref = selected.successes
            data = self.object(ref, 8388672, b"PSQLSUCC")
            require(
                uint(data, 32, 8) == selected.generation
                and uint(data, 40, 8) == ref.attempt
                and uint(data, 48) == ref.ordinal
                and uint(data, 56, 8) == selected.last,
                "history header",
            )
            zero(data[12:16])
            zero(data[52:56])
            previous = 0
            for at in range(64, len(data), 8):
                attempt = uint(data, at, 8)
                require(previous < attempt <= selected.last, "history order")
                successes.append(attempt)
                previous = attempt
            require(previous == selected.last, "history final receipt")
            tables = self.catalog(selected)
        return dict(
            database=self.database.hex(),
            issued=selected.issued,
            generation=selected.generation,
            successes=successes,
            tables=tables,
            roots_settled=a == b == fence and a is not None,
            cleanup_names=sorted(private)
            + sorted(set(entries) & {"ROOT.A.next", "ROOT.B.next"}),
            reachable=sorted(self.reached),
            unreferenced=sorted(set(self.objects) - self.reached),
            reachable_bytes=sum(self.objects[n].st_size for n in self.reached),
            unreferenced_bytes=sum(
                meta.st_size
                for n, meta in self.objects.items()
                if n not in self.reached
            ),
            read_bytes=self.read_bytes,
            decoded_values=self.values,
        )


def inspect(directory, limits=Limits()):
    if sys.platform not in ("darwin", "linux"):
        raise OSError("catalog inspection requires macOS or Linux file locking")

    import fcntl

    checker = Checker(directory, limits)
    # Cooperating engine writers use the same exclusive flock on Darwin/Unix.
    # Work on an offline copy; no defense against noncooperating external writes.
    require(stat.S_ISDIR(checker.path.lstat().st_mode), "database is not a directory")
    with os.fdopen(
        os.open(checker.path / "LOCK", os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK),
        "rb",
    ) as lease:
        require(stat.S_ISREG(os.fstat(lease.fileno()).st_mode), "LOCK is not regular")
        fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return checker.graph()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("database", type=Path)
    parser.add_argument("--max-objects", type=int, default=4096)
    parser.add_argument("--max-read-bytes", type=int, default=64 * 1024 * 1024)
    parser.add_argument("--max-values", type=int, default=1_000_000)
    args = parser.parse_args(argv)
    try:
        result = inspect(
            args.database,
            Limits(args.max_objects, args.max_read_bytes, args.max_values),
        )
    except LimitExceeded as error:
        print(f"limit: {error}", file=sys.stderr)
        return 3
    except (Invalid, OSError) as error:
        print(f"invalid or unavailable: {error}", file=sys.stderr)
        return 2
    json.dump(result, sys.stdout, ensure_ascii=True, sort_keys=True)
    print()
    return 0


if __name__ == "__main__":
    sys.exit(main())
