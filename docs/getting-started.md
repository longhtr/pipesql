# Create and query a declared table

This walkthrough creates a table, appends four rows, closes and reopens the
database, and computes a total for each region. It uses the ordinary Rust
library on macOS or Linux. Use the pinned toolchain and [build
prerequisites](testing.md#prerequisites); the [platform
matrix](testing.md#platform-status) distinguishes exercised behavior from the
remaining qualification requirements.

The complete program lives in [examples/declared.rs](../examples/declared.rs).
Read it alongside this walkthrough: it declares `sales(region, amount)`, writes
a single typed batch, commits, closes, and reopens before preparing the query.

## Create and query the table

From the repository root, create a temporary parent directory and run the
example. Keep this shell open so the directory variable remains available:

```sh
pipesql_example_dir=$(mktemp -d)
cargo run --release --offline --locked --example declared -- "$pipesql_example_dir/sales"
```

Successful completion prints:

```text
north total=15 rows=3 present=2
south total=20 rows=1 present=1
```

## Follow the operation

The region bitmap marks all four values as present. The amount bitmap marks
only the first three: the last north row has NULL amount, regardless of its
payload. `COUNT(*)` counts all rows; `COUNT(amount)` counts present amounts.
`SUM(amount)` ignores NULL and gives north a total of 15.

A successful
`write` creates private data; `commit` publishes it. Reopen checks that the
table and rows survived closing the first handle. `GROUP AND ORDER BY`
establishes the displayed order. `Progress` means more work remains, and output
is complete only after `Finished` and successful process exit.

The example leaves the database in that directory. Running the command again
with the same path fails because creation requires a new database. Existing
databases use `Database::open`. The memory and temporary limits account for
engine-owned resources, not the caller or whole-process RSS. For typed outcomes,
abort/reopen requirements and retained query lifetimes, see
[interfaces.md](interfaces.md). For supported syntax and bounds, see
[language.md](language.md#current-public-query-manifest).

The CLI can open and query this database. Table declaration and typed append
currently require the library; the CLI's `create` and `load` commands use the
legacy `lineitem` schema.

## Finish and clean up

If the program reports an error, do not treat any printed rows as a complete
result. Dropping a failed query releases its buffers. An unfinished append can
leave recovery work for a later open; drop alone does not roll it back. The
[transaction outcomes](transactions.md#outcomes) explain when to reopen and
resolve a retained token.

When finished, remove only the temporary example directory created above:

```sh
rm -r -- "$pipesql_example_dir"
```

Continue with [the preparation
walkthrough](frontend.md#trace-a-query-through-preparation) to follow a query
from names to typed column identities. Keep the example source open when tracing
the append, commit, and query lifetimes.

## Observe grouping with less memory

[The grouping example](../examples/grouping.rs) creates 8,192 sales rows across
4,096 numbered regions. Each region has two amounts, 1 and 3. Its query counts
and sums each region, finds its smallest and largest amounts, then emits regions
in ascending order:

```sql
FROM sales
|> AGGREGATE COUNT(*) AS n, SUM(amount) AS total,
             MIN(amount) AS smallest, MAX(amount) AS largest
   GROUP AND ORDER BY region
```

SUM, MIN, and MAX share evaluation of `amount` but retain different state: a sum
and two extrema. Each new row updates those states without retaining the whole
region's input. The final values are 4, 1, and 3 for every region.

Run it twice with fresh database paths. Both runs generate identical rows. They
use a separate setup budget before reopening with the specified query budget:

```sh
pipesql_grouping_dir=$(mktemp -d)
cargo run --release --offline --locked --example grouping -- "$pipesql_grouping_dir/memory" 2000000
cargo run --release --offline --locked --example grouping -- "$pipesql_grouping_dir/spill" 1200000
```

Both runs must print `verified 4096 groups: region=0..4095, n=2, total=4, smallest=1, largest=3`.
The program checks every ordered row and requires `Finished`; matching a prefix
does not pass. It also checks that query reservations return to their baseline
after dropping the result.

Read the second output line to compare sampled logical memory and temporary
bytes. With the reviewed macOS and GNU arm64 Linux builds, the first run uses no
temporary bytes and the second reaches 803,016 temporary bytes. These observations
are specific to this workload and build. A small budget alone does not establish spilling: the
2,000,000-byte run still fits its groups in memory. Temporary bytes measure
reserved scratch-file extents, not filesystem blocks, total I/O, or process RSS.
The example does not measure allocator-usable memory or performance.

Follow [the grouping execution path](execution.md#follow-the-grouping-example)
to see what changes between these runs. When finished, remove the owned inputs:

```sh
rm -r -- "$pipesql_grouping_dir"
```

## Follow a join through grouping and sorting

The [composed example](../examples/composed.rs) uses the same 4,096 regions, but
makes some amounts NULL and joins each row to every row with the same region:

```sql
FROM sales AS s
|> JOIN sales AS copies ON s.region = copies.region
|> AGGREGATE COUNT(*) AS n, COUNT(s.amount) AS present, SUM(s.amount) AS total
   GROUP BY s.region
|> ORDER BY region DESC
```

Each region has two source rows, so the self-join produces four pairs. Each
amount on the left occurs twice in the joined input. The example constructs
these three cases and checks every result, from region 4095 down to 0:

| Region remainder after division by 4 | Source amounts | Joined rows | Present amounts | Sum |
| --- | --- | ---: | ---: | ---: |
| 0 | NULL, NULL | 4 | 0 | NULL |
| 1 | NULL, 3 | 4 | 2 | 6 |
| 2 or 3 | 1, 3 | 4 | 4 | 8 |

Run both budgets with fresh database paths:

```sh
pipesql_composed_dir=$(mktemp -d)
cargo run --release --offline --locked --example composed -- "$pipesql_composed_dir/comfortable" 12000000
cargo run --release --offline --locked --example composed -- "$pipesql_composed_dir/constrained" 2200000
```

Both must print `verified 4096 descending groups: four joined pairs per key, nullable counts and sums`.
The next line reports sampled logical memory and temporary bytes. Unlike the
preceding grouping-only example, this query uses temporary storage even with
ample memory: its join and separate ORDER BY use sorted inputs. Total temporary
bytes alone cannot identify which operator spilled. The constrained run checks
that the composed query still completes with the same rows under a smaller
shared memory budget; it does not establish a performance improvement.

After checking successful completion and release, the program executes the
query again, waits until temporary storage is reserved, cancels it, and checks
that dropping the failed result restores the same reservation baseline.
Follow [the composed execution path](execution.md#follow-the-composed-example)
with the source open. When finished, remove the inputs:

```sh
rm -r -- "$pipesql_composed_dir"
```
