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

The [snapshot example](#keep-an-old-snapshot-readable) extends this lifecycle to
an old prepared query that remains readable after a newer append and reclamation.

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

To try column transformations, numeric expressions, set operations or partition
count, use [Explore queries on a declared table](query-examples.md). That guide
creates its own sample database, so you can begin it after cleaning up this one.

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

## Retain facts with missing dimensions

The [LEFT JOIN example](../examples/left_join.rs) creates `facts(region, amount)`
and `regions(id, name)`, appends their rows, and reopens the database. Run it from
the repository root with a fresh path:

```sh
pipesql_left_join_dir=$(mktemp -d)
cargo run --release --offline --locked --example left_join -- "$pipesql_left_join_dir/facts"
```

Successful completion prints:

```text
unmatched total=90 rows=2
north total=30 rows=2
south total=30 rows=1
```

The [query](../examples/left-join.sql) joins each fact to its region and then
groups by the region name. Region 3 has no dimension row, and the last fact has
a NULL region key. LEFT JOIN retains both facts with NULL right-side values;
their amounts, 40 and 50, form the NULL group. The printed word `unmatched` is
only the example's display label. The query result contains SQL NULL.

Change LEFT JOIN to JOIN to see those two facts disappear. A later
`WHERE r.name IS NULL` would retain only unmatched rows; placing a filter inside
the right input instead changes which dimension rows can match. See the
[execution trace](execution.md#equality-joins) for matching and duplicate replay.
To follow construction and failure cleanup, start at
[`Database::execute`](../src/execution/admission.rs), then the
[runtime admissions](../src/execution/runtime.rs) and
[join controller](../src/execution/blocking/join.rs). Sources open only after
operator admission. The runtime keeps the controller's inline charge until its
vector is physically freed, including when source opening fails before the first
result step. The [resource contract](resources.md#join-ordering-and-distinct-admission)
explains payload and temporary ownership.

The [default-region query](../examples/default-region.sql) uses
`COALESCE(r.id, 0)` to put unmatched facts in region 0. Run it against the same
database:

```sh
cargo run --release --offline --locked -- query --database "$pipesql_left_join_dir/facts" \
  --query-file examples/default-region.sql --memory-limit-bytes 8000000 --temp-limit-bytes 4000000
```

The query reports required INT64 `region`, nullable INT64 `total`, and required
INT64 `n`. Its rows are `(0, 90, 2)`, `(1, 30, 2)` and `(2, 30, 1)`, followed by
`row_count=3` and `status=queried`. COALESCE evaluates its fallback only when the
first value is NULL; the default is part of the SQL result.

The [missing-regions query](../examples/missing-regions.sql) compares complete
region values with the dimension identifiers:

```sh
cargo run --release --offline --locked -- query --database "$pipesql_left_join_dir/facts" \
  --query-file examples/missing-regions.sql --memory-limit-bytes 8000000 --temp-limit-bytes 4000000
```

It returns nullable INT64 `region` with rows `NULL` and `3`, followed by
`row_count=2` and `status=queried`. EXCEPT DISTINCT removes matching values and
returns each surviving value once. NULL survives here because the dimension
input contains no NULL identifier.

The [shared-regions query](../examples/shared-regions.sql) finds region values
present in both inputs:

```sh
cargo run --release --offline --locked -- query --database "$pipesql_left_join_dir/facts" \
  --query-file examples/shared-regions.sql --memory-limit-bytes 8000000 --temp-limit-bytes 4000000
```

It returns required INT64 `region` with rows `1` and `2`, followed by
`row_count=2` and `status=queried`. INTERSECT DISTINCT emits each shared value
once. Its output is required because the dimension identifier cannot be NULL.

## Exclude a sentinel from an aggregate

Suppose amount 20 marks an unavailable measurement in the same facts database.
Run [sentinel-amounts.sql](../examples/sentinel-amounts.sql):

```sh
cargo run --release --offline --locked -- query --database "$pipesql_left_join_dir/facts" \
  --query-file "$PWD/examples/sentinel-amounts.sql" \
  --memory-limit-bytes 8000000 --temp-limit-bytes 4000000
```

The single row is `(130, 4, 5)`: nullable INT64 `total`, required INT64 `measured`
and required INT64 `nrows`, followed by `row_count=1` and `status=queried`.
NULLIF returns NULL for the sentinel. SUM and COUNT of that expression skip it,
while COUNT(*) still counts all five facts. A WHERE filter would also remove the
row from COUNT(*).

Trace the [numeric evaluator](../src/scalar/evaluation.rs): NULLIF evaluates both
arguments in order, compares their common numeric values and retains the first
value unless equality is true. An outer COALESCE can supply a default. See the
[numeric contract](language.md#current-public-query-manifest) for coercion,
NULLs, NaNs and errors.

## Keep NULL rows when excluding a sentinel

To exclude region 3 and retain facts whose region is unknown, run
[null-safe-region.sql](../examples/null-safe-region.sql) on the same database:

```sh
cargo run --release --offline --locked -- query --database "$pipesql_left_join_dir/facts" \
  --query-file "$PWD/examples/null-safe-region.sql" \
  --memory-limit-bytes 8000000 --temp-limit-bytes 4000000
```

The single row is `(110, 4)`: nullable INT64 `total` and required INT64 `nrows`,
followed by `row_count=1` and `status=queried`. `IS DISTINCT FROM 3` is true for
NULL, so the unknown region's amount 50 remains. Replace it with `!=3` and the
result becomes `(60, 3)` because ordinary comparison produces UNKNOWN for NULL.

The [predicate decision](../src/execution/predicate.rs) handles NULL before
ordinary comparison, then applies the enclosing Boolean negation. It borrows the
same prepared literal and demands the same column as other filters. The
[language contract](language.md#current-public-query-manifest) specifies type
compatibility and the NaN distinction.

## Reconcile repeated facts

Use the same facts database. Region 1 occurs
twice in facts and once in regions. Run
[repeated-regions.sql](../examples/repeated-regions.sql):

```sh
cargo run --release --offline --locked -- query --database "$pipesql_left_join_dir/facts" \
  --query-file "$PWD/examples/repeated-regions.sql" \
  --memory-limit-bytes 8000000 --temp-limit-bytes 4000000
```

Require three rows: NULL, 1 and 3, followed by `status=queried` and successful
process exit. EXCEPT ALL subtracts one occurrence of each region in the right
input. Region 1 therefore retains one occurrence, while region 2 retains none.
EXCEPT DISTINCT would remove both occurrences of region 1. INTERSECT ALL instead
retains one occurrence per matched pair; with these inputs it returns 1 and 2.

Trace the [sorted-set merge](../src/execution/blocking/sorted_set.rs): it advances
both input cursors when rows match. The cursors already hold checked sorted rows,
so duplicate reconciliation needs no separate table of counts. The
[language contract](language.md#except-all-and-intersect-all) defines equivalence,
NULLability, representative bits and demanded errors.

Remove this example's database when finished:

```sh
rm -rf "$pipesql_left_join_dir"
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

## Keep an old snapshot readable

Run [examples/snapshots.rs](../examples/snapshots.rs) from the repository root:

```sh
pipesql_snapshot_dir=$(mktemp -d)
cargo run --release --offline --locked --example snapshots -- "$pipesql_snapshot_dir/sales"
```

The program requires one fresh absolute database path. It creates that database
and leaves it available afterward. Successful completion prints exactly:

```text
old before append: [10, 20]
old after reclaim: [10, 20]
new after reclaim: [10, 20, 30]
receipts after reclaim: durable generations=[2, 3], aborted=Aborted
new after old plan drops: [10, 20, 30]
receipts after old plan drops: durable generations=[2, 3], aborted=Aborted
reopened: [10, 20, 30]
receipts reopened: durable generations=[2, 3], aborted=Aborted
```

Follow the operations in `main`. The first append commits amounts 10 and 20.
`prepare` captures that catalog generation in `old`; finishing its first execution
releases the result buffers while the prepared plan keeps its snapshot pin.
The program then begins an empty append, retains its issued transaction identity,
and explicitly aborts it. That attempt resolves as `Aborted` without adding rows
or a data generation. Appending 30 publishes a newer generation. Preparing
`current` captures the new view, but executing `old` again still reads the
original two rows.

The first `reclaim` runs while both plans are pinned. `verify_rows` checks every
row against the literal expected values and requires successful completion.
`ORDER BY amount` makes their order explicit. Dropping `old` releases its pin;
the second reclamation can remove objects that neither retained data views nor
transaction history require. Its return value counts removed filenames, so the example does not
predict bytes freed or a fixed removal count. Finally, the program drops the
remaining plan, closes the database and verifies the latest rows after reopening.

`append_amounts` returns each successful `Commit`. `verify_outcomes` checks that
both receipts still resolve to those exact commits and that the aborted attempt
stays aborted after the later publication, both reclamations and reopening.
Declaration creates generation 1; the successful appends create generations 2
and 3. The aborted issuance does not consume a data generation. These copied
receipts survive closing the handle; prepared queries must be dropped first
because they borrow the database and retain its snapshot pins.

For the implementation, follow `Database::prepare` into
[prepare_catalog](../src/frontend/binding.rs), where the prepared query retains its catalog
snapshot. Then follow append publication in
[the transaction trace](transactions.md#follow-snapshot-pins-and-retained-outcomes) and reclamation in
[Reachable::open](../src/catalog_snapshot/reclaim.rs), which captures current and
pinned views before unlinking obsolete objects. The
[public contract](interfaces.md#reclaim-obsolete-catalog-objects) defines failure
and cleanup outcomes. This sequential
example demonstrates snapshot lifetime; the existing concurrency tests and
platform qualifications own broader claims.

Remove only the example directory when finished:

```sh
rm -r -- "$pipesql_snapshot_dir"
```
