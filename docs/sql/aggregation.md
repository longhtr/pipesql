# Aggregation

AGGREGATE replaces many input rows with one result per group. Without GROUP BY,
it produces one result for the entire input. Group keys come first in the output,
followed by aggregate entries in written order. Every entry needs an alias.

```sql
FROM sales
|> AGGREGATE SUM(amount) AS total,
             COUNT(*) AS n,
             COUNT(amount) AS present
   GROUP AND ORDER BY region
```

For north amounts 10, 5, and NULL, this gives total 15, n 3, and present 2.
The missing amount changes COUNT(*) but not COUNT(amount) or SUM. Earlier columns
that are neither keys nor aggregate outputs disappear from the next stage.

## Choose the calculation

| Function | Accepted argument | Result type |
| --- | --- | --- |
| SUM | Numeric expression. | Argument type, nullable. |
| AVG | Numeric expression. | DOUBLE, nullable. |
| COUNT(*) | Every input row. | INT64, required. |
| COUNT(expression) | Numeric expression or direct STRING/DATE column. | INT64, required. |
| MIN / MAX | Numeric expression or direct STRING/DATE column. | Argument type, nullable. |

Arguments ignore NULL. COUNT(expression) counts non-NULL values, including NaN and
empty text. An empty global input returns one row with zero counts and NULL for
the other functions. Empty grouped input returns no rows. A nonempty group with
only NULL arguments has zero COUNT(expression) and NULL for SUM, AVG, MIN, and MAX.

Numeric arguments use ordinary checked expression evaluation before accumulation.
COUNT does not accumulate a sum, but `COUNT(amount * amount)` can still fail if a
demanded multiplication overflows. Aggregate DISTINCT and unlisted functions are
unsupported.

## Define groups

Declared tables can group by visible INT64, DOUBLE, STRING, or DATE columns,
including nullable columns. Compute and name an expression in an earlier stage
before using it as a key. Keys must have distinct identities: listing two aliases
of the same value is rejected.

All aggregate stages share ten output identities, counting their keys and entries.
A single stage can therefore use at most nine keys. Later stages consume the
remaining allowance. A projection may repeat those results within the ordinary
64-column limit.

Grouping equality treats NULLs together, all NaNs together, and both zero signs
together. Other values use typed equality; STRING compares exact UTF-8. The engine
retains the first scanned representation of an equivalent key, but scan order is
not a SQL ordering guarantee.

GROUP BY promises no output order. GROUP AND ORDER BY sorts keys lexicographically
ascending: NULL first, then NaN before other DOUBLE values, numeric order for
numbers and dates, and UTF-8 byte order for text. Subsequent ordinary filters and
projections preserve that order, even when they hide a key.

Legacy `lineitem` supports the same aggregate signatures but groups by at most two
distinct, nonnullable source STRING keys. Its small key domain allows a different
execution strategy; [execution](../execution.md) explains that distinction.

## Integer accumulation

SUM(INT64) keeps an exact intermediate sum and checks the final answer against
INT64. Intermediate totals may exceed that range and later return to it. AVG(INT64)
returns DOUBLE without requiring its sum to fit INT64.

This is different from evaluating a chain of scalar additions, each of which must
fit its type. The wider aggregate state does not rescue an overflowing scalar
argument calculated before it reaches SUM.

## Floating accumulation

DOUBLE SUM and AVG must account for all demanded inputs before deciding a finite
range failure. A temporary sum outside DOUBLE range cannot hide a later NaN,
infinity, or opposite-signed contribution. Let `M` be the largest finite DOUBLE:

| Input | Required outcome |
| --- | --- |
| `M, M` | SUM overflows; AVG is `M`. |
| `M, M, -M` | SUM is `M`. |
| `M, M, NaN` | SUM and AVG are NaN. |

NaN input produces NaN. Infinity inputs can produce infinity or NaN according to
their signs. SUM preserves the first value's zero sign according to its fold.
These rules constrain exceptional outcomes, not arbitrary rounding. Memory and
disk strategies preserve the argument sequence within each group, but floating
results have no general cross-engine bit-identity guarantee.

An AVG-only result must not acquire the overflow requirement of an unused SUM.
A later NaN also cannot rescue a failure computing an earlier scalar argument.

## Extrema

MIN and MAX preserve the input type. Numbers and DATE use numeric order; STRING
uses Unicode code-point order without collation or normalization. Empty text is
a value.

DOUBLE extrema return NaN if any demanded argument is NaN, preserving the first
NaN payload in the fold sequence. If both zero signs occur, MIN selects negative
zero and MAX positive zero. Infinities follow numeric order. A prior NaN does not
skip later demanded argument errors.

## Required values and failures

A later output or predicate determines whether an aggregate value is needed.
Unused arguments are not evaluated, but grouping keys and required order remain
necessary. Filters after aggregation run in written order; a rejected group need
not evaluate a later filter or finalize an unused SUM.

Demanded argument errors and retained final numeric overflow are checked before
aggregate rows are returned. Errors reading spilled results may still happen
during emission, so this is not permission to stop checking the result cursor.
[Error spans](expressions.md#errors-and-spans) identify the aggregate call, including
when its value feeds a later projection.
