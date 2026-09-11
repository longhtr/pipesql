#!/usr/bin/env python3
"""Independent rational rounding oracle; no imports from the Rust implementation."""
from fractions import Fraction
import math
from pathlib import Path
import random
import struct
import sys

ROOT = Path(__file__).resolve().parent.parent
MAX = float.fromhex("0x1.fffffffffffffp+1023")


def bits(value):
    return struct.pack(">d", value).hex()


def value(raw):
    return struct.unpack(">d", raw.to_bytes(8, "big"))[0]


def power(exponent):
    return Fraction(1 << exponent) if exponent >= 0 else Fraction(1, 1 << -exponent)


def rounded(number):
    if number == 0:
        return number
    magnitude = abs(number)
    exponent = magnitude.numerator.bit_length() - magnitude.denominator.bit_length()
    if magnitude < power(exponent):
        exponent -= 1
    unit = power(max(exponent - 52, -1074))
    scaled = magnitude / unit
    quotient, remainder = divmod(scaled.numerator, scaled.denominator)
    if 2 * remainder > scaled.denominator or (
        2 * remainder == scaled.denominator and quotient % 2
    ):
        quotient += 1
    return quotient * unit * (-1 if number < 0 else 1)


def expected_sum(values):
    if any(math.isnan(x) for x in values):
        return "nan"
    if any(math.isinf(x) for x in values):
        positive, negative = math.inf in values, -math.inf in values
        return (
            "nan"
            if positive and negative
            else bits(math.inf if positive else -math.inf)
        )
    total = Fraction(values[0])
    negative_zero = bits(values[0]) == "8000000000000000"
    for x in values[1:]:
        negative_zero = (
            total == 0 and x == 0 and negative_zero and bits(x).startswith("8")
        )
        total = rounded(total + Fraction(x))
    if abs(total) > Fraction(MAX):
        return "overflow"
    return bits(-0.0 if total == 0 and negative_zero else float(total))


def expected_average(values):
    if any(not math.isfinite(x) for x in values):
        return expected_sum(values) + "," + expected_sum(values)
    total = values[0]
    for x in values[1:]:
        total += x
        if not math.isfinite(total):
            break
    if math.isfinite(total):
        result = bits(total / len(values))
        return result + "," + result
    exact = sum(map(Fraction, values), Fraction()) / len(values)
    # A conservative forward-error envelope, plus the convex-hull obligation.
    error = (
        8
        * len(values)
        * Fraction(1, 1 << 52)
        * max(map(lambda x: abs(Fraction(x)), values))
    )
    lower = max(Fraction(min(values)), exact - error)
    upper = min(Fraction(max(values)), exact + error)
    lo, hi = float(lower), float(upper)
    if Fraction(lo) > lower:
        lo = math.nextafter(lo, -math.inf)
    if Fraction(hi) < upper:
        hi = math.nextafter(hi, math.inf)
    return bits(lo) + "," + bits(hi)


def vectors():
    special = [
        0.0,
        -0.0,
        value(1),
        -value(1),
        1.0,
        -1.0,
        MAX,
        -MAX,
        math.inf,
        -math.inf,
        math.nan,
    ]
    sequences = [[a, b, c] for a in [MAX, -MAX] for b in [MAX, -MAX] for c in special]
    sequences += [[x] for x in special] + [[x, x] for x in special]
    sequences += [[MAX, MAX, -MAX, -MAX, x] for x in special]
    sequences += [[MAX, MAX, -MAX, -MAX, MAX, MAX, x] for x in special]
    rng = random.Random(20260908)
    for index in range(512):
        sequence = [value(rng.getrandbits(64)) for _ in range(1 + index % 32)]
        if index % 2 == 0:
            sequence[:0] = [MAX, MAX]
        if index % 4 == 0:
            sequence += [-MAX, -MAX]
        sequences.append(sequence)
    return "".join(
        ";".join([",".join(map(bits, xs)), expected_sum(xs), expected_average(xs)])
        + "\n"
        for xs in sequences
    )


if __name__ == "__main__":
    path = ROOT / "tests/fixtures/aggregate-semantics/rounding.txt"
    if not __debug__ or sys.argv[1:] not in ([], ["--check"]):
        raise SystemExit(
            "usage: aggregate-rounding-vectors.py [--check]; assertions required"
        )
    expected = vectors()
    if sys.argv[1:] == ["--check"]:
        if path.read_text() != expected:
            raise SystemExit("aggregate rounding vectors differ from their generator")
        print("aggregate rounding vectors reproduce byte-for-byte")
    else:
        path.write_text(expected)
