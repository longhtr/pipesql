# Pipe SQL

A query begins with FROM. Each `|>` stage consumes the preceding table and produces
the next one. The final table is the result; SELECT is optional.

```sql
FROM sales
|> WHERE amount IS NOT NULL
|> AGGREGATE SUM(amount) AS total GROUP BY region
|> ORDER BY total DESC
```

Read this as “take sales, keep present amounts, total each region, then sort the
totals.” A filter after AGGREGATE would see regional totals instead of sales rows.
The [execution chapter](../execution.md) follows this distinction through the engine.

Only the forms below are supported. Traditional `SELECT ... FROM ...` query blocks,
WITH, correlated or scalar subqueries, EXISTS, UNNEST, table functions, user-defined
functions, and volatile functions are not accepted. Writes use the library or CLI.

## Values and calculations

Columns have one of four types: INT64, DOUBLE, STRING, or DATE. Each may permit
NULL. There are no Boolean-valued columns, decimal, bytes, timestamp, interval,
array, or struct values. Predicate truth is used for filtering rather than stored
as a column type.

- [Expressions](expressions.md) defines arithmetic, conditional results, text,
  dates, predicates, and demanded errors.
- [Aggregation](aggregation.md) reduces rows to grouped or global results.
- [Windows](windows.md) adds counts and running totals without reducing rows.

Supporting a type as a projected value does not imply that every function or key
position accepts it. For example, ordinary ORDER BY accepts DOUBLE, but window
keys currently do not.

## Sources and stages

| Form | Result |
| --- | --- |
| `FROM table [AS alias]` | The source's columns; its name is the default range name. |
| `FROM (pipe_query) [AS alias]` | An independent child query's output. |
| `AS alias` | Name the current row; replace previous range names. |
| `SELECT expression [AS alias], ...` | Choose or compute output columns. No star expansion. |
| `EXTEND expression [[AS] alias], ...` | Append columns to the input. |
| `SET name=expression, ...` | Replace named columns in their positions. |
| `DROP name, ...` | Remove matching ordinary columns; at least one must remain. |
| `RENAME old AS new, ...` | Change names while retaining values and positions. |
| `WHERE predicate` | Keep rows whose predicate is TRUE. |
| `AGGREGATE call AS name, ... [GROUP [AND ORDER] BY key, ...]` | Produce global or grouped calculations. |
| `[LEFT [OUTER]] JOIN input ON left.column = right.column` | Match declared-table inputs on one equality key. |
| `DISTINCT` | Remove duplicate complete rows. |
| `UNION ALL\|DISTINCT (pipe_query) [, ...]` | Combine positional inputs. |
| `EXCEPT ALL\|DISTINCT (pipe_query) [, ...]` | Subtract rows or occurrences. |
| `INTERSECT ALL\|DISTINCT (pipe_query) [, ...]` | Keep shared rows or occurrences. |
| `ORDER BY key [ASC\|DESC] [NULLS FIRST\|LAST], ...` | Sort declared-table rows. |
| `LIMIT count [OFFSET skip]` | Skip rows, then take a prefix. |

SELECT and EXTEND accept direct references, scalar expressions, and the supported
window calls. SET accepts the nonanalytic forms. Reducing aggregate calls belong
to AGGREGATE. Transformations and aggregation can repeat; every stage uses its
immediate input, not the original source table.

All declared sources in a prepared query use the same pinned catalog. The separate
legacy database exposes only `lineitem`: required DOUBLE `l_quantity`,
`l_extendedprice`, `l_discount`, and `l_tax`; one-byte STRING `l_returnflag` and
`l_linestatus`; and DATE `l_shipdate`. Legacy queries do not support JOIN, set
operations, DISTINCT, or standalone ORDER BY. Its input grammar is in
[the CLI reference](../cli.md#legacy-input).

## Names, positions, and identities

An identifier starts with an ASCII letter or underscore and continues with ASCII
letters, digits, or underscores. Names match without regard to ASCII case and
have at most 32 bytes. Quoted identifiers are unsupported. The pinned GoogleSQL
reserved words, including ALIGN, GRAPH_TABLE, MATCH_RECOGNIZE, and QUALIFY, cannot
be aliases. Nonreserved syntax words such as DATE and AGGREGATE can.

A name is how SQL addresses a column. An identity records which value it is. A
direct reference, even parenthesized, keeps its identity and implicit name.
Unary numeric `+` keeps identity but has no implicit name. Other computations
receive fresh identities and need aliases to acquire names.

Unnamed columns remain in the result; the API reports `name: None`. ORDER BY can
address them by position. Duplicate names are allowed in output but are errors
when a later reference cannot identify one column, even if the duplicates share
an identity.

Every SELECT or EXTEND list resolves against its input before introducing aliases.
A sibling expression cannot use a new alias from the same list. EXTEND preserves
all input columns, then appends its entries in written order. A duplicate alias
adds another column rather than replacing one.

SET, DROP, and RENAME take unqualified targets. Each list resolves against the
original input, so SET expressions see old values and RENAME can swap names.
Repeated targets are errors. SET and RENAME require an unambiguous target; DROP
removes all matching ordinary columns. SET assigns a fresh identity and may change
type or nullability. RENAME retains identity, type, nullability, and position.

## Qualified names

A range names a set of columns, as in `FROM sales AS s` followed by `s.amount`.
AS replaces prior ranges. WHERE and EXTEND preserve them; SELECT and AGGREGATE
remove them. An EXTEND alias is an ordinary column, not a new member of an earlier
range.

SET, DROP, and RENAME leave original range members intact:

```sql
FROM sales AS s
|> SET amount=amount+1
|> SELECT s.amount AS original, amount AS adjusted
```

Here `s.amount` is the old value and `amount` is its replacement. DROP removes
ordinary output, not a member retained by a range. SET and DROP remove a range
whose own name matches a target; RENAME rejects such a target. An unqualified
range name takes precedence over an ordinary column of the same name. Whole-row
scalar values are unsupported, so use a qualified member in that case.

Duplicate ranges are rejected. Duplicate member names are ambiguous even when
they share an identity. Qualified members work in projections, predicates,
numeric arguments, grouping, and ordering.

## Child queries

A parenthesized FROM-based query can supply FROM or JOIN. Its output names, types,
nullability, and identities become the input; its inner ranges do not. Without an
explicit alias, the child supplies no range name. It cannot refer to outer or
sibling inputs.

A child boundary does not force unused calculations. It clears the child's result
order, although ORDER BY and LIMIT inside the child still decide which rows it
selects. Use an outer ORDER BY to arrange those rows. Children share the complete
query's limits; each boundary consumes one stage and nesting uses at most 16
parser frames.

## Joins

JOIN accepts one equality between same-type columns on opposite inputs. Either
operand order is valid. An input may be a named table or a child query. Compound
conditions, coercion between key types, USING, RIGHT JOIN, and FULL JOIN are
unsupported.

Every matching pair appears. Two left rows and three right rows with the same key
produce six rows. NULL and NaN do not match; signed zeros do. A LEFT JOIN adds one
row with NULL right-hand columns for each unmatched left row. Left nullability is
unchanged; all right outputs are nullable.

A later WHERE filters the joined result. Rejecting every matched pair does not
turn its left row into an unmatched row. Joins establish no result order.

## DISTINCT and set operations

DISTINCT compares the complete visible row. NULLs match NULLs, all NaNs match,
and both zero signs match. Other values use typed equality; STRING uses exact
UTF-8 bytes. One input representation survives each class, with no promise about
which floating bits or row order are chosen.

Even a field dropped afterward participates in duplicate removal. Every unique
input identity receives one replacement identity, preserving output positions,
names, and shared-identity relationships. Visible ranges remap participating
members and lose hidden ones. DISTINCT supports all 64 visible columns, consumes
no aggregate-output budget, and returns no rows for empty input. DISTINCT ON,
a BY clause, and aggregate-call DISTINCT are unsupported.

Set operations match columns by position. Inputs need equal widths and identical
types: INT64 does not widen to DOUBLE, and untyped NULL does not acquire another
type. Names come from the leftmost input. Ranges are cleared. Each position gets
a separate new identity, even if the left input repeated one column there.

Each argument is an independent parenthesized pipe query. At least one is required;
a trailing comma is allowed. Arguments combine left to right. TABLE arguments,
hints, and matching by name are unsupported.

For a complete row with `m` left occurrences and `n` right occurrences:

| Operation | Output copies | Output column permits NULL if |
| --- | --- | --- |
| UNION ALL | `m + n` | Either input permits it. |
| UNION DISTINCT | One if either count is positive. | Either input permits it. |
| EXCEPT ALL | `max(m - n, 0)` | The left input permits it. |
| EXCEPT DISTINCT | One if `m > 0` and `n = 0`. | The left input permits it. |
| INTERSECT ALL | `min(m, n)` | Both inputs permit it. |
| INTERSECT DISTINCT | One if both counts are positive. | Both inputs permit it. |

Comparison uses DISTINCT equality. EXCEPT and INTERSECT retain representatives
from the left; UNION may choose either input. Stored value bits survive, but the
chosen equivalent representation is unspecified. No set operation establishes
result order. Branch ORDER BY and LIMIT still determine branch contents.

UNION ALL can omit unused output positions and stop before later branches when
a downstream LIMIT finishes. Every branch still binds and receives admission.
The other set forms need complete-row comparison: all comparison fields and all
inputs are consumed before output, even if an input is empty. A positive LIMIT
cannot hide a demanded error in a later input. LIMIT zero with zero offset may
leave the whole producer unvisited after admission.

Each argument adds a binary set stage plus its input stages. A UNION DISTINCT
list adds one further DISTINCT stage; nested lists have their own removal stage.

## Order

ORDER BY keys are visible columns or one-based column positions. ASC defaults to
NULLS FIRST; DESC defaults to NULLS LAST. Expressions and collations are unsupported.
Repeated keys are legal. Rows tied on every key have no promised relative order.

INT64 and DATE use numeric order. STRING uses UTF-8 byte order. DOUBLE places all
NaNs below negative infinity and treats signed zeros as equal; sorting preserves
the original value bits.

| Stage | Effect on established order |
| --- | --- |
| Nonanalytic SELECT, EXTEND, SET, DROP, RENAME, WHERE, AS | Preserve it, even when a projection hides a key. |
| LIMIT | Preserve the selected subsequence. |
| ORDER BY | Replace it with the new key list. |
| GROUP AND ORDER BY | Establish ascending group-key order. |
| Ordinary AGGREGATE, JOIN, DISTINCT, set operation, child-query boundary, window | Clear it. |

A hidden key remains ordering information, not an output column that can be
referenced by name. File layout, hash iteration, and sorter stability do not
establish SQL order. Add ORDER BY when the sequence matters.

## LIMIT

Count and offset are nonnegative INT64 constant expressions; offset defaults to
zero. Each can reach INT64_MAX even when their sum would exceed it. NULL, DOUBLE,
parameters, column references, and LIMIT ALL are unsupported. Both constants are
checked during preparation, including when input will be empty.

LIMIT skips up to the offset and returns at most the count. A later WHERE sees
only that selection and cannot be moved before LIMIT without checking semantics.
Skipped rows, prefetch, and blocking work may still fail. A completed LIMIT zero
returns no rows; only zero count with zero offset avoids requesting upstream
work after admission.

## Query limits

| Item | Limit |
| --- | ---: |
| UTF-8 source bytes | 4,096 |
| Tokens | 160 |
| Identifier / literal payload bytes | 32 |
| Normalized stages | 16 |
| Visible columns / source-column pool | 64 / 64 |
| Aggregate outputs across all stages, including keys | 10 |
| Numeric operations / numeric parser-stack slots | 32 / 32 |
| Child-query parser frames | 16 |

A comparison or NULL test consumes one predicate stage. IN consumes one per
candidate; BETWEEN consumes two. Parentheses and Boolean NOT consume no additional
stages. Numeric calls consume operation slots as described in
[expressions](expressions.md). Text, syntax, and binding limits fail before a
partial prepared plan can escape.

## Semantic source

PipeSQL's subset is informed by GoogleSQL `2026.7.2`, commit
`0e7d7073ed0360be587a5efa0fa78abeee00f17b`: its
[pipe syntax](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/pipe-syntax.md)
and [query semantics](https://github.com/google/googlesql/blob/0e7d7073ed0360be587a5efa0fa78abeee00f17b/docs/query-syntax.md).
PipeSQL has its own parser and engine. Its documented demand, ordering, and raw-bit
rules remain local guarantees where upstream allows other evaluation schedules
or representations. Neither these forms nor the retained Q1/Q6 examples establish
complete GoogleSQL or TPC-H compatibility.
