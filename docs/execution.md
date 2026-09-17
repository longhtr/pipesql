# How a report executes

A pipe query describes successive tables. The engine turns that description into
operators that exchange bounded batches. Some can produce rows while reading;
others must finish collecting input before their first answer exists.

Use this report to follow the difference:

```sql
FROM sales
|> WHERE amount IS NOT NULL
|> EXTEND amount + 1 AS adjusted
|> AGGREGATE SUM(adjusted) AS total GROUP BY region
|> ORDER BY total DESC
```

For north amounts 10, 5, and NULL and south amount 20, the answer is south/21,
then north/17. The filter removes the missing amount. EXTEND adds one to each
remaining sale, not one to each regional sum.

## Preparation establishes meaning

`prepare` pins a committed catalog and parses the query. The binder resolves
`amount` and `region` against that catalog, checks their types, and assigns an
identity to `adjusted`. After AGGREGATE, only `region` and `total` remain visible.
ORDER BY therefore sorts regional totals, not individual sales.

Preparation also checks the whole query's names and limits. An unused expression
with an unknown column still fails. Numeric constants in WHERE or LIMIT evaluate
during preparation; arithmetic in an ordinary projection runs only if demanded
at execution. The prepared query owns the facts it needs, so the caller can release
the original SQL text. Error spans retain byte offsets, not a copy of that text.

## Demand determines what to compute

Working backward from output, planning finds the columns each operator needs.
For this report, the scan needs `region` and `amount`. It does not need any other
stored columns. The projection needs to calculate `adjusted` only for rows kept
by the filter.

Demand is more than the final SELECT list. A hidden sort key still decides order;
a grouping key still decides membership; DISTINCT still compares the complete
row. Removing these values merely because they are absent from output would
change the answer or conceal a required failure.

This also explains why a filter's position matters:

```sql
FROM sales
|> WHERE amount != 5
|> SELECT 100 / (amount - 5) AS ratio
```

The division is not needed for amount 5, which the filter rejects. Put a stage
that requires the quotient before that filter, and the division can fail first.
[Expression evaluation](sql/expressions.md#evaluation) defines the exact boundaries;
execution must preserve those rules when combining projections and filters.

## Admission establishes the workspace

`execute` constructs physical operators and reserves their required buffers before
opening input streams. Logical column identities become validated batch positions.
Each source occurrence has its own scan cursor, including two reads of the same
table in a self-join. Sharing that cursor would let one consumer take the other's
rows.

Mandatory downstream workspace is protected before an earlier aggregate receives
optional memory. This prevents one operator from consuming the resources another
needs to finish. If admission fails, already-created owners are released; a partial
execution object does not escape to the caller.

## The caller drives a state machine

`QueryResult::step` advances bounded work. Operators ask the runtime for input or
replay; they do not recursively call upstream operators. The runtime walks its
operator array and keeps an input batch alive until its consumer asks for another.
Only the final operator's batches become public result batches.

The four step outcomes are:

| Outcome | Meaning |
| --- | --- |
| `Rows(batch)` | Consume these borrowed rows before stepping again. |
| `Progress` | Work advanced without an output batch; continue. |
| `Finished` | The complete query succeeded. |
| `Failed(error)` | Execution stopped; any earlier rows are an incomplete answer. |

The cursor releases runtime storage on completion, failure, or drop. Failure is
terminal: stepping again does not retry the operation. The separately owned
prepared query can be executed again against the same snapshot.

## Grouping can change strategy without changing arithmetic

General grouping first tries to accumulate groups in a hash table. If that table
cannot hold the input, the engine discards it and replays the input through a
sorter. Sorting places equal keys together and preserves their original row
sequence. The reducer can then accumulate one group at a time.

Why replay instead of merging partial sums? Floating-point addition depends on
the order of operations. Combining partial aggregates could produce a different
calculation from folding the original arguments. Replay preserves each group's
argument sequence across the memory and disk paths.

Before returning aggregate rows, the engine checks demanded argument failures
and the final values of retained groups. A group rejected by a later filter need
not finalize an unused SUM. This does not suppress errors computing the SUM's
required input arguments. On the disk path, later I/O or corrupt result records
can still fail during emission; callers must continue to `Finished`.

Global aggregation and the legacy loader's small key domain use a denser strategy.
The choice and its capacities live in
[aggregation.rs](../src/execution/aggregation.rs); the general fallback is in
[grouping.rs](../src/execution/aggregation/grouping.rs).

## Sorting supplies an order, not a meaning

The external sorter fills a bounded buffer, sorts it, and writes a run. It merges
runs in passes, reading one scratch file while writing the other. Run headers
travel with the runs, so their number does not require an ever-growing memory
index. Readers validate record shape, extent, counts, checksums, and merge order.

Different consumers interpret sorted rows differently:

- ORDER BY returns them in key order.
- DISTINCT keeps one representative of each equal complete row.
- A join matches equal keys and revisits the right-hand group for every matching
  left row. Two left rows and three right rows produce six pairs without keeping
  the whole group in memory.
- Set operations compare row occurrences according to ALL or DISTINCT.
- Running SUM completes a peer group before assigning its total to every peer.

Sorting NULLs together is useful for all of these algorithms. It does not make
NULLs match in a join. Their independent SQL rules are as important as the shared
I/O mechanism.

## LIMIT does not bound the scan

An ORDER BY followed by LIMIT must first find which rows belong at the front.
A count window must first count its input. A join must construct its sorted inputs.
These operations may read all input even when the caller asks for one result row.

LIMIT can stop a streaming producer after enough rows, but skipped rows, prefetch,
and required earlier work can fail. `LIMIT 0` with zero offset requests no upstream
rows after admission; it does not bypass parsing, binding, or admission itself.

Follow [planning](../src/execution/planning.rs),
[runtime](../src/execution/runtime.rs), and
[blocking algorithms](../src/execution/blocking.rs) for the implementation. Their
local comments describe buffer and state ownership; the [SQL reference](sql/README.md)
defines the behavior those choices must preserve.
