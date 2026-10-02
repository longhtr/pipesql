# Window functions

A window computes across related rows while retaining one output row for every
input row. Use it to attach a count or running total without collapsing the report
into groups. Empty input stays empty.

PipeSQL supports COUNT and ordered INT64 SUM as whole SELECT or EXTEND expressions.
To calculate with their results, name them and use a later stage. SET, WHERE,
AGGREGATE, ORDER BY, and LIMIT do not accept window calls. Named windows, explicit
frame clauses, ranking functions, navigation functions, analytic DISTINCT, and
nested windows are unsupported.

## Count the partition

```sql
FROM sales
|> EXTEND COUNT(*) OVER (PARTITION BY region) AS regional_rows
|> ORDER BY region, amount
```

Every row receives the count of rows with the same region, including rows with
NULL amounts. A filter before the window changes the counted input. A filter
afterward changes the visible output but does not recompute the count.

`COUNT(*) OVER ()` counts the entire input and works for declared tables and the
legacy path. PARTITION BY accepts one through eight visible INT64, STRING, or
DATE columns. NULL keys share a partition; STRING equality uses exact UTF-8.
DOUBLE keys and key expressions are unsupported. Project a computed key earlier.

COUNT returns required INT64. Its frame covers the full partition from unbounded
preceding to unbounded following. Other COUNT arguments and ordered COUNT are
unsupported. An unused count does not demand its partition keys.

## Sum through the current peers

```sql
FROM sales
|> EXTEND SUM(amount) OVER (PARTITION BY region ORDER BY day) AS running
|> ORDER BY region, day, amount
```

Within each region, this sums amounts through the current day. If two rows have
the same day, both include both amounts. For example:

| day | amount | running |
| --- | ---: | ---: |
| 2026-01-01 | 10 | 30 |
| 2026-01-01 | 20 | 30 |
| 2026-01-02 | NULL | 30 |
| 2026-01-03 | 5 | 35 |

The first two rows are *peers*: every ORDER BY key compares equal. The implicit
frame is RANGE from unbounded preceding through the entire current peer group.
Row-by-row accumulation would produce 10 then 30 and would answer a different
question.

SUM requires a visible INT64 argument. It accepts zero through eight partition
keys and one through eight order keys, all visible INT64, STRING, or DATE columns.
Directions and NULL placement follow ordinary ORDER BY, including its defaults.
Partition equality follows COUNT above. Key expressions, DOUBLE input, unordered
SUM, and explicit frames are unsupported.

The result is nullable INT64. NULL arguments are ignored; a frame with no present
arguments returns NULL. Intermediate sums are exact and may exceed INT64. Only a
demanded final frame outside the range raises `ArithmeticOverflow`, at that SUM
call's span. A later contribution can bring a subsequent frame back into range,
but it cannot rescue an earlier frame that was demanded and overflowed.

Filters or CASE can leave a frame result unused, and LIMIT can stop before later
frames. A wholly unused SUM does not demand its argument or keys. Required input
computations still obey earlier operator boundaries.

## Compose windows deliberately

Calls in one SELECT or EXTEND must describe the same window calculation: function,
argument where applicable, and ordered partition and sort lists. Repeated keys
remain part of those lists. Repeated calls receive separate result identities
and spans. Use separate stages for different windows.

Every expression sees the projection's original input; a window cannot use an
alias introduced beside it. Ordinary expressions may appear alongside a window,
but arithmetic cannot wrap the call in that same expression.

A window clears the input's result order. Its ORDER BY chooses the calculation
sequence, not presentation order; add a final ordinary ORDER BY when needed.
The producer consumes its required input before emitting rows, and ordered SUM
sorts it first. A later LIMIT therefore cannot avoid earlier input or key errors.
Ordinary expressions beside the window evaluate during emission and can remain
unused for later rows. LIMIT zero with zero offset may leave the window unvisited
after admission.
