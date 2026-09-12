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

## Transform columns while retaining the original values

Before removing the database, run [examples/extend.sql](../examples/extend.sql)
through the CLI:

```sh
cargo run --release --offline --locked --bin pipesql -- query \
  --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/extend.sql" \
  --memory-limit-bytes 16000000 --temp-limit-bytes 8000000
```

EXTEND keeps `region` and `amount` and appends `doubled`. SET replaces the ordinary
`amount` with that value. RENAME changes its name to `subtotal`, and DROP removes
the temporary `doubled` output. The final EXTEND computes `adjusted` from the
subtotal. Each expression list sees its complete input before publishing changes.

The range `s` still names the original columns, so `s.amount` returns the amount
before SET. SET assigns a new identity; RENAME preserves that identity. The NULL
amount produces NULL computed values, and the filter removes that row before
sorting.

The CLI reports schema, rows, and completion. Its decoded result is:

| region | amount | subtotal | adjusted |
| --- | ---: | ---: | ---: |
| north | 5 | 10 | 11 |
| north | 10 | 20 | 21 |
| south | 20 | 40 | 41 |

Require `status=queried` and a successful process exit before accepting the
result. To follow the implementation, read `bind_extend`, `bind_set`,
`bind_rename`, and `bind_drop` in the
[binder](../src/frontend/binding.rs), then read the [projection demand
rules](language.md#computed-projection-demand). These transformations share their
producer's execution controller; only demanded numeric definitions are evaluated.

## Combine pipeline results

Run [examples/union.sql](../examples/union.sql) against the same database:

```sh
cargo run --release --offline --locked --bin pipesql -- query \
  --database "$pipesql_example_dir/sales" \
  --query-file "$PWD/examples/union.sql" \
  --memory-limit-bytes 16000000 --temp-limit-bytes 8000000
```

The first branch selects north's sales. The second selects every sale of at least
10. UNION ALL keeps both copies of the north sale worth 10. It matches columns by
position: the second branch's `area` and `value` feed the first branch's `region`
and `amount`. Grouping therefore produces:

| region | total | n |
| --- | ---: | ---: |
| north | 25 | 4 |
| south | 20 | 1 |

The NULL amount contributes a row to COUNT(*) but no value to SUM. GROUP AND ORDER
BY establishes the displayed order; union itself establishes none. Require
`status=queried` and successful process exit before accepting the output.

Follow `bind_union` in the [binder](../src/frontend/binding.rs) to see how each
output position gets a fresh identity and two input mappings. The
[demand pass](../src/execution/planning/demand.rs) translates required outputs
into branch inputs. The [union consumer](../src/execution/union.rs) copies one
batch at a time while the [scheduler](../src/execution/runtime.rs) retains child
ownership. See the [union contract](language.md#union-all) for scope and errors.

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

Both runs must print `verified groups=4096 rows=8192 skewed=false`, followed by
the configured memory limit.
The program checks every ordered row and requires `Finished`; matching a prefix
does not pass. It also checks that query reservations return to their baseline
after dropping the result.

Read the second output line to compare sampled logical memory and temporary
bytes. With the reviewed macOS and GNU arm64 Linux builds, the first run uses no
temporary bytes and the second reaches 803,016 temporary bytes. These observations
are specific to this workload and build. A small budget alone does not establish spilling: the
2,000,000-byte run still fits its groups in memory. Temporary bytes measure
reserved scratch-file extents, not filesystem blocks, total I/O, or process RSS.
The third output line reports execution and validation seconds, including result
construction, all public query steps, row checking, and result destruction. It
excludes database creation, opening, preparation, and closing. The example does
not measure allocator-usable memory. One timing sample is not a performance
comparison; use repeated runs with identical inputs and record the toolchain,
platform, memory budget, and timing scope.

The optional group count and distribution keep the same 8,192 input rows while
changing cardinality and skew:

```sh
cargo run --release --offline --locked --example grouping -- "$pipesql_grouping_dir/few-skewed" 1200000 32 skewed
```

The first pass contributes 128 rows of amount 1 to each of 32 regions. The second
contributes all 4,096 rows of amount 3 to region zero. The example independently
checks every count, sum, minimum, and maximum, including maximum 1 in regions
that receive no second-pass rows. Omitting the optional arguments retains the
4,096-group even distribution.

Follow [the grouping execution path](execution.md#follow-the-grouping-example)
to see what changes between these runs. When finished, remove the owned inputs:

```sh
rm -r -- "$pipesql_grouping_dir"
```

## Measure STRING grouping costs

[examples/string_grouping.rs](../examples/string_grouping.rs) compares group
count, string width, and memory budget. Each key occurs twice: once with an
all-`a` string and once with an all-`z` string. The program checks every ordered
key, both extrema, count 2, successful completion, and resource release.

Run the eight combinations from the repository root:

```sh
pipesql_strings_dir=$(mktemp -d)
cargo build --release --offline --locked --example string_grouping
for groups in 4 256; do
  for width in 8 65536; do
    for memory in 4000000 80000000; do
      target/release/examples/string_grouping \
        "$pipesql_strings_dir/g${groups}-w${width}-m${memory}" \
        "$groups" "$width" "$memory"
    done
  done
done
```

Arguments are a fresh absolute database path, group count, string bytes, and
query memory bytes, followed by optional batch rows (default 1). Setup uses a
separate budget. The default keeps one row per input unit for both widths.
Timing excludes setup, open, and preparation; it includes execution,
full result validation, and destruction of the result owner.

To explore bulk append, add `4` for either width or `256` for eight-byte text:

```sh
target/release/examples/string_grouping \
  "$pipesql_strings_dir/bulk-short" 256 8 4000000 256
target/release/examples/string_grouping \
  "$pipesql_strings_dir/bulk-wide" 256 65536 16000000 4
```

Only batch sizes 1, 4, and 256 are accepted. A batch size of 256 requires
short text; invalid combinations fail before database creation. Each pass ends
with a smaller batch when necessary. Bulk append retains the rows and their order,
but changes native input units and therefore setup and query I/O costs. Compare
whole-process time separately from the printed execution/validation time; a
batching improvement does not measure a change to the hash implementation.

Short STRING extrema use compact hash storage. In the maintained workload,
256 groups of eight-byte strings remain in memory at 4 MB. Maximum-width strings
still require the spill path at 4 MB and remain in memory at 80 MB. Four groups
fit at either budget. Arena growth temporarily owns both old and new buffers,
so wider values can raise peak reservations during copying.

The [resource contract](resources.md#declared-grouping-admission) explains
admission and replacement. The [original measurements](../notes/evidence.md#string-grouping-costs)
record the fixed-slot baseline that motivated this representation change.

The counters report sampled logical reservations, not allocator-usable memory,
filesystem blocks, cumulative I/O, or RSS. Timings are workload observations,
not a benchmark ranking. The caller's two expected strings are outside the
database counters. After inspecting the results, remove the generated databases:

```sh
rm -r -- "$pipesql_strings_dir"
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
The next lines report sampled logical memory and temporary bytes, successful-query
step counts, and execution/validation time. That timer includes admission,
every checked result, completion, and result destruction. It excludes setup,
opening, preparation, the later cancellation exercise, and close. Whole-process time
includes those operations, so compare it separately.

`Progress` counts bounded work returns without output; `rows` counts returned
batches, not result rows. The [scheduler](execution.md#runtime-scheduling) performs
one producer quantum per call, including input requests and sorter work. A larger
count alone does not establish a scheduling defect or explain where time was
spent.

This query uses temporary storage even with ample memory: its join and separate
ORDER BY use sorted inputs. Total temporary
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
