"""Independent public checks for aggregate and projection composition."""

import argparse
import hashlib
import datetime
import operator
import json
import math
from pathlib import Path
import runpy
import struct
import sys
import tempfile

from check_process import run as run_process
from check_support import build_cli, require_executable

ROOT = Path(__file__).resolve().parents[1]


class SnapshotEncoder:
    """Own the independent snapshot oracle for one composition campaign."""

    def __init__(self):
        self.model = runpy.run_path(str(ROOT / "tools/snapshot-fixtures.py"))

    def write(self, path, rows):
        """Encode a valid format-4 snapshot independently of the engine decoder."""
        model = self.model
        crc = model["CRC"]
        path.mkdir()
        (path / "units").mkdir()
        (path / "private").mkdir()
        vectors = model["vectors"]()
        header = bytearray(vectors["UNIT"][:4096])
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
        unit = model["LEGACY"]["refresh_unit_checksums"](
            header + descriptors + bytes(3072) + payload
        )
        assert len(unit) == offset
        metadata_crc = struct.unpack_from("<I", unit, 108)[0]
        for name in ("ROOT.A", "ROOT.B", "WAL"):
            value = bytearray(vectors[name])
            struct.pack_into("<QQ", value, 88, len(rows), len(unit))
            struct.pack_into("<I", value, 104, metadata_crc)
            struct.pack_into("<I", value, 108, 0)
            struct.pack_into("<I", value, 108, crc(value))
            (path / name).write_bytes(value)
        (path / "CONTROL").write_bytes(vectors["CONTROL"])
        (path / "LOCK").write_bytes(b"")
        (path / "units/0000000000000001.unit").write_bytes(unit)
        return {
            str(p.relative_to(path)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(path.rglob("*"))
            if p.is_file()
        }


def raw(value):
    return struct.unpack("<Q", struct.pack("<d", value))[0]


def double(value):
    return struct.unpack("<d", struct.pack("<Q", value))[0]


def encoded(value):
    if value is None:
        return "null"
    if isinstance(value, int):
        return f"int64:{value}"
    return f"{raw(value):016x}"


def parse_rows(stdout):
    rows = []
    for line in stdout.splitlines():
        if line.startswith("row="):
            rows.append(
                [
                    part.rsplit(":", 1)[-1] if part.startswith("double:") else part
                    for part in line[4:].split("|")
                ]
            )
    return rows


GLOBAL_SQL = "FROM lineitem |> AGGREGATE SUM(l_quantity) AS s, AVG(l_quantity) AS a, COUNT(*) AS n"
GROUPED_SQL = "FROM lineitem |> AGGREGATE SUM(l_quantity) AS qs, AVG(l_quantity) AS qa, SUM(l_extendedprice) AS ps, AVG(l_extendedprice) AS pa, SUM(l_discount) AS ds, AVG(l_discount) AS da, SUM(l_tax) AS ts, COUNT(*) AS n GROUP AND ORDER BY l_returnflag,l_linestatus"
OVERFLOW_EXPRESSION = "l_quantity*(9223372036854775807+1)"
MAX_DOUBLE = sys.float_info.max
EPOCH = datetime.date(1970, 1, 1)
GROUP_SUM_SQL = "FROM lineitem |> AGGREGATE SUM(l_quantity) AS total GROUP BY l_returnflag"


def source_rows():
    dates = [
        datetime.date.fromisoformat(value)
        for value in [
            "0001-01-01",
            "1900-02-28",
            "1969-12-31",
            "1970-01-01",
            "1993-12-31",
            "1994-01-01",
            "1994-06-15",
            "1994-12-31",
            "1995-01-01",
            "1998-09-17",
            "1998-09-18",
            "1998-09-19",
            "2020-02-29",
            "2021-02-28",
            "9999-12-31",
        ]
    ]
    # Independent input-domain enumeration; never infer key bytes from a dense
    # group index or copy the engine's mapping formula.
    keys = [byte for byte in range(256) if 32 <= byte <= 126 and byte != ord("|")]
    assert len(keys) == 94 and keys[0] == 32 and 124 not in keys
    rows = [
        [
            raw(float(i % 31)),
            raw(float(i % 97 + 1)),
            raw((i % 17) / 128),
            raw((i % 9) / 4),
            keys[i % len(keys)],
            keys[(i // len(keys)) % len(keys)],
            (dates[i % len(dates)] - EPOCH).days,
        ]
        for i in range(65537)
    ]
    return rows


def expected_groups(source_rows, keys, entries, prefix):
    groups = {}
    if not keys:
        groups[()] = []
    for row in source_rows:
        groups.setdefault(tuple(row[key] for key in keys), []).append(row)
    expected = []
    for key, group in sorted(groups.items()):
        result = [f"string:{value:02x}" for value in key]
        for kind, column in entries:
            if kind == "count":
                value = len(group)
            elif not group:
                value = None
            else:
                values = [
                    column(row) if callable(column) else double(row[column])
                    for row in group
                ]
                # Input values in this corpus are exactly representable binary fractions.
                value = values[0]
                for other in values[1:]:
                    value += other
                if kind == "avg":
                    value /= len(values)
            result.append(encoded(value))
        expected.append(result)
    if prefix is not None:
        count, offset = prefix
        expected = expected[offset:][:count]
    return expected


QUERY_LIMITS = ["--memory-limit-bytes", "2000000", "--temp-limit-bytes", "1000000"]


class QueryChecks:
    """Run stock queries and retain observations; expected rows come from callers."""

    def __init__(self, cli, work):
        self.cli = cli
        self.work = work
        self.observations = []

    def run(self, database, sql):
        (self.work / "query.sql").write_text(sql)
        return run_process(
            [
                str(self.cli),
                "query",
                "--database",
                str(self.work / database),
                "--query-file",
                str(self.work / "query.sql"),
                *QUERY_LIMITS,
            ],
            capture_output=True,
            text=True,
            timeout=30,
            cwd=ROOT,
        )

    def aggregate(
        self,
        label,
        sql,
        source_rows,
        keys,
        entries,
        ordered=False,
        database="data",
        prefix=None,
    ):
        expected = expected_groups(source_rows, keys, entries, prefix)
        call = self.run(database, sql)
        assert call.returncode == 0, (label, call.stdout, call.stderr)
        actual = parse_rows(call.stdout)
        assert actual == expected if ordered else sorted(actual) == sorted(expected), (
            label,
            actual[:3],
            expected[:3],
        )
        assert (
            f"row_count={len(expected)}" in call.stdout
            and "status=queried" in call.stdout
        )
        self.observations.append(
            {
                "case": label,
                "rows": len(expected),
                "sha256": hashlib.sha256(call.stdout.encode()).hexdigest(),
            }
        )

    def composed(self, label, sql, expected, database="data", ordered=True):
        call = self.run(database, sql)
        assert call.returncode == 0, (label, call.stdout, call.stderr)
        actual = parse_rows(call.stdout)
        assert actual == expected if ordered else sorted(actual) == sorted(expected), (
            label,
            actual[:4],
            expected[:4],
        )
        assert (
            f"row_count={len(expected)}" in call.stdout
            and "status=queried" in call.stdout
        )
        self.observations.append(
            {
                "case": label,
                "rows": len(expected),
                "sha256": hashlib.sha256(call.stdout.encode()).hexdigest(),
            }
        )


def check_grouping(queries, rows):
    queries.aggregate("global", GLOBAL_SQL, rows, [], [("sum", 0), ("avg", 0), ("count", 0)])
    queries.aggregate(
        "empty-global",
        GLOBAL_SQL,
        [],
        [],
        [("sum", 0), ("avg", 0), ("count", 0)],
        database="empty",
    )
    entries = [
        ("sum", 0),
        ("avg", 0),
        ("sum", 1),
        ("avg", 1),
        ("sum", 2),
        ("avg", 2),
        ("sum", 3),
        ("count", 0),
    ]
    queries.aggregate("all-groups", GROUPED_SQL, rows, [4, 5], entries, True)
    # The independent group model establishes order before selecting a prefix;
    # these cases make no assumption about an unordered source's scan order.
    for count in [0, 1, 3, 99999, 2**63 - 1]:
        for offset in [0, 1, 256, 2**63 - 1]:
            queries.aggregate(
                f"limit-groups-{count}-{offset}",
                f"{GROUPED_SQL} |> LIMIT {count} OFFSET {offset}",
                rows,
                [4, 5],
                entries,
                True,
                prefix=(count, offset),
            )
    queries.aggregate("empty-groups", GROUPED_SQL, [], [4, 5], entries, True, database="empty")
    queries.aggregate(
        "filtered-groups",
        GROUPED_SQL.replace(" |> AGGREGATE", " |> WHERE l_quantity < 25.0 |> AGGREGATE"),
        [row for row in rows if double(row[0]) < 25],
        [4, 5],
        entries,
        True,
    )
    queries.aggregate(
        "filtered-empty",
        GLOBAL_SQL.replace(" |> AGGREGATE", " |> WHERE l_quantity < 0.0 |> AGGREGATE"),
        [],
        [],
        [("sum", 0), ("avg", 0), ("count", 0)],
    )
    queries.aggregate(
        "alias-swap",
        "FROM lineitem |> SELECT l_quantity AS price,l_extendedprice AS qty,l_returnflag AS flag |> AGGREGATE SUM(qty) AS total,AVG(price) AS mean GROUP AND ORDER BY flag",
        rows,
        [4],
        [("sum", 1), ("avg", 0)],
        True,
    )
    queries.aggregate(
        "counts-only",
        "FROM lineitem |> AGGREGATE COUNT(*) AS n GROUP BY l_linestatus",
        rows,
        [5],
        [("count", 0)],
    )
    queries.aggregate(
        "no-input-columns",
        "FROM lineitem |> AGGREGATE COUNT(*) AS n",
        rows,
        [],
        [("count", 0)],
    )
    queries.aggregate(
        "duplicate-input",
        "FROM lineitem |> SELECT l_quantity AS q,l_quantity AS other |> AGGREGATE SUM(other) AS s,SUM(q) AS again,AVG(q) AS a",
        rows,
        [],
        [("sum", 0), ("sum", 0), ("avg", 0)],
    )


def check_expressions_and_reference_queries(queries, rows):
    q1 = "FROM lineitem |> AGGREGATE SUM(l_quantity) AS sum_qty,SUM(l_extendedprice) AS sum_base_price,SUM(l_extendedprice*(1-l_discount)) AS sum_disc_price,SUM(l_extendedprice*(1-l_discount)*(1+l_tax)) AS sum_charge,AVG(l_quantity) AS avg_qty,AVG(l_extendedprice) AS avg_price,AVG(l_discount) AS avg_disc,COUNT(*) AS count_order GROUP AND ORDER BY l_returnflag,l_linestatus"
    q1_entries = [
        ("sum", 0),
        ("sum", 1),
        ("sum", lambda r: double(r[1]) * (1 - double(r[2]))),
        ("sum", lambda r: double(r[1]) * (1 - double(r[2])) * (1 + double(r[3]))),
        ("avg", 0),
        ("avg", 1),
        ("avg", 2),
        ("count", 0),
    ]
    deep = "l_quantity" + "".join("+(" + "1" for _ in range(15)) + ")" * 15
    deep_sql = f"FROM lineitem |> AGGREGATE SUM({deep}) AS complex,SUM(l_quantity) AS q,SUM(l_extendedprice) AS p,SUM(l_discount) AS d,SUM(l_tax) AS t GROUP AND ORDER BY l_returnflag,l_linestatus"
    queries.aggregate(
        "deep-expression-under-cap",
        deep_sql,
        rows,
        [4, 5],
        [
            ("sum", lambda r: double(r[0]) + 15),
            ("sum", 0),
            ("sum", 1),
            ("sum", 2),
            ("sum", 3),
        ],
        True,
    )
    queries.aggregate("q1-five-states-without-date", q1, rows, [4, 5], q1_entries, True)
    for label, expr, model in [
        ("precedence", "l_quantity+2*3", lambda r: double(r[0]) + 6),
        ("parentheses", "(l_quantity+2)*3", lambda r: (double(r[0]) + 2) * 3),
        ("unary", "-(l_quantity-2)*-3", lambda r: -(double(r[0]) - 2) * -3),
        (
            "integer-intermediate",
            "l_extendedprice*(9007199254740993-9007199254740992)",
            lambda r: double(r[1]),
        ),
        (
            "int64-min",
            "l_quantity+(-9223372036854775808+9223372036854775807)",
            lambda r: double(r[0]) - 1,
        ),
        ("signed-zero", "l_quantity*-0.0", lambda r: double(r[0]) * -0.0),
        (
            "exact-operation-bound",
            "-(" + "+".join(["l_quantity"] + ["1"] * 15) + ")",
            lambda r: -(double(r[0]) + 15),
        ),
        (
            "exact-nesting-bound",
            "(" * 32 + "l_quantity" + ")" * 32,
            lambda r: double(r[0]),
        ),
    ]:
        queries.aggregate(
            label,
            f"FROM lineitem |> AGGREGATE SUM({expr}) AS value",
            rows,
            [],
            [("sum", model)],
        )
        queries.aggregate(
            "computed-" + label,
            f"FROM lineitem |> SELECT {expr} AS x |> AGGREGATE SUM(x) AS value",
            rows,
            [],
            [("sum", model)],
        )
        queries.aggregate(
            "computed-filter-" + label,
            f"FROM lineitem |> SELECT {expr} AS x |> WHERE x < 20 |> AGGREGATE SUM(x) AS value",
            [r for r in rows if model(r) < 20],
            [],
            [("sum", model)],
        )
    cutoff = (datetime.date(1998, 12, 1) - datetime.timedelta(days=74) - EPOCH).days
    queries.aggregate(
        "actual-q1-source",
        (ROOT / "tests/fixtures/upstream/q1-upstream.pipe.sql").read_text(),
        [r for r in rows if r[6] <= cutoff],
        [4, 5],
        q1_entries,
        True,
    )
    q6_rows = [
        r
        for r in rows
        if (datetime.date(1994, 1, 1) - EPOCH).days
        <= r[6]
        < (datetime.date(1995, 1, 1) - EPOCH).days
        and 0.08 - 0.01 <= double(r[2]) <= 0.08 + 0.01
        and double(r[0]) < 25
    ]
    assert q6_rows
    q6 = (ROOT / "tests/fixtures/q6.pipe.sql").read_text()
    queries.aggregate(
        "actual-q6-source",
        q6,
        q6_rows,
        [],
        [("sum", lambda r: double(r[1]) * double(r[2]))],
    )


def check_dates_and_predicates(queries, work, encoder, rows):
    for symbol, compare in [
        ("<", operator.lt),
        ("<=", operator.le),
        ("=", operator.eq),
        ("!=", operator.ne),
        (">=", operator.ge),
        (">", operator.gt),
    ]:
        bound = (datetime.date(1994, 1, 1) - EPOCH).days
        queries.aggregate(
            "date-" + symbol,
            f"FROM lineitem |> WHERE l_shipdate {symbol} DATE '1994-01-01' |> AGGREGATE COUNT(*) AS n",
            [r for r in rows if compare(r[6], bound)],
            [],
            [("count", 0)],
        )
    date_cases = [
        ("DATE_ADD(DATE '2020-01-31',INTERVAL 1 MONTH)", datetime.date(2020, 2, 29)),
        ("DATE_ADD(DATE '2020-02-29',INTERVAL 1 YEAR)", datetime.date(2021, 2, 28)),
        ("DATE_SUB(DATE '1995-01-01',INTERVAL 1 DAY)", datetime.date(1994, 12, 31)),
        ("DATE_ADD(DATE '1995-01-01',INTERVAL -1 DAY)", datetime.date(1994, 12, 31)),
        ("DATE_SUB(DATE '1993-12-31',INTERVAL -1 DAY)", datetime.date(1994, 1, 1)),
        (
            "DATE_SUB(DATE_ADD(DATE '2020-02-29',INTERVAL 1 YEAR),INTERVAL 1 YEAR)",
            datetime.date(2020, 2, 28),
        ),
        ("DATE '0001-01-01'", datetime.date.min),
        ("DATE '9999-12-31'", datetime.date.max),
        (
            "DATE_ADD(" * 8 + "DATE '1994-01-01'" + ",INTERVAL 0 DAY)" * 8,
            datetime.date(1994, 1, 1),
        ),
    ]
    for literal, expected_date in date_cases:
        bound = (expected_date - EPOCH).days
        queries.aggregate(
            "date-fold-" + literal,
            f"FROM lineitem |> WHERE l_shipdate <= {literal} |> AGGREGATE COUNT(*) AS n",
            [r for r in rows if r[6] <= bound],
            [],
            [("count", 0)],
        )
    queries.aggregate(
        "date-alias",
        "FROM lineitem |> SELECT l_shipdate AS ship |> WHERE ship >= DATE '1994-01-01' AND ship < DATE '1995-01-01' |> AGGREGATE COUNT(*) AS n",
        [r for r in rows if 8766 <= r[6] < 9131],
        [],
        [("count", 0)],
    )
    queries.aggregate(
        "date-between",
        "FROM lineitem |> WHERE l_shipdate BETWEEN DATE '1994-01-01' AND DATE '1994-12-31' |> AGGREGATE COUNT(*) AS n",
        [r for r in rows if 8766 <= r[6] <= 9130],
        [],
        [("count", 0)],
    )
    queries.aggregate(
        "reversed-between",
        "FROM lineitem |> WHERE l_quantity BETWEEN 25 AND 2 |> AGGREGATE COUNT(*) AS n",
        [],
        [],
        [("count", 0)],
    )
    for literal in [
        "DATE '1900-02-29'",
        "DATE '0000-01-01'",
        "DATE '9999-13-01'",
        "DATE_ADD(DATE '9999-12-31',INTERVAL 1 DAY)",
        "DATE_SUB(DATE '0001-01-01',INTERVAL 1 MONTH)",
        "DATE_ADD(DATE '1970-01-01',INTERVAL 9223372036854775807 YEAR)",
        "DATE_ADD(DATE '1970-01-01',INTERVAL 1.5 DAY)",
        "DATE_ADD(DATE '1970-01-01',INTERVAL 1 HOUR)",
        "DATE_ADD(" * 9 + "DATE '1994-01-01'" + ",INTERVAL 0 DAY)" * 9,
    ]:
        call = queries.run(
            "empty",
            f"FROM lineitem |> WHERE l_shipdate < {literal} |> AGGREGATE COUNT(*) AS n",
        )
        assert (
            call.returncode == 1 and not call.stdout and "panicked" not in call.stderr
        ), (literal, call.stdout, call.stderr)
        queries.observations.append(
            {"case": "invalid-date", "literal": literal, "outcome": "refused"}
        )
    for predicate in [
        "l_quantity < DATE '1994-01-01'",
        "l_shipdate < 1994",
        " AND ".join(["l_quantity > 0"] * 17),
    ]:
        call = queries.run(
            "empty", f"FROM lineitem |> WHERE {predicate} |> AGGREGATE COUNT(*) AS n"
        )
        assert (
            call.returncode == 1 and not call.stdout and "panicked" not in call.stderr
        ), (predicate, call.stdout, call.stderr)
        queries.observations.append(
            {"case": "invalid-condition", "predicate": predicate, "outcome": "refused"}
        )
    queries.aggregate(
        "normalized-stage-bound",
        "FROM lineitem |> WHERE "
        + " AND ".join(["l_quantity > 0"] * 15)
        + " |> AGGREGATE COUNT(*) AS n",
        [r for r in rows if double(r[0]) > 0],
        [],
        [("count", 0)],
    )
    bad_rows = [r.copy() for r in rows]
    bad_rows[-1][6] = 2932897
    encoder.write(work / "bad-date", bad_rows)
    for mode, sql in [
        (
            "scan",
            "FROM lineitem |> WHERE l_shipdate >= DATE '0001-01-01' |> SELECT l_quantity",
        ),
        (
            "aggregate",
            "FROM lineitem |> WHERE l_shipdate >= DATE '0001-01-01' |> AGGREGATE COUNT(*) AS n",
        ),
    ]:
        call = queries.run("bad-date", sql)
        assert (
            call.returncode == 1
            and "stored DATE" in call.stderr
            and "status=queried" not in call.stdout
        ), (mode, call.stdout[-200:], call.stderr)
        assert bool(parse_rows(call.stdout)) == (mode == "scan"), mode
        queries.observations.append(
            {
                "case": "invalid-stored-date-" + mode,
                "outcome": "failed",
                "prefix_rows": len(parse_rows(call.stdout)),
            }
        )
    queries.aggregate(
        "undemanded-stored-date",
        "FROM lineitem |> WHERE l_quantity < 0 AND l_shipdate >= DATE '0001-01-01' |> AGGREGATE COUNT(*) AS n",
        [],
        [],
        [("count", 0)],
        database="bad-date",
    )


def check_numeric_failures(queries, work, encoder):
    queries.aggregate(
        "filtered-argument-not-evaluated",
        f"FROM lineitem |> WHERE l_quantity < 0.0 |> AGGREGATE SUM({OVERFLOW_EXPRESSION}) AS value",
        [],
        [],
        [("sum", 0)],
    )
    queries.aggregate(
        "empty-argument-not-evaluated",
        f"FROM lineitem |> AGGREGATE SUM({OVERFLOW_EXPRESSION}) AS value",
        [],
        [],
        [("sum", 0)],
        database="empty",
    )
    for expr in [OVERFLOW_EXPRESSION, "l_quantity*(-(-9223372036854775808))"]:
        call = queries.run("data", f"FROM lineitem |> AGGREGATE SUM({expr}) AS value")
        assert (
            call.returncode == 1
            and "overflow" in call.stderr.lower()
            and "row=" not in call.stdout
        ), (expr, call.stdout, call.stderr)
        queries.observations.append(
            {
                "case": "integer-operation-overflow",
                "expression": expr,
                "outcome": "refused",
            }
        )
    for expr in [
        "l_quantity+9223372036854775808",
        "l_quantity+-9223372036854775809",
        "l_quantity+",
        "(" * 33 + "l_quantity" + ")" * 33,
        "+".join(["l_quantity"] * 17),
    ]:
        call = queries.run("empty", f"FROM lineitem |> AGGREGATE SUM({expr}) AS value")
        assert (
            call.returncode == 1 and not call.stdout and "panicked" not in call.stderr
        ), (expr, call.stdout, call.stderr)
        queries.observations.append(
            {"case": "invalid-scalar-program", "expression": expr, "outcome": "refused"}
        )
    for label, values, mode, expected in [
        ("mean-only", [MAX_DOUBLE, MAX_DOUBLE], "AVG", MAX_DOUBLE),
        ("sum-cancel", [MAX_DOUBLE, MAX_DOUBLE, -MAX_DOUBLE], "SUM", MAX_DOUBLE),
        ("late-nan", [MAX_DOUBLE, MAX_DOUBLE, float("nan")], "SUM", float("nan")),
        ("late-inf", [MAX_DOUBLE, MAX_DOUBLE, float("inf")], "SUM", float("inf")),
    ]:
        encoder.write(
            work / label,
            [[raw(value), raw(1.0), raw(0.0), raw(0.0), 65, 70, 0] for value in values],
        )
        call = queries.run(label, f"FROM lineitem |> AGGREGATE {mode}(l_quantity) AS value")
        assert call.returncode == 0, (label, call.stderr)
        actual = double(int(parse_rows(call.stdout)[0][0], 16))
        assert math.isnan(actual) if math.isnan(expected) else actual == expected, (
            label,
            actual,
        )
        queries.observations.append({"case": label, "outcome": "matched"})
    call = queries.run("late-nan", "FROM lineitem |> AGGREGATE SUM(l_quantity*2.0) AS value")
    assert (
        call.returncode == 1
        and "overflow" in call.stderr.lower()
        and "row=" not in call.stdout
    ), (call.stdout, call.stderr)
    queries.observations.append(
        {"case": "scalar-overflow-before-late-nan", "outcome": "refused"}
    )
    for sql in [
        "FROM lineitem |> AGGREGATE SUM(l_quantity) AS value",
        "FROM lineitem |> AGGREGATE AVG(l_quantity) AS mean,SUM(l_quantity) AS total",
    ]:
        call = queries.run("mean-only", sql)
        assert (
            call.returncode == 1
            and "overflow" in call.stderr.lower()
            and "row=" not in call.stdout
        ), (sql, call.stdout, call.stderr)
        queries.observations.append(
            {"case": "demanded-sum-overflow", "sql": sql, "outcome": "refused"}
        )
    for label, sql in [
        ("date-sum", "FROM lineitem |> AGGREGATE SUM(l_shipdate) AS x"),
        (
            "numeric-group",
            "FROM lineitem |> AGGREGATE COUNT(*) AS n GROUP BY l_quantity",
        ),
        ("count-value", "FROM lineitem |> AGGREGATE COUNT(l_quantity) AS n"),
    ]:
        call = queries.run("empty", sql)
        assert call.returncode == 1 and not call.stdout, (
            label,
            call.stdout,
            call.stderr,
        )
        queries.observations.append({"case": label, "outcome": "refused"})



def check_derived_queries(queries, work, encoder):
    # Unequal group sizes distinguish aggregation of groups (60) from
    # flattening the pipeline into an average of source rows (40).
    encoder.write(
        work / "repeated",
        [
            [raw(value), raw(1.0), raw(0.0), raw(0.0), key, 70, 0]
            for value, key in [(10.0, 65), (20.0, 65), (90.0, 66)]
        ],
    )
    queries.composed(
        "derived-average-of-group-sums",
        f"FROM ({GROUP_SUM_SQL}) AS totals |> AGGREGATE AVG(totals.total) AS mean",
        [[encoded(60.0)]],
        "repeated",
    )
    queries.composed(
        "nested-derived-computation",
        "FROM (FROM (FROM lineitem |> SELECT l_quantity+1 AS x) |> SELECT x*2 AS y) |> AGGREGATE SUM(y) AS total",
        [[encoded(246.0)]],
        "repeated",
    )
    queries.composed(
        "derived-ordered-group-prefix",
        "FROM (FROM lineitem |> AGGREGATE SUM(l_quantity) AS total GROUP AND ORDER BY l_returnflag |> LIMIT 1) |> SELECT total",
        [[encoded(30.0)]],
        "repeated",
    )
    queries.composed(
        "derived-duplicate-outputs",
        "FROM (FROM lineitem |> AGGREGATE COUNT(*) AS n |> SELECT n AS x,n AS x)",
        [[encoded(3), encoded(3)]],
        "repeated",
    )
    queries.composed(
        "derived-empty-global",
        "FROM (FROM lineitem |> AGGREGATE AVG(l_quantity) AS x) |> SELECT x",
        [[encoded(None)]],
        "empty",
    )
    queries.composed(
        "derived-undemanded-overflow",
        f"FROM ({GLOBAL_SQL}) |> SELECT a",
        [[encoded(MAX_DOUBLE)]],
        "mean-only",
    )
    call = queries.run("mean-only", f"FROM ({GLOBAL_SQL}) |> SELECT s")
    assert (
        call.returncode == 1
        and "overflow" in call.stderr.lower()
        and not parse_rows(call.stdout)
    )
    queries.observations.append({"case": "derived-demanded-overflow", "outcome": "refused"})
    for sql in [
        "FROM (FROM lineitem AS hidden) |> SELECT hidden.l_quantity",
        "FROM (FROM lineitem) |> SELECT lineitem.l_quantity",
        "FROM (FROM lineitem |> SELECT l_quantity AS x,l_quantity AS x) |> SELECT x",
        "FROM (SELECT l_quantity FROM lineitem)",
        "FROM (FROM lineitem;)",
    ]:
        call = queries.run("repeated", sql)
        assert call.returncode == 1 and not parse_rows(call.stdout), (
            sql,
            call.stdout,
            call.stderr,
        )
        queries.observations.append(
            {
                "case": "derived-invalid-scope-or-syntax",
                "sql": sql,
                "outcome": "refused",
            }
        )


def check_text_null_and_boolean_filters(queries):
    for symbol, compare in [
        ("<", operator.lt),
        ("<=", operator.le),
        ("=", operator.eq),
        ("!=", operator.ne),
        (">=", operator.ge),
        (">", operator.gt),
    ]:
        for literal in ["'A'", r"'\x41'", '"A"', "'é'", "''"]:
            target = "é" if literal == "'é'" else "" if literal == "''" else "A"
            expected = [
                [
                    encoded(
                        sum(
                            v
                            for v, k in [(10.0, "A"), (20.0, "A"), (90.0, "B")]
                            if compare(k, target)
                        )
                    )
                ]
            ]
            if not any(compare(k, target) for k in ["A", "A", "B"]):
                expected = [[encoded(None)]]
            queries.composed(
                "stored-text-filter",
                f"FROM lineitem |> WHERE l_returnflag {symbol} {literal} |> AGGREGATE SUM(l_quantity) AS total",
                expected,
                "repeated",
            )
    queries.composed(
        "grouped-text-filter",
        "FROM lineitem |> AGGREGATE SUM(l_quantity) AS total GROUP AND ORDER BY l_returnflag |> WHERE l_returnflag = 'B' |> SELECT total",
        [[encoded(90.0)]],
        "repeated",
    )
    queries.composed(
        "escaped-date-filter",
        r"FROM lineitem |> WHERE l_shipdate = DATE '\x31970-01-01' |> AGGREGATE COUNT(*) AS n",
        [[encoded(3)]],
        "repeated",
    )
    for column in ["l_quantity", "l_returnflag", "l_shipdate"]:
        for test, count in [("IS NULL", 0), ("IS NOT NULL", 3)]:
            queries.composed(
                "legacy-null-predicate",
                f"FROM lineitem |> WHERE {column} {test} |> AGGREGATE COUNT(*) AS n",
                [[encoded(count)]],
                "repeated",
            )
    for test, expected in [("IS NULL", [[encoded(None)]]), ("IS NOT NULL", [])]:
        queries.composed(
            "empty-sum-null-predicate",
            f"FROM lineitem |> AGGREGATE SUM(l_quantity) AS x |> WHERE x {test}",
            expected,
            "empty",
        )
    for test in ["IS NULL", "IS NOT NULL"]:
        call = queries.run(
            "repeated",
            f"FROM lineitem |> SELECT l_quantity*1e308 AS x |> WHERE x {test}",
        )
        assert (
            call.returncode == 1
            and "overflow" in call.stderr.lower()
            and not parse_rows(call.stdout)
        ), (call.stdout, call.stderr)
        queries.observations.append(
            {"case": "null-predicate-demanded-overflow", "outcome": "refused"}
        )
    for predicate, expected in [
        ("NOT l_quantity < 20", 110.0),
        ("l_quantity<15 OR l_quantity>50", 100.0),
        ("NOT (l_quantity<15 OR l_quantity>50)", 20.0),
        ("l_returnflag='A' OR l_quantity>50", 120.0),
        ("NOT l_returnflag='A'", 90.0),
        ("NOT l_shipdate < DATE '1970-01-01'", 120.0),
        ("l_quantity<15 OR l_quantity>50 AND l_returnflag='A'", 10.0),
        ("(l_quantity<15 OR l_quantity>50) AND l_returnflag='A'", 10.0),
    ]:
        # The retained repeated fixture has quantities 10, 20, 90 and flags A,A,B.
        queries.composed(
            "legacy-boolean-filter",
            f"FROM lineitem |> WHERE {predicate} |> AGGREGATE SUM(l_quantity) AS total",
            [[encoded(expected)]],
            "repeated",
        )
    for predicate in ["l_quantity<0 AND x>0", "l_quantity>=0 OR x>0"]:
        expected = (
            [] if "AND" in predicate else [[encoded(v)] for v in [10.0, 20.0, 90.0]]
        )
        queries.composed(
            "boolean-skipped-computation",
            f"FROM lineitem |> SELECT l_quantity,l_quantity*1e308 AS x |> WHERE {predicate} |> SELECT l_quantity",
            expected,
            "repeated",
        )


def check_repeated_aggregation(queries):
    queries.composed(
        "average-of-group-sums",
        GROUP_SUM_SQL + " |> AGGREGATE AVG(total) AS mean",
        [[encoded(60.0)]],
        "repeated",
    )
    queries.composed(
        "computed-group-input",
        GROUP_SUM_SQL + " |> SELECT total*2 AS total |> AGGREGATE AVG(total) AS mean",
        [[encoded(120.0)]],
        "repeated",
    )
    queries.composed(
        "three-aggregate-stages",
        GROUP_SUM_SQL
        + " |> AGGREGATE SUM(total) AS subtotal GROUP BY l_returnflag |> AGGREGATE AVG(subtotal) AS mean",
        [[encoded(60.0)]],
        "repeated",
    )
    queries.composed(
        "empty-group-input-to-global",
        GROUP_SUM_SQL + " |> WHERE total < 0 |> AGGREGATE SUM(total) AS total,COUNT(*) AS n",
        [[encoded(None), encoded(0)]],
        "repeated",
    )
    queries.composed(
        "count-empty-global-output",
        GLOBAL_SQL + " |> AGGREGATE COUNT(*) AS again",
        [[encoded(1)]],
        "empty",
    )
    queries.composed(
        "empty-global-input-to-global",
        "FROM lineitem |> AGGREGATE COUNT(*) AS n |> AGGREGATE SUM(n) AS n",
        [[encoded(0)]],
        "empty",
    )
    queries.composed(
        "transitive-undemanded-overflow",
        GLOBAL_SQL + " |> AGGREGATE AVG(a) AS mean",
        [[encoded(MAX_DOUBLE)]],
        "mean-only",
    )
    queries.composed(
        "filtered-intermediate-overflow",
        GLOBAL_SQL + " |> WHERE n < 0 |> AGGREGATE SUM(s) AS total",
        [[encoded(None)]],
        "mean-only",
    )
    for suffix in [
        " |> AGGREGATE SUM(s) AS total",
        " |> SELECT s*2 AS doubled |> AGGREGATE AVG(doubled) AS mean",
    ]:
        call = queries.run("mean-only", GLOBAL_SQL + suffix)
        assert (
            call.returncode == 1
            and "overflow" in call.stderr.lower()
            and not parse_rows(call.stdout)
        ), (suffix, call.stdout, call.stderr)
        queries.observations.append(
            {
                "case": "transitive-demanded-overflow",
                "suffix": suffix,
                "outcome": "refused",
            }
        )


def check_post_aggregate_demand(queries, rows):
    queries.composed(
        "drop-overflowing-sum", GLOBAL_SQL + " |> SELECT a", [[encoded(MAX_DOUBLE)]], "mean-only"
    )
    queries.composed(
        "swap-aggregate-aliases",
        GLOBAL_SQL + " |> SELECT a AS s,n AS a |> SELECT a,s,s",
        [[encoded(2), encoded(MAX_DOUBLE), encoded(MAX_DOUBLE)]],
        "mean-only",
    )
    queries.composed(
        "drop-overflowing-argument",
        f"FROM lineitem |> AGGREGATE SUM({OVERFLOW_EXPRESSION}) AS s,COUNT(*) AS n |> SELECT n",
        [[encoded(len(rows))]],
    )
    queries.composed(
        "drop-key-output-retains-order",
        GROUPED_SQL
        + " |> SELECT n,l_linestatus AS status,l_returnflag AS flag |> WHERE n > 7 |> SELECT flag,n",
        [
            [f"string:{key[0]:02x}", encoded(count)]
            for key, count in sorted(
                __import__("collections").Counter((r[4], r[5]) for r in rows).items()
            )
            if count > 7
        ],
    )
    for symbol, compare in [
        ("<", operator.lt),
        ("<=", operator.le),
        ("=", operator.eq),
        ("!=", operator.ne),
        (">=", operator.ge),
        (">", operator.gt),
    ]:
        queries.composed(
            "nullable-aggregate-" + symbol,
            GLOBAL_SQL + f" |> WHERE a {symbol} 0 |> SELECT n",
            [],
            "empty",
        )
        queries.composed(
            "count-comparison-" + symbol,
            GLOBAL_SQL + f" |> WHERE n {symbol} 2 |> SELECT n",
            [[encoded(2)]] if compare(2, 2) else [],
            "mean-only",
        )
    queries.composed(
        "exact-int64-sum",
        "FROM lineitem |> AGGREGATE SUM(9007199254740993) AS s,AVG(9007199254740993) AS a",
        [[encoded(18014398509481986), encoded(float(9007199254740993))]],
        "mean-only",
    )
    queries.composed(
        "empty-int64-sum",
        "FROM lineitem |> AGGREGATE SUM(1) AS s,AVG(1) AS a,COUNT(*) AS n",
        [[encoded(None), encoded(None), encoded(0)]],
        "empty",
    )
    queries.composed(
        "empty-count-zero",
        GLOBAL_SQL + " |> WHERE n = 0 |> SELECT a,n",
        [[encoded(None), encoded(0)]],
        "empty",
    )
    queries.composed(
        "integer-bound-folding",
        GLOBAL_SQL + " |> WHERE n = 9007199254740993-9007199254740991 |> SELECT n",
        [[encoded(2)]],
        "mean-only",
    )
    queries.composed(
        "count-int64-max",
        GLOBAL_SQL + " |> WHERE n < 9223372036854775807 |> SELECT n",
        [[encoded(2)]],
        "mean-only",
    )
    queries.composed(
        "count-double-coercion",
        GLOBAL_SQL + " |> WHERE n > 1.5 |> SELECT n",
        [[encoded(2)]],
        "mean-only",
    )
    queries.composed(
        "post-between-and",
        GLOBAL_SQL + " |> WHERE n BETWEEN 1 AND 3 AND a > 0 |> SELECT a,n",
        [[encoded(MAX_DOUBLE), encoded(2)]],
        "mean-only",
    )
    queries.composed("all-groups-filtered", GROUPED_SQL + " |> WHERE n < 0 |> SELECT qs", [])
    queries.composed(
        "overflowing-result-filtered-out",
        GLOBAL_SQL + " |> WHERE n < 0 |> SELECT s",
        [],
        "mean-only",
    )
    queries.composed(
        "overflowing-sum-filter-skipped",
        GLOBAL_SQL + " |> WHERE n < 0 |> WHERE s > 0 |> SELECT n",
        [],
        "mean-only",
    )
    queries.composed(
        "shared-average-filters-overflowing-sum",
        GLOBAL_SQL + " |> WHERE a < 0 |> SELECT s",
        [],
        "mean-only",
    )
    queries.composed("empty-group-projection", GROUPED_SQL + " |> SELECT n", [], "empty")
    queries.composed(
        "source-alias-replaced-by-aggregate",
        "FROM lineitem |> SELECT l_quantity AS n |> AGGREGATE AVG(n) AS n |> SELECT n",
        [[encoded(MAX_DOUBLE)]],
        "mean-only",
    )
    for suffix in [
        " |> SELECT s",
        " |> WHERE s > 0 |> SELECT n",
        " |> WHERE s > 0 AND n < 0 |> SELECT n",
    ]:
        call = queries.run("mean-only", GLOBAL_SQL + suffix)
        assert (
            call.returncode == 1
            and "overflow" in call.stderr.lower()
            and not parse_rows(call.stdout)
        ), (suffix, call.stdout, call.stderr)
        queries.observations.append(
            {
                "case": "post-demanded-sum-overflow",
                "suffix": suffix,
                "outcome": "refused",
            }
        )
    for sql in [
        GLOBAL_SQL + " |> SELECT l_quantity",
        GLOBAL_SQL + " |> SELECT n AS a,a AS a |> WHERE a > 0",
        GLOBAL_SQL + " |> SELECT a AS x,a AS x |> SELECT x",
        GLOBAL_SQL + " |> SELECT n |> WHERE a > 0",
        GLOBAL_SQL + " |> WHERE n < DATE '2000-01-01' |> SELECT a",
        "FROM lineitem |> AGGREGATE SUM(missing) AS s,COUNT(*) AS n |> SELECT n",
        "FROM lineitem |> AGGREGATE SUM(l_shipdate) AS s,COUNT(*) AS n |> SELECT n",
    ]:
        call = queries.run("empty", sql)
        assert (
            call.returncode == 1 and not call.stdout and "panicked" not in call.stderr
        ), (sql, call.stdout, call.stderr)
        queries.observations.append(
            {"case": "invalid-post-aggregate-binding", "sql": sql, "outcome": "refused"}
        )


def check_stored_corruption_and_bits(queries, work, encoder, rows):
    for column in [4, 5]:
        for bad in [0, 31, 124, 127, 255]:
            label = f"invalid-key-{column}-{bad}"
            row = rows[0].copy()
            row[column] = bad
            encoder.write(work / label, [row])
            key = ["l_returnflag", "l_linestatus"][column - 4]
            for shape in [
                f"SELECT {key}",
                f"AGGREGATE COUNT(*) AS n GROUP BY {key}",
                f"WHERE {key} IS NULL",
                f"WHERE {key} IS NOT NULL",
            ]:
                call = queries.run(label, "FROM lineitem |> " + shape)
                assert (
                    call.returncode == 1
                    and "stored key" in call.stderr
                    and not parse_rows(call.stdout)
                ), (label, shape, call.stdout, call.stderr)
                queries.observations.append(
                    {"case": label, "shape": shape, "outcome": "refused"}
                )
            queries.composed(
                label + "-undemanded",
                "FROM lineitem |> AGGREGATE COUNT(*) AS n",
                [[encoded(1)]],
                label,
            )
    encoder.write(work / "price-corrupt", [rows[0]])
    unit = work / "price-corrupt/units/0000000000000001.unit"
    data = bytearray(unit.read_bytes())
    data[28672 + 8] ^= 1
    unit.write_bytes(data)
    call = queries.run(
        "price-corrupt", "FROM lineitem |> AGGREGATE SUM(l_extendedprice) AS total"
    )
    assert (
        call.returncode == 1
        and "payload checksum" in call.stderr
        and not parse_rows(call.stdout)
    ), (call.stdout, call.stderr)
    queries.observations.append({"case": "demanded-price-checksum", "outcome": "refused"})
    for test in ["IS NULL", "IS NOT NULL"]:
        call = queries.run(
            "price-corrupt",
            f"FROM lineitem |> WHERE l_extendedprice {test} |> AGGREGATE COUNT(*) AS n",
        )
        assert (
            call.returncode == 1
            and "payload checksum" in call.stderr
            and not parse_rows(call.stdout)
        ), (call.stdout, call.stderr)
        queries.observations.append(
            {"case": "null-predicate-demanded-corruption", "outcome": "refused"}
        )
    queries.composed(
        "undemanded-price-checksum",
        "FROM lineitem |> WHERE l_quantity < 0 |> AGGREGATE SUM(l_extendedprice) AS total",
        [[encoded(None)]],
        "price-corrupt",
    )
    for label, number in [("subnormal", double(1)), ("negative-zero", -0.0)]:
        encoder.write(
            work / label, [[raw(number), raw(1.0), raw(0.0), raw(0.0), 65, 70, 0]]
        )
        queries.composed(
            label + "-projection",
            "FROM lineitem |> SELECT l_quantity",
            [[encoded(number)]],
            label,
        )
        queries.composed(
            label + "-sum",
            "FROM lineitem |> AGGREGATE SUM(l_quantity) AS total",
            [[encoded(number)]],
            label,
        )


def campaign(cli, work):
    work.mkdir()
    encoder = SnapshotEncoder()
    rows = source_rows()
    encoder.write(work / "data", rows)
    run_process(
        [str(cli), "create", "--database", str(work / "empty"), *QUERY_LIMITS],
        check=True,
        capture_output=True,
        cwd=ROOT,
        timeout=90,
    )
    queries = QueryChecks(cli, work)
    check_grouping(queries, rows)
    check_expressions_and_reference_queries(queries, rows)
    check_dates_and_predicates(queries, work, encoder, rows)
    check_numeric_failures(queries, work, encoder)
    check_derived_queries(queries, work, encoder)
    check_text_null_and_boolean_filters(queries)
    check_repeated_aggregation(queries)
    check_post_aggregate_demand(queries, rows)
    check_stored_corruption_and_bits(queries, work, encoder, rows)
    print(json.dumps(queries.observations, indent=2))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stock_cli", nargs="?", type=Path, help="identified stock CLI")
    parser.add_argument(
        "work_directory",
        nargs="?",
        type=Path,
        help="new case directory, required with STOCK_CLI",
    )
    options = parser.parse_args(argv)
    if not __debug__:
        parser.error("composition checks require Python assertions")
    if (options.stock_cli is None) != (options.work_directory is None):
        parser.error("supply both STOCK_CLI and a new WORK_DIRECTORY, or neither")

    def run_cases(binary, work):
        identity = hashlib.sha256(binary.read_bytes()).hexdigest()
        print(f"composition artifact sha256={identity}", file=sys.stderr)
        campaign(binary, work)
        assert (
            hashlib.sha256(binary.read_bytes()).hexdigest() == identity
        ), "CLI changed during campaign"

    if options.stock_cli is not None:
        binary = require_executable(options.stock_cli.resolve())
        run_cases(binary, options.work_directory.resolve())
    else:
        with tempfile.TemporaryDirectory(prefix="pipesql-composition-") as temporary:
            work = Path(temporary).resolve()
            release = build_cli(work)
            run_cases(release / "pipesql", work / "cases")


if __name__ == "__main__":
    main()
