#!/usr/bin/env python3
"""Negative controls for the independent Q1 evidence reader (stdlib only)."""

import csv
import io
import runpy
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
CHECK = runpy.run_path(str(ROOT / "tools/oracles/q1_compare.py"))
ONE = "3ff0000000000000"


def oracle(keys=("R", "F")):
    fields = list(keys)
    for _ in range(7):
        fields.extend(["1", "bits=" + ONE])
    fields.extend(["1", "count_bits=0000000000000001"])
    output = io.StringIO()
    csv.writer(output, lineterminator="").writerow(fields)
    return [
        "rows=1",
        "demanded=1",
        "groups=1",
        "fingerprint=0000000000000000",
        output.getvalue(),
    ]


def production(keys=("R", "F")):
    types = ["string:required"] * 2 + ["double:nullable"] * 7 + ["int64:required"]
    schema = "|".join(f"c{index}:{kind}" for index, kind in enumerate(types))
    values = ["string:" + key.encode().hex() for key in keys]
    values += ["double:1:" + ONE] * 7 + ["int64:1"]
    return [
        "status=queried",
        "column_count=10",
        "columns=" + schema,
        "row=" + "|".join(values),
        "row_count=1",
    ]


class ComparisonTests(unittest.TestCase):
    def test_comma_quote_and_space_keys_survive_csv(self):
        for keys in [("R", "F"), (",", '"'), (" ", "~")]:
            self.assertEqual(
                CHECK["parse_oracle"](oracle(keys)),
                CHECK["parse_production"](production(keys)),
            )

    def test_missing_and_malformed_oracle_rows_are_not_skipped(self):
        for value in [
            oracle()[:-1],
            oracle()[:-1] + ["malformed row"],
            oracle()[:-1] + [oracle()[-1] + ",extra"],
        ]:
            with self.assertRaises((AssertionError, csv.Error)):
                CHECK["parse_oracle"](value)

    def test_oracle_counts_and_duplicate_headers_are_checked(self):
        for old, new in [
            ("groups=1", "groups=0"),
            ("demanded=1", "demanded=0"),
            ("rows=1", "rows=0"),
            ("count_bits=0000000000000001", "count_bits=0000000000000002"),
        ]:
            with self.assertRaises(AssertionError):
                CHECK["parse_oracle"]([line.replace(old, new) for line in oracle()])
        with self.assertRaises(AssertionError):
            CHECK["parse_oracle"](oracle() + ["groups=1"])

    def test_typed_output_schema_and_values_are_checked(self):
        for old, new in [
            ("column_count=10", "column_count=9"),
            ("string:required", "string:nullable"),
            ("double:1:", "string:1:"),
            (ONE, "1"),
            ("int64:1", "int64:0"),
            ("string:52", "string:5253"),
            ("row_count=1", "row_count=0"),
        ]:
            with self.assertRaises(AssertionError):
                CHECK["parse_production"](
                    [line.replace(old, new) for line in production()]
                )

    def test_double_displays_match_bits_in_both_sources(self):
        validate = CHECK["validate_double_display"]
        cases = [
            ("0", 0),
            ("-0", 0x8000000000000000),
            ("1", int(ONE, 16)),
            ("5e-324", 1),
            ("1.7976931348623157e308", 0x7FEFFFFFFFFFFFFF),
            ("inf", 0x7FF0000000000000),
            ("-inf", 0xFFF0000000000000),
            ("NaN", 0x7FF8000000000001),
            ("NaN", 0xFFF0000000000001),
        ]
        for text, encoded in cases:
            validate(text, encoded)
            for wrong in ["incorrect", "", " 1", "+1", "0_1"]:
                with self.subTest(text=text, wrong=wrong), self.assertRaises(
                    AssertionError
                ):
                    validate(wrong, encoded)
        for text, encoded in [
            ("0", 0x8000000000000000),
            ("-0", 0),
            ("inf", 0xFFF0000000000000),
            ("NaN", int(ONE, 16)),
            ("1e999", 0x7FF0000000000000),
            ("nan", 0x7FF8000000000000),
        ]:
            with self.assertRaises(AssertionError):
                validate(text, encoded)
        for display in ["incorrect", "2", "NaN", "-0"]:
            with self.assertRaises(AssertionError):
                CHECK["parse_production"](
                    [
                        line.replace("double:1:", f"double:{display}:")
                        for line in production()
                    ]
                )
            lines = oracle()
            fields = next(csv.reader([lines[-1]]))
            fields[2] = display
            output = io.StringIO()
            csv.writer(output, lineterminator="").writerow(fields)
            lines[-1] = output.getvalue()
            with self.assertRaises(AssertionError):
                CHECK["parse_oracle"](lines)

    def test_order_and_duplicate_groups_are_checked(self):
        first = CHECK["parse_production"](production(("A", "F")))[0]
        second = CHECK["parse_production"](production(("R", "F")))[0]
        CHECK["validate_rows"]([first, second])
        for rows in [[second, first], [first, first]]:
            with self.assertRaises(AssertionError):
                CHECK["validate_rows"](rows)

    def test_read_is_bounded_before_allocation(self):
        limit = CHECK["MAX_BYTES"]
        with patch.object(
            Path, "open", return_value=io.BytesIO(b"x" * (limit + 2))
        ) as opened:
            with self.assertRaises(AssertionError):
                CHECK["read_bounded"](Path("unused"))
            opened.assert_called_once_with("rb")

        # A stream rejects the unbounded read that the old implementation used.
        class BoundedStream(io.BytesIO):
            def read(self, size=-1):
                self.assert_size(size)
                return super().read(size)

            @staticmethod
            def assert_size(size):
                assert size == limit + 1

        with patch.object(Path, "open", return_value=BoundedStream(b"ok\n")):
            self.assertEqual(CHECK["read_bounded"](Path("unused")), ["ok"])


if __name__ == "__main__":
    if not __debug__:
        raise SystemExit("comparison tests require Python assertions")
    unittest.main()
